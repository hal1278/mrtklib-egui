//! Resolve conversion paths once, without filesystem access or GNSS keyword expansion.

use crate::settings::{OUTPUTS, Settings};
use std::path::{Component, Path, PathBuf};

/// Absolute lexical normalization: aliases through links are deliberately not resolved.
pub(crate) fn absolute(path: &Path, directory: &Path) -> Result<PathBuf, String> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        directory.join(path)
    };
    if !joined.is_absolute() {
        return Err(format!(
            "cannot resolve {} against {} to an absolute path",
            path.display(),
            directory.display()
        ));
    }
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Parent components at the filesystem root stay at that root.
                if normalized.file_name().is_some() {
                    normalized.pop();
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

/// One set of paths for automatic naming, collision checks, preview and execution.
/// Empty edited output fields stay absent; disabled slots are selected by the caller.
pub(crate) struct ResolvedPaths {
    pub input: PathBuf,
    pub outputs: [Option<PathBuf>; 9],
}

impl ResolvedPaths {
    pub fn new(settings: &Settings, cwd: &Path) -> Result<Self, String> {
        if settings.input.is_empty() {
            return Err("Select an input file".into());
        }
        let input = absolute(Path::new(&settings.input), cwd)?;
        let directory = if settings.directory_enabled {
            absolute(Path::new(&settings.directory), cwd)?
        } else {
            input.parent().unwrap_or(&input).to_path_buf()
        };
        let automatic = automatic_paths(settings, &input, &directory);
        let mut outputs = std::array::from_fn(|_| None);
        for (row, output) in outputs.iter_mut().enumerate() {
            *output = if settings.outputs[row].edited {
                if settings.outputs[row].path.is_empty() {
                    None
                } else {
                    Some(absolute(Path::new(&settings.outputs[row].path), cwd)?)
                }
            } else {
                Some(automatic[row].clone())
            };
            if settings.output_enabled(row) && output.as_ref() == Some(&input) {
                return Err(format!(
                    "{} output resolves to the input file; choose a different output path",
                    OUTPUTS[row].0
                ));
            }
        }
        Ok(Self { input, outputs })
    }
}

impl Settings {
    pub fn row_available(&self, row: usize) -> bool {
        match row {
            2..=7 => self.options.version.starts_with('2'),
            8 => self.format != 14,
            _ => true,
        }
    }

    pub fn output_enabled(&self, row: usize) -> bool {
        self.row_available(row) && self.outputs[row].enabled
    }

    pub fn refresh_outputs(&mut self, input_changed: bool, cwd: &Path) -> Result<(), String> {
        if input_changed {
            for row in &mut self.outputs {
                row.edited = false;
            }
        }
        let paths = if self.input.is_empty() {
            std::array::from_fn(|_| None)
        } else {
            ResolvedPaths::new(self, cwd)?.outputs
        };
        for (row, path) in self.outputs.iter_mut().zip(paths) {
            if !row.edited {
                row.path = path.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
            }
        }
        Ok(())
    }
}

fn automatic_paths(settings: &Settings, input: &Path, directory: &Path) -> [PathBuf; 9] {
    let base = input
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(['*', '?'], "0");
    let stem = base
        .rsplit_once('.')
        .map_or(base.as_str(), |(stem, _)| stem);
    std::array::from_fn(|row| {
        let mut path = if settings.options.rinex2_name {
            let gps_only =
                settings.options.systems == [true, false, false, false, false, false, false];
            let suffix = match row {
                0 => "%r%n0.%yO",
                1 if settings.options.version.starts_with('2') || gps_only => "%r%n0.%yN",
                1 => "%r%n0.%yP",
                2 => "%r%n0.%yG",
                3 => "%r%n0.%yH",
                4 => "%r%n0.%yQ",
                5 => "%r%n0.%yL",
                6 => "%r%n0.%yC",
                7 => "%r%n0.%yI",
                _ => "%r%n0_%y.sbs",
            };
            directory.join(suffix)
        } else {
            directory.join(format!("{stem}.{}", OUTPUTS[row].2))
        };
        if path == input {
            let mut value = path.into_os_string();
            value.push("_");
            path = value.into();
        }
        path
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cwd() -> PathBuf {
        std::env::temp_dir().join("conv-naming")
    }
    fn paths(settings: &Settings) -> [String; 9] {
        ResolvedPaths::new(settings, &cwd())
            .unwrap()
            .outputs
            .map(|path| path.unwrap().to_string_lossy().into_owned())
    }
    fn expected(path: &str) -> String {
        cwd().join(path).to_string_lossy().into_owned()
    }

    #[test]
    fn absolute_paths_are_lexically_normalized_without_filesystem_access() {
        let cwd = cwd();
        for path in [
            "input.obs",
            "./input.obs",
            "unused/../input.obs",
            "unused/./nested/../../input.obs",
        ] {
            assert_eq!(
                absolute(Path::new(path), &cwd).unwrap(),
                cwd.join("input.obs")
            );
        }
        assert_eq!(
            absolute(&cwd.join("unused/../input.obs"), &cwd).unwrap(),
            cwd.join("input.obs")
        );
        assert_eq!(
            absolute(Path::new("../input.obs"), &cwd).unwrap(),
            cwd.parent().unwrap().join("input.obs")
        );
        let root = cwd.ancestors().last().unwrap();
        assert_eq!(
            absolute(Path::new("../../input.obs"), root).unwrap(),
            root.join("input.obs")
        );
    }

    #[test]
    fn automatic_collision_uses_resolved_input_and_output_directory() {
        let cwd = cwd();
        let input = cwd.join("input.obs");
        for (input_text, directory) in [
            ("input.obs".to_owned(), cwd.to_str().unwrap().to_owned()),
            (input.to_str().unwrap().to_owned(), ".".to_owned()),
            (
                "missing/../input.obs".to_owned(),
                "also-missing/../.".to_owned(),
            ),
        ] {
            let mut s = Settings {
                input: input_text,
                directory_enabled: true,
                directory,
                ..Default::default()
            };
            let resolved = ResolvedPaths::new(&s, &cwd).unwrap();
            assert_eq!(resolved.input, input);
            assert_eq!(
                resolved.outputs[0].as_ref().unwrap(),
                &cwd.join("input.obs_")
            );
            assert!(resolved.outputs.iter().flatten().all(|p| {
                p.is_absolute()
                    && p.components()
                        .all(|c| !matches!(c, Component::CurDir | Component::ParentDir))
            }));
            s.refresh_outputs(true, &cwd).unwrap();
            assert_eq!(Path::new(&s.outputs[0].path), cwd.join("input.obs_"));
            assert_eq!(
                ResolvedPaths::new(&s, &cwd).unwrap().outputs,
                resolved.outputs
            );
        }
    }

    #[test]
    fn enabled_edited_output_equal_to_input_is_rejected_without_renaming() {
        for (row, (label, _, _)) in OUTPUTS.iter().enumerate() {
            let mut s = Settings {
                input: "input.obs".into(),
                ..Default::default()
            };
            s.options.version = "2.12".into();
            s.outputs[row].path = "sub/.././input.obs".into();
            s.outputs[row].edited = true;
            let error = ResolvedPaths::new(&s, &cwd()).err().unwrap();
            assert!(error.contains(label) && error.contains("output resolves to the input file"));
            assert_eq!(s.outputs[row].path, "sub/.././input.obs");
            s.outputs[row].enabled = false;
            assert_eq!(
                ResolvedPaths::new(&s, &cwd()).unwrap().outputs[row],
                Some(cwd().join("input.obs"))
            );
        }
        let mut s = Settings {
            input: "input.obs".into(),
            ..Default::default()
        };
        s.outputs[2].path = s.input.clone();
        s.outputs[2].edited = true;
        assert!(
            ResolvedPaths::new(&s, &cwd()).is_ok(),
            "unavailable RINEX 3 slots are not enabled outputs"
        );
    }

    #[test]
    fn extensions_wildcards_directory_and_input_collision() {
        let mut s = Settings {
            input: "data/a*b?.raw.ubx".into(),
            ..Default::default()
        };
        assert_eq!(paths(&s)[0], expected("data/a0b0.raw.obs"));
        s.input = "data/input".into();
        assert_eq!(paths(&s)[1], expected("data/input.nav"));
        s.input = "data/.ubx".into();
        assert_eq!(paths(&s)[0], expected("data/.obs"));
        s.input = "data/input.obs".into();
        assert_eq!(paths(&s)[0], expected("data/input.obs_"));
        s.directory_enabled = true;
        s.directory = "out".into();
        assert_eq!(paths(&s)[0], expected("out/input.obs"));
    }
    #[test]
    fn row_rules_templates_and_edited_paths() {
        let mut s = Settings {
            input: "data/test.ubx".into(),
            ..Default::default()
        };
        assert!((2..8).all(|row| !s.output_enabled(row)));
        s.options.version = "2.12".into();
        assert!((0..9).all(|row| s.output_enabled(row)));
        s.format = 14;
        assert!(!s.output_enabled(8));
        s.options.rinex2_name = true;
        assert_eq!(
            paths(&s),
            [
                "%r%n0.%yO",
                "%r%n0.%yN",
                "%r%n0.%yG",
                "%r%n0.%yH",
                "%r%n0.%yQ",
                "%r%n0.%yL",
                "%r%n0.%yC",
                "%r%n0.%yI",
                "%r%n0_%y.sbs"
            ]
            .map(|p| expected(&format!("data/{p}")))
        );
        s.options.version = "3.04".into();
        assert_eq!(paths(&s)[1], expected("data/%r%n0.%yP"));
        s.options.systems = [true, false, false, false, false, false, false];
        assert_eq!(paths(&s)[1], expected("data/%r%n0.%yN"));
        s.directory_enabled = true;
        s.directory = "elsewhere".into();
        assert_eq!(paths(&s)[0], expected("elsewhere/%r%n0.%yO"));
        s.outputs[0].path = "custom.obs".into();
        s.outputs[0].edited = true;
        s.refresh_outputs(false, &cwd()).unwrap();
        assert_eq!(s.outputs[0].path, "custom.obs");
        s.input = "new.ubx".into();
        s.refresh_outputs(true, &cwd()).unwrap();
        assert_eq!(s.outputs[0].path, expected("elsewhere/%r%n0.%yO"));
        assert!(!s.outputs[0].edited);
    }
}
