//! GUI-owned JSON state, independent of egui and MRTKLIB configuration handling.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const VERSIONS: [&str; 8] = [
    "2.10", "2.11", "2.12", "3.00", "3.01", "3.02", "3.03", "3.04",
];
pub(crate) const FORMATS: [(&str, &str); 15] = [
    ("Auto", ""),
    ("RTCM 2", "rtcm2"),
    ("RTCM 3", "rtcm3"),
    ("NovAtel OEM7", "nov"),
    ("NovAtel OEM3", "oem3"),
    ("u-blox UBX", "ubx"),
    ("Superstar II", "ss2"),
    ("Hemisphere", "hemis"),
    ("SkyTraq", "stq"),
    ("Javad GREIS", "javad"),
    ("NVS BINR", "nvs"),
    ("BINEX", "binex"),
    ("Trimble RT17", "rt17"),
    ("Septentrio SBF", "sbf"),
    ("RINEX", "rinex"),
];
pub(crate) const SYSTEMS: [(&str, &str); 7] = [
    ("GPS", "G"),
    ("GLO", "R"),
    ("GAL", "E"),
    ("QZS", "J"),
    ("SBS", "S"),
    ("BDS", "C"),
    ("NavIC", "I"),
];
pub(crate) const OUTPUTS: [(&str, &str, &str); 9] = [
    ("OBS", "-o", "obs"),
    ("NAV", "-n", "nav"),
    ("GNAV", "-g", "gnav"),
    ("HNAV", "-h", "hnav"),
    ("QNAV", "-q", "qnav"),
    ("LNAV", "-l", "lnav"),
    ("CNAV", "-b", "cnav"),
    ("INAV", "-i", "inav"),
    ("SBS", "-s", "sbs"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TimeField {
    pub enabled: bool,
    pub date: String,
    pub clock: String,
}
impl Default for TimeField {
    fn default() -> Self {
        Self {
            enabled: false,
            date: "2020/01/01".into(),
            clock: "00:00:00".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Output {
    pub enabled: bool,
    pub path: String,
    pub edited: bool,
}
impl Default for Output {
    fn default() -> Self {
        Self {
            enabled: true,
            path: String::new(),
            edited: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ConvertOptions {
    pub version: String,
    pub station: String,
    pub rinex2_name: bool,
    pub marker: [String; 3],
    pub observer: [String; 2],
    pub receiver: [String; 3],
    pub antenna: [String; 2],
    pub position: [String; 3],
    pub delta: [String; 3],
    pub comments: [String; 2],
    pub iono: bool,
    pub time_corr: bool,
    pub leap_sec: bool,
    pub half_cycle: bool,
    pub systems: [bool; 7],
    pub excluded: String,
    pub receiver_options: String,
    pub tolerance: String,
    pub frequencies: u8,
}
impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            version: "3.04".into(),
            station: "0000".into(),
            rinex2_name: false,
            marker: Default::default(),
            observer: Default::default(),
            receiver: Default::default(),
            antenna: Default::default(),
            position: std::array::from_fn(|_| "0".into()),
            delta: std::array::from_fn(|_| "0".into()),
            comments: Default::default(),
            iono: false,
            time_corr: false,
            leap_sec: false,
            half_cycle: false,
            systems: [true; 7],
            excluded: String::new(),
            receiver_options: String::new(),
            tolerance: "0.005".into(),
            frequencies: 5,
        }
    }
}
impl ConvertOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !VERSIONS.contains(&self.version.as_str()) || !(1..=5).contains(&self.frequencies) {
            return Err("invalid RINEX version or frequency count".into());
        }
        for (label, parts) in [
            ("Observer/Agency", self.observer.as_slice()),
            ("Receiver #/type/version", self.receiver.as_slice()),
            ("Antenna #/type", self.antenna.as_slice()),
            ("Approx Pos XYZ", self.position.as_slice()),
            ("Antenna Delta H/E/N", self.delta.as_slice()),
        ] {
            if parts.iter().any(|part| part.contains('/')) {
                return Err(format!(
                    "{label}: '/' cannot be represented by mrtk convert"
                ));
            }
            let mut empty = false;
            for part in parts {
                if part.is_empty() {
                    empty = true;
                } else if empty {
                    return Err(format!(
                        "{label}: an empty field before a nonempty field cannot be represented by mrtk convert"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub input: String,
    pub format: usize,
    pub start: TimeField,
    pub end: TimeField,
    pub interval_enabled: bool,
    pub interval: String,
    pub directory_enabled: bool,
    pub directory: String,
    pub outputs: [Output; 9],
    pub options: ConvertOptions,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            input: String::new(),
            format: 0,
            start: TimeField::default(),
            end: TimeField::default(),
            interval_enabled: false,
            interval: "1".into(),
            directory_enabled: false,
            directory: String::new(),
            outputs: Default::default(),
            options: ConvertOptions::default(),
        }
    }
}

pub(crate) fn settings_file(executable: &Path) -> Result<PathBuf, String> {
    let directory = mrtklib_egui_common::paths::settings_directory(
        std::env::var_os("MRTKLIB_EGUI_SETTINGS_DIR"),
        executable,
    )?;
    let mut filename = executable
        .file_stem()
        .ok_or("GUI executable has no file stem")?
        .to_os_string();
    filename.push(".json");
    Ok(directory.join(filename))
}

/// The warning is returned so the caller can print it once, without overwriting
/// the source file. A nonexistent file is normal on first launch.
pub(crate) fn load(path: &Path) -> (Settings, Option<String>) {
    let result = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => Err(e.to_string()),
    }
    .and_then(|settings| {
        if settings.format >= FORMATS.len() {
            return Err("invalid input format".into());
        }
        // Editable CLI fields are validated before conversion, so a user's
        // unfinished edits can still round-trip through GUI settings.
        if !VERSIONS.contains(&settings.options.version.as_str())
            || !(1..=5).contains(&settings.options.frequencies)
        {
            return Err("invalid RINEX version or frequency count".into());
        }
        Ok(settings)
    });
    match result {
        Ok(settings) => (settings, None),
        Err(error) => (
            Settings::default(),
            Some(format!(
                "cannot read settings {}: {error}; using defaults",
                path.display()
            )),
        ),
    }
}

pub(crate) fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("cannot save settings {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_filename_uses_the_gui_executable_stem() {
        assert_eq!(
            settings_file(Path::new("/suite/custom-conv.exe"))
                .unwrap()
                .file_name()
                .unwrap(),
            "custom-conv.json"
        );
    }

    #[test]
    fn slash_tokenization_rejects_gaps_but_allows_trailing_empty_fields() {
        let mut options = ConvertOptions::default();
        options.observer[1] = "Agency".into();
        assert!(options.validate().unwrap_err().contains("empty field"));
        options.observer[0] = "Observer".into();
        options.validate().unwrap();
        options.receiver = ["SN", "", "version"].map(String::from);
        assert!(options.validate().unwrap_err().contains("Receiver"));
        options.receiver[2].clear();
        options.validate().unwrap();
        options.antenna[1] = "Type".into();
        assert!(options.validate().unwrap_err().contains("Antenna"));
        options.antenna[0] = "SN".into();
        options.position[1].clear();
        assert!(options.validate().unwrap_err().contains("XYZ"));
        options.position[1] = "0".into();
        options.delta[0].clear();
        assert!(options.validate().unwrap_err().contains("Delta"));
    }

    #[test]
    fn round_trip_and_invalid_files_are_not_overwritten() {
        let directory = std::env::temp_dir().join(format!(
            "conv-settings-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("gui.json");
        assert_eq!(load(&path), (Settings::default(), None));
        let mut settings = Settings {
            input: "入力.ubx".into(),
            ..Default::default()
        };
        settings.outputs[1].edited = true;
        settings.options.receiver_options = "-A -B".into();
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), (settings, None));
        for bytes in [
            "{broken",
            r#"{"format":99}"#,
            r#"{"options":{"frequencies":0}}"#,
        ] {
            std::fs::write(&path, bytes).unwrap();
            let (settings, warning) = load(&path);
            assert_eq!(settings, Settings::default());
            assert!(warning.unwrap().contains("using defaults"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        }
        assert!(
            load(&directory).1.is_some(),
            "unreadable settings must warn"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
