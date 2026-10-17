//! GUI JSON settings, independent of MRTKLIB configuration.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const PLOTS: [&str; 3] = ["Gnd Trk", "Position", "NSat"];
pub(crate) const ORIGINS: [&str; 6] = [
    "Start Pos",
    "End Pos",
    "Average Pos",
    "Linear Fit Pos",
    "Base Station",
    "Lat/Lon/Hgt",
];
pub(crate) const SIZES: [u32; 8] = [1, 2, 3, 4, 5, 10, 15, 20];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Options {
    pub week_time: bool,
    pub auto_fit: bool,
    pub dms: bool,
    pub statistics: bool,
    pub style: usize,
    pub mark_size: u32,
    pub origin: usize,
    pub llh: [f64; 3],
    pub y_range: f64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            week_time: false,
            auto_fit: true,
            dms: false,
            statistics: false,
            style: 0,
            mark_size: 2,
            origin: 2,
            llh: [0.0; 3],
            y_range: 5.0,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if self.origin >= ORIGINS.len() || self.style > 2 || !SIZES.contains(&self.mark_size) {
            return Err("invalid plot option selection".into());
        }
        if !self.y_range.is_finite() || self.y_range <= 0.0 {
            return Err("Y-range must be positive and finite".into());
        }
        if self.llh.iter().any(|v| !v.is_finite())
            || self.llh[0].abs() > 90.0
            || self.llh[1].abs() > 180.0
        {
            return Err("origin requires latitude ±90°, longitude ±180°, and finite height".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub last_file: Option<PathBuf>,
    pub plot: usize,
    pub components: [bool; 3],
    pub quality: u8,
    pub options: Options,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            last_file: None,
            plot: 0,
            components: [true; 3],
            quality: 0,
            options: Options::default(),
        }
    }
}
pub(crate) fn settings_file(executable: &Path) -> Result<PathBuf, String> {
    let directory = mrtklib_egui_common::paths::settings_directory(
        std::env::var_os("MRTKLIB_EGUI_SETTINGS_DIR"),
        executable,
    )?;
    let stem = executable
        .file_stem()
        .ok_or("GUI executable has no file stem")?;
    let mut filename = stem.to_os_string();
    filename.push(".json");
    Ok(directory.join(filename))
}
pub(crate) fn load(path: &Path) -> Result<Settings, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(format!("cannot read settings {}: {e}", path.display())),
    };
    let result = (|| {
        let settings: Settings = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        settings.options.validate()?;
        if settings.plot >= PLOTS.len() || settings.quality > 6 {
            return Err("invalid plot or quality selection".into());
        }
        Ok(settings)
    })();
    result.map_err(|e: String| format!("cannot read settings {}: {e}", path.display()))
}
pub(crate) fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("cannot save settings {}: {e}", path.display()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_and_errors() {
        let dir = std::env::temp_dir().join(format!(
            "plot-settings-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("plot.json");
        assert_eq!(load(&path).unwrap(), Settings::default());
        let s = Settings {
            last_file: Some("入力.pos".into()),
            plot: 2,
            quality: 6,
            options: Options {
                origin: 5,
                auto_fit: false,
                llh: [35.0, 139.0, 10.0],
                week_time: true,
                ..Default::default()
            },
            ..Default::default()
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path).unwrap(), s);
        for text in ["{bad", r#"{"plot":9}"#, r#"{"options":{"y_range":0}}"#] {
            std::fs::write(&path, text).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
