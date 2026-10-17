//! Conv path hints for the shared native dialog worker.
use crate::naming;
pub(crate) use mrtklib_egui_common::file_dialog::{FileDialog, Target};
use std::path::{Path, PathBuf};

/// Dialog hints must be absolute too. An empty hint uses the OS dialog's default;
/// browsing can replace an invalid relative input without trying to resolve it.
pub(crate) fn hint(
    target: Target,
    value: &str,
    input: &str,
    home: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let expanded = naming::expand_home(value, home)?;
    if expanded.is_absolute() {
        return naming::normalize_absolute(&expanded).map(Some);
    }
    if matches!(target, Target::Input) {
        return Ok(None);
    }
    let input = naming::input_path(input, home)?;
    naming::output_path(value, &input, home).map(Some)
}

pub(crate) fn selected_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Selected path cannot be represented as UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dialog_hints_use_the_input_directory_or_home_and_never_a_relative_directory() {
        let home = std::env::temp_dir().join("conv-dialog-home");
        let input = home.join("data/in.ubx");
        let input = input.to_str().unwrap();
        for target in [Target::Directory, Target::Output(0)] {
            assert_eq!(
                hint(target, "../out", input, Some(&home)).unwrap(),
                Some(home.join("out"))
            );
            assert_eq!(
                hint(target, "~/out", input, Some(&home)).unwrap(),
                Some(home.join("out"))
            );
            assert!(
                hint(target, "~/out", input, None)
                    .unwrap_err()
                    .contains("home directory is unavailable")
            );
        }
        assert_eq!(
            hint(Target::Input, "~/in.ubx", "", Some(&home)).unwrap(),
            Some(home.join("in.ubx"))
        );
        assert!(
            hint(Target::Input, "relative.ubx", "", None)
                .unwrap()
                .is_none()
        );
        assert!(hint(Target::Directory, "", "", None).unwrap().is_none());
    }
}
