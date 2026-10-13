//! The client and the session against the in-process mock server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use mrtklib_egui_mock::{Config, Running, start};
use mrtklib_egui_rpc::{
    Authentication, Client, Command, Connection, Error, Event, EventData, Model, RequestError,
    Session, SessionOptions, SolutionStatus, State, StopReason,
};

const TOKEN: &str = "test-token_0123456789";
const TIMEOUT: Duration = Duration::from_secs(5);

fn fast() -> Config {
    Config {
        token: Some(TOKEN.to_string()),
        transition: Duration::from_millis(100),
        solution_interval: Duration::from_millis(20),
        ..Config::default()
    }
}

fn connect(running: &Running) -> Client {
    Client::connect(running.url(), Some(TOKEN), TIMEOUT)
        .expect("connect")
        .0
}

/// A client that collects the events it polls.
struct Poller {
    client: Client,
    cursor: u64,
    events: Vec<Event>,
}

impl Poller {
    fn new(running: &Running) -> Self {
        let mut client = connect(running);
        let cursor = client.sync().expect("sync").seq;
        Self {
            client,
            cursor,
            events: Vec::new(),
        }
    }

    fn until(&mut self, done: impl Fn(&[Event]) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&self.events) {
            assert!(
                Instant::now() < deadline,
                "timed out; events: {:#?}",
                self.events
            );
            let page = self
                .client
                .get_events(self.cursor, None)
                .expect("getEvents");
            assert!(!page.lost, "events lost");
            self.events.extend(page.events);
            self.cursor = page.next;
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn states(events: &[Event]) -> Vec<State> {
    events
        .iter()
        .filter_map(|e| match &e.data {
            EventData::Status(lifecycle) => Some(lifecycle.state),
            _ => None,
        })
        .collect()
}

fn solutions(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e.data, EventData::Solution(_)))
        .count()
}

fn last_state(events: &[Event]) -> Option<State> {
    states(events).last().copied()
}

fn rpc_error(error: Error) -> mrtklib_egui_rpc::RpcError {
    match error {
        Error::Rpc(error) => error,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[test]
fn token_handshake() {
    let running = start(fast()).unwrap();
    let (client, version) = Client::connect(running.url(), Some(TOKEN), TIMEOUT).unwrap();
    assert_eq!(version.protocol_version, "0.1");
    assert_eq!(version.authentication, Authentication::Token);
    client.close().expect("normal close");

    let wrong = Client::connect(running.url(), Some("wrong-token"), TIMEOUT).unwrap_err();
    assert!(matches!(wrong, Error::Unauthorized), "{wrong:?}");
    let missing = Client::connect(running.url(), None, TIMEOUT).unwrap_err();
    assert!(matches!(missing, Error::Unauthorized), "{missing:?}");

    let open = start(Config {
        token: None,
        ..fast()
    })
    .unwrap();
    let (_, version) = Client::connect(open.url(), None, TIMEOUT).unwrap();
    assert_eq!(version.authentication, Authentication::None);
}

#[test]
fn start_publishes_solutions_and_stop_ends_the_run() {
    let running = start(fast()).unwrap();
    let mut poller = Poller::new(&running);
    let status = poller.client.get_status().unwrap();
    assert_eq!(status.status.lifecycle.state, State::Stopped);
    assert_eq!(status.status.mode, None);

    poller.client.start().unwrap();
    poller.until(|events| solutions(events) >= 12);
    let status = poller.client.get_status().unwrap();
    assert_eq!(status.status.mode.as_deref(), Some("kinematic"));
    assert!(status.status.solution.is_some());
    assert!(
        poller.events.windows(2).all(|w| w[0].seq < w[1].seq),
        "seq increases in publish order"
    );

    poller.client.stop().unwrap();
    poller.until(|events| last_state(events) == Some(State::Stopped));
    assert_eq!(
        states(&poller.events),
        [
            State::Starting,
            State::Running,
            State::Stopping,
            State::Stopped
        ]
    );
    let stopped = poller
        .events
        .iter()
        .rev()
        .find_map(|e| match &e.data {
            EventData::Status(lifecycle) => Some(lifecycle),
            _ => None,
        })
        .unwrap();
    assert_eq!(stopped.stop_reason, Some(StopReason::Request));
    let fix = poller.events.iter().any(|e| {
        matches!(&e.data, EventData::Solution(s) if s.status == SolutionStatus::Fix
            && s.position.is_some() && s.geoid.is_some())
    });
    assert!(fix, "the run reaches a fixed solution");

    // Stopping a stopped server succeeds without effect.
    let before = poller.client.get_status().unwrap().seq;
    assert_eq!(poller.client.stop().unwrap().seq, before);
}

#[test]
fn lifecycle_requests_are_refused_while_busy_or_running() {
    let running = start(Config {
        transition: Duration::from_millis(300),
        ..fast()
    })
    .unwrap();
    let mut poller = Poller::new(&running);
    poller.client.start().unwrap();
    let busy = rpc_error(poller.client.start().unwrap_err());
    assert_eq!(busy.reason.as_deref(), Some("busy"));
    assert_eq!(busy.state, Some(State::Starting));

    poller.until(|events| last_state(events) == Some(State::Running));
    let already = rpc_error(poller.client.start().unwrap_err());
    assert_eq!(already.code, 1002);
    assert_eq!(already.reason.as_deref(), Some("alreadyRunning"));
}

#[test]
fn invalid_parameters_are_an_rpc_error() {
    let running = start(fast()).unwrap();
    let mut client = connect(&running);
    let ahead = rpc_error(client.get_events(1_000_000, None).unwrap_err());
    assert_eq!(ahead.code, -32602);
    assert_eq!(ahead.reason.as_deref(), Some("invalidParams"));
    let zero = rpc_error(client.get_events(0, Some(0)).unwrap_err());
    assert_eq!(zero.reason.as_deref(), Some("invalidParams"));
    // The connection stays usable after an RPC error.
    client.get_status().unwrap();
}

#[test]
fn a_failed_start_reports_last_error() {
    let running = start(Config {
        fail_start: true,
        ..fast()
    })
    .unwrap();
    let mut poller = Poller::new(&running);
    poller.client.start().unwrap();
    poller.until(|events| last_state(events) == Some(State::Stopped));
    let lifecycle = poller.client.get_status().unwrap().status.lifecycle;
    assert_eq!(lifecycle.stop_reason, Some(StopReason::Error));
    assert_eq!(lifecycle.last_error.unwrap().reason, "startFailed");
}

#[test]
fn input_end_stops_the_run_by_itself() {
    let running = start(Config {
        input_end_after: Some(3),
        ..fast()
    })
    .unwrap();
    let mut poller = Poller::new(&running);
    poller.client.start().unwrap();
    poller.until(|events| last_state(events) == Some(State::Stopped));
    assert_eq!(solutions(&poller.events), 3);
    let lifecycle = poller.client.get_status().unwrap().status.lifecycle;
    assert_eq!(lifecycle.stop_reason, Some(StopReason::InputEnd));
}

#[test]
fn lost_events_and_paging() {
    let running = start(Config {
        event_capacity: 5,
        solution_interval: Duration::from_millis(10),
        ..fast()
    })
    .unwrap();
    let mut client = connect(&running);
    client.start().unwrap();
    thread::sleep(Duration::from_millis(400));
    let page = client.get_events(0, None).unwrap();
    assert!(page.lost && page.events.is_empty());
    assert!(page.oldest_seq.unwrap() > 1);

    let seq = client.sync().unwrap().seq;
    thread::sleep(Duration::from_millis(40));
    let page = client.get_events(seq, Some(1)).unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.more);
}

fn wait_for(session: &Session, what: &str, done: impl Fn(&Model) -> bool) -> Model {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let model = session.model();
        if done(&model) {
            return model;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {model:#?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn open_session(running: &Running, options: SessionOptions) -> Session {
    Session::spawn(
        running.url().to_string(),
        Some(TOKEN.to_string()),
        options,
        || {},
    )
}

fn quick() -> SessionOptions {
    SessionOptions {
        poll_interval: Duration::from_millis(20),
        timeout: Duration::from_secs(2),
        ..SessionOptions::default()
    }
}

fn is_running(model: &Model) -> bool {
    model
        .lifecycle
        .as_ref()
        .is_some_and(|l| l.state == State::Running)
}

#[test]
fn a_session_follows_start_and_stop() {
    let running = start(fast()).unwrap();
    let session = open_session(&running, quick());
    let model = wait_for(&session, "connection", |m| {
        matches!(m.connection, Connection::Connected { .. }) && m.lifecycle.is_some()
    });
    assert_eq!(model.lifecycle.unwrap().state, State::Stopped);

    session.request(Command::Start);
    let model = wait_for(&session, "a solution while running", |m| {
        is_running(m) && m.mode.is_some() && m.solution.is_some()
    });
    assert_eq!(model.mode.as_deref(), Some("kinematic"));
    assert!(model.log.iter().any(|m| m.text.contains("started")));
    let outcome = model.last_request.unwrap();
    assert_eq!((outcome.command, outcome.result), (Command::Start, Ok(())));

    session.request(Command::Stop);
    let model = wait_for(&session, "stopped", |m| {
        m.lifecycle
            .as_ref()
            .is_some_and(|l| l.state == State::Stopped)
    });
    // The last run's mode and solution stay after the stop ...
    assert_eq!(model.mode.as_deref(), Some("kinematic"));
    assert!(model.solution.is_some());
    assert_eq!(
        model.lifecycle.unwrap().stop_reason,
        Some(StopReason::Request)
    );

    // ... and go when the next run starts.
    session.request(Command::Start);
    let model = wait_for(&session, "the next start", |m| {
        m.lifecycle
            .as_ref()
            .is_some_and(|l| l.state == State::Starting)
    });
    assert!(model.mode.is_none() && model.solution.is_none());
    wait_for(&session, "the next run", |m| {
        is_running(m) && m.mode.is_some()
    });
    session.close();
}

#[test]
fn notify_is_called_after_the_model_changed() {
    let running = start(fast()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let session = Session::spawn(
        running.url().to_string(),
        Some(TOKEN.to_string()),
        quick(),
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
        },
    );
    wait_for(&session, "connection", |m| m.lifecycle.is_some());
    // The session notified, at the latest, after the first sync, so the model it saw was
    // already current.
    let after_sync = calls.load(Ordering::SeqCst);
    assert!(after_sync >= 1);
    session.request(Command::Start);
    wait_for(&session, "running", |m| is_running(m) && m.mode.is_some());
    assert!(calls.load(Ordering::SeqCst) > after_sync);
}

#[test]
fn a_refused_request_is_recorded_and_the_session_goes_on() {
    let running = start(fast()).unwrap();
    let session = open_session(&running, quick());
    session.request(Command::Start);
    wait_for(&session, "running", |m| is_running(m) && m.mode.is_some());
    session.request(Command::Start);
    let model = wait_for(
        &session,
        "the refusal",
        |m| matches!(&m.last_request, Some(o) if o.result.is_err()),
    );
    let Err(RequestError::Rpc(error)) = model.last_request.unwrap().result else {
        panic!("expected an RPC error");
    };
    assert_eq!(error.reason.as_deref(), Some("alreadyRunning"));
    assert!(matches!(model.connection, Connection::Connected { .. }));
}

#[test]
fn a_slow_session_loses_events_and_recovers_through_sync() {
    let running = start(Config {
        event_capacity: 5,
        transition: Duration::from_millis(50),
        solution_interval: Duration::from_millis(10),
        ..fast()
    })
    .unwrap();
    let session = open_session(
        &running,
        SessionOptions {
            poll_interval: Duration::from_millis(300),
            ..quick()
        },
    );
    wait_for(&session, "connection", |m| m.lifecycle.is_some());
    session.request(Command::Start);
    let model = wait_for(&session, "recovery", |m| {
        m.lost_events > 0 && is_running(m) && m.mode.is_some() && m.solution.is_some()
    });
    assert!(model.seq > 0);

    session.request(Command::Stop);
    wait_for(&session, "stopped after losses", |m| {
        m.lifecycle
            .as_ref()
            .is_some_and(|l| l.state == State::Stopped)
    });
}

#[test]
fn shutdown_ends_the_session_as_a_server_exit() {
    let running = start(fast()).unwrap();
    let session = open_session(&running, quick());
    wait_for(&session, "connection", |m| m.lifecycle.is_some());
    session.request(Command::Shutdown);
    let model = wait_for(&session, "the end", |m| {
        !matches!(m.connection, Connection::Connected { .. })
    });
    assert_eq!(model.connection, Connection::ServerExited);
    assert_eq!(
        model.last_request.map(|o| (o.command, o.result)),
        Some((Command::Shutdown, Ok(())))
    );
    running.wait();
}

#[test]
fn close_returns_promptly_and_leaves_the_server_running() {
    let running = start(fast()).unwrap();
    let session = open_session(&running, quick());
    wait_for(&session, "connection", |m| m.lifecycle.is_some());
    let begun = Instant::now();
    session.close();
    assert!(begun.elapsed() < Duration::from_secs(2));
    // The server is still there for the next client.
    connect(&running).close().unwrap();
}

#[test]
fn the_token_does_not_appear_in_debug_or_error_text() {
    let running = start(fast()).unwrap();
    let client = connect(&running);
    assert!(!format!("{client:?}").contains(TOKEN));
    let session = open_session(&running, quick());
    wait_for(&session, "connection", |m| m.lifecycle.is_some());
    assert!(!format!("{session:?} {:?}", session.model()).contains(TOKEN));
    drop(client);
    drop(session);

    // A refused handshake: the error shows no token, neither the wrong one that was sent.
    let wrong = "wrong-token_abcdef";
    let error = Client::connect(running.url(), Some(wrong), TIMEOUT).unwrap_err();
    let text = format!("{error:?} {error}");
    assert!(!text.contains(wrong) && !text.contains(TOKEN), "{text}");
    let session = Session::spawn(
        running.url().to_string(),
        Some(wrong.to_string()),
        quick(),
        || {},
    );
    let model = wait_for(&session, "the refusal", |m| {
        matches!(m.connection, Connection::Disconnected { .. })
    });
    let text = format!("{session:?} {model:?}");
    assert!(!text.contains(wrong) && !text.contains(TOKEN), "{text}");
    assert!(
        matches!(model.connection, Connection::Disconnected { ref error } if error.contains("401"))
    );
}
