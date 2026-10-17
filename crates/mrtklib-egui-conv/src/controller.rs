//! Own finite conversions and close/overwrite decisions; no egui dependency.

use crate::{
    command,
    settings::{self, Settings},
    view,
};
use mrtklib_egui_supervisor::{CommandLine, Options, ReadinessMode, State, Supervisor};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub(crate) struct PendingConversion {
    pub paths: Vec<PathBuf>,
    command: CommandLine,
}

pub(crate) struct Controller {
    pub settings: Settings,
    program: Result<PathBuf, String>,
    cwd: Result<PathBuf, String>,
    settings_file: Result<PathBuf, String>,
    supervisor: Option<Supervisor>,
    process: Option<State>,
    pub overwrite: Option<PendingConversion>,
    pub close_dialog: bool,
    pub closing: bool,
    pub error: Option<String>,
    preview: String,
    notify: Arc<dyn Fn() + Send + Sync>,
    saved: bool,
}

impl Controller {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        let executable = std::env::current_exe().map_err(|e| format!("cannot locate GUI: {e}"));
        let program = mrtklib_egui_common::paths::resolve_program(
            std::env::var_os("MRTKLIB_EGUI_MRTK"),
            || executable.clone(),
            cfg!(windows),
        );
        let settings_file = executable
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|exe| settings::settings_file(exe));
        let (settings, warning) = match &settings_file {
            Ok(path) => settings::load(path),
            Err(error) => (Settings::default(), Some(error.clone())),
        };
        if let Some(warning) = &warning {
            eprintln!(
                "Warning: {}",
                mrtklib_egui_supervisor::mask::mask(warning, &[])
            );
        }
        let mut controller = Self {
            settings,
            program,
            settings_file,
            cwd: std::env::current_dir()
                .map_err(|e| format!("cannot resolve current directory: {e}")),
            supervisor: None,
            process: None,
            overwrite: None,
            close_dialog: false,
            closing: false,
            error: warning,
            preview: String::new(),
            notify: Arc::new(notify),
            saved: false,
        };
        controller.refresh_outputs(false);
        controller
    }

    fn command(&self) -> Result<command::PreparedConversion, String> {
        // A path-resolution error has no fallback settings location, as in navi.
        self.settings_file.as_ref().map_err(Clone::clone)?;
        command::build(
            &self.settings,
            self.program.as_ref().map_err(Clone::clone)?,
            self.cwd.as_ref().map_err(Clone::clone)?,
        )
    }

    pub fn refresh_outputs(&mut self, input_changed: bool) {
        let result = self
            .cwd
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|cwd| self.settings.refresh_outputs(input_changed, cwd));
        if let Err(error) = result {
            self.error = Some(error);
        }
    }

    pub fn tick(&mut self) {
        if let Some(supervisor) = &self.supervisor {
            self.process = Some(supervisor.state());
        }
        if !self.busy() && !self.closing && self.overwrite.is_none() {
            self.preview = match self.command() {
                Ok(prepared) => prepared.command.preview(),
                Err(error) => mrtklib_egui_supervisor::mask::mask(&error, &[]),
            };
        }
        if self.closing && !self.busy() {
            self.save();
        }
    }

    pub fn busy(&self) -> bool {
        self.supervisor
            .as_ref()
            .is_some_and(|s| !s.wait_exit(Duration::ZERO))
    }

    pub fn view(&self) -> view::View {
        view::build(self.process.as_ref(), self.error.as_deref())
    }
    pub fn preview(&self) -> &str {
        &self.preview
    }

    pub fn convert(&mut self) {
        if self.busy() || self.closing || self.overwrite.is_some() {
            return;
        }
        self.error = None;
        let result = self.command().and_then(|prepared| {
            let paths = prepared.existing_outputs()?;
            Ok(PendingConversion {
                paths,
                command: prepared.command,
            })
        });
        match result {
            Ok(pending) if !pending.paths.is_empty() => {
                self.preview = pending.command.preview();
                self.overwrite = Some(pending);
            }
            Ok(pending) => self.start(pending.command),
            Err(error) => self.error = Some(error),
        }
    }

    pub fn confirm_overwrite(&mut self) {
        if let Some(pending) = self.overwrite.take() {
            self.start(pending.command);
        }
    }

    fn start(&mut self, command: CommandLine) {
        self.preview = command.preview();
        let notify = Arc::clone(&self.notify);
        self.supervisor = Some(Supervisor::spawn(
            command,
            &[],
            Options {
                readiness: ReadinessMode::None,
                ..Options::default()
            },
            move || notify(),
        ));
        self.process = self.supervisor.as_ref().map(Supervisor::state);
    }

    pub fn request_close(&mut self) {
        if self.closing {
            return;
        }
        if self.busy() {
            self.close_dialog = true;
        } else {
            self.overwrite = None;
            self.closing = true;
            self.save();
        }
    }

    pub fn kill_and_close(&mut self) {
        if !self.close_dialog {
            return;
        }
        if let Some(supervisor) = &self.supervisor {
            supervisor.kill();
        }
        self.close_dialog = false;
        self.closing = true;
        (self.notify)();
    }

    pub fn exit_done(&self) -> bool {
        self.closing && !self.busy() && self.saved
    }

    pub fn save(&mut self) {
        if self.saved {
            return;
        }
        let result = self
            .settings_file
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|path| settings::save(path, &self.settings));
        if let Err(error) = result {
            eprintln!("{}", mrtklib_egui_supervisor::mask::mask(&error, &[]));
            self.error = Some(error);
        }
        self.saved = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, time::Instant};

    struct Fixture {
        directory: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "conv-controller-{}",
                mrtklib_egui_supervisor::generate_token()
            ));
            std::fs::create_dir(&directory).unwrap();
            Self { directory }
        }
        fn controller(&self) -> Controller {
            let mut c = Controller::new(|| {});
            c.settings_file = Ok(self.directory.join("gui.json"));
            c.cwd = Ok(self.directory.clone());
            c.settings = Settings {
                input: "test.ubx".into(),
                ..Default::default()
            };
            c.refresh_outputs(true);
            c
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    #[ignore = "subprocess fixture"]
    fn finite_child() {
        eprint!("working\r");
        std::thread::sleep(Duration::from_secs(1));
        eprintln!("done");
    }

    fn until(c: &mut Controller, done: impl Fn(&Controller) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            c.tick();
            if done(c) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out: {}", c.view().message);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn close_wait_preserves_child_and_kill_requires_explicit_close_choice() {
        for kill in [false, true] {
            let fixture = Fixture::new();
            let mut c = fixture.controller();
            c.start(CommandLine {
                program: std::env::current_exe().unwrap(),
                args: [
                    "--exact",
                    "controller::tests::finite_child",
                    "--ignored",
                    "--nocapture",
                ]
                .map(Into::into)
                .to_vec(),
                working_directory: None,
            });
            until(&mut c, |c| {
                c.view().lines.iter().any(|line| line == "working")
            });
            assert!(c.busy());
            c.request_close();
            assert!(c.close_dialog);
            assert!(!c.closing);
            assert!(!c.exit_done());
            assert!(!fixture.directory.join("gui.json").exists());
            if kill {
                c.kill_and_close();
            } else {
                c.close_dialog = false; // Wait action.
                assert!(c.busy());
            }
            until(&mut c, |c| !c.busy());
            assert_eq!(
                c.process.as_ref().unwrap().exit_status.unwrap().success(),
                !kill
            );
            c.request_close();
            c.tick();
            assert!(c.exit_done());
            assert!(fixture.directory.join("gui.json").exists());
        }
    }

    #[test]
    fn normalized_automatic_collision_checks_the_suffixed_output_before_launch() {
        for case in 0..3 {
            let fixture = Fixture::new();
            let mut c = fixture.controller();
            let input = fixture.directory.join("input.obs");
            let output = fixture.directory.join("input.obs_");
            std::fs::write(&input, b"input must survive").unwrap();
            std::fs::write(&output, b"existing output").unwrap();
            c.program = Ok(fixture.directory.join("unused-mrtk"));
            c.settings = Settings {
                input: match case {
                    0 => "input.obs".into(),
                    1 => input.to_str().unwrap().into(),
                    _ => "missing/../input.obs".into(),
                },
                directory_enabled: true,
                directory: match case {
                    0 => fixture.directory.to_str().unwrap().into(),
                    1 => ".".into(),
                    _ => "also-missing/../.".into(),
                },
                ..Default::default()
            };
            c.refresh_outputs(true);
            assert!(c.error.is_none(), "{:?}", c.error);
            c.tick();
            assert!(c.preview().contains(output.to_str().unwrap()));
            c.convert();
            assert!(c.error.is_none(), "{:?}", c.error);
            assert!(c.supervisor.is_none(), "must await overwrite consent");
            let pending = c.overwrite.as_ref().unwrap();
            assert_eq!(pending.paths, std::slice::from_ref(&output));
            assert_eq!(c.preview(), pending.command.preview());
            let args = &pending.command.args;
            let index = args.iter().position(|arg| arg == "-o").unwrap();
            assert_eq!(Path::new(&args[index + 1]), output);
            assert_eq!(Path::new(args.last().unwrap()), input);
            assert_eq!(std::fs::read(input).unwrap(), b"input must survive");
            assert_eq!(std::fs::read(output).unwrap(), b"existing output");
        }
    }

    #[test]
    fn edited_output_alias_of_input_never_starts_a_child() {
        let fixture = Fixture::new();
        let mut c = fixture.controller();
        c.settings.input = "./input.obs".into();
        c.settings.outputs[0].path = "missing/../input.obs".into();
        c.settings.outputs[0].edited = true;
        c.convert();
        assert!(
            c.error
                .as_ref()
                .unwrap()
                .contains("OBS output resolves to the input file")
        );
        assert!(c.overwrite.is_none());
        assert!(c.supervisor.is_none());
        assert_eq!(c.settings.outputs[0].path, "missing/../input.obs");
    }

    #[test]
    fn settings_path_resolution_error_prevents_conversion_without_fallback() {
        let fixture = Fixture::new();
        let mut c = fixture.controller();
        c.settings_file = Err("MRTKLIB_EGUI_SETTINGS_DIR is empty".into());
        c.convert();
        assert!(c.supervisor.is_none());
        assert!(c.error.as_ref().unwrap().contains("is empty"));
    }

    #[test]
    fn overwrite_cancel_and_confirm_use_one_frozen_command() {
        let fixture = Fixture::new();
        let mut c = fixture.controller();
        c.program = Ok(fixture.directory.join("missing-mrtk"));
        std::fs::write(fixture.directory.join("test.obs"), b"original").unwrap();
        c.convert();
        assert_eq!(
            c.overwrite.as_ref().unwrap().paths,
            [fixture.directory.join("test.obs")]
        );
        assert!(c.supervisor.is_none());
        c.overwrite = None; // Cancel action.
        assert_eq!(
            std::fs::read(fixture.directory.join("test.obs")).unwrap(),
            b"original"
        );
        c.convert();
        let expected = c.overwrite.as_ref().unwrap().command.preview();
        c.settings.input = "changed.ubx".into();
        c.confirm_overwrite();
        assert_eq!(c.preview(), expected);
        assert_eq!(c.supervisor.as_ref().unwrap().state().errors.len(), 1);
        // The launch failed, but the pending command itself was kept unchanged.
        assert!(expected.contains("test.ubx"));
        assert!(!c.busy());
        assert!(c.view().message.contains("cannot start child"));
    }

    fn required(name: &str) -> PathBuf {
        let value = std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| panic!("{name} must be set for real_backend_ tests"));
        PathBuf::from(value)
            .canonicalize()
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    #[ignore = "requires MRTKLIB_EGUI_TEST_MRTK and MRTKLIB_EGUI_TEST_MRTKLIB_DIR"]
    fn real_backend_convert_ubx() {
        let program = required("MRTKLIB_EGUI_TEST_MRTK");
        let tree = required("MRTKLIB_EGUI_TEST_MRTKLIB_DIR");
        let directory = std::env::temp_dir().join(format!(
            "conv-real-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut controller = Controller::new(|| {});
        controller.program = Ok(program);
        controller.cwd = Ok(directory.clone());
        controller.settings_file = Ok(directory.join("gui.json"));
        controller.settings = Settings {
            input: tree
                .join("tests/data/rtklib/rcvraw/ubx_20080526.ubx")
                .to_str()
                .unwrap()
                .into(),
            format: 5,
            directory_enabled: true,
            directory: directory.to_str().unwrap().into(),
            ..Default::default()
        };
        for (row, output) in controller.settings.outputs.iter_mut().enumerate() {
            output.enabled = row < 2;
        }
        controller.refresh_outputs(true);
        controller.convert();
        assert!(controller.error.is_none(), "{:?}", controller.error);
        assert!(controller.overwrite.is_none());
        let deadline = Instant::now() + Duration::from_secs(30);
        while controller.busy() {
            assert!(
                Instant::now() < deadline,
                "conversion timed out: {}",
                controller.view().message
            );
            std::thread::sleep(Duration::from_millis(10));
            controller.tick();
        }
        controller.tick();
        assert_eq!(
            controller
                .process
                .as_ref()
                .unwrap()
                .exit_status
                .unwrap()
                .code(),
            Some(0),
            "{}",
            controller.view().lines.join("\n")
        );
        assert!(
            std::fs::metadata(Path::new(&controller.settings.outputs[0].path))
                .unwrap()
                .len()
                > 0
        );
        controller.request_close();
        assert!(controller.exit_done());
        drop(controller);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
