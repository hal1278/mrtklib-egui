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
pub(crate) const GRID_FORMATS: [&str; 5] =
    ["OFF", "Grid", "Grid/Label", "Circles", "Circles/Label"];
pub(crate) const TEXT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 6.0..=30.0;
pub(crate) const SIZES: [u32; 8] = [1, 2, 3, 4, 5, 10, 15, 20];
/// RTKPLOT's RangeList labels and YRange values in metres.
pub(crate) const ORIGIN_RANGES: [(&str, f64); 23] = [
    ("5cm", 0.05),
    ("10cm", 0.1),
    ("20cm", 0.2),
    ("50cm", 0.5),
    ("1m", 1.0),
    ("2m", 2.0),
    ("5m", 5.0),
    ("10m", 10.0),
    ("20m", 20.0),
    ("50m", 50.0),
    ("100m", 100.0),
    ("200m", 200.0),
    ("500m", 500.0),
    ("1km", 1_000.0),
    ("2km", 2_000.0),
    ("5km", 5_000.0),
    ("10km", 10_000.0),
    ("20km", 20_000.0),
    ("50km", 50_000.0),
    ("100km", 100_000.0),
    ("200km", 200_000.0),
    ("500km", 500_000.0),
    ("1000km", 1_000_000.0),
];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Options {
    pub week_time: bool,
    /// `h:m:s UTC` instead of `h:m:s GPST` when `week_time` is off.
    pub utc: bool,
    pub auto_fit: bool,
    pub dms: bool,
    pub statistics: bool,
    pub show_g_label: usize,
    pub grid_label_size: f32,
    pub statistics_size: f32,
    pub style: usize,
    pub mark_size: u32,
    pub origin: usize,
    pub llh: [f64; 3],
    pub y_range: f64,
    pub anim_cycle: i32,
    pub ref_cycle: u32,
    pub tile_url: String,
    pub tile_attribution: String,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            week_time: false,
            utc: false,
            auto_fit: true,
            dms: false,
            statistics: false,
            show_g_label: 1,
            grid_label_size: 9.0,
            statistics_size: 12.0,
            style: 0,
            mark_size: 2,
            origin: 2,
            llh: [0.0; 3],
            y_range: 5.0,
            anim_cycle: 10,
            ref_cycle: 100,
            tile_url: String::new(),
            tile_attribution: String::new(),
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if self.origin >= ORIGINS.len()
            || self.style > 2
            || self.show_g_label >= GRID_FORMATS.len()
            || !SIZES.contains(&self.mark_size)
        {
            return Err("invalid plot option selection".into());
        }
        if !TEXT_SIZE_RANGE.contains(&self.grid_label_size)
            || !TEXT_SIZE_RANGE.contains(&self.statistics_size)
        {
            return Err("Grid label and statistics sizes must be between 6 and 30 pt".into());
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
        if !self.tile_url.is_empty()
            && (!(self.tile_url.starts_with("https://") || self.tile_url.starts_with("http://"))
                || !["{z}", "{x}", "{y}"]
                    .iter()
                    .all(|field| self.tile_url.contains(field)))
        {
            return Err("Tile URL requires http:// or https:// and {z}, {x}, {y}".into());
        }
        Ok(())
    }
}
/// rtktrace's indexed visibility mask; All includes the unknown quality endpoints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct QualityFilter(pub [bool; 8]);
impl Default for QualityFilter {
    fn default() -> Self {
        Self([true; 8])
    }
}
impl QualityFilter {
    pub fn visible(&self, quality: u8) -> bool {
        self.0.get(usize::from(quality)).copied().unwrap_or(false)
    }
    pub fn toggle_all(&mut self) {
        let enable = !self.0.iter().all(|visible| *visible);
        self.0.fill(enable);
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ManualRate {
    pub path: PathBuf,
    pub hz: f64,
    #[serde(default)]
    pub entered: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub last_file: Option<PathBuf>,
    pub last_file2: Option<PathBuf>,
    pub manual_rates: Vec<ManualRate>,
    pub plot: usize,
    pub components: [bool; 3],
    pub qualities: QualityFilter,
    pub interval_percentages: bool,
    pub height: bool,
    pub cursor_llh: bool,
    pub map: bool,
    pub show_grid: bool,
    pub options: Options,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            last_file: None,
            last_file2: None,
            manual_rates: vec![],
            plot: 0,
            components: [true; 3],
            qualities: QualityFilter::default(),
            interval_percentages: false,
            height: false,
            cursor_llh: false,
            map: false,
            show_grid: true,
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
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let mut settings: Settings =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if value.get("qualities").is_none()
            && let Some(old) = value.get("quality")
        {
            let q = old
                .as_u64()
                .filter(|q| *q <= 6)
                .ok_or("invalid quality selection")?;
            settings.qualities = QualityFilter(std::array::from_fn(|i| q == 0 || i as u64 == q));
        }
        settings.options.validate()?;
        let mut paths = std::collections::BTreeSet::new();
        if settings.manual_rates.iter().any(|r| {
            !r.hz.is_finite()
                || r.hz <= 0.0
                || r.entered
                    .as_ref()
                    .is_some_and(|text| text.parse::<f64>().ok() != Some(r.hz))
                || !paths.insert(&r.path)
        }) {
            return Err("invalid or duplicate manual file rate".into());
        }
        if settings.plot >= PLOTS.len() {
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
    fn quality_mask_is_indexed_independent_and_all_includes_unknowns() {
        let mut q = QualityFilter::default();
        q.0[2] = false;
        assert!(!q.visible(2));
        assert!(q.visible(1));
        assert!(q.visible(0));
        assert!(!q.visible(8));
        q.0.fill(false);
        q.toggle_all();
        assert_eq!(q.0, [true; 8]);
        q.toggle_all();
        assert_eq!(q.0, [false; 8]);
        q.0[1] = true;
        q.toggle_all();
        assert_eq!(q.0, [true; 8]);
    }
    #[test]
    fn origin_range_units_and_order() {
        assert_eq!(ORIGIN_RANGES.first(), Some(&("5cm", 0.05)));
        assert_eq!(ORIGIN_RANGES.last(), Some(&("1000km", 1_000_000.0)));
        for (label, metres) in ORIGIN_RANGES {
            let (number, unit) = label.split_at(label.find(|c: char| !c.is_ascii_digit()).unwrap());
            let scale = match unit {
                "cm" => 0.01,
                "m" => 1.0,
                "km" => 1_000.0,
                _ => panic!("unknown range unit: {unit}"),
            };
            assert_eq!(number.parse::<f64>().unwrap() * scale, metres);
        }
        assert!(ORIGIN_RANGES.windows(2).all(|r| r[0].1 < r[1].1));
    }
    #[test]
    fn text_sizes_validate_both_limits_and_nonfinite_values_independently() {
        for size in [6.0, 9.0, 12.0, 30.0] {
            assert!(
                Options {
                    grid_label_size: size,
                    statistics_size: size,
                    ..Default::default()
                }
                .validate()
                .is_ok()
            );
        }
        for size in [5.99, 30.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                Options {
                    grid_label_size: size,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
            assert!(
                Options {
                    statistics_size: size,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn settings_round_trip_and_errors() {
        let dir = std::env::temp_dir().join(format!(
            "plot-settings-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("plot.json");
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert!(Options::default().tile_url.is_empty());
        assert!(Options::default().tile_attribution.is_empty());
        assert!(Settings::default().show_grid);
        let s = Settings {
            last_file: Some("入力.pos".into()),
            last_file2: Some("比較.pos".into()),
            plot: 2,
            map: true,
            show_grid: false,
            manual_rates: vec![ManualRate {
                path: "入力.pos".into(),
                hz: 1.0,
                entered: Some("1.00000".into()),
            }],
            qualities: QualityFilter([false, false, false, false, false, false, true, false]),
            options: Options {
                show_g_label: 4,
                grid_label_size: 18.5,
                statistics_size: 24.0,
                origin: 5,
                auto_fit: false,
                llh: [35.0, 139.0, 10.0],
                week_time: true,
                y_range: 0.05,
                anim_cycle: 20,
                ref_cycle: 250,
                tile_url: "http://localhost/{z}/{x}/{y}".into(),
                tile_attribution: "Local test tiles".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path).unwrap(), s);
        for format in 0..GRID_FORMATS.len() {
            let mut settings = s.clone();
            settings.options.show_g_label = format;
            save(&path, &settings).unwrap();
            assert_eq!(load(&path).unwrap(), settings);
        }
        std::fs::write(
            &path,
            r#"{"options":{"style":2,"tile_url":"http://localhost/{z}/{x}/{y}"}}"#,
        )
        .unwrap();
        let old_options = load(&path).unwrap().options;
        assert_eq!(old_options.show_g_label, 1);
        assert_eq!(old_options.grid_label_size, 9.0);
        assert_eq!(old_options.statistics_size, 12.0);
        assert_eq!(old_options.style, 2);
        assert_eq!(old_options.tile_url, "http://localhost/{z}/{x}/{y}");
        std::fs::write(&path, r#"{"quality":6}"#).unwrap();
        let migrated = load(&path).unwrap();
        assert!(!migrated.map);
        assert!(migrated.show_grid);
        assert!(migrated.options.tile_url.is_empty());
        assert!(migrated.options.tile_attribution.is_empty());
        assert_eq!(migrated.options.show_g_label, 1);
        assert_eq!(migrated.options.grid_label_size, 9.0);
        assert_eq!(migrated.options.statistics_size, 12.0);
        assert_eq!(migrated.options.anim_cycle, 10);
        assert_eq!(migrated.options.ref_cycle, 100);
        assert!(migrated.qualities.visible(6));
        assert!(!migrated.qualities.visible(1));
        for text in [
            r#"{"quality":7}"#,
            r#"{"manual_rates":[{"path":"a.pos","hz":0}]}"#,
            "{bad",
            r#"{"plot":9}"#,
            r#"{"options":{"show_g_label":5}}"#,
            r#"{"options":{"show_g_label":-1}}"#,
            r#"{"options":{"y_range":0}}"#,
            r#"{"options":{"grid_label_size":5.9}}"#,
            r#"{"options":{"statistics_size":30.1}}"#,
            r#"{"options":{"ref_cycle":-1}}"#,
            r#"{"options":{"anim_cycle":1.5}}"#,
            r#"{"options":{"tile_url":"http://localhost/no-template"}}"#,
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
