//! Own the process supervisor and RPC session; no GUI toolkit dependency.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mrtklib_egui_rpc::{Command, Model, Session, SessionOptions};
use mrtklib_egui_supervisor::{Options, Readiness, State, Supervisor, generate_token};

use crate::{
    program,
    view::{self, View},
};

/// Provisional: shutdown stops positioning, saves navigation data and closes output
/// files. Five seconds allows that on a slow machine; killing afterwards keeps the
/// backend tied to the GUI lifetime. The supervisor also bounds output draining.
const EXIT_GRACE: Duration = Duration::from_secs(5);

pub(crate) struct Controller {
    supervisor: Option<Supervisor>,
    process: Option<State>,
    session: Option<Session>,
    model: Model,
    preview: String,
    token: String,
    launch_error: Option<String>,
    notify: Arc<dyn Fn() + Send + Sync>,
    closing: bool,
    exit_done: Arc<AtomicBool>,
}

impl Controller {
    pub(crate) fn launch(notify: impl Fn() + Send + Sync + 'static) -> Self {
        let token = generate_token();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);
        let mut controller = Self {
            supervisor: None,
            process: None,
            session: None,
            model: Model::default(),
            preview: String::new(),
            token,
            launch_error: None,
            notify,
            closing: false,
            exit_done: Arc::new(AtomicBool::new(false)),
        };
        match program::command_line() {
            Ok(command) => {
                controller.preview = command.preview();
                let notify = Arc::clone(&controller.notify);
                controller.supervisor = Some(Supervisor::spawn(
                    command,
                    &[("MRTK_RPC_TOKEN".into(), controller.token.clone())],
                    Options::default(),
                    move || notify(),
                ));
            }
            Err(error) => controller.launch_error = Some(error),
        }
        controller
    }

    pub(crate) fn tick(&mut self) {
        if let Some(supervisor) = &self.supervisor {
            let state = supervisor.state();
            self.connect_when_ready(&state);
            self.process = Some(state);
        }
        if let Some(session) = &self.session {
            self.model = session.model();
        }
    }

    fn connect_when_ready(&mut self, process: &State) {
        if self.session.is_none()
            && !self.closing
            && process.exit_status.is_none()
            && let Readiness::Ready { url } = &process.readiness
        {
            let notify = Arc::clone(&self.notify);
            self.session = Some(Session::spawn(
                url.clone(),
                Some(self.token.clone()),
                SessionOptions::default(),
                move || notify(),
            ));
        }
    }

    pub(crate) fn view(&self) -> View {
        view::build(
            &self.model,
            self.process.as_ref(),
            &self.preview,
            self.launch_error.as_deref(),
            self.closing,
            &self.token,
        )
    }

    pub(crate) fn request(&self, command: Command) {
        let view = self.view();
        if view.action_enabled
            && command == view.action
            && let Some(session) = &self.session
        {
            session.request(command);
        }
    }

    /// Start closing without waiting on the UI thread. Completion is published after
    /// process exit and bounded draining, before Session's potentially slower join.
    pub(crate) fn begin_exit(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        let supervisor = self.supervisor.take();
        let session = self.session.take();
        let done = Arc::clone(&self.exit_done);
        let notify = Arc::clone(&self.notify);
        std::thread::Builder::new()
            .name("navi-shutdown".into())
            .spawn(move || {
                if let Some(session) = &session {
                    session.request(Command::Shutdown);
                }
                if let Some(supervisor) = supervisor {
                    supervisor.stop(EXIT_GRACE);
                    drop(supervisor);
                }
                done.store(true, Ordering::Release);
                notify();
                drop(session);
            })
            .expect("cannot start shutdown worker");
    }

    pub(crate) fn exit_done(&self) -> bool {
        self.exit_done.load(Ordering::Acquire)
    }
    pub(crate) fn closing(&self) -> bool {
        self.closing
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.begin_exit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrtklib_egui_mock::{Config, start};
    use mrtklib_egui_rpc::{Connection, State as LifecycleState};
    use std::collections::VecDeque;
    use std::time::Instant;

    fn attached(url: &str) -> Controller {
        let mut controller = Controller {
            supervisor: None,
            process: None,
            session: None,
            model: Model::default(),
            preview: "/suite/mrtk run --rpc-port 0 --no-console".into(),
            token: "controller-test-token".into(),
            launch_error: None,
            notify: Arc::new(|| {}),
            closing: false,
            exit_done: Arc::new(AtomicBool::new(false)),
        };
        let mut process = State {
            lines: VecDeque::new(),
            dropped_lines: 0,
            readiness: Readiness::Waiting,
            exit_status: None,
            errors: VecDeque::new(),
            dropped_errors: 0,
        };
        controller.connect_when_ready(&process);
        assert!(controller.session.is_none());
        process.readiness = Readiness::Ready { url: url.into() };
        controller.connect_when_ready(&process);
        controller.process = Some(process);
        controller
    }

    fn until(controller: &mut Controller, done: impl Fn(&Controller) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            controller.tick();
            if done(controller) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out: {}",
                controller.view().status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn config() -> Config {
        Config {
            token: Some("controller-test-token".into()),
            transition: Duration::from_millis(250),
            solution_interval: Duration::from_millis(50),
            ..Config::default()
        }
    }

    // Another workspace package's binary has no CARGO_BIN_EXE path here. Exercise
    // the same readiness/session/controller mapping with the in-process mock instead.
    #[test]
    fn controller_follows_run_stop_next_run_and_server_exit() {
        let running = start(config()).unwrap();
        let mut controller = attached(running.url());
        until(&mut controller, |c| c.view().action_enabled);
        assert_eq!(controller.view().action, Command::Start);
        assert_eq!(
            controller.model.lifecycle.as_ref().unwrap().state,
            LifecycleState::Stopped
        );
        // Repeated ready snapshots must retain the existing session.
        let session = controller.session.as_ref().unwrap() as *const Session;
        controller.connect_when_ready(&controller.process.clone().unwrap());
        assert_eq!(
            session,
            controller.session.as_ref().unwrap() as *const Session
        );
        controller.request(Command::Start);
        until(&mut controller, |c| {
            c.model.lifecycle.as_ref().unwrap().state == LifecycleState::Running
                && c.model.solution.is_some()
                && c.model.mode.is_some()
        });
        assert!(controller.view().status.contains("Mode: kinematic"));
        assert_eq!(controller.view().action, Command::Stop);
        controller.request(Command::Stop);
        until(&mut controller, |c| {
            c.model.lifecycle.as_ref().unwrap().state == LifecycleState::Stopped
        });
        let stopped = controller.view();
        assert_ne!(stopped.rows[0].1, "----");
        assert_eq!(stopped.time_color, view::GRAY);
        assert!(stopped.status.contains("stopReason: request"));
        controller.request(Command::Start);
        until(&mut controller, |c| {
            c.model.lifecycle.as_ref().unwrap().state == LifecycleState::Running
                && c.model.solution.is_some()
                && c.model.mode.is_some()
        });
        controller
            .session
            .as_ref()
            .unwrap()
            .request(Command::Shutdown);
        until(&mut controller, |c| {
            c.model.connection == Connection::ServerExited
        });
        assert!(!controller.view().action_enabled);
        assert!(controller.view().status.contains("server exited"));
        running.wait();
    }

    #[test]
    fn failed_start_is_visible_and_start_becomes_available_again() {
        let running = start(Config {
            fail_start: true,
            ..config()
        })
        .unwrap();
        let mut controller = attached(running.url());
        until(&mut controller, |c| c.view().action_enabled);
        controller.request(Command::Start);
        until(&mut controller, |c| {
            c.model
                .lifecycle
                .as_ref()
                .is_some_and(|l| l.last_error.is_some())
        });
        assert!(controller.view().status.contains("lastError: startFailed"));
        assert!(controller.view().action_enabled);
        assert_eq!(controller.view().action, Command::Start);
        controller
            .session
            .as_ref()
            .unwrap()
            .request(Command::Shutdown);
        until(&mut controller, |c| {
            c.model.connection == Connection::ServerExited
        });
        // A ended session is kept, rather than reconnecting to the ready URL.
        controller.connect_when_ready(&controller.process.clone().unwrap());
        assert_eq!(
            controller.session.as_ref().unwrap().model().connection,
            Connection::ServerExited
        );
        running.wait();
    }
}
