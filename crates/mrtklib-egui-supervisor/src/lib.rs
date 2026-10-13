//! Child process lifetime, bounded output and the `mrtk run` readiness signal.

pub mod mask;

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Deserialize;

const READY_PREFIX: &[u8] = b"MRTK-RPC-READY ";
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Generate the protocol's 32 random bytes as unpadded base64url (43 characters).
/// Panics if the OS random source fails; no weaker random source is used.
pub fn generate_token() -> String {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("operating system random source failed");
    data_encoding::BASE64URL_NOPAD.encode(&bytes)
}

/// The same program and argv are used for preview and execution. Secrets belong
/// in the separate environment parameter of [`Supervisor::spawn`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl CommandLine {
    /// A masked display string, with double quotes and escapes for ambiguous words.
    /// This is not shell syntax; non-UTF-8 OS strings are decoded lossily for display.
    pub fn preview(&self) -> String {
        std::iter::once(self.program.as_os_str())
            .chain(self.args.iter().map(OsString::as_os_str))
            .map(|word| {
                let text = mask::mask(&word.to_string_lossy(), &[]);
                if !text.is_empty()
                    && text
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || "_./:-=@+*".contains(ch))
                {
                    text
                } else {
                    format!("{text:?}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Provisional bounds and startup timing.
#[derive(Clone, Debug)]
pub struct Options {
    /// Default 64 KiB, far above normal output; bounds pending bytes per pipe.
    /// An overlong display line is replaced entirely by a visible marker.
    pub max_line_bytes: usize,
    /// Default 1000 characters after masking, plus `…` when truncated: keeps a
    /// log-panel line readable. Revisit when the log panel is designed.
    pub max_display_chars: usize,
    /// Default 1000 retained output lines and supervisor errors each. Bounded
    /// history, as in the RPC session; revisit when the log panel is designed.
    pub max_lines: usize,
    /// Default 10 s: listening is reported immediately; allows a slow machine
    /// but reports a stuck start. Revisit with the real `mrtk run`.
    pub ready_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_line_bytes: 64 * 1024,
            max_display_chars: 1000,
            max_lines: 1000,
            ready_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    Waiting,
    Ready { url: String },
    Failed(String),
}

/// A copy for the UI; errors and output have already passed through masking.
#[derive(Clone, Debug)]
pub struct State {
    pub lines: VecDeque<OutputLine>,
    pub dropped_lines: u64,
    pub readiness: Readiness,
    pub exit_status: Option<ExitStatus>,
    pub errors: VecDeque<String>,
    pub dropped_errors: u64,
}

struct Inner {
    state: State,
    ready_seen: bool,
    readers_done: usize,
    stderr_done: bool,
    ended: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
    notify: Box<dyn Fn() + Send + Sync>,
    options: Options,
    secrets: Vec<String>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("supervisor state lock poisoned")
    }

    fn update(&self, change: impl FnOnce(&mut Inner)) {
        change(&mut self.lock());
        self.changed.notify_all();
        (self.notify)();
    }

    fn display(&self, text: &str) -> String {
        let secrets: Vec<_> = self.secrets.iter().map(String::as_str).collect();
        let masked = mask::mask(text, &secrets);
        let mut chars = masked.chars();
        let mut display: String = chars
            .by_ref()
            .take(self.options.max_display_chars)
            .collect();
        if chars.next().is_some() {
            display.push('…');
        }
        display
    }

    fn error(&self, inner: &mut Inner, text: &str) {
        inner.state.errors.push_back(self.display(text));
        while inner.state.errors.len() > self.options.max_lines {
            inner.state.errors.pop_front();
            inner.state.dropped_errors += 1;
        }
    }

    fn fail(&self, inner: &mut Inner, text: &str) {
        self.error(inner, text);
        if matches!(inner.state.readiness, Readiness::Waiting) {
            inner.state.readiness = Readiness::Failed(self.display(text));
        }
    }

    fn exit_before_ready(&self, inner: &mut Inner) {
        if inner.stderr_done
            && matches!(inner.state.readiness, Readiness::Waiting)
            && let Some(status) = inner.state.exit_status
        {
            self.fail(
                inner,
                &format!("process exited before the ready line: {status}"),
            );
        }
    }

    fn output(&self, stream: Stream, text: &str) {
        let text = self.display(text);
        self.update(|inner| {
            inner.state.lines.push_back(OutputLine { stream, text });
            while inner.state.lines.len() > self.options.max_lines {
                inner.state.lines.pop_front();
                inner.state.dropped_lines += 1;
            }
        });
    }

    fn ready(&self, bytes: Option<&[u8]>) {
        #[derive(Deserialize)]
        struct Ready {
            url: String,
        }
        self.update(|inner| {
            if inner.ready_seen {
                self.error(inner, "a second ready line was received");
                return;
            }
            inner.ready_seen = true;
            let result = bytes
                .ok_or("the ready line exceeded max_line_bytes")
                .and_then(|bytes| {
                    std::str::from_utf8(bytes).map_err(|_| "the ready line was not valid UTF-8")
                })
                .and_then(|text| {
                    serde_json::from_str::<Ready>(text)
                        .map_err(|_| "the ready line was not a JSON object with a string url")
                });
            match result {
                Ok(ready) if matches!(inner.state.readiness, Readiness::Waiting) => {
                    inner.state.readiness = Readiness::Ready { url: ready.url };
                }
                Ok(_) => {} // A late line cannot undo a reported startup failure.
                Err(reason) => self.fail(inner, reason),
            }
        });
    }
}

/// Owns one child. Reader threads update a snapshot; a monitor polls every 100 ms
/// so `kill` never waits behind a blocking `Child::wait`. Drop kills and joins.
pub struct Supervisor {
    shared: Arc<Shared>,
    child: Arc<Mutex<Option<Child>>>,
    threads: Vec<JoinHandle<()>>,
    readers: usize,
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Supervisor")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Start exactly this argv, with null stdin and two piped outputs. Secret
    /// environment values are also masked in all display output and errors.
    /// Spawn failures are in the state. `notify` runs outside the state lock;
    /// it should promptly request a repaint, as in the RPC session.
    pub fn spawn(
        command_line: CommandLine,
        secret_env: &[(String, String)],
        options: Options,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                state: State {
                    lines: VecDeque::new(),
                    dropped_lines: 0,
                    readiness: Readiness::Waiting,
                    exit_status: None,
                    errors: VecDeque::new(),
                    dropped_errors: 0,
                },
                ready_seen: false,
                readers_done: 0,
                stderr_done: false,
                ended: false,
            }),
            changed: Condvar::new(),
            notify: Box::new(notify),
            options,
            secrets: secret_env.iter().map(|(_, value)| value.clone()).collect(),
        });
        let mut supervisor = Self {
            shared,
            child: Arc::new(Mutex::new(None)),
            threads: Vec::new(),
            readers: 0,
        };
        let started = Instant::now();
        let result = Command::new(command_line.program)
            .args(command_line.args)
            .envs(secret_env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match result {
            Ok(child) => child,
            Err(error) => {
                supervisor.shared.update(|inner| {
                    inner.ended = true;
                    supervisor
                        .shared
                        .fail(inner, &format!("cannot start child: {error}"));
                });
                return supervisor;
            }
        };
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        *supervisor.child.lock().expect("child lock poisoned") = Some(child);
        let pipes: [(Stream, Box<dyn Read + Send>); 2] = [
            (Stream::Stdout, Box::new(stdout)),
            (Stream::Stderr, Box::new(stderr)),
        ];
        for (stream, pipe) in pipes {
            let shared = Arc::clone(&supervisor.shared);
            let result = thread::Builder::new()
                .name(format!("mrtk-{stream:?}"))
                .spawn(move || read_pipe(pipe, stream, &shared));
            if !supervisor.add_thread(result) {
                supervisor.reap();
                return supervisor;
            }
            supervisor.readers += 1;
        }
        let shared = Arc::clone(&supervisor.shared);
        let child = Arc::clone(&supervisor.child);
        let result = thread::Builder::new()
            .name("mrtk-process-monitor".into())
            .spawn(move || monitor(&child, &shared, started));
        if !supervisor.add_thread(result) {
            supervisor.reap();
        }
        supervisor
    }

    /// Copy the current state without waiting for readiness or exit.
    pub fn state(&self) -> State {
        self.shared.lock().state.clone()
    }

    /// Wait on a condition variable until startup succeeds or fails.
    pub fn wait_ready(&self) -> Result<String, String> {
        let mut inner = self.shared.lock();
        loop {
            match &inner.state.readiness {
                Readiness::Waiting => {
                    inner = self
                        .shared
                        .changed
                        .wait(inner)
                        .expect("state lock poisoned");
                }
                Readiness::Ready { url } => return Ok(url.clone()),
                Readiness::Failed(reason) => return Err(reason.clone()),
            }
        }
    }

    /// Force termination. The monitor reaps and records exit; failures are displayed.
    pub fn kill(&self) {
        let result = {
            let mut child = self.child.lock().expect("child lock poisoned");
            child.as_mut().map(Child::kill)
        };
        if let Some(Err(error)) = result {
            self.shared.update(|inner| {
                self.shared
                    .error(inner, &format!("cannot kill child: {error}"));
            });
        }
    }

    /// Wait for exit and drained output, or return false at the timeout. A spawn
    /// failure is already ended, even though it has no exit status.
    pub fn wait_exit(&self, timeout: Duration) -> bool {
        let inner = self.shared.lock();
        let (inner, _) = self
            .shared
            .changed
            .wait_timeout_while(inner, timeout, |inner| {
                !inner.ended || inner.readers_done < self.readers
            })
            .expect("state lock poisoned");
        inner.ended && inner.readers_done >= self.readers
    }

    /// Allow the caller's shutdown request to finish, then force termination and
    /// reap. This call also waits for all output to be drained.
    pub fn stop(&self, grace: Duration) {
        if !self.wait_exit(grace) {
            self.reap();
        }
        // Reaping itself need not await pipe readers; wait for their final updates.
        let mut inner = self.shared.lock();
        while !inner.ended || inner.readers_done < self.readers {
            inner = self
                .shared
                .changed
                .wait(inner)
                .expect("state lock poisoned");
        }
    }

    fn add_thread(&mut self, result: io::Result<JoinHandle<()>>) -> bool {
        match result {
            Ok(thread) => {
                self.threads.push(thread);
                true
            }
            Err(error) => {
                self.shared.update(|inner| {
                    self.shared
                        .fail(inner, &format!("cannot start supervisor thread: {error}"));
                });
                false
            }
        }
    }

    fn reap(&self) {
        self.kill();
        // Poll with the child lock released between calls, just as the monitor does.
        loop {
            if self.shared.lock().ended {
                break;
            }
            if poll_child(&self.child, &self.shared) {
                break;
            }
            let inner = self.shared.lock();
            drop(
                self.shared
                    .changed
                    .wait_timeout(inner, POLL_INTERVAL)
                    .expect("state lock poisoned"),
            );
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if !self.shared.lock().ended {
            self.reap();
        }
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                self.shared
                    .update(|inner| self.shared.error(inner, "a supervisor thread panicked"));
            }
        }
    }
}

fn poll_child(child: &Mutex<Option<Child>>, shared: &Shared) -> bool {
    let result = {
        let mut child = child.lock().expect("child lock poisoned");
        match child.as_mut() {
            Some(child) => child.try_wait(),
            None => return true,
        }
    };
    match result {
        Ok(Some(status)) => {
            shared.update(|inner| {
                inner.state.exit_status = Some(status);
                inner.ended = true;
                shared.exit_before_ready(inner);
            });
            true
        }
        Ok(None) => false,
        Err(error) => {
            shared.update(|inner| shared.fail(inner, &format!("cannot poll child exit: {error}")));
            false
        }
    }
}

fn monitor(child: &Mutex<Option<Child>>, shared: &Shared, started: Instant) {
    loop {
        if shared.lock().ended || poll_child(child, shared) {
            return;
        }
        let mut inner = shared.lock();
        if matches!(inner.state.readiness, Readiness::Waiting)
            && started.elapsed() >= shared.options.ready_timeout
        {
            shared.fail(&mut inner, "the ready timeout passed");
            drop(inner);
            shared.changed.notify_all();
            (shared.notify)();
            inner = shared.lock();
        }
        let wait = if matches!(inner.state.readiness, Readiness::Waiting) {
            POLL_INTERVAL.min(
                shared
                    .options
                    .ready_timeout
                    .saturating_sub(started.elapsed()),
            )
        } else {
            POLL_INTERVAL
        };
        drop(
            shared
                .changed
                .wait_timeout_while(inner, wait, |inner| !inner.ended)
                .expect("state lock poisoned"),
        );
    }
}

fn read_pipe(mut pipe: Box<dyn Read + Send>, stream: Stream, shared: &Shared) {
    let mut line = Line::default();
    let mut buffer = [0; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                for &byte in &buffer[..length] {
                    if byte == b'\n' {
                        line.finish(stream, shared);
                        line = Line::default();
                    } else {
                        line.push(byte, stream, shared);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                shared.update(|inner| {
                    shared.fail(
                        inner,
                        &format!(
                            "cannot read {stream:?}: {error}; an incomplete line was not shown"
                        ),
                    )
                });
                // A read failure can cut a credential in the middle, just as a
                // byte limit can. Do not display the incomplete pending line.
                line = Line::default();
                break;
            }
        }
    }
    if !line.bytes.is_empty() || line.overlong {
        line.finish(stream, shared);
    }
    shared.update(|inner| {
        inner.readers_done += 1;
        if stream == Stream::Stderr {
            inner.stderr_done = true;
        }
        shared.exit_before_ready(inner);
    });
}

struct Line {
    bytes: Vec<u8>,
    prefix_bytes: usize,
    prefix_matches: bool,
    overlong: bool,
    reported: bool,
}

impl Default for Line {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            prefix_bytes: 0,
            prefix_matches: true,
            overlong: false,
            reported: false,
        }
    }
}

impl Line {
    fn is_ready(&self, stream: Stream) -> bool {
        stream == Stream::Stderr && self.prefix_matches && self.prefix_bytes == READY_PREFIX.len()
    }

    fn push(&mut self, byte: u8, stream: Stream, shared: &Shared) {
        if self.prefix_bytes < READY_PREFIX.len() {
            self.prefix_matches &= byte == READY_PREFIX[self.prefix_bytes];
            self.prefix_bytes += 1;
        }
        if !self.overlong {
            if self.bytes.len() < shared.options.max_line_bytes {
                self.bytes.push(byte);
            } else {
                self.bytes.clear();
                self.overlong = true;
            }
        }
        if self.overlong
            && (!self.prefix_matches
                || self.prefix_bytes == READY_PREFIX.len()
                || stream == Stream::Stdout)
        {
            self.report_overlong(stream, shared);
        }
    }

    fn report_overlong(&mut self, stream: Stream, shared: &Shared) {
        if self.reported {
            return;
        }
        self.reported = true;
        if self.is_ready(stream) {
            shared.ready(None);
        } else {
            shared.output(
                stream,
                &format!(
                    "(a line longer than {} bytes was not shown)",
                    shared.options.max_line_bytes
                ),
            );
        }
    }

    fn finish(&mut self, stream: Stream, shared: &Shared) {
        if self.overlong {
            self.report_overlong(stream, shared);
            return;
        }
        if self.bytes.last() == Some(&b'\r') {
            self.bytes.pop();
        }
        if self.is_ready(stream) {
            shared.ready(Some(&self.bytes[READY_PREFIX.len()..]));
        } else {
            shared.output(stream, &String::from_utf8_lossy(&self.bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_supervisor(notify: impl Fn() + Send + Sync + 'static) -> Supervisor {
        Supervisor::spawn(
            CommandLine {
                program: PathBuf::new(),
                args: vec![],
            },
            &[("MRTK_RPC_TOKEN".into(), "unit-secret".into())],
            Options::default(),
            notify,
        )
    }

    #[test]
    fn notify_can_read_shared_state() {
        let holder = Arc::new(Mutex::new(None::<std::sync::Weak<Shared>>));
        let callback_holder = Arc::clone(&holder);
        let supervisor = empty_supervisor(move || {
            if let Some(shared) = callback_holder
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|weak| weak.upgrade())
            {
                assert!(
                    shared.inner.try_lock().is_ok(),
                    "notify held the state lock"
                );
            }
        });
        *holder.lock().unwrap() = Some(Arc::downgrade(&supervisor.shared));
        supervisor.shared.output(Stream::Stdout, "hello");
    }

    #[test]
    fn read_errors_are_masked_and_do_not_display_partial_credentials() {
        struct Broken {
            first: bool,
        }
        impl Read for Broken {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                if self.first {
                    self.first = false;
                    let partial = b"user:partial-password";
                    bytes[..partial.len()].copy_from_slice(partial);
                    Ok(partial.len())
                } else {
                    Err(io::Error::other("unit-secret"))
                }
            }
        }
        let supervisor = empty_supervisor(|| {});
        read_pipe(
            Box::new(Broken { first: true }),
            Stream::Stderr,
            &supervisor.shared,
        );
        let state = supervisor.state();
        assert!(state.lines.is_empty());
        assert!(
            state
                .errors
                .back()
                .unwrap()
                .contains("cannot read Stderr: ***")
        );
        assert!(!format!("{state:?}").contains("unit-secret"));
    }

    #[test]
    fn token_format_and_randomness() {
        let first = generate_token();
        assert_eq!(first.len(), 43);
        assert!(
            first
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
        );
        assert_eq!(
            data_encoding::BASE64URL_NOPAD
                .decode(first.as_bytes())
                .unwrap()
                .len(),
            32
        );
        assert_ne!(first, generate_token());
    }

    #[test]
    fn preview_quotes_and_masks_without_changing_argv() {
        let command = CommandLine {
            program: "a program/mrtk".into(),
            args: ["run", "", "a\"b\\c\n", "ntrip://user:pass@host/MNT"]
                .map(OsString::from)
                .into(),
        };
        assert_eq!(
            command.preview(),
            "\"a program/mrtk\" run \"\" \"a\\\"b\\\\c\\n\" ntrip://***@host/MNT"
        );
        assert_eq!(command.args[3], "ntrip://user:pass@host/MNT");
    }
}
