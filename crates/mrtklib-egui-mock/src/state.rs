//! Server state: lifecycle transitions, the event buffer and the simulated solutions.
//!
//! The rules follow MRTKLIB `docs/rpc/` §5–§8: lifecycle requests respond when they take
//! effect and their outcome follows as status events; requests during a transition or a
//! restart get `busy`; `shutdown` is recorded during a transition and carried out after it;
//! events stay in the buffer until newer events push them out, also across runs.

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::Config;
use crate::solution;

pub(crate) struct Shared {
    pub(crate) config: Config,
    state: Mutex<State>,
    /// Set when the server ends: after a `shutdown`, or when the `Running` handle is dropped.
    pub(crate) exit: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Life {
    Stopped,
    Starting,
    Running,
    Stopping,
}

impl Life {
    fn name(self) -> &'static str {
        match self {
            Life::Stopped => "stopped",
            Life::Starting => "starting",
            Life::Running => "running",
            Life::Stopping => "stopping",
        }
    }
}

struct State {
    life: Life,
    stop_reason: Option<&'static str>,
    last_error: Option<Value>,
    /// A restart holds the lifecycle from its stop to its start.
    restarting: bool,
    shutdown: bool,
    seq: u64,
    events: VecDeque<Value>,
    last_solution: Option<Value>,
    /// Solutions published in the current run.
    epoch: u64,
}

/// Errors of the lifecycle methods; `rpc` turns them into JSON-RPC errors.
pub(crate) enum LifecycleError {
    Busy(&'static str),
    AlreadyRunning,
    ShuttingDown,
}

/// A `getEvents` cursor beyond the last published seq.
pub(crate) struct CursorAhead;

impl Shared {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            config,
            state: Mutex::new(State {
                life: Life::Stopped,
                stop_reason: None,
                last_error: None,
                restarting: false,
                shutdown: false,
                seq: 0,
                events: VecDeque::new(),
                last_solution: None,
                epoch: 0,
            }),
            exit: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned lock means a mock thread panicked; stop instead of hiding it.
        self.state.lock().expect("mock state lock poisoned")
    }

    fn publish(&self, state: &mut State, kind: &str, data: Value) {
        state.seq += 1;
        state
            .events
            .push_back(json!({ "seq": state.seq, "kind": kind, "data": data }));
        while state.events.len() > self.config.event_capacity {
            state.events.pop_front();
        }
    }

    fn publish_lifecycle(&self, state: &mut State) {
        let data = lifecycle(state);
        self.publish(state, "status", data);
    }

    fn log(&self, state: &mut State, text: &str) {
        self.publish(state, "logMessage", json!({ "level": 1, "text": text }));
    }

    /// Result of `getStatus` and `sync`.
    pub(crate) fn status(&self) -> Value {
        let state = self.lock();
        let mut status = json!({ "lifecycle": lifecycle(&state) });
        if state.life == Life::Running {
            status["mode"] = json!("kinematic");
            if let Some(solution) = &state.last_solution {
                status["solution"] = solution.clone();
            }
        }
        json!({ "seq": state.seq, "status": status })
    }

    /// Result of `getEvents`.
    pub(crate) fn events(&self, after: u64, limit: Option<u64>) -> Result<Value, CursorAhead> {
        let state = self.lock();
        if after > state.seq {
            return Err(CursorAhead);
        }
        let oldest = state.events.front().map_or(state.seq + 1, seq_of);
        if after < state.seq && oldest > after + 1 {
            return Ok(json!({
                "events": [], "next": after, "more": false, "lost": true, "oldestSeq": oldest
            }));
        }
        let limit = limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(usize::MAX)
            .min(self.config.page_limit);
        let pending: Vec<&Value> = state.events.iter().filter(|e| seq_of(e) > after).collect();
        let page: Vec<Value> = pending.iter().take(limit).map(|e| (*e).clone()).collect();
        let next = page.last().map_or(after, seq_of);
        Ok(json!({ "events": page, "next": next, "more": pending.len() > limit, "lost": false }))
    }

    fn check_lifecycle_request(&self, state: &State) -> Result<(), LifecycleError> {
        if state.shutdown {
            return Err(LifecycleError::ShuttingDown);
        }
        if state.restarting || matches!(state.life, Life::Starting | Life::Stopping) {
            return Err(LifecycleError::Busy(state.life.name()));
        }
        Ok(())
    }
}

fn seq_of(event: &Value) -> u64 {
    event["seq"].as_u64().expect("every event has a seq")
}

fn lifecycle(state: &State) -> Value {
    let mut value = json!({ "state": state.life.name() });
    if let Some(reason) = state.stop_reason {
        value["stopReason"] = json!(reason);
    }
    if let Some(error) = &state.last_error {
        value["lastError"] = error.clone();
    }
    value
}

pub(crate) fn start(shared: &Arc<Shared>) -> Result<u64, LifecycleError> {
    let mut state = shared.lock();
    shared.check_lifecycle_request(&state)?;
    if state.life == Life::Running {
        return Err(LifecycleError::AlreadyRunning);
    }
    begin_start(shared, &mut state);
    Ok(state.seq)
}

pub(crate) fn stop(shared: &Arc<Shared>) -> Result<u64, LifecycleError> {
    let mut state = shared.lock();
    shared.check_lifecycle_request(&state)?;
    if state.life == Life::Running {
        begin_stop(shared, &mut state, "request");
    }
    Ok(state.seq)
}

pub(crate) fn restart(shared: &Arc<Shared>) -> Result<u64, LifecycleError> {
    let mut state = shared.lock();
    shared.check_lifecycle_request(&state)?;
    if state.life == Life::Running {
        state.restarting = true;
        begin_stop(shared, &mut state, "request");
    } else {
        begin_start(shared, &mut state);
    }
    Ok(state.seq)
}

pub(crate) fn shutdown(shared: &Arc<Shared>) -> u64 {
    let mut state = shared.lock();
    if !state.shutdown {
        state.shutdown = true;
        carry_out_shutdown(shared, &mut state);
    }
    state.seq
}

/// Act on a recorded shutdown once no transition is in progress.
fn carry_out_shutdown(shared: &Arc<Shared>, state: &mut State) {
    if !state.shutdown || state.restarting {
        return;
    }
    match state.life {
        Life::Running => begin_stop(shared, state, "request"),
        Life::Stopped => shared.exit.store(true, std::sync::atomic::Ordering::SeqCst),
        Life::Starting | Life::Stopping => {}
    }
}

fn begin_start(shared: &Arc<Shared>, state: &mut State) {
    state.life = Life::Starting;
    state.stop_reason = None;
    shared.publish_lifecycle(state);
    let shared = Arc::clone(shared);
    thread::spawn(move || {
        thread::sleep(shared.config.transition);
        let mut state = shared.lock();
        state.restarting = false;
        if shared.config.fail_start {
            state.life = Life::Stopped;
            state.stop_reason = Some("error");
            state.last_error = Some(json!({
                "reason": "startFailed",
                "message": "mock: the start fails because fail_start is set",
            }));
            shared.publish_lifecycle(&mut state);
            shared.log(&mut state, "mock: start failed");
        } else {
            state.life = Life::Running;
            state.last_error = None;
            state.epoch = 0;
            state.last_solution = None;
            shared.publish_lifecycle(&mut state);
            shared.log(&mut state, "mock: positioning started");
        }
        carry_out_shutdown(&shared, &mut state);
    });
}

fn begin_stop(shared: &Arc<Shared>, state: &mut State, reason: &'static str) {
    state.life = Life::Stopping;
    state.stop_reason = Some(reason);
    shared.publish_lifecycle(state);
    let shared = Arc::clone(shared);
    thread::spawn(move || {
        thread::sleep(shared.config.transition);
        let mut state = shared.lock();
        state.life = Life::Stopped;
        shared.publish_lifecycle(&mut state);
        if state.restarting {
            begin_start(&shared, &mut state);
        } else {
            carry_out_shutdown(&shared, &mut state);
        }
    });
}

/// Publish one solution per interval while running, until the server ends.
pub(crate) fn ticker(shared: &Arc<Shared>) {
    let step = Duration::from_millis(10);
    let mut waited = Duration::ZERO;
    while !shared.exit.load(std::sync::atomic::Ordering::SeqCst) {
        thread::sleep(step);
        waited += step;
        if waited < shared.config.solution_interval {
            continue;
        }
        waited = Duration::ZERO;
        let mut state = shared.lock();
        if state.life != Life::Running {
            continue;
        }
        let solution = solution::at(state.epoch, shared.config.solution_interval);
        state.epoch += 1;
        state.last_solution = Some(solution.clone());
        shared.publish(&mut state, "solution", solution);
        if shared
            .config
            .input_end_after
            .is_some_and(|n| state.epoch >= n)
        {
            begin_stop(shared, &mut state, "inputEnd");
        }
    }
}
