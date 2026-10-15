//! Portable fake children re-execute this test binary; no shell or helper binary.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{self, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use mrtklib_egui_rpc::Client;
use mrtklib_egui_supervisor::{CommandLine, Options, Readiness, State, Stream, Supervisor};

const TOKEN: &str = "test-supervisor_token-0123456789";
const URL: &str = "ws://127.0.0.1:43215/rpc";
const TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

fn command() -> CommandLine {
    CommandLine {
        working_directory: None,
        program: std::env::current_exe().unwrap(),
        args: ["--exact", "child", "--nocapture"]
            .map(OsString::from)
            .into(),
    }
}

fn env(behavior: &str) -> Vec<(String, String)> {
    vec![
        (
            "SUPERVISOR_TEST_CHILD".into(),
            format!("supervisor-child-behavior:{behavior}"),
        ),
        ("MRTK_RPC_TOKEN".into(), TOKEN.into()),
    ]
}

fn spawn(behavior: &str, options: Options) -> Supervisor {
    Supervisor::spawn(command(), &env(behavior), options, || {})
}

fn exited(supervisor: &Supervisor) -> State {
    assert!(supervisor.wait_exit(TIMEOUT), "{:?}", supervisor.state());
    let state = supervisor.state();
    assert!(state.exit_status.is_some());
    assert!(!format!("{state:?}").contains(TOKEN));
    assert!(!format!("{supervisor:?}").contains(TOKEN));
    state
}

fn ready() {
    eprintln!("MRTK-RPC-READY {{\"url\":\"{URL}\"}}");
}

fn idle() -> ! {
    loop {
        thread::park();
    }
}

struct Grandchild {
    socket: Option<TcpStream>,
}

impl Grandchild {
    fn cleanup(&mut self) {
        let mut socket = self.socket.take().unwrap();
        socket.write_all(&[1]).unwrap();
        assert_eq!(socket.read(&mut [0]).unwrap(), 0, "grandchild did not exit");
    }
}

impl Drop for Grandchild {
    fn drop(&mut self) {
        if let Some(mut socket) = self.socket.take() {
            // Best effort during assertion unwinding; the grandchild also has a
            // five-second lifetime bound, so test failure cannot leave it running.
            let _ = socket.write_all(&[1]);
        }
    }
}

fn inherited_pipes(
    behavior: &str,
    options: Options,
) -> (Supervisor, Grandchild, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut variables = env(behavior);
    variables.push((
        "SUPERVISOR_TEST_ADDRESS".into(),
        listener.local_addr().unwrap().to_string(),
    ));
    let (sender, notifications) = mpsc::channel();
    let process = Supervisor::spawn(command(), &variables, options, move || {
        // The receiver can disappear before a detached reader's final notification.
        let _ = sender.send(());
    });
    let started = Instant::now();
    let mut socket = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(started.elapsed() < TIMEOUT, "grandchild did not connect");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("cannot accept grandchild: {error}"),
        }
    };
    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
    let mut started_byte = [0];
    socket.read_exact(&mut started_byte).unwrap();
    assert_eq!(started_byte, [1]);
    (
        process,
        Grandchild {
            socket: Some(socket),
        },
        notifications,
    )
}

fn until_state(
    process: &Supervisor,
    notifications: &mpsc::Receiver<()>,
    done: impl Fn(&State) -> bool,
) {
    let started = Instant::now();
    while !done(&process.state()) {
        notifications
            .recv_timeout(TIMEOUT.saturating_sub(started.elapsed()))
            .unwrap();
    }
}

#[test]
fn child() {
    let Ok(behavior) = std::env::var("SUPERVISOR_TEST_CHILD") else {
        return;
    };
    match behavior.strip_prefix("supervisor-child-behavior:").unwrap() {
        "stdin-eof" => {
            ready();
            assert_eq!(std::io::stdin().read(&mut [0]).unwrap(), 0);
            println!("cwd={}", std::env::current_dir().unwrap().display());
        }
        "ready" => {
            ready();
            idle();
        }
        "unknown" => {
            eprintln!("MRTK-RPC-READY {{\"url\":\"{URL}\",\"extra\":{{\"nested\":true}}}}");
        }
        "exit" => process::exit(3),
        "never" => idle(),
        "malformed" => eprintln!("MRTK-RPC-READY broken-{TOKEN}"),
        "missing-url" => eprintln!("MRTK-RPC-READY {{\"other\":true}}"),
        "non-string-url" => eprintln!("MRTK-RPC-READY {{\"url\":12}}"),
        "array-ready" => eprintln!("MRTK-RPC-READY [\"{URL}\"]"),
        "duplicate" => {
            ready();
            ready();
        }
        "many-ready" => {
            for _ in 0..5 {
                ready();
            }
        }
        "mask" => {
            ready();
            println!(
                "env-matches={}",
                std::env::var("MRTK_RPC_TOKEN").unwrap() == TOKEN
            );
            println!("stdout user:pass@host prefix{TOKEN}suffix");
            eprintln!("stderr user:pass@host prefix{TOKEN}suffix");
            println!("truncate {} user:pass@host", "界".repeat(10));
        }
        "overlong" => {
            ready();
            for mut pipe in [
                Box::new(std::io::stdout()) as Box<dyn Write>,
                Box::new(std::io::stderr()),
            ] {
                writeln!(pipe, "{}user:pass@host{TOKEN}", "x".repeat(1000)).unwrap();
                writeln!(pipe, "normal\r").unwrap();
            }
        }
        "many" => {
            ready();
            for index in 0..20 {
                println!("line {index}");
            }
        }
        "bytes" => {
            ready();
            std::io::stdout()
                .write_all(b"invalid \xff\xfe\r\ncarriage\r\r\nlast-no-newline")
                .unwrap();
        }
        "invalid-ready" => {
            std::io::stderr()
                .write_all(b"MRTK-RPC-READY {\"url\":\"\xff\"}\n")
                .unwrap();
        }
        "overlong-ready" => {
            eprintln!(
                "MRTK-RPC-READY {{\"url\":\"{URL}\",\"extra\":\"{}\"}}",
                "x".repeat(1000)
            );
        }
        "stdout-ready" => println!("MRTK-RPC-READY {{\"url\":\"{URL}\"}}"),
        "prefix-no-space" => eprintln!("MRTK-RPC-READY{{\"url\":\"{URL}\"}}"),
        "tiny-line" => {
            eprintln!("MRTK-RPC-READY {{\"url\":\"{URL}\"}}");
        }
        "drop" => {
            let socket =
                TcpStream::connect(std::env::var("SUPERVISOR_TEST_ADDRESS").unwrap()).unwrap();
            ready();
            // Keep the socket open so the parent observes EOF only after termination.
            std::hint::black_box(&socket);
            idle();
        }
        "grandchild" => {
            let mut socket =
                TcpStream::connect(std::env::var("SUPERVISOR_TEST_ADDRESS").unwrap()).unwrap();
            socket.set_read_timeout(Some(TIMEOUT)).unwrap();
            socket.write_all(&[1]).unwrap();
            // Hold both inherited pipes until cleanup, with a bounded fallback
            // lifetime if the parent test fails before sending its cleanup byte.
            let _ = socket.read(&mut [0]);
            eprintln!("grandchild cleaned up");
            std::io::stdout().flush().unwrap();
            std::io::stderr().flush().unwrap();
            // Keep the socket until process exit: EOF confirms OS cleanup, rather
            // than just the socket going out of scope before the process ends.
            process::exit(0);
        }
        "inherited-exit" | "inherited-ready" | "inherited-kill" => {
            let command = command();
            let grandchild = process::Command::new(command.program)
                .args(command.args)
                .env(
                    "SUPERVISOR_TEST_CHILD",
                    "supervisor-child-behavior:grandchild",
                )
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            // This child deliberately leaves its descendant for the test to
            // clean up, modelling programs launched by mrtk with inherited pipes.
            std::hint::black_box(&grandchild);
            if behavior.ends_with("inherited-ready") || behavior.ends_with("inherited-kill") {
                ready();
            }
            if behavior.ends_with("inherited-kill") {
                idle();
            }
            process::exit(3);
        }
        "mock" => {
            let server = mrtklib_egui_mock::start(mrtklib_egui_mock::Config {
                token: Some(std::env::var("MRTK_RPC_TOKEN").unwrap()),
                ..Default::default()
            })
            .unwrap();
            eprintln!("MRTK-RPC-READY {{\"url\":\"{}\"}}", server.url());
            server.wait();
        }
        _ => panic!("unknown child behavior"),
    }
    // Avoid libtest's final status text interfering with output assertions.
    std::io::stdout().flush().unwrap();
    std::io::stderr().flush().unwrap();
    process::exit(0);
}

#[test]
fn ready_notify_and_kill() {
    let notifications = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&notifications);
    let process = Supervisor::spawn(command(), &env("ready"), Options::default(), move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(process.wait_ready().unwrap(), URL);
    assert!(!process.wait_exit(Duration::ZERO));
    assert!(
        process
            .state()
            .lines
            .iter()
            .all(|line| !line.text.contains("MRTK-RPC-READY"))
    );
    process.kill();
    let state = exited(&process);
    assert!(!state.exit_status.unwrap().success());
    assert!(notifications.load(Ordering::SeqCst) > 0);
}

#[test]
fn unknown_members_and_ready_just_before_exit() {
    let process = spawn("unknown", Options::default());
    assert_eq!(process.wait_ready().unwrap(), URL);
    assert!(exited(&process).errors.is_empty());
}

#[test]
fn exit_before_ready_records_status() {
    let process = spawn("exit", Options::default());
    let error = process.wait_ready().unwrap_err();
    assert!(error.contains("before the ready line") && error.contains('3'));
    assert_eq!(exited(&process).exit_status.unwrap().code(), Some(3));
}

#[test]
fn timeout_and_forced_stop_after_grace() {
    let process = spawn(
        "never",
        Options {
            ready_timeout: Duration::from_millis(40),
            ..Default::default()
        },
    );
    assert_eq!(
        process.wait_ready().unwrap_err(),
        "the ready timeout passed"
    );
    let start = Instant::now();
    process.stop(Duration::from_millis(30));
    assert!(start.elapsed() >= Duration::from_millis(30));
    assert!(start.elapsed() < TIMEOUT);
    assert!(matches!(exited(&process).readiness, Readiness::Failed(_)));
}

#[test]
fn malformed_ready_errors_do_not_include_contents() {
    for behavior in [
        "malformed",
        "missing-url",
        "non-string-url",
        "array-ready",
        "invalid-ready",
        "overlong-ready",
        "tiny-line",
    ] {
        let process = spawn(
            behavior,
            Options {
                max_line_bytes: if behavior == "tiny-line" { 3 } else { 128 },
                ..Default::default()
            },
        );
        assert!(process.wait_ready().is_err(), "{behavior}");
        let state = exited(&process);
        assert_eq!(state.errors.len(), 1, "{behavior}: {state:?}");
        assert!(
            state
                .lines
                .iter()
                .all(|line| !line.text.contains("MRTK-RPC-READY"))
        );
    }
}

#[test]
fn second_ready_line_is_an_error_but_preserves_first_url() {
    let process = spawn("duplicate", Options::default());
    assert_eq!(process.wait_ready().unwrap(), URL);
    let state = exited(&process);
    assert_eq!(
        state.errors.iter().collect::<Vec<_>>(),
        [&"a second ready line was received".to_string()]
    );
    assert!(
        state
            .lines
            .iter()
            .all(|line| !line.text.contains("MRTK-RPC-READY"))
    );
}

#[test]
fn readiness_is_only_the_exact_stderr_prefix() {
    for behavior in ["stdout-ready", "prefix-no-space"] {
        let process = spawn(behavior, Options::default());
        assert!(process.wait_ready().is_err());
        assert!(
            exited(&process)
                .lines
                .iter()
                .any(|line| line.text.starts_with("MRTK-RPC-READY"))
        );
    }
}

#[test]
fn secret_env_reaches_child_and_is_masked_on_both_streams() {
    assert!(!command().preview().contains(TOKEN));
    let process = spawn("mask", Options::default());
    process.wait_ready().unwrap();
    let state = exited(&process);
    assert!(
        state
            .lines
            .iter()
            .any(|line| line.text == "env-matches=true")
    );
    for stream in [Stream::Stdout, Stream::Stderr] {
        assert!(state.lines.iter().any(|line| line.stream == stream && line.text.contains("***@host prefix***suffix")));
    }
    assert!(!format!("{state:?}").contains("user:pass"));

    let process = spawn(
        "mask",
        Options {
            max_display_chars: 23,
            ..Default::default()
        },
    );
    process.wait_ready().unwrap();
    let state = exited(&process);
    let line = state
        .lines
        .iter()
        .find(|line| line.text.starts_with("truncate"))
        .unwrap();
    assert_eq!(line.text, format!("truncate {} ***…", "界".repeat(10)));
}

#[test]
fn overlong_display_lines_are_wholly_discarded_then_reading_resumes() {
    let process = spawn(
        "overlong",
        Options {
            max_line_bytes: 128,
            ..Default::default()
        },
    );
    process.wait_ready().unwrap();
    let state = exited(&process);
    for stream in [Stream::Stdout, Stream::Stderr] {
        assert_eq!(
            state
                .lines
                .iter()
                .filter(|line| line.stream == stream && line.text.contains("longer than 128 bytes"))
                .count(),
            1
        );
        assert!(
            state
                .lines
                .iter()
                .any(|line| line.stream == stream && line.text == "normal")
        );
    }
    assert!(state.lines.iter().all(|line| !line.text.contains("xxx")));
    assert!(state.errors.is_empty());
}

#[test]
fn retained_lines_are_bounded_and_drops_counted() {
    // libtest prints two stdout header lines before the selected child test.
    let full = spawn("many", Options::default());
    full.wait_ready().unwrap();
    let full_count = exited(&full).lines.len();
    let process = spawn(
        "many",
        Options {
            max_lines: 3,
            ..Default::default()
        },
    );
    process.wait_ready().unwrap();
    let state = exited(&process);
    assert_eq!(state.lines.len(), 3);
    assert_eq!(state.dropped_lines, (full_count - 3) as u64);
    assert_eq!(
        state
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["line 17", "line 18", "line 19"]
    );
}

#[test]
fn errors_are_bounded_and_zero_limits_are_supported() {
    let process = spawn(
        "many-ready",
        Options {
            max_lines: 1,
            ..Default::default()
        },
    );
    process.wait_ready().unwrap();
    let state = exited(&process);
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.dropped_errors, 3);

    let process = spawn(
        "many",
        Options {
            max_lines: 0,
            max_display_chars: 0,
            ..Default::default()
        },
    );
    process.wait_ready().unwrap();
    let state = exited(&process);
    assert!(state.lines.is_empty());
    assert!(state.dropped_lines >= 20);

    let process = spawn(
        "tiny-line",
        Options {
            max_line_bytes: 0,
            ready_timeout: Duration::ZERO,
            ..Default::default()
        },
    );
    assert!(process.wait_ready().is_err());
    exited(&process);
}

#[test]
fn lossy_utf8_one_cr_and_unterminated_final_line() {
    let process = spawn("bytes", Options::default());
    process.wait_ready().unwrap();
    let state = exited(&process);
    for text in ["invalid ��", "carriage\r", "last-no-newline"] {
        assert!(
            state.lines.iter().any(|line| line.text == text),
            "{state:?}"
        );
    }
}

#[test]
fn drop_kills_reaps_and_joins() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut variables = env("drop");
    variables.push((
        "SUPERVISOR_TEST_ADDRESS".into(),
        listener.local_addr().unwrap().to_string(),
    ));
    let process = Supervisor::spawn(command(), &variables, Options::default(), || {});
    process.wait_ready().unwrap();
    let (mut connection, _) = listener.accept().unwrap();
    connection.set_read_timeout(Some(TIMEOUT)).unwrap();
    let start = Instant::now();
    drop(process);
    assert!(start.elapsed() < TIMEOUT);
    assert_eq!(
        connection.read(&mut [0]).unwrap(),
        0,
        "child is still running"
    );
}

#[test]
fn drop_is_bounded_when_a_grandchild_holds_both_pipes() {
    let (process, mut grandchild, _) = inherited_pipes(
        "inherited-kill",
        Options {
            drain_timeout: DRAIN_TIMEOUT,
            ..Default::default()
        },
    );
    assert_eq!(process.wait_ready().unwrap(), URL);
    let started = Instant::now();
    drop(process);
    let elapsed = started.elapsed();
    assert!(elapsed >= DRAIN_TIMEOUT);
    assert!(
        elapsed < DRAIN_TIMEOUT + Duration::from_millis(500),
        "drop took {elapsed:?}"
    );
    grandchild.cleanup();
}

#[test]
fn stop_is_bounded_and_detached_readers_still_update_state() {
    for behavior in ["inherited-ready", "inherited-kill"] {
        let (process, mut grandchild, notifications) = inherited_pipes(
            behavior,
            Options {
                drain_timeout: DRAIN_TIMEOUT,
                ..Default::default()
            },
        );
        assert_eq!(process.wait_ready().unwrap(), URL);
        let started = Instant::now();
        process.stop(Duration::ZERO);
        let elapsed = started.elapsed();
        assert!(
            elapsed < DRAIN_TIMEOUT + Duration::from_millis(500),
            "stop took {elapsed:?}"
        );
        assert!(process.wait_exit(Duration::ZERO));
        let state = process.state();
        assert!(matches!(state.readiness, Readiness::Ready { .. }));
        assert_eq!(
            state
                .errors
                .iter()
                .filter(|error| error.contains("output is no longer awaited"))
                .count(),
            1
        );
        // A second stop uses the original deadline instead of waiting another drain interval.
        let started = Instant::now();
        process.stop(Duration::ZERO);
        assert!(started.elapsed() < DRAIN_TIMEOUT);
        grandchild.cleanup();
        until_state(&process, &notifications, |state| {
            state
                .lines
                .iter()
                .any(|line| line.text == "grandchild cleaned up")
        });
    }
}

#[test]
fn exit_before_ready_is_bounded_even_when_stderr_is_inherited() {
    let (process, mut grandchild, _) = inherited_pipes(
        "inherited-exit",
        Options {
            drain_timeout: DRAIN_TIMEOUT,
            ..Default::default()
        },
    );
    let started = Instant::now();
    let error = process.wait_ready().unwrap_err();
    assert!(started.elapsed() < DRAIN_TIMEOUT + Duration::from_millis(500));
    assert!(
        error.contains("before the ready line") && error.contains('3'),
        "{error}"
    );
    let state = exited(&process);
    assert_eq!(state.exit_status.unwrap().code(), Some(3));
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.contains("output is no longer awaited"))
    );
    grandchild.cleanup();
}

#[test]
fn ready_timeout_remains_active_after_exit_while_draining() {
    let (process, mut grandchild, notifications) = inherited_pipes(
        "inherited-exit",
        Options {
            ready_timeout: Duration::from_millis(400),
            drain_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    );
    until_state(&process, &notifications, |state| {
        state.exit_status.is_some()
    });
    assert_eq!(process.state().readiness, Readiness::Waiting);
    assert_eq!(
        process.wait_ready().unwrap_err(),
        "the ready timeout passed"
    );
    assert!(
        !process
            .state()
            .errors
            .iter()
            .any(|error| error.contains("output is no longer awaited"))
    );
    grandchild.cleanup();
    exited(&process);
}

#[test]
fn spawn_failure_is_displayed_and_does_not_block_waits() {
    let command = CommandLine {
        working_directory: None,
        program: std::env::temp_dir().join(format!("missing-supervisor-{TOKEN}")),
        args: vec![],
    };
    let process = Supervisor::spawn(command, &env("exit"), Options::default(), || {});
    assert!(
        process
            .wait_ready()
            .unwrap_err()
            .contains("cannot start child")
    );
    assert!(process.wait_exit(Duration::ZERO));
    assert!(process.state().exit_status.is_none());
    assert!(!format!("{:?}", process.state()).contains(TOKEN));
    process.stop(Duration::ZERO);
}

#[test]
fn supervised_mock_authenticates_and_shuts_down_gracefully() {
    let process = spawn("mock", Options::default());
    let url = process.wait_ready().unwrap();
    let (mut client, _) = Client::connect(&url, Some(TOKEN), TIMEOUT).unwrap();
    client.sync().unwrap();
    client.shutdown().unwrap();
    process.stop(TIMEOUT);
    let state = exited(&process);
    assert!(state.exit_status.unwrap().success());
    assert!(state.errors.is_empty());
}

#[test]
fn held_stdin_stays_open_until_stop_and_working_directory_is_executed() {
    let directory = std::env::temp_dir();
    let mut command = command();
    command.working_directory = Some(directory.clone());
    let process = Supervisor::spawn(
        command,
        &env("stdin-eof"),
        Options {
            stdin: mrtklib_egui_supervisor::Stdin::HeldOpen,
            ..Options::default()
        },
        || {},
    );
    process.wait_ready().unwrap();
    assert!(!process.wait_exit(Duration::from_millis(150)));
    process.stop(TIMEOUT);
    let state = exited(&process);
    assert_eq!(state.exit_status.unwrap().code(), Some(0));
    assert!(
        state
            .lines
            .iter()
            .any(|line| line.text == format!("cwd={}", directory.display()))
    );
}
