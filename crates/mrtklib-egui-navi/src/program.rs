//! Resolve the backend without searching PATH; preview and execution share one argv.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use mrtklib_egui_common::paths::{resolve_program, settings_directory};
use mrtklib_egui_supervisor::CommandLine;

pub(crate) fn command_line() -> Result<CommandLine, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("cannot locate GUI: {error}"))?;
    let program = resolve_program(
        std::env::var_os("MRTKLIB_EGUI_MRTK"),
        || Ok(executable.clone()),
        cfg!(windows),
    )?;
    let settings = settings_directory(std::env::var_os("MRTKLIB_EGUI_SETTINGS_DIR"), &executable)?;
    run_command(program, settings, &executable)
}

pub(crate) fn run_command(
    program: PathBuf,
    settings: PathBuf,
    executable: &Path,
) -> Result<CommandLine, String> {
    let stem = executable
        .file_stem()
        .ok_or("GUI executable has no file stem")?;
    let mut filename = stem.to_os_string();
    filename.push(".toml");
    Ok(CommandLine {
        program,
        args: [
            "run",
            "--rpc-port",
            "0",
            "--no-console",
            "--exit-on-stdin-eof",
            "--config",
        ]
        .map(OsString::from)
        .into_iter()
        .chain([settings.join(filename).into_os_string()])
        .collect(),
        working_directory: Some(settings),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_override_and_explicit_configuration() {
        let exe = Path::new("/suite/mrtklib-egui-navi.exe");
        assert_eq!(settings_directory(None, exe).unwrap(), Path::new("/suite"));
        assert_eq!(
            settings_directory(Some("settings".into()), exe).unwrap(),
            std::env::current_dir().unwrap().join("settings")
        );
        assert!(
            settings_directory(Some(OsString::new()), exe)
                .unwrap_err()
                .contains("is empty")
        );
        let command = run_command("/dev/mrtk".into(), "/settings dir".into(), exe).unwrap();
        assert_eq!(
            command.working_directory.as_deref(),
            Some(Path::new("/settings dir"))
        );
        assert_eq!(
            command.args.last().unwrap(),
            "/settings dir/mrtklib-egui-navi.toml"
        );
        assert_eq!(
            command.preview(),
            "Working directory: \"/settings dir\" | Program: /dev/mrtk run --rpc-port 0 --no-console --exit-on-stdin-eof --config \"/settings dir/mrtklib-egui-navi.toml\""
        );
    }
}
