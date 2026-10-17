//! Build one argv value for execution and preview; MRTKLIB owns option semantics.

use crate::{
    naming::{ResolvedPaths, normalize_absolute},
    settings::{FORMATS, OUTPUTS, SYSTEMS, Settings},
};
use mrtklib_egui_supervisor::CommandLine;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(crate) struct PreparedConversion {
    pub command: CommandLine,
    outputs: Vec<PathBuf>,
}

impl std::fmt::Debug for PreparedConversion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedConversion")
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

impl PreparedConversion {
    /// Check the exact selected paths used in argv; no second resolution pass.
    pub fn existing_outputs(&self) -> Result<Vec<PathBuf>, String> {
        let mut paths = Vec::new();
        for path in &self.outputs {
            if !path.as_os_str().to_string_lossy().contains('%')
                && path
                    .try_exists()
                    .map_err(|e| format!("cannot check {}: {e}", path.display()))?
                && !paths.contains(path)
            {
                paths.push(path.clone());
            }
        }
        Ok(paths)
    }
}

pub(crate) fn build(
    settings: &Settings,
    program: &Path,
    home: Option<&Path>,
) -> Result<PreparedConversion, String> {
    settings.options.validate()?;
    let resolved = ResolvedPaths::new(settings, home)?;
    let format = FORMATS
        .get(settings.format)
        .ok_or("invalid input format")?
        .1;
    let mut args: Vec<OsString> = vec!["convert".into()];
    if !format.is_empty() {
        args.extend(["-r".into(), format.into()]);
    }
    for (flag, time) in [("-ts", &settings.start), ("-te", &settings.end)] {
        if time.enabled {
            args.extend([
                flag.into(),
                time.date.clone().into(),
                time.clock.clone().into(),
            ]);
        }
    }
    let mut pair = |flag: &str, value: &str| {
        args.extend([flag.into(), value.into()]);
    };
    if settings.interval_enabled {
        pair("-ti", &settings.interval);
    }
    let o = &settings.options;
    pair("-v", &o.version);
    pair("-f", &o.frequencies.to_string());
    pair("-tt", &o.tolerance);
    if let Some(mask) = o.signal_mask.allow_list()? {
        pair("-mask", &mask);
    }
    if o.rinex2_name {
        pair("-c", &o.station);
    }
    for (flag, value) in [
        ("-hm", &o.marker[0]),
        ("-hn", &o.marker[1]),
        ("-ht", &o.marker[2]),
        ("-ro", &o.receiver_options),
    ] {
        if !value.is_empty() {
            pair(flag, value);
        }
    }
    for (flag, parts) in [
        ("-ho", o.observer.as_slice()),
        ("-hr", o.receiver.as_slice()),
        ("-ha", o.antenna.as_slice()),
        ("-hp", o.position.as_slice()),
        ("-hd", o.delta.as_slice()),
    ] {
        if parts.iter().any(|p| !p.is_empty()) {
            pair(flag, &parts.join("/"));
        }
    }
    for comment in &o.comments {
        if !comment.is_empty() {
            pair("-hc", comment);
        }
    }
    for (checked, (_, code)) in o.systems.iter().zip(SYSTEMS) {
        if !checked {
            pair("-y", code);
        }
    }
    for satellite in o.excluded.split_whitespace() {
        pair("-x", satellite);
    }
    let mut outputs = Vec::new();
    for (row, (_, flag, _)) in OUTPUTS.iter().enumerate() {
        let path = if settings.output_enabled(row) {
            let path = resolved.outputs[row]
                .as_ref()
                .ok_or_else(|| format!("{} output path is empty", OUTPUTS[row].0))?;
            outputs.push(path.clone());
            path.as_os_str().to_owned()
        } else {
            OsString::new()
        };
        args.extend([(*flag).into(), path]);
    }
    for (flag, enabled) in [
        ("-oi", o.iono),
        ("-ot", o.time_corr),
        ("-ol", o.leap_sec),
        ("-halfc", o.half_cycle),
    ] {
        if enabled {
            args.push(flag.into());
        }
    }
    args.push(resolved.input.into_os_string());
    Ok(PreparedConversion {
        command: CommandLine {
            program: normalize_absolute(program)?,
            args,
            working_directory: None,
        },
        outputs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Settings {
        let mut s = Settings {
            input: std::env::temp_dir()
                .join("data/input.ubx")
                .to_str()
                .unwrap()
                .into(),
            ..Default::default()
        };
        s.refresh_outputs(true, None).unwrap();
        s
    }
    fn argv(s: &Settings) -> Vec<String> {
        build(s, &std::env::temp_dir().join("mrtk"), None)
            .unwrap()
            .command
            .args
            .iter()
            .map(|s| s.to_str().unwrap().into())
            .collect()
    }
    fn values(args: &[String], flag: &str) -> Vec<String> {
        args.windows(2)
            .filter(|p| p[0] == flag)
            .map(|p| p[1].clone())
            .collect()
    }

    #[test]
    fn defaults_all_slots_absolute_paths_and_format_mappings() {
        let s = fixture();
        let command = build(&s, &std::env::temp_dir().join("mrtk"), None).unwrap();
        assert!(command.command.program.is_absolute());
        assert!(command.command.working_directory.is_none());
        let args = argv(&s);
        assert_eq!(args[0], "convert");
        assert_eq!(values(&args, "-tt"), ["0.005"]);
        assert_eq!(values(&args, "-f"), ["5"]);
        assert_eq!(values(&args, "-v"), ["3.04"]);
        for (_, flag, _) in OUTPUTS {
            assert_eq!(values(&args, flag).len(), 1);
        }
        for (_, flag, _) in OUTPUTS.iter().take(8).skip(2) {
            assert_eq!(values(&args, flag), [""]);
        }
        assert!(!args.contains(&"-d".into()));
        assert!(!args.contains(&"-c".into()));
        assert!(Path::new(args.last().unwrap()).is_absolute());
        for (index, (_, format)) in FORMATS.iter().enumerate() {
            let mut s = fixture();
            s.format = index;
            let args = argv(&s);
            if index == 0 {
                assert!(values(&args, "-r").is_empty());
            } else {
                assert_eq!(values(&args, "-r"), [*format]);
            }
            if index == 14 {
                assert_eq!(values(&args, "-s"), [""]);
            }
        }
        for (row, (_, flag, _)) in OUTPUTS.iter().enumerate() {
            let mut s = fixture();
            s.options.version = "2.12".into();
            s.outputs[row].enabled = false;
            assert_eq!(values(&argv(&s), flag), [""]);
        }
    }

    #[test]
    fn signal_mask_is_omitted_by_default_or_passed_once_as_one_allow_list() {
        let mut s = fixture();
        assert!(values(&argv(&s), "-mask").is_empty());
        s.options.signal_mask.set_all(false);
        assert!(
            build(&s, &std::env::temp_dir().join("mrtk"), None)
                .unwrap_err()
                .contains("at least one")
        );
        s.options.signal_mask.0[0][0] = true;
        s.options.signal_mask.0[2][16] = true;
        assert_eq!(values(&argv(&s), "-mask"), ["GL1C,EL8L"]);
    }

    #[test]
    fn every_reachable_option_and_argument_boundaries() {
        let mut s = fixture();
        s.start.enabled = true;
        s.start.date = "2008/05/26".into();
        s.start.clock = "01:02:03".into();
        s.end.enabled = true;
        s.end.date = "2008/05/27".into();
        s.end.clock = "04:05:06".into();
        s.interval_enabled = true;
        s.interval = "0.25".into();
        let o = &mut s.options;
        o.version = "2.12".into();
        o.rinex2_name = true;
        o.station = "TEST".into();
        o.marker = ["Marker name", "1234", "GEODETIC"].map(String::from);
        o.observer = ["An Observer", "An Agency"].map(String::from);
        o.receiver = ["123", "Receiver type", "1.0"].map(String::from);
        o.antenna = ["456", "Antenna type"].map(String::from);
        o.position = ["1", "2", "3"].map(String::from);
        o.delta = ["4", "5", "6"].map(String::from);
        o.comments = ["first comment", "second comment"].map(String::from);
        o.receiver_options = "-TADJ=1 -EPHALL".into();
        o.excluded = "G01  R02\nE03".into();
        o.systems = [false; 7];
        o.frequencies = 2;
        o.tolerance = "0.01".into();
        o.iono = true;
        o.time_corr = true;
        o.leap_sec = true;
        o.half_cycle = true;
        let args = argv(&s);
        for triplet in [
            ["-ts", "2008/05/26", "01:02:03"],
            ["-te", "2008/05/27", "04:05:06"],
        ] {
            assert!(args.windows(3).any(|p| p == triplet));
        }
        for (flag, value) in [
            ("-ti", "0.25"),
            ("-v", "2.12"),
            ("-c", "TEST"),
            ("-f", "2"),
            ("-tt", "0.01"),
            ("-hm", "Marker name"),
            ("-hn", "1234"),
            ("-ht", "GEODETIC"),
            ("-ho", "An Observer/An Agency"),
            ("-hr", "123/Receiver type/1.0"),
            ("-ha", "456/Antenna type"),
            ("-hp", "1/2/3"),
            ("-hd", "4/5/6"),
            ("-ro", "-TADJ=1 -EPHALL"),
        ] {
            assert_eq!(values(&args, flag), [value], "{flag}");
        }
        assert_eq!(values(&args, "-hc"), ["first comment", "second comment"]);
        assert_eq!(values(&args, "-y"), SYSTEMS.map(|(_, code)| code));
        assert_eq!(values(&args, "-x"), ["G01", "R02", "E03"]);
        for flag in ["-oi", "-ot", "-ol", "-halfc"] {
            assert!(args.contains(&flag.into()));
        }
        for (row, (_, flag, _)) in OUTPUTS.iter().enumerate() {
            assert!(!values(&args, flag)[0].is_empty(), "row {row}");
        }
    }

    #[test]
    fn each_unchecked_system_adds_only_its_own_exclusion() {
        for (index, (_, code)) in SYSTEMS.iter().enumerate() {
            let mut s = fixture();
            s.options.systems[index] = false;
            assert_eq!(values(&argv(&s), "-y"), [*code]);
        }
    }

    #[test]
    fn slash_joined_fields_reject_literal_slashes() {
        for group in 0..5 {
            let mut s = fixture();
            let parts = match group {
                0 => &mut s.options.observer[..],
                1 => &mut s.options.receiver[..],
                2 => &mut s.options.antenna[..],
                3 => &mut s.options.position[..],
                _ => &mut s.options.delta[..],
            };
            parts[0] = "cannot/pass".into();
            assert!(
                build(&s, &std::env::temp_dir().join("mrtk"), None)
                    .unwrap_err()
                    .contains("'/'")
            );
        }
        let mut s = fixture();
        s.outputs[0].path.clear();
        s.outputs[0].edited = true;
        assert!(
            build(&s, &std::env::temp_dir().join("mrtk"), None)
                .unwrap_err()
                .contains("OBS output path is empty")
        );
    }

    #[test]
    fn conventional_names_and_suppressed_slots_are_explicit() {
        let mut s = fixture();
        s.options.rinex2_name = true;
        s.outputs[0].enabled = false;
        s.refresh_outputs(false, None).unwrap();
        let args = argv(&s);
        assert_eq!(values(&args, "-c"), ["0000"]);
        assert_eq!(values(&args, "-o"), [""]);
        assert!(values(&args, "-n")[0].ends_with("%r%n0.%yP"));
    }

    #[test]
    fn home_and_input_relative_outputs_reach_preview_and_argv_as_absolute_paths() {
        let home = std::env::temp_dir().join("conv-command-home");
        let mut s = Settings {
            input: "~/data/in.ubx".into(),
            directory_enabled: true,
            directory: "~/out/unused/..".into(),
            ..Default::default()
        };
        s.outputs[1].edited = true;
        s.outputs[1].path = "../custom.nav".into();
        let prepared = build(&s, &home.join("mrtk"), Some(&home)).unwrap();
        let args: Vec<_> = prepared
            .command
            .args
            .iter()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            values(&args, "-o"),
            [home.join("out/in.obs").to_str().unwrap()]
        );
        assert_eq!(
            values(&args, "-n"),
            [home.join("custom.nav").to_str().unwrap()]
        );
        assert_eq!(
            args.last().unwrap(),
            home.join("data/in.ubx").to_str().unwrap()
        );
        let preview = prepared.command.preview();
        for path in ["out/in.obs", "custom.nav", "data/in.ubx"] {
            assert!(preview.contains(home.join(path).to_str().unwrap()));
        }
    }

    #[test]
    fn prepared_paths_are_shared_by_preview_argv_and_overwrite_check() {
        let directory = std::env::temp_dir().join(format!(
            "conv-prepared-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let input = directory.join("input.obs");
        let output = directory.join("input.obs_");
        std::fs::write(&input, b"input").unwrap();
        std::fs::write(&output, b"existing").unwrap();
        let mut s = Settings {
            input: directory
                .join("missing/../input.obs")
                .to_str()
                .unwrap()
                .into(),
            directory_enabled: true,
            directory: ".".into(),
            ..Default::default()
        };
        let prepared = build(&s, &directory.join("missing/../mrtk"), None).unwrap();
        assert_eq!(prepared.command.program, directory.join("mrtk"));
        let preview = prepared.command.preview();
        s.input = "changed.ubx".into();
        s.outputs[0].path = "changed.obs".into();
        assert_eq!(
            prepared.existing_outputs().unwrap(),
            std::slice::from_ref(&output)
        );
        let args: Vec<_> = prepared
            .command
            .args
            .iter()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(values(&args, "-o"), [output.to_str().unwrap()]);
        assert_eq!(args.last().unwrap(), input.to_str().unwrap());
        assert!(preview.contains(output.to_str().unwrap()));
        assert_eq!(std::fs::read(input).unwrap(), b"input");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn overwrite_lists_existing_enabled_nonkeyword_paths_once() {
        let directory = std::env::temp_dir().join(format!(
            "conv-overwrite-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut s = Settings {
            input: directory.join("test.ubx").to_str().unwrap().into(),
            ..Default::default()
        };
        s.refresh_outputs(true, None).unwrap();
        for name in ["test.obs", "test.nav", "test.gnav", "%r.sbs"] {
            std::fs::write(directory.join(name), b"existing").unwrap();
        }
        s.outputs[8].path = "%r.sbs".into();
        s.outputs[8].edited = true;
        assert_eq!(
            build(&s, &directory.join("mrtk"), None)
                .unwrap()
                .existing_outputs()
                .unwrap(),
            [directory.join("test.obs"), directory.join("test.nav")]
        );
        s.outputs[0].enabled = false;
        assert_eq!(
            build(&s, &directory.join("mrtk"), None)
                .unwrap()
                .existing_outputs()
                .unwrap(),
            [directory.join("test.nav")]
        );
        s.outputs[0].enabled = true;
        s.outputs[0].path = "unused/.././test.nav".into();
        s.outputs[0].edited = true;
        assert_eq!(
            build(&s, &directory.join("mrtk"), None)
                .unwrap()
                .existing_outputs()
                .unwrap(),
            [directory.join("test.nav")]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
