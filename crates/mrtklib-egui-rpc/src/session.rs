//! A worker thread that keeps a shared model of `mrtk run` up to date (MRTKLIB `docs/rpc/`
//! §6, §7), for a GUI that reads the model every frame.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::client::{Client, Error, RpcError};
use crate::types::{
    Event, EventData, EventPage, Lifecycle, LogMessage, Snapshot, Solution, State, Version,
};

/// Timing and size settings of a [`Session`]. The defaults are provisional.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// Time between `getEvents` calls while no more events are ready. Default 100 ms: the
    /// example of the protocol document, and a client polling every 0.1-1 s loses no events
    /// in normal operation (§7).
    pub poll_interval: Duration,
    /// Bound for connecting and for each request, from its send to its response. Default 5 s.
    /// Lifecycle requests are answered when they take effect, not when the transition ends
    /// (§5), so a healthy server answers quickly; 5 s leaves room for a loaded machine and
    /// still lets the GUI report a hung server. Provisional: revisit after measuring the
    /// real `mrtk run`.
    pub timeout: Duration,
    /// Number of log messages that the model keeps (the newest ones). Default 1000, so
    /// that memory stays bounded when level-2 messages flood (§9). Provisional: set it
    /// when the log panel of the navigation application is designed.
    pub log_capacity: usize,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(100),
            timeout: Duration::from_secs(5),
            log_capacity: 1000,
        }
    }
}

/// A lifecycle request (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Stop,
    Restart,
    Shutdown,
}

/// State of the connection to the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    Connecting,
    Connected {
        version: Version,
    },
    /// The server ended the connection with status 1001, as it does when it exits (after
    /// `shutdown`, or when the process is terminated). Not a failure.
    ServerExited,
    /// The connection ended. `error` is text for display and holds no token. There is no
    /// automatic reconnection: start a new session.
    Disconnected {
        error: String,
    },
}

/// Why a request did not succeed.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestError {
    /// The server refused the request (for example `busy`); branch on `reason`.
    Rpc(RpcError),
    /// The request could not be carried out: the connection failed or is gone.
    Connection(String),
}

/// The outcome of the latest [`Session::request`]. Success only means that the server
/// accepted it: the state change is seen in `lifecycle`, through the status events.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestOutcome {
    pub command: Command,
    pub result: Result<(), RequestError>,
}

/// What the GUI shows. It follows the protocol's events in publish order, so it is always
/// the state after some event.
///
/// `mode` and `solution` belong to the current or the last run: they are kept after the run
/// stops, so that the final solution stays visible, and are cleared when the next run
/// starts (a `starting` status event). The GUI decides how to show them outside `running`.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub connection: Connection,
    /// The `seq` of the last event applied (or of the last `sync`).
    pub seq: u64,
    /// `None` until the first `sync`.
    pub lifecycle: Option<Lifecycle>,
    /// Positioning mode of the current or last run. `None` before the first run, after a
    /// `sync` outside `running` (a `sync` holds no mode then), and when a run ended before
    /// the mode could be read.
    pub mode: Option<String>,
    /// The latest solution event of the current or last run. After a `sync` outside
    /// `running` it is `None`: the last solution is not known then (events that were lost
    /// are not recovered, §7).
    pub solution: Option<Solution>,
    /// The newest log messages, at most [`SessionOptions::log_capacity`].
    pub log: VecDeque<LogMessage>,
    /// How many times `getEvents` reported lost events. The number of missing events is
    /// not known; the lifecycle is recovered by `sync`, the missing solutions and log
    /// messages are not (§7).
    pub lost_events: u64,
    pub last_request: Option<RequestOutcome>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            connection: Connection::Connecting,
            seq: 0,
            lifecycle: None,
            mode: None,
            solution: None,
            log: VecDeque::new(),
            lost_events: 0,
            last_request: None,
        }
    }
}

impl Model {
    /// Replace the state with a `sync` result (§7).
    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        self.seq = snapshot.seq;
        self.lifecycle = Some(snapshot.status.lifecycle);
        self.mode = snapshot.status.mode;
        self.solution = snapshot.status.solution;
    }

    /// Apply one event (§7). An event of an unknown kind is skipped (§4).
    fn apply_event(&mut self, event: Event, log_capacity: usize) {
        self.seq = event.seq;
        match event.data {
            EventData::Status(lifecycle) => {
                // A new run starts: forget the last run's mode and solution. The final
                // solutions of a run can follow its `stopping` event (§7), so they are not
                // cleared before that.
                if lifecycle.state == State::Starting {
                    self.mode = None;
                    self.solution = None;
                }
                self.lifecycle = Some(lifecycle);
            }
            EventData::Solution(solution) => self.solution = Some(solution),
            EventData::LogMessage(message) => {
                self.log.push_back(message);
                while self.log.len() > log_capacity {
                    self.log.pop_front();
                }
            }
            EventData::Unknown { .. } => {}
        }
    }
}

struct Shared {
    model: Mutex<Model>,
    notify: Box<dyn Fn() + Send + Sync>,
    /// Set by the owner before it sends `Close`. The worker looks at it before every
    /// request it sends, so a stop is not delayed by queued commands or by repeated
    /// recovery from lost events.
    stop: AtomicBool,
}

impl Shared {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn lock(&self) -> MutexGuard<'_, Model> {
        // The lock is held only for short updates that cannot panic; a poisoned lock would
        // be a bug, and stopping is better than showing a model that stopped changing.
        self.model.lock().expect("session model lock poisoned")
    }

    /// Change the model, then tell the owner (after the lock is released, because the
    /// callback may read the model).
    fn update(&self, change: impl FnOnce(&mut Model)) {
        change(&mut self.lock());
        (self.notify)();
    }
}

enum Message {
    Request(Command),
    Close,
}

/// A connection to `mrtk run` that a worker thread keeps current. The owner reads
/// [`Session::model`] (for example on every frame) and sends requests with
/// [`Session::request`].
///
/// The worker connects, calls `sync`, then polls `getEvents` every `poll_interval`
/// (at once again while `more` is set). After `lost` it calls `sync` again. When the
/// connection ends, for whatever reason, the worker ends; there is no automatic reconnect.
pub struct Session {
    shared: Arc<Shared>,
    sender: Sender<Message>,
    worker: Option<JoinHandle<()>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("connection", &self.shared.lock().connection)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Start a session. `notify` is called from the worker thread after the model changed
    /// (a GUI passes a repaint request; this crate does not depend on any GUI toolkit).
    /// The token is used for the handshake only and then dropped by the worker.
    pub fn spawn(
        url: String,
        token: Option<String>,
        options: SessionOptions,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Session {
        let shared = Arc::new(Shared {
            model: Mutex::new(Model::default()),
            notify: Box::new(notify),
            stop: AtomicBool::new(false),
        });
        let (sender, receiver) = mpsc::channel();
        let worker = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("mrtk-rpc-session".into())
                .spawn(move || {
                    let end = work(&shared, &receiver, &url, token, &options);
                    // The final state and the requests that were queued but never sent are
                    // set under one lock, with `Session::request` checking the state under
                    // the same lock: no request is lost without a trace.
                    shared.update(|model| {
                        match end {
                            Ok(()) => {}
                            Err(error) if error.is_server_exit() => {
                                model.connection = Connection::ServerExited;
                            }
                            Err(error) => {
                                model.connection = Connection::Disconnected {
                                    error: error.to_string(),
                                };
                            }
                        }
                        for message in receiver.try_iter() {
                            if let Message::Request(command) = message {
                                model.last_request = Some(not_sent(command));
                            }
                        }
                    });
                })
        };
        let worker = match worker {
            Ok(handle) => Some(handle),
            Err(error) => {
                shared.update(|model| {
                    model.connection = Connection::Disconnected {
                        error: format!("cannot start the worker thread: {error}"),
                    };
                });
                None
            }
        };
        Session {
            shared,
            sender,
            worker,
        }
    }

    /// A copy of the current model.
    pub fn model(&self) -> Model {
        self.shared.lock().clone()
    }

    /// Queue a lifecycle request. The worker sends it between polls and records the
    /// outcome in [`Model::last_request`].
    pub fn request(&self, command: Command) {
        let mut model = self.shared.lock();
        let ended = matches!(
            model.connection,
            Connection::ServerExited | Connection::Disconnected { .. }
        );
        if !ended && self.sender.send(Message::Request(command)).is_ok() {
            return;
        }
        // The connection has ended: nobody would ever send the request.
        model.last_request = Some(not_sent(command));
        drop(model);
        (self.shared.notify)();
    }

    /// Stop the worker: it closes the WebSocket normally, then this call returns. The
    /// worker looks at the stop request before every request it sends, and each request
    /// and the closing are bounded by the timeout, so the wait is bounded by about twice
    /// the timeout. Requests that were queued but not sent are recorded as not sent.
    pub fn close(mut self) {
        self.stop_worker();
    }

    fn stop_worker(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // `Close` wakes the worker if it waits; a failed send means that it has ended.
        let _ = self.sender.send(Message::Close);
        if let Some(worker) = self.worker.take() {
            // A panic of the worker was reported by the panic hook; nothing more to do here.
            let _ = worker.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

fn not_sent(command: Command) -> RequestOutcome {
    RequestOutcome {
        command,
        result: Err(RequestError::Connection(
            "the connection has ended; the request was not sent".into(),
        )),
    }
}

/// Whether the worker goes on.
enum Flow {
    Continue,
    Stop,
}

/// The worker body. `Ok` means that the owner asked to stop.
fn work(
    shared: &Shared,
    receiver: &Receiver<Message>,
    url: &str,
    token: Option<String>,
    options: &SessionOptions,
) -> Result<(), Error> {
    let connected = Client::connect(url, token.as_deref(), options.timeout);
    drop(token);
    let (client, version) = connected?;
    shared.update(|model| model.connection = Connection::Connected { version });

    let mut worker = Worker {
        client,
        shared,
        receiver,
        log_capacity: options.log_capacity,
        cursor: 0,
        last_status_seq: 0,
        want_mode: false,
    };
    worker.sync()?;
    worker.serve(options.poll_interval)?;
    worker.client.close()
}

struct Worker<'a> {
    client: Client,
    shared: &'a Shared,
    receiver: &'a Receiver<Message>,
    log_capacity: usize,
    /// The `after` of the next `getEvents`.
    cursor: u64,
    /// The `seq` of the last status event applied (or of the last `sync`).
    last_status_seq: u64,
    /// A status event said `running` and the mode is not known yet.
    want_mode: bool,
}

impl Worker<'_> {
    /// Wait for requests and poll on schedule, until the owner closes the session.
    fn serve(&mut self, poll_interval: Duration) -> Result<(), Error> {
        let mut next_poll = Instant::now() + poll_interval;
        loop {
            if self.shared.stopping() {
                return Ok(());
            }
            let wait = next_poll.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(wait) {
                Ok(message) => {
                    if let Flow::Stop = self.handle(message)? {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Flow::Stop = self.poll()? {
                        return Ok(());
                    }
                    next_poll = Instant::now() + poll_interval;
                }
                // The session is gone without a close message.
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }

    fn handle(&mut self, message: Message) -> Result<Flow, Error> {
        match message {
            Message::Close => Ok(Flow::Stop),
            Message::Request(command) if self.shared.stopping() => {
                self.shared
                    .update(|model| model.last_request = Some(not_sent(command)));
                Ok(Flow::Stop)
            }
            Message::Request(command) => self.execute(command).map(|()| Flow::Continue),
        }
    }

    /// Send a lifecycle request and record its outcome. The lifecycle itself changes only
    /// through the events.
    fn execute(&mut self, command: Command) -> Result<(), Error> {
        let result = match command {
            Command::Start => self.client.start(),
            Command::Stop => self.client.stop(),
            Command::Restart => self.client.restart(),
            Command::Shutdown => self.client.shutdown(),
        };
        let (result, failure) = match result {
            Ok(_) => (Ok(()), None),
            // A refused request leaves the connection usable.
            Err(Error::Rpc(error)) => (Err(RequestError::Rpc(error)), None),
            Err(error) => (
                Err(RequestError::Connection(error.to_string())),
                Some(error),
            ),
        };
        self.shared
            .update(|model| model.last_request = Some(RequestOutcome { command, result }));
        failure.map_or(Ok(()), Err)
    }

    /// Fetch events until none are ready now; handle requests that arrive in between.
    fn poll(&mut self) -> Result<Flow, Error> {
        loop {
            if self.shared.stopping() {
                return Ok(Flow::Stop);
            }
            let page = self.client.get_events(self.cursor, None)?;
            if page.lost {
                self.shared.update(|model| model.lost_events += 1);
                self.sync()?;
                continue;
            }
            let more = page.more;
            self.apply_page(page);
            self.resolve_mode()?;
            if !more {
                return Ok(Flow::Continue);
            }
            match self.receiver.try_recv() {
                Ok(message) => {
                    if let Flow::Stop = self.handle(message)? {
                        return Ok(Flow::Stop);
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return Ok(Flow::Stop),
            }
        }
    }

    /// Replace the state with `sync` and continue from its `seq`.
    fn sync(&mut self) -> Result<(), Error> {
        let snapshot = self.client.sync()?;
        self.cursor = snapshot.seq;
        self.last_status_seq = snapshot.seq;
        self.want_mode = false;
        self.shared.update(|model| model.apply_snapshot(snapshot));
        Ok(())
    }

    fn apply_page(&mut self, page: EventPage) {
        self.cursor = page.next;
        if page.events.is_empty() {
            return;
        }
        for event in &page.events {
            if let EventData::Status(lifecycle) = &event.data {
                self.last_status_seq = event.seq;
                self.want_mode = lifecycle.state == State::Running;
            }
        }
        let capacity = self.log_capacity;
        self.shared.update(|model| {
            for event in page.events {
                model.apply_event(event, capacity);
            }
        });
    }

    /// After a status event said `running`, learn the mode with `getStatus` (§6).
    ///
    /// Only the mode is taken from the response. The lifecycle and the solutions come from
    /// the events, in publish order; the response may be newer than the events applied so
    /// far, and applying it whole would show a state that later events then undo. A
    /// response older than the last applied status event is stale and discarded (§6); the
    /// next poll asks again.
    fn resolve_mode(&mut self) -> Result<(), Error> {
        if !self.want_mode {
            return Ok(());
        }
        let snapshot = self.client.get_status()?;
        if snapshot.seq < self.last_status_seq {
            return Ok(());
        }
        self.want_mode = false;
        if snapshot.status.lifecycle.state != State::Running {
            // The run is over already; the status events that follow say so.
            return Ok(());
        }
        let mode = snapshot.status.mode;
        self.shared.update(|model| {
            if model
                .lifecycle
                .as_ref()
                .is_some_and(|l| l.state == State::Running)
            {
                model.mode = mode;
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(value: serde_json::Value) -> Event {
        serde_json::from_value(value).unwrap()
    }

    fn solution_event(seq: u64) -> Event {
        event(json!({
            "seq": seq, "kind": "solution",
            "data": { "time": { "week": 1, "tow": 1.0 }, "leapSeconds": 18, "status": "none" }
        }))
    }

    fn status_event(seq: u64, state: &str) -> Event {
        event(json!({ "seq": seq, "kind": "status", "data": { "state": state } }))
    }

    #[test]
    fn mode_and_solution_stay_after_the_run_stops_and_go_when_the_next_starts() {
        let mut model = Model::default();
        model.apply_event(status_event(1, "running"), 10);
        model.mode = Some("kinematic".into());
        model.apply_event(solution_event(2), 10);
        model.apply_event(status_event(3, "stopping"), 10);
        // A final solution of the run can follow `stopping` (§7).
        model.apply_event(solution_event(4), 10);
        model.apply_event(status_event(5, "stopped"), 10);
        assert_eq!(model.mode.as_deref(), Some("kinematic"));
        assert!(model.solution.is_some());
        assert_eq!(model.seq, 5);

        model.apply_event(status_event(6, "starting"), 10);
        assert_eq!((model.mode.clone(), model.solution.clone()), (None, None));
    }

    #[test]
    fn the_log_keeps_only_the_newest_messages() {
        let mut model = Model::default();
        for (seq, text) in [(1, "a"), (2, "b"), (3, "c")] {
            let e = event(json!({ "seq": seq, "kind": "logMessage",
                                  "data": { "level": 1, "text": text } }));
            model.apply_event(e, 2);
        }
        let texts: Vec<&str> = model.log.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["b", "c"]);
    }

    #[test]
    fn an_unknown_kind_is_skipped_but_its_seq_is_applied() {
        let mut model = Model::default();
        model.apply_event(event(json!({ "seq": 9, "kind": "later", "data": 1 })), 10);
        assert_eq!(model.seq, 9);
        assert!(model.lifecycle.is_none() && model.log.is_empty());
    }

    #[test]
    fn a_request_after_the_worker_ended_is_reported() {
        let session = Session::spawn(
            "ws://127.0.0.1:1/rpc".into(),
            None,
            SessionOptions {
                timeout: Duration::from_millis(500),
                ..SessionOptions::default()
            },
            || {},
        );
        // Nothing listens on port 1: the worker ends with a connection error.
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(session.model().connection, Connection::Connecting) {
            assert!(Instant::now() < deadline, "the worker did not end");
            thread::sleep(Duration::from_millis(10));
        }
        session.request(Command::Start);
        let model = session.model();
        assert!(matches!(model.connection, Connection::Disconnected { .. }));
        let outcome = model.last_request.expect("the failed request is recorded");
        assert_eq!(outcome.command, Command::Start);
        assert!(matches!(outcome.result, Err(RequestError::Connection(_))));
    }
}
