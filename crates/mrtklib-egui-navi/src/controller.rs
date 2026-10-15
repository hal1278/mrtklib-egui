//! Own the process supervisor and RPC session; no GUI toolkit dependency.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mrtklib_egui_rpc::{Command, Model, Session, SessionOptions};
use mrtklib_egui_supervisor::{
    CommandLine, Options, Readiness, State, Stdin, Supervisor, generate_token,
};

use crate::{
    program,
    view::{self, View},
};

/// Shutdown stops positioning, saves navigation data and closes output
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
    config_note: Option<String>,
    working_directory: String,
    start_operation: Option<StartOperation>,
    pub(crate) overwrite_dialog: Option<OverwriteDialog>,
    notify: Arc<dyn Fn() + Send + Sync>,
    closing: bool,
    exit_done: Arc<AtomicBool>,
}

#[derive(Default)]
struct StartOperation {
    consented: Vec<String>,
}

// A row owns one choice, so other per-path choices can be added with configuration RPC.
pub(crate) struct OutputChoice {
    pub(crate) path: String,
    pub(crate) overwrite: bool,
}

pub(crate) struct OverwriteDialog {
    pub(crate) paths: Vec<OutputChoice>,
}

impl OverwriteDialog {
    pub(crate) fn all_checked(&self) -> bool {
        !self.paths.is_empty() && self.paths.iter().all(|row| row.overwrite)
    }
}

impl Controller {
    pub(crate) fn launch(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self::launch_command(program::command_line(), notify)
    }

    fn launch_command(
        command: Result<CommandLine, String>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
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
            config_note: None,
            working_directory: String::new(),
            start_operation: None,
            overwrite_dialog: None,
            notify,
            closing: false,
            exit_done: Arc::new(AtomicBool::new(false)),
        };
        match command {
            Ok(command) => {
                controller.preview = command.preview();
                controller.working_directory = command
                    .working_directory
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string());
                let config =
                    std::path::Path::new(command.args.last().expect("navi config argument"));
                match config.try_exists() {
                    Ok(false) => {
                        controller.config_note = Some(format!(
                            "Configuration not found: {}; mrtk uses its default options",
                            config.display()
                        ))
                    }
                    Ok(true) => {}
                    Err(error) => {
                        controller.launch_error = Some(format!(
                            "cannot check configuration {}: {error}",
                            config.display()
                        ));
                        return controller;
                    }
                }
                let notify = Arc::clone(&controller.notify);
                controller.supervisor = Some(Supervisor::spawn(
                    command,
                    &[("MRTK_RPC_TOKEN".into(), controller.token.clone())],
                    Options {
                        stdin: Stdin::HeldOpen,
                        ..Options::default()
                    },
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
        self.update_start_operation();
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
        let mut view = view::build(
            &self.model,
            self.process.as_ref(),
            &self.preview,
            self.launch_error.as_deref(),
            self.closing,
            &self.token,
        );
        if let Some(note) = &self.config_note {
            view.status.push_str(" | ");
            view.status.push_str(&self.display(note));
        }
        view.action_enabled &= self.start_operation.is_none();
        view
    }

    pub(crate) fn request(&mut self, command: Command) {
        let view = self.view();
        if view.action_enabled
            && command == view.action
            && let Some(session) = &self.session
        {
            if command == Command::Start {
                self.start_operation = Some(StartOperation::default());
                session.request_start(Vec::new());
            } else {
                session.request(command);
            }
        }
    }

    fn update_start_operation(&mut self) {
        if self.start_operation.is_none() {
            return;
        }
        if !matches!(
            self.model.connection,
            mrtklib_egui_rpc::Connection::Connected { .. }
        ) {
            self.cancel_start();
            return;
        }
        if self.overwrite_dialog.is_some() {
            return;
        }
        let Some(outcome) = &self.model.last_request else {
            return;
        };
        if outcome.command != Command::Start || outcome.result.is_err() {
            self.cancel_start();
            return;
        }
        let Some(boundary) = outcome.accepted_seq else {
            return;
        };
        let Some(event_seq) = self.model.lifecycle_seq else {
            // A recovery snapshot cannot prove whose start failed; retain its status
            // text and let a fresh Start operation obtain its own event boundary.
            if self.model.seq > boundary {
                self.cancel_start();
            }
            return;
        };
        if event_seq <= boundary {
            return;
        }
        let Some(lifecycle) = &self.model.lifecycle else {
            return;
        };
        match lifecycle.state {
            mrtklib_egui_rpc::State::Stopped => {
                if let Some(error) = &lifecycle.last_error
                    && lifecycle.stop_reason == Some(mrtklib_egui_rpc::StopReason::Error)
                    && error.reason == "outputExists"
                {
                    self.overwrite_dialog = Some(OverwriteDialog {
                        paths: error
                            .paths
                            .iter()
                            .map(|path| OutputChoice {
                                path: path.clone(),
                                overwrite: false,
                            })
                            .collect(),
                    });
                } else {
                    self.cancel_start();
                }
            }
            mrtklib_egui_rpc::State::Running => self.cancel_start(),
            _ => {}
        }
    }

    pub(crate) fn display(&self, text: &str) -> String {
        mrtklib_egui_supervisor::mask::mask(text, &[&self.token])
    }

    pub(crate) fn working_directory(&self) -> String {
        self.display(&self.working_directory)
    }

    pub(crate) fn cancel_start(&mut self) {
        self.overwrite_dialog = None;
        self.start_operation = None;
    }

    pub(crate) fn confirm_overwrite(&mut self) {
        if self.closing
            || !self
                .overwrite_dialog
                .as_ref()
                .is_some_and(OverwriteDialog::all_checked)
        {
            return;
        }
        if let (Some(dialog), Some(operation), Some(session)) = (
            self.overwrite_dialog.take(),
            &mut self.start_operation,
            &self.session,
        ) {
            for choice in dialog.paths {
                if !operation.consented.contains(&choice.path) {
                    operation.consented.push(choice.path);
                }
            }
            session.request_start(operation.consented.clone());
            // Discard the previous refusal until the new request outcome arrives.
            self.model.last_request = None;
        }
    }

    /// Start closing without waiting on the UI thread. Completion is published after
    /// process exit and bounded draining, before Session's potentially slower join.
    pub(crate) fn begin_exit(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.cancel_start();
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
            config_note: None,
            working_directory: String::new(),
            start_operation: None,
            overwrite_dialog: None,
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
    #[test]
    fn overwrite_dialog_requires_every_consent_and_cancel_resets_the_operation() {
        let running = start(Config {
            existing_outputs: vec!["b.pos".into(), "a.pos".into()],
            ..config()
        })
        .unwrap();
        let mut controller = attached(running.url());
        until(&mut controller, |c| c.view().action_enabled);
        controller.request(Command::Start);
        until(&mut controller, |c| c.overwrite_dialog.is_some());
        let dialog = controller.overwrite_dialog.as_mut().unwrap();
        assert_eq!(
            dialog
                .paths
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            ["b.pos", "a.pos"]
        );
        dialog.paths[0].overwrite = true;
        assert!(!dialog.all_checked());
        controller.confirm_overwrite();
        assert!(controller.overwrite_dialog.is_some());
        controller.cancel_start();
        assert!(controller.view().action_enabled);
        controller.request(Command::Start);
        until(&mut controller, |c| c.overwrite_dialog.is_some());
        assert!(
            controller
                .start_operation
                .as_ref()
                .unwrap()
                .consented
                .is_empty()
        );
        assert!(
            controller
                .overwrite_dialog
                .as_ref()
                .unwrap()
                .paths
                .iter()
                .all(|row| !row.overwrite)
        );
        for row in &mut controller.overwrite_dialog.as_mut().unwrap().paths {
            row.overwrite = true;
        }
        controller.confirm_overwrite();
        until(&mut controller, |c| {
            c.model
                .lifecycle
                .as_ref()
                .is_some_and(|l| l.state == LifecycleState::Running)
        });
        assert!(controller.start_operation.is_none());
        assert!(controller.overwrite_dialog.is_none());
    }

    #[test]
    fn another_clients_failure_does_not_open_a_dialog() {
        let running = start(Config {
            existing_outputs: vec!["output.pos".into()],
            ..config()
        })
        .unwrap();
        let mut controller = attached(running.url());
        until(&mut controller, |c| c.view().action_enabled);
        let (mut peer, _) = mrtklib_egui_rpc::Client::connect(
            running.url(),
            Some("controller-test-token"),
            Duration::from_secs(2),
        )
        .unwrap();
        peer.start(&[]).unwrap();
        until(&mut controller, |c| {
            c.view().status.contains("outputExists")
        });
        assert!(controller.overwrite_dialog.is_none());
    }

    #[test]
    fn refusal_must_be_a_status_event_after_the_start_boundary_and_consents_accumulate() {
        use mrtklib_egui_rpc::{LastError, Lifecycle, RequestOutcome, StopReason};
        let running = start(config()).unwrap();
        let mut controller = attached(running.url());
        until(&mut controller, |c| c.view().action_enabled);
        controller.start_operation = Some(StartOperation {
            consented: vec!["earlier.pos".into()],
        });
        controller.model.last_request = Some(RequestOutcome {
            command: Command::Start,
            accepted_seq: Some(10),
            result: Ok(()),
        });
        controller.model.lifecycle = Some(Lifecycle {
            state: LifecycleState::Stopped,
            stop_reason: Some(StopReason::Error),
            last_error: Some(LastError {
                reason: "outputExists".into(),
                message: "exists".into(),
                paths: vec!["new.pos".into()],
            }),
        });
        for seq in [None, Some(9), Some(10)] {
            controller.model.lifecycle_seq = seq;
            controller.update_start_operation();
            assert!(controller.overwrite_dialog.is_none());
        }
        controller.model.lifecycle_seq = Some(11);
        controller.update_start_operation();
        controller.overwrite_dialog.as_mut().unwrap().paths[0].overwrite = true;
        controller.confirm_overwrite();
        assert_eq!(
            controller.start_operation.as_ref().unwrap().consented,
            ["earlier.pos", "new.pos"]
        );
        until(&mut controller, |c| c.model.last_request.is_some());
        assert!(
            controller
                .model
                .last_request
                .as_ref()
                .unwrap()
                .result
                .is_ok()
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "real_backend_tests.rs"]
mod real_backend_tests;
