//! Opt-in Linux interoperability tests. Fixtures and backend are supplied locally.

use super::*;
use mrtklib_egui_rpc::{Connection, State as LifecycleState};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Instant;

struct Fixture {
    directory: PathBuf,
    command: CommandLine,
    output: String,
}

impl Fixture {
    fn new() -> Self {
        fn required(name: &str) -> PathBuf {
            let value = std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| panic!("{name} must be set for real_backend_ tests"));
            let path = PathBuf::from(value);
            path.canonicalize()
                .unwrap_or_else(|error| panic!("{name}: {}: {error}", path.display()))
        }
        let mrtk = required("MRTKLIB_EGUI_TEST_MRTK");
        let tree = required("MRTKLIB_EGUI_TEST_MRTKLIB_DIR");
        let directory = std::env::temp_dir().join(format!(
            "navi-real-{}-{}",
            std::process::id(),
            generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let output = directory.join("solution.pos").to_str().unwrap().to_owned();
        let command =
            program::run_command(mrtk, directory.clone(), Path::new("mrtklib-egui-navi")).unwrap();
        let fixture = Self {
            directory,
            command,
            output,
        };
        let mut tar = ProcessCommand::new("tar")
            .arg("-xzf")
            .arg(tree.join("tests/data/malib/MALIB_OSS_data.tar.gz"))
            .arg("-C")
            .arg(&fixture.directory)
            .stdin(Stdio::null())
            .spawn()
            .expect("system tar is required for MALIB fixtures");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = tar.try_wait().unwrap() {
                assert!(status.success(), "MALIB extraction failed: {status}");
                break;
            }
            if Instant::now() >= deadline {
                tar.kill().unwrap();
                tar.wait().unwrap();
                panic!("MALIB extraction exceeded 30 s");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let source = std::fs::read_to_string(tree.join("conf/malib/rtkrcv.toml")).unwrap();
        let output_literal = "./tests/data/malib/out/rtkrcv_test.pos";
        assert_eq!(
            source.matches(output_literal).count(),
            1,
            "MALIB output fixture changed"
        );
        assert!(source.contains("./tests/data/malib/") && source.contains("::T::x10"));
        // Literal fixture substitutions, following MRTKLIB's RPC tests; no TOML parser.
        let config = source.replace(output_literal, &fixture.output).replace(
            "./tests/data/malib/",
            &format!("{}/data/", fixture.directory.display()),
        );
        std::fs::write(Path::new(fixture.command.args.last().unwrap()), config).unwrap();
        fixture
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn until(controller: &mut Controller, done: impl Fn(&Controller) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
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
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn state(controller: &Controller) -> Option<LifecycleState> {
    controller.model.lifecycle.as_ref().map(|l| l.state)
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_TEST_MRTK and MRTKLIB_EGUI_TEST_MRTKLIB_DIR"]
fn real_backend_controller_replay_overwrite_and_rpc_shutdown() {
    let fixture = Fixture::new();
    let mut controller = Controller::launch_command(Ok(fixture.command.clone()), || {});
    until(&mut controller, |c| c.view().action_enabled);
    assert!(controller.config_note.is_none());
    controller.request(Command::Start);
    until(&mut controller, |c| {
        state(c) == Some(LifecycleState::Running)
            && c.model.last_valid_solution.is_some()
            && c.model.mode.is_some()
    });
    assert_eq!(controller.model.mode.as_deref(), Some("ppp-kine"));
    assert!(
        controller
            .model
            .last_valid_solution
            .as_ref()
            .unwrap()
            .position
            .is_some()
    );
    controller.request(Command::Stop);
    until(&mut controller, |c| {
        state(c) == Some(LifecycleState::Stopped)
    });
    controller.request(Command::Start);
    until(&mut controller, |c| c.overwrite_dialog.is_some());
    assert_eq!(state(&controller), Some(LifecycleState::Stopped));
    let error = controller
        .model
        .lifecycle
        .as_ref()
        .unwrap()
        .last_error
        .as_ref()
        .unwrap();
    assert_eq!(error.reason, "outputExists");
    assert_eq!(
        error.paths.as_slice(),
        std::slice::from_ref(&fixture.output)
    );
    let dialog = controller.overwrite_dialog.as_mut().unwrap();
    assert_eq!(dialog.paths.len(), 1);
    assert_eq!(dialog.paths[0].path, fixture.output);
    dialog.paths[0].overwrite = true;
    controller.confirm_overwrite();
    until(&mut controller, |c| {
        state(c) == Some(LifecycleState::Running)
    });
    controller
        .session
        .as_ref()
        .unwrap()
        .request(Command::Shutdown);
    until(&mut controller, |c| {
        c.model.connection == Connection::ServerExited
    });
    let supervisor = controller.supervisor.as_ref().unwrap();
    assert!(
        supervisor.wait_exit(Duration::from_secs(5)),
        "{:?}",
        supervisor.state()
    );
    assert_eq!(supervisor.state().exit_status.unwrap().code(), Some(0));
    assert!(fixture.directory.join("rtkrcv.nav").is_file());
    // Reap and join before removing the fixture directory.
    drop(controller.supervisor.take());
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_TEST_MRTK and MRTKLIB_EGUI_TEST_MRTKLIB_DIR"]
fn real_backend_stdin_eof_without_rpc_saves_navigation() {
    let fixture = Fixture::new();
    let supervisor = Supervisor::spawn(
        fixture.command.clone(),
        &[("MRTK_RPC_TOKEN".into(), generate_token())],
        Options {
            stdin: Stdin::HeldOpen,
            ..Options::default()
        },
        || {},
    );
    supervisor.wait_ready().unwrap();
    assert!(!supervisor.wait_exit(Duration::from_millis(150)));
    supervisor.stop(EXIT_GRACE);
    assert_eq!(
        supervisor.state().exit_status.unwrap().code(),
        Some(0),
        "{:?}",
        supervisor.state()
    );
    assert!(fixture.directory.join("rtkrcv.nav").is_file());
}
