//! Resolve application paths without PATH search or fallback locations.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub fn resolve_program(
    override_path: Option<OsString>,
    executable: impl FnOnce() -> Result<PathBuf, String>,
    windows: bool,
) -> Result<PathBuf, String> {
    // An override does not require current_exe to succeed.
    match override_path {
        Some(path) => {
            if path.is_empty() {
                return Err("MRTKLIB_EGUI_MRTK is empty".into());
            }
            let path = PathBuf::from(path);
            if path.is_absolute() {
                Ok(path)
            } else {
                // A bare filename must also be a path, not a PATH lookup.
                Ok(std::env::current_dir()
                    .map_err(|error| format!("cannot resolve development executable: {error}"))?
                    .join(path))
            }
        }
        None => bundled_program(&executable()?, windows),
    }
}

fn bundled_program(executable: &Path, windows: bool) -> Result<PathBuf, String> {
    let directory = executable
        .parent()
        .ok_or_else(|| "GUI executable has no parent directory".to_string())?;
    Ok(directory.join(if windows { "mrtk.exe" } else { "mrtk" }))
}

pub fn settings_directory(
    override_path: Option<OsString>,
    executable: &Path,
) -> Result<PathBuf, String> {
    match override_path {
        Some(path) => {
            if path.is_empty() {
                return Err("MRTKLIB_EGUI_SETTINGS_DIR is empty".into());
            }
            let path = PathBuf::from(path);
            if path.is_absolute() {
                Ok(path)
            } else {
                Ok(std::env::current_dir()
                    .map_err(|error| format!("cannot resolve settings directory: {error}"))?
                    .join(path))
            }
        }
        None => executable
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "GUI executable has no parent directory".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_override_wins_without_looking_up_gui() {
        assert_eq!(
            resolve_program(
                Some("/development/mock".into()),
                || panic!("override must win"),
                false
            )
            .unwrap(),
            Path::new("/development/mock")
        );
        assert_eq!(
            resolve_program(None, || Err("cannot locate GUI".into()), false).unwrap_err(),
            "cannot locate GUI"
        );
        assert_eq!(
            resolve_program(None, || Ok("/suite/gui".into()), false).unwrap(),
            Path::new("/suite/mrtk")
        );
    }

    #[test]
    fn relative_override_is_a_path_and_empty_override_is_an_error() {
        assert_eq!(
            resolve_program(Some("mock".into()), || panic!("override must win"), false).unwrap(),
            std::env::current_dir().unwrap().join("mock")
        );
        assert_eq!(
            resolve_program(
                Some(OsString::new()),
                || panic!("no silent fallback"),
                false
            )
            .unwrap_err(),
            "MRTKLIB_EGUI_MRTK is empty"
        );
    }

    #[test]
    fn bundled_program_is_next_to_gui_for_both_platforms() {
        let executable = Path::new("suite with spaces/gui");
        assert_eq!(
            bundled_program(executable, false).unwrap(),
            Path::new("suite with spaces/mrtk")
        );
        assert_eq!(
            bundled_program(executable, true).unwrap(),
            Path::new("suite with spaces/mrtk.exe")
        );
    }

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
    }
}
