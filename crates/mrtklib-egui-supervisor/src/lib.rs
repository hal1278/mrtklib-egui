//! Child process lifetime, bounded output and the `mrtk run` readiness signal.

pub mod mask;

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
#[derive(Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub working_directory: Option<PathBuf>,
}

impl fmt::Debug for CommandLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandLine")
            .field("preview", &self.preview())
            .finish()
    }
}

impl CommandLine {
    /// A masked display string, with double quotes and escapes for ambiguous words.
    /// This is not shell syntax; non-UTF-8 OS strings are decoded lossily for display.
    pub fn preview(&self) -> String {
        let argv = std::iter::once(self.program.as_os_str())
            .chain(self.args.iter().map(OsString::as_os_str))
            .map(|word| quote(mask::mask(&word.to_string_lossy(), &[])))
            .collect::<Vec<_>>()
            .join(" ");
        match &self.working_directory {
            Some(directory) => format!(
                "Working directory: {} | Program: {argv}",
                quote_always(mask::mask(&directory.to_string_lossy(), &[]))
            ),
            None => argv,
        }
    }
}

/// Display quoting for the preview, which is never executed. Windows paths keep single
/// backslashes; elsewhere Rust's escaped quoting makes control characters visible.
fn quote(text: String) -> String {
    let plain = |ch: char| {
        ch.is_ascii_alphanumeric() || "_./:-=@+*".contains(ch) || (cfg!(windows) && ch == '\\')
    };
    if !text.is_empty() && text.chars().all(plain) {
        text
    } else {
        quote_always(text)
    }
}
fn quote_always(text: String) -> String {
    if cfg!(windows) && !text.chars().any(char::is_control) {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        format!("{text:?}")
    }
}

/// Child stdin ownership. Held-open pipes are never written to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stdin {
    #[default]
    Null,
    HeldOpen,
}

/// Whether this process reports the `mrtk run` readiness line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadinessMode {
    #[default]
    ReadyLine,
    None,
}

/// Bounds and startup timing.
#[derive(Clone, Debug)]
pub struct Options {
    pub readiness: ReadinessMode,
    pub stdin: Stdin,
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
    /// but reports a stuck start. Real Linux replay readiness measured 23–33 ms.
    pub ready_timeout: Duration,
    /// Default 1 s after reaping: the child's output is already in the pipe and
    /// drains within milliseconds. This bound matters when a descendant holds
    /// the pipe open and keeps GUI shutdown responsive; readers then detach.
    pub drain_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            readiness: ReadinessMode::ReadyLine,
            stdin: Stdin::Null,
            max_line_bytes: 64 * 1024,
            max_display_chars: 1000,
            max_lines: 1000,
            ready_timeout: Duration::from_secs(10),
            drain_timeout: Duration::from_secs(1),
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
    NotRequired,
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
    readers_expected: usize,
    stderr_done: bool,
    ended: bool,
    exited_at: Option<Instant>,
    drain_expired: bool,
}

impl Inner {
    fn awaiting_output(&self) -> bool {
        self.readers_done < self.readers_expected && !self.drain_expired
    }
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
        if matches!(
            inner.state.readiness,
            Readiness::Waiting | Readiness::NotRequired
        ) {
            inner.state.readiness = Readiness::Failed(self.display(text));
        }
    }

    fn exit_before_ready(&self, inner: &mut Inner) {
        if (inner.stderr_done || inner.drain_expired)
            && matches!(inner.state.readiness, Readiness::Waiting)
            && let Some(status) = inner.state.exit_status
        {
            self.fail(
                inner,
                &format!("process exited before the ready line: {status}"),
            );
        }
    }

    /// A single deadline shared by the monitor, stop and drop; never restart it.
    fn expire_drain(&self, inner: &mut Inner) -> bool {
        if inner.awaiting_output()
            && inner
                .exited_at
                .is_some_and(|at| at.elapsed() >= self.options.drain_timeout)
        {
            inner.drain_expired = true;
            self.error(
                inner,
                "the output drain timeout passed; output is no longer awaited",
            );
            self.exit_before_ready(inner);
            true
        } else {
            false
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
                    serde_json::from_str::<serde_json::Value>(text)
                        .ok()
                        .and_then(|value| {
                            value.as_object()?.get("url")?.as_str().map(str::to_owned)
                        })
                        .ok_or("the ready line was not a JSON object with a string url")
                });
            match result {
                Ok(url) if matches!(inner.state.readiness, Readiness::Waiting) => {
                    inner.state.readiness = Readiness::Ready { url };
                }
                Ok(_) => {} // A late line cannot undo a reported startup failure.
                Err(reason) => self.fail(inner, reason),
            }
        });
    }
}

/// Owns one child. Reader threads update a snapshot; a monitor polls every 100 ms
/// so `kill` never waits behind a blocking `Child::wait`. Drop kills, reaps and
/// drains for a bounded time; readers detach when a descendant holds a pipe.
pub struct Supervisor {
    shared: Arc<Shared>,
    child: Arc<Mutex<Option<Child>>>,
    stdin: Mutex<Option<ChildStdin>>,
    reader_threads: Mutex<Vec<JoinHandle<()>>>,
    monitor: Option<JoinHandle<()>>,
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Supervisor")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Start exactly this argv, with the selected stdin and two piped outputs. Secret
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
                    readiness: match options.readiness {
                        ReadinessMode::ReadyLine => Readiness::Waiting,
                        ReadinessMode::None => Readiness::NotRequired,
                    },
                    exit_status: None,
                    errors: VecDeque::new(),
                    dropped_errors: 0,
                },
                ready_seen: false,
                readers_done: 0,
                readers_expected: 0,
                stderr_done: false,
                ended: false,
                exited_at: None,
                drain_expired: false,
            }),
            changed: Condvar::new(),
            notify: Box::new(notify),
            options,
            secrets: secret_env.iter().map(|(_, value)| value.clone()).collect(),
        });
        let mut supervisor = Self {
            shared,
            child: Arc::new(Mutex::new(None)),
            reader_threads: Mutex::new(Vec::new()),
            monitor: None,
            stdin: Mutex::new(None),
        };
        let started = Instant::now();
        let mut command = Command::new(command_line.program);
        command.args(command_line.args);
        if let Some(directory) = command_line.working_directory {
            command.current_dir(directory);
        }
        let result = command
            .envs(secret_env.iter().map(|(name, value)| (name, value)))
            .stdin(match supervisor.shared.options.stdin {
                Stdin::Null => Stdio::null(),
                Stdin::HeldOpen => Stdio::piped(),
            })
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
        *supervisor.stdin.lock().expect("stdin lock poisoned") = child.stdin.take();
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
            match result {
                Ok(thread) => {
                    supervisor
                        .reader_threads
                        .lock()
                        .expect("reader handles lock poisoned")
                        .push(thread);
                    supervisor.shared.lock().readers_expected += 1;
                }
                Err(error) => {
                    supervisor.thread_error(error);
                    supervisor.reap();
                    return supervisor;
                }
            }
        }
        let shared = Arc::clone(&supervisor.shared);
        let child = Arc::clone(&supervisor.child);
        let result = thread::Builder::new()
            .name("mrtk-process-monitor".into())
            .spawn(move || monitor(&child, &shared, started));
        match result {
            Ok(thread) => supervisor.monitor = Some(thread),
            Err(error) => {
                supervisor.thread_error(error);
                supervisor.reap();
            }
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
                Readiness::NotRequired => return Err("this process has no readiness line".into()),
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

    /// Wait for exit and output draining (or its bounded timeout), or return false
    /// at this call's timeout. A spawn failure is already ended without a status.
    pub fn wait_exit(&self, timeout: Duration) -> bool {
        let inner = self.shared.lock();
        let (inner, _) = self
            .shared
            .changed
            .wait_timeout_while(inner, timeout, |inner| {
                !inner.ended || inner.awaiting_output()
            })
            .expect("state lock poisoned");
        inner.ended && !inner.awaiting_output()
    }

    /// Close held stdin, allow normal shutdown to finish, then force termination and
    /// reap. After reaping, await output for at most the remaining drain timeout,
    /// then detach readers; descendants may still hold the pipes open.
    pub fn stop(&self, grace: Duration) {
        // Separate from the child lock: closing must never queue behind child polling.
        self.stdin.lock().expect("stdin lock poisoned").take();
        let inner = self.shared.lock();
        let (inner, _) = self
            .shared
            .changed
            .wait_timeout_while(inner, grace, |inner| !inner.ended)
            .expect("state lock poisoned");
        let ended = inner.ended;
        drop(inner);
        if !ended {
            self.reap();
        }
        self.drain_output();
        self.reader_threads
            .lock()
            .expect("reader handles lock poisoned")
            .clear();
    }

    fn thread_error(&self, error: io::Error) {
        self.shared.update(|inner| {
            self.shared
                .fail(inner, &format!("cannot start supervisor thread: {error}"));
        });
    }

    fn drain_output(&self) {
        loop {
            let mut inner = self.shared.lock();
            if !inner.awaiting_output() {
                return;
            }
            if self.shared.expire_drain(&mut inner) {
                drop(inner);
                self.shared.changed.notify_all();
                (self.shared.notify)();
                return;
            }
            let wait = self.shared.options.drain_timeout.saturating_sub(
                inner
                    .exited_at
                    .expect("child reaped before draining")
                    .elapsed(),
            );
            drop(
                self.shared
                    .changed
                    .wait_timeout_while(inner, wait, |inner| inner.awaiting_output())
                    .expect("state lock poisoned"),
            );
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
        self.stdin.get_mut().expect("stdin lock poisoned").take();
        if !self.shared.lock().ended {
            self.reap();
        }
        self.drain_output();
        self.reader_threads
            .get_mut()
            .expect("reader handles lock poisoned")
            .clear();
        // Only the monitor has a guaranteed end; inherited pipes can keep readers
        // blocked indefinitely. Dropping their handles detaches them safely.
        if let Some(thread) = self.monitor.take()
            && thread.join().is_err()
        {
            self.shared
                .update(|inner| self.shared.error(inner, "the supervisor monitor panicked"));
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
                if !inner.ended {
                    inner.state.exit_status = Some(status);
                    inner.exited_at = Some(Instant::now());
                    inner.ended = true;
                    shared.exit_before_ready(inner);
                }
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
        if !shared.lock().ended {
            poll_child(child, shared);
        }
        let mut inner = shared.lock();
        let mut changed = false;
        if matches!(inner.state.readiness, Readiness::Waiting)
            && started.elapsed() >= shared.options.ready_timeout
        {
            shared.fail(&mut inner, "the ready timeout passed");
            changed = true;
        }
        changed |= shared.expire_drain(&mut inner);
        let finished = inner.ended && !inner.awaiting_output();
        if changed {
            drop(inner);
            shared.changed.notify_all();
            (shared.notify)();
            inner = shared.lock();
        }
        if finished {
            return;
        }
        let mut wait = if matches!(inner.state.readiness, Readiness::Waiting) {
            POLL_INTERVAL.min(
                shared
                    .options
                    .ready_timeout
                    .saturating_sub(started.elapsed()),
            )
        } else {
            POLL_INTERVAL
        };
        if let Some(at) = inner.exited_at {
            wait = wait.min(shared.options.drain_timeout.saturating_sub(at.elapsed()));
        }
        let ended = inner.ended;
        drop(
            shared
                .changed
                .wait_timeout_while(inner, wait, |inner| {
                    if ended {
                        inner.awaiting_output()
                    } else {
                        !inner.ended
                    }
                })
                .expect("state lock poisoned"),
        );
    }
}

fn read_pipe(mut pipe: Box<dyn Read + Send>, stream: Stream, shared: &Shared) {
    let mut line = Line::default();
    let mut buffer = [0; 8192];
    let mut after_cr = false;
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                for &byte in &buffer[..length] {
                    if after_cr && byte == b'\n' {
                        after_cr = false;
                        continue;
                    }
                    after_cr = false;
                    // Machine-readable lines retain their LF framing, including
                    // embedded CR bytes. Only display lines use CR as a delimiter.
                    if byte == b'\n' || (byte == b'\r' && !line.is_ready(stream, shared)) {
                        line.finish(stream, shared);
                        line = Line::default();
                        after_cr = byte == b'\r';
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
    fn is_ready(&self, stream: Stream, shared: &Shared) -> bool {
        shared.options.readiness == ReadinessMode::ReadyLine
            && stream == Stream::Stderr
            && self.prefix_matches
            && self.prefix_bytes == READY_PREFIX.len()
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
        if self.is_ready(stream, shared) {
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
        if self.is_ready(stream, shared) {
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
                working_directory: None,
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
    fn display_cr_lf_crlf_and_limits_work_across_read_boundaries() {
        struct Chunks {
            bytes: std::io::Cursor<Vec<u8>>,
            size: usize,
        }
        impl Read for Chunks {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let count = bytes.len().min(self.size);
                self.bytes.read(&mut bytes[..count])
            }
        }
        for size in [1, 2, 7, 8192] {
            let supervisor = Supervisor::spawn(
                CommandLine {
                    program: PathBuf::new(),
                    args: vec![],
                    working_directory: None,
                },
                &[],
                Options {
                    readiness: ReadinessMode::None,
                    max_line_bytes: 32,
                    ..Options::default()
                },
                || {},
            );
            let bytes = format!(
                "first\rsecond\r\nthird\n{}\rafter long\ruser:pass@host\rfinal",
                "x".repeat(64)
            )
            .into_bytes();
            read_pipe(
                Box::new(Chunks {
                    bytes: std::io::Cursor::new(bytes),
                    size,
                }),
                Stream::Stderr,
                &supervisor.shared,
            );
            assert_eq!(
                supervisor
                    .state()
                    .lines
                    .iter()
                    .map(|line| line.text.clone())
                    .collect::<Vec<_>>(),
                [
                    "first",
                    "second",
                    "third",
                    "(a line longer than 32 bytes was not shown)",
                    "after long",
                    "***@host",
                    "final"
                ],
                "chunk size {size}"
            );
        }
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
            working_directory: None,
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
        let debug = format!("{command:?}");
        assert!(debug.contains("ntrip://***@host/MNT"));
        assert!(!debug.contains("user:pass"));
    }
}
