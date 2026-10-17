//! Resolve conversion paths once, without filesystem access or GNSS keyword expansion.

use crate::settings::{OUTPUTS, Settings};
use std::path::{Component, Path, PathBuf};

/// Absolute lexical normalization: aliases through links are deliberately not resolved.
pub(crate) fn normalize_absolute(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("An absolute path is required: {}", path.display()));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
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

pub(crate) fn expand_home(value: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let suffix = if value == "~" {
        Some("")
    } else {
        value.strip_prefix("~/")
    };
    match suffix {
        Some(suffix) => {
            let home = home.ok_or("Cannot expand ~: the home directory is unavailable")?;
            // Keep a second slash from replacing the home directory in Path::join.
            Ok(home.join(suffix.trim_start_matches('/')))
        }
        None => Ok(value.into()),
    }
}

pub(crate) fn input_path(value: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    if value.is_empty() {
        return Err("Select an input file".into());
    }
    let path = expand_home(value, home)?;
    if !path.is_absolute() {
        return Err(
            "Input file must be an absolute path or ~/...; relative inputs are not supported"
                .into(),
        );
    }
    normalize_absolute(&path)
}

/// The input is already resolved; relative outputs always use its parent directory.
pub(crate) fn output_path(
    value: &str,
    input: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let path = expand_home(value, home)?;
    normalize_absolute(&input.parent().unwrap_or(input).join(path))
}

/// One set of paths for automatic naming, collision checks, preview and execution.
/// Empty edited output fields stay absent; disabled slots are selected by the caller.
pub(crate) struct ResolvedPaths {
    pub input: PathBuf,
    pub outputs: [Option<PathBuf>; 9],
}

impl ResolvedPaths {
    pub fn new(settings: &Settings, home: Option<&Path>) -> Result<Self, String> {
        let input = input_path(&settings.input, home)?;
        let directory = if settings.directory_enabled {
            output_path(&settings.directory, &input, home)?
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
                    Some(output_path(&settings.outputs[row].path, &input, home)?)
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

    pub fn refresh_outputs(
        &mut self,
        input_changed: bool,
        home: Option<&Path>,
    ) -> Result<(), String> {
        if input_changed {
            for row in &mut self.outputs {
                row.edited = false;
            }
        }
        let paths = if self.input.is_empty() {
            std::array::from_fn(|_| None)
        } else {
            ResolvedPaths::new(self, home)?.outputs
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

    fn root() -> PathBuf {
        std::env::temp_dir().join("conv-naming")
    }
    fn expected(path: &str) -> String {
        root().join(path).to_str().unwrap().to_owned()
    }
    fn fixture(input: &str) -> Settings {
        Settings {
            input: expected(input),
            ..Default::default()
        }
    }
    fn paths(settings: &Settings) -> [String; 9] {
        ResolvedPaths::new(settings, None)
            .unwrap()
            .outputs
            .map(|path| path.unwrap().to_str().unwrap().to_owned())
    }

    #[test]
    fn absolute_paths_are_lexically_normalized_without_filesystem_access() {
        for path in [
            "input.obs",
            "./input.obs",
            "unused/../input.obs",
            "unused/./nested/../../input.obs",
        ] {
            assert_eq!(
                normalize_absolute(&root().join(path)).unwrap(),
                root().join("input.obs")
            );
        }
        assert!(normalize_absolute(Path::new("relative")).is_err());
        let root = root();
        let filesystem_root = root.ancestors().last().unwrap();
        assert_eq!(
            normalize_absolute(&filesystem_root.join("../../input.obs")).unwrap(),
            filesystem_root.join("input.obs")
        );
    }

    #[test]
    fn relative_inputs_are_refused_without_consulting_the_current_directory() {
        for input in [
            "input.obs",
            "./input.obs",
            "missing/../input.obs",
            "~someone/in.ubx",
        ] {
            let s = Settings {
                input: input.into(),
                ..Default::default()
            };
            let error = ResolvedPaths::new(&s, Some(&root())).err().unwrap();
            assert!(error.contains("absolute path or ~/..."), "{error}");
        }
    }

    #[test]
    fn outputs_use_the_input_directory_and_never_the_current_directory() {
        // Neither the resolver nor naming takes a current-directory argument. Changing
        // the selected output directory must not change the base of an edited path.
        let mut s = fixture("data/in.ubx");
        s.directory_enabled = true;
        s.outputs[1].edited = true;
        s.outputs[1].path = "edited/../custom.nav".into();
        for (directory, obs) in [
            ("out", "data/out/in.obs"),
            ("../out", "out/in.obs"),
            (".", "data/in.obs"),
            ("unused/../out", "data/out/in.obs"),
        ] {
            s.directory = directory.into();
            assert_eq!(paths(&s)[0], expected(obs));
            assert_eq!(paths(&s)[1], expected("data/custom.nav"));
        }
    }

    #[test]
    fn home_expansion_applies_to_input_directory_and_edited_paths() {
        let home = root().join("home");
        let mut s = Settings {
            input: "~/in.ubx".into(),
            directory_enabled: true,
            ..Default::default()
        };
        for directory in ["~/out", "~"] {
            s.directory = directory.into();
            s.outputs[1].edited = true;
            s.outputs[1].path = "~/nav/../custom.nav".into();
            let resolved = ResolvedPaths::new(&s, Some(&home)).unwrap();
            assert_eq!(resolved.input, home.join("in.ubx"));
            let output_directory = if directory == "~" {
                home.clone()
            } else {
                home.join("out")
            };
            assert_eq!(resolved.outputs[0], Some(output_directory.join("in.obs")));
            assert_eq!(resolved.outputs[1], Some(home.join("custom.nav")));
        }
        s.outputs[1].path = "~".into();
        assert_eq!(
            ResolvedPaths::new(&s, Some(&home)).unwrap().outputs[1],
            Some(home.clone())
        );
        assert_eq!(input_path("~", Some(&home)).unwrap(), home);
        // ~user is a literal directory for outputs, never an account lookup.
        assert_eq!(
            output_path("~other/out", &root().join("in.ubx"), Some(&home)).unwrap(),
            root().join("~other/out")
        );
    }

    #[test]
    fn missing_home_is_an_error_only_when_expansion_is_needed() {
        let mut s = fixture("in.ubx");
        assert!(ResolvedPaths::new(&s, None).is_ok());
        for field in [0, 1, 2] {
            let mut s = s.clone();
            match field {
                0 => s.input = "~/in.ubx".into(),
                1 => {
                    s.directory_enabled = true;
                    s.directory = "~".into();
                }
                _ => {
                    s.outputs[0].edited = true;
                    s.outputs[0].path = "~/out.obs".into();
                }
            }
            assert!(
                ResolvedPaths::new(&s, None)
                    .err()
                    .unwrap()
                    .contains("home directory is unavailable")
            );
        }
        s.directory = "~".into(); // An unchecked directory is not used.
        assert!(ResolvedPaths::new(&s, None).is_ok());
    }

    #[test]
    fn automatic_collision_uses_resolved_input_and_output_directory() {
        for directory in [expected("."), ".".into(), "also-missing/../.".into()] {
            let mut s = fixture("missing/../input.obs");
            s.directory_enabled = true;
            s.directory = directory;
            let resolved = ResolvedPaths::new(&s, None).unwrap();
            assert_eq!(resolved.input, root().join("input.obs"));
            assert_eq!(resolved.outputs[0], Some(root().join("input.obs_")));
            assert!(resolved.outputs.iter().flatten().all(|p| {
                p.is_absolute()
                    && p.components()
                        .all(|c| !matches!(c, Component::CurDir | Component::ParentDir))
            }));
            s.refresh_outputs(true, None).unwrap();
            assert_eq!(s.outputs[0].path, expected("input.obs_"));
            assert_eq!(
                ResolvedPaths::new(&s, None).unwrap().outputs,
                resolved.outputs
            );
        }
    }

    #[test]
    fn enabled_edited_output_equal_to_input_is_rejected_without_renaming() {
        for (row, (label, _, _)) in OUTPUTS.iter().enumerate() {
            let mut s = fixture("input.obs");
            s.options.version = "2.12".into();
            s.outputs[row].path = "sub/.././input.obs".into();
            s.outputs[row].edited = true;
            let error = ResolvedPaths::new(&s, None).err().unwrap();
            assert!(error.contains(label) && error.contains("output resolves to the input file"));
            assert_eq!(s.outputs[row].path, "sub/.././input.obs");
            s.outputs[row].enabled = false;
            assert_eq!(
                ResolvedPaths::new(&s, None).unwrap().outputs[row],
                Some(root().join("input.obs"))
            );
        }
        let mut s = fixture("input.obs");
        s.outputs[2].path = s.input.clone();
        s.outputs[2].edited = true;
        assert!(
            ResolvedPaths::new(&s, None).is_ok(),
            "unavailable RINEX 3 slots are not enabled outputs"
        );
    }

    #[test]
    fn extensions_wildcards_directory_and_input_collision() {
        let mut s = fixture("data/a*b?.raw.ubx");
        assert_eq!(paths(&s)[0], expected("data/a0b0.raw.obs"));
        s.input = expected("data/input");
        assert_eq!(paths(&s)[1], expected("data/input.nav"));
        s.input = expected("data/.ubx");
        assert_eq!(paths(&s)[0], expected("data/.obs"));
        s.input = expected("data/input.obs");
        assert_eq!(paths(&s)[0], expected("data/input.obs_"));
        s.directory_enabled = true;
        s.directory = "out".into();
        assert_eq!(paths(&s)[0], expected("data/out/input.obs"));
    }

    #[test]
    fn row_rules_templates_and_edited_paths() {
        let mut s = fixture("data/test.ubx");
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
        assert_eq!(paths(&s)[0], expected("data/elsewhere/%r%n0.%yO"));
        s.outputs[0].path = "custom.obs".into();
        s.outputs[0].edited = true;
        s.refresh_outputs(false, None).unwrap();
        assert_eq!(s.outputs[0].path, "custom.obs");
        s.input = expected("new.ubx");
        s.refresh_outputs(true, None).unwrap();
        assert_eq!(s.outputs[0].path, expected("elsewhere/%r%n0.%yO"));
        assert!(!s.outputs[0].edited);
    }
}
