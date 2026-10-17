//! RTKCONV automatic output naming. No GNSS data or time keywords are decoded.

use crate::settings::{OUTPUTS, Settings};
use std::path::{Path, PathBuf};

pub(crate) fn absolute(path: &Path, directory: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        directory.join(path)
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

    pub fn refresh_outputs(&mut self, input_changed: bool) {
        let paths = automatic_paths(self);
        for (row, path) in self.outputs.iter_mut().zip(paths) {
            if input_changed {
                row.edited = false;
            }
            if !row.edited {
                row.path = path;
            }
        }
    }
}

pub(crate) fn automatic_paths(settings: &Settings) -> [String; 9] {
    if settings.input.is_empty() {
        return Default::default();
    }
    let input = Path::new(&settings.input);
    let directory = if settings.directory_enabled {
        Path::new(&settings.directory)
    } else {
        input.parent().unwrap_or(Path::new(""))
    };
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
        path.to_string_lossy().into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extensions_wildcards_directory_and_input_collision() {
        let mut s = Settings {
            input: "data/a*b?.raw.ubx".into(),
            ..Default::default()
        };
        assert_eq!(automatic_paths(&s)[0], "data/a0b0.raw.obs");
        s.input = "data/input".into();
        assert_eq!(automatic_paths(&s)[1], "data/input.nav");
        s.input = "data/.ubx".into();
        assert_eq!(automatic_paths(&s)[0], "data/.obs");
        s.input = "data/input.obs".into();
        assert_eq!(automatic_paths(&s)[0], "data/input.obs_");
        s.directory_enabled = true;
        s.directory = "out".into();
        assert_eq!(automatic_paths(&s)[0], "out/input.obs");
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
            automatic_paths(&s),
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
            .map(|p| format!("data/{p}"))
        );
        s.options.version = "3.04".into();
        assert_eq!(automatic_paths(&s)[1], "data/%r%n0.%yP");
        s.options.systems = [true, false, false, false, false, false, false];
        assert_eq!(automatic_paths(&s)[1], "data/%r%n0.%yN");
        s.directory_enabled = true;
        s.directory = "elsewhere".into();
        assert_eq!(automatic_paths(&s)[0], "elsewhere/%r%n0.%yO");
        s.outputs[0].path = "custom.obs".into();
        s.outputs[0].edited = true;
        s.refresh_outputs(false);
        assert_eq!(s.outputs[0].path, "custom.obs");
        s.input = "new.ubx".into();
        s.refresh_outputs(true);
        assert_eq!(s.outputs[0].path, "elsewhere/%r%n0.%yO");
        assert!(!s.outputs[0].edited);
    }
}
