//! Build one argv value for execution and preview; MRTKLIB owns option semantics.

use crate::{
    naming::absolute,
    settings::{FORMATS, OUTPUTS, SYSTEMS, Settings},
};
use mrtklib_egui_supervisor::CommandLine;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(crate) fn build(
    settings: &Settings,
    program: &Path,
    cwd: &Path,
) -> Result<CommandLine, String> {
    settings.options.validate()?;
    if settings.input.is_empty() {
        return Err("Select an input file".into());
    }
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
    for (row, (_, flag, _)) in OUTPUTS.iter().enumerate() {
        let path = if settings.output_enabled(row) {
            if settings.outputs[row].path.is_empty() {
                return Err(format!("{} output path is empty", OUTPUTS[row].0));
            }
            absolute(Path::new(&settings.outputs[row].path), cwd).into_os_string()
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
    args.push(absolute(Path::new(&settings.input), cwd).into_os_string());
    Ok(CommandLine {
        program: absolute(program, cwd),
        args,
        working_directory: None,
    })
}

pub(crate) fn existing_outputs(settings: &Settings, cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for row in 0..9 {
        let value = &settings.outputs[row].path;
        if settings.output_enabled(row) && !value.contains('%') {
            let path = absolute(Path::new(value), cwd);
            if path
                .try_exists()
                .map_err(|e| format!("cannot check {}: {e}", path.display()))?
                && !paths.contains(&path)
            {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Settings {
        let mut s = Settings {
            input: "data/input.ubx".into(),
            ..Default::default()
        };
        s.refresh_outputs(true);
        s
    }
    fn argv(s: &Settings) -> Vec<String> {
        build(s, Path::new("mrtk"), &std::env::temp_dir())
            .unwrap()
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
        let command = build(&s, Path::new("mrtk"), &std::env::temp_dir()).unwrap();
        assert!(command.program.is_absolute());
        assert!(command.working_directory.is_none());
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
                build(&s, Path::new("mrtk"), &std::env::temp_dir())
                    .unwrap_err()
                    .contains("'/'")
            );
        }
        let mut s = fixture();
        s.outputs[0].path.clear();
        assert!(
            build(&s, Path::new("mrtk"), &std::env::temp_dir())
                .unwrap_err()
                .contains("OBS output path is empty")
        );
    }

    #[test]
    fn conventional_names_and_suppressed_slots_are_explicit() {
        let mut s = fixture();
        s.options.rinex2_name = true;
        s.outputs[0].enabled = false;
        s.refresh_outputs(false);
        let args = argv(&s);
        assert_eq!(values(&args, "-c"), ["0000"]);
        assert_eq!(values(&args, "-o"), [""]);
        assert!(values(&args, "-n")[0].ends_with("%r%n0.%yP"));
    }

    #[test]
    fn overwrite_lists_existing_enabled_nonkeyword_paths_once() {
        let directory = std::env::temp_dir().join(format!(
            "conv-overwrite-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut s = Settings {
            input: "test.ubx".into(),
            ..Default::default()
        };
        s.refresh_outputs(true);
        for name in ["test.obs", "test.nav", "test.gnav", "%r.sbs"] {
            std::fs::write(directory.join(name), b"existing").unwrap();
        }
        s.outputs[8].path = "%r.sbs".into();
        assert_eq!(
            existing_outputs(&s, &directory).unwrap(),
            [directory.join("test.obs"), directory.join("test.nav")]
        );
        s.outputs[0].enabled = false;
        assert_eq!(
            existing_outputs(&s, &directory).unwrap(),
            [directory.join("test.nav")]
        );
        s.outputs[0].enabled = true;
        s.outputs[0].path = "test.nav".into();
        assert_eq!(
            existing_outputs(&s, &directory).unwrap(),
            [directory.join("test.nav")]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
