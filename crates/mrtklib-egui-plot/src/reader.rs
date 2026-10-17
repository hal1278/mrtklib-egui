//! Strict reader for the MRTKLIB text solution schema (not console output).
use mrtklib_egui_plotkit::{SolutionPoint, llh_to_ecef};
use std::{io::BufRead, path::Path};

#[derive(Clone, Debug)]
pub(crate) struct Dataset {
    pub points: Vec<SolutionPoint>,
    pub base: Option<[f64; 3]>,
    pub relative: bool,
    pub time_system: String,
    pub malformed: usize,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Coordinates {
    Degrees,
    Dms,
    Xyz,
    Enu,
}
struct Layout {
    coordinates: Coordinates,
    time_system: String,
    velocity: bool,
}
impl Layout {
    fn heading(line: &str) -> Result<Option<Self>, String> {
        let fields: Vec<_> = line.trim_start_matches('%').split_whitespace().collect();
        if fields.is_empty() || !["GPST", "UTC", "JST"].contains(&fields[0]) {
            return Ok(None);
        }
        let coordinates=match fields.get(1).copied() {
            Some("latitude(deg)")=>Coordinates::Degrees,
            Some("latitude(d'\")")=>Coordinates::Dms,
            Some("x-ecef(m)")=>Coordinates::Xyz,
            Some("e-baseline(m)")=>Coordinates::Enu,
            _=>return Err("Unrecognised solution heading or non-whitespace custom separator; only whitespace separators are supported".into()),
        };
        let mut expected = match coordinates {
            Coordinates::Degrees => vec![
                "latitude(deg)",
                "longitude(deg)",
                "height(m)",
                "Q",
                "ns",
                "sdn(m)",
                "sde(m)",
                "sdu(m)",
                "sdne(m)",
                "sdeu(m)",
                "sdun(m)",
                "age(s)",
                "ratio",
            ],
            Coordinates::Dms => vec![
                "latitude(d'\")",
                "longitude(d'\")",
                "height(m)",
                "Q",
                "ns",
                "sdn(m)",
                "sde(m)",
                "sdu(m)",
                "sdne(m)",
                "sdeu(m)",
                "sdue(m)",
                "age(s)",
                "ratio",
            ],
            Coordinates::Xyz => vec![
                "x-ecef(m)",
                "y-ecef(m)",
                "z-ecef(m)",
                "Q",
                "ns",
                "sdx(m)",
                "sdy(m)",
                "sdz(m)",
                "sdxy(m)",
                "sdyz(m)",
                "sdzx(m)",
                "age(s)",
                "ratio",
            ],
            Coordinates::Enu => vec![
                "e-baseline(m)",
                "n-baseline(m)",
                "u-baseline(m)",
                "Q",
                "ns",
                "sde(m)",
                "sdn(m)",
                "sdu(m)",
                "sden(m)",
                "sdnu(m)",
                "sdue(m)",
                "age(s)",
                "ratio",
            ],
        };
        let velocity = fields.len() == 23 && coordinates != Coordinates::Enu;
        if velocity {
            expected.extend(if coordinates == Coordinates::Xyz {
                [
                    "vx(m/s)", "vy(m/s)", "vz(m/s)", "sdvx", "sdvy", "sdvz", "sdvxy", "sdvyz",
                    "sdvzx",
                ]
            } else {
                [
                    "vn(m/s)", "ve(m/s)", "vu(m/s)", "sdvn", "sdve", "sdvu", "sdvne", "sdveu",
                    "sdvun",
                ]
            });
        }
        if fields[1..] != expected {
            return Err("Unrecognised solution columns or custom separator; only the MRTKLIB whitespace layout is supported".into());
        }
        Ok(Some(Self {
            coordinates,
            time_system: fields[0].into(),
            velocity,
        }))
    }
    fn position(&self, fields: &[&str], reference: bool) -> Option<[f64; 3]> {
        let dms = self.coordinates == Coordinates::Dms
            || (reference && self.coordinates == Coordinates::Enu && fields.len() == 7);
        let p = if dms {
            [
                angle(&fields[0..3])?,
                angle(&fields[3..6])?,
                number(fields[6])?,
            ]
        } else {
            [number(fields[0])?, number(fields[1])?, number(fields[2])?]
        };
        if self.coordinates == Coordinates::Xyz
            || (self.coordinates == Coordinates::Enu && !reference)
        {
            return Some(p);
        }
        if p[0].abs() > 90.0 || p[1].abs() > 180.0 {
            return None;
        }
        Some(llh_to_ecef(p))
    }
    fn row(&self, line: &str, calendar: bool) -> Option<SolutionPoint> {
        let f: Vec<_> = line.split_whitespace().collect();
        let dms = self.coordinates == Coordinates::Dms;
        let count = if dms { 19 } else { 15 } + if self.velocity { 9 } else { 0 };
        if f.len() != count {
            return None;
        }
        let time = if calendar {
            crate::time::calendar(f[0], f[1])?
        } else {
            let week: u32 = f[0].parse().ok()?;
            let tow = number(f[1])?;
            if !(0.0..604800.0).contains(&tow) {
                return None;
            }
            f64::from(week) * 604800.0 + tow
        };
        let position = self.position(&f[2..], false)?;
        let q = if dms { 9 } else { 5 };
        let quality: u8 = f[q].parse().ok()?;
        if quality > 7 {
            return None;
        }
        let satellites = f[q + 1].parse().ok()?;
        // Validate even the unused covariance/optional velocity fields.
        for field in &f[q + 2..] {
            number(field)?;
        }
        Some(SolutionPoint {
            time,
            position,
            quality,
            satellites,
            age: number(f[q + 8])?,
            ratio: number(f[q + 9])?,
        })
    }
}
fn number(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}
fn angle(parts: &[&str]) -> Option<f64> {
    let d = number(parts[0])?;
    let m = number(parts[1])?;
    let s = number(parts[2])?;
    if d.fract() != 0.0
        || !(0.0..60.0).contains(&m)
        || m.fract() != 0.0
        || !(0.0..60.0).contains(&s)
    {
        return None;
    }
    Some((d.abs() + m / 60.0 + s / 3600.0) * if parts[0].starts_with('-') { -1.0 } else { 1.0 })
}

pub(crate) fn read(path: &Path) -> Result<Dataset, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("cannot open solution {}: {e}", path.display()))?;
    parse(std::io::BufReader::new(file))
}
fn parse(reader: impl BufRead) -> Result<Dataset, String> {
    parse_with_time_layout(reader, None)
}
fn parse_with_time_layout(
    reader: impl BufRead,
    mut calendar: Option<bool>,
) -> Result<Dataset, String> {
    let mut layout = None;
    let mut reference = None;
    let mut dataset = Dataset {
        points: vec![],
        base: None,
        relative: false,
        time_system: String::new(),
        malformed: 0,
        warnings: vec![],
    };
    let mut before_heading = 0;
    for line in reader.lines() {
        let line = line.map_err(|e| format!("cannot read solution text: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('%') {
            if line.contains("lat/lon/height=Tokyo/") {
                return Err(
                    "Tokyo-datum solution coordinates are unsupported; WGS84 is required".into(),
                );
            }
            if line.contains("lat/lon/height=WGS84/geodetic") {
                dataset.warnings.push(
                    "Geodetic height: plotting the stored height without geoid restoration".into(),
                );
            }

            if let Some((name, value)) = line.split_once(':')
                && name.trim_start_matches('%').trim() == "ref pos"
            {
                reference = Some(value.to_owned());
            }
            if let Some(detected) = Layout::heading(line)? {
                if layout.is_some() {
                    return Err("Multiple solution headings are not supported in one file".into());
                }
                dataset.relative = detected.coordinates == Coordinates::Enu;
                dataset.time_system = detected.time_system.clone();
                layout = Some(detected);
            }
            continue;
        }
        let Some(layout) = &layout else {
            before_heading += 1;
            continue;
        };
        // Padding in the writer's heading depends on precision as well as format.
        // Resolve the disjoint timestamp grammars from the first complete row.
        let point = if let Some(calendar) = calendar {
            layout.row(line, calendar)
        } else if let Some(point) = layout.row(line, true) {
            calendar = Some(true);
            Some(point)
        } else if let Some(point) = layout.row(line, false) {
            calendar = Some(false);
            Some(point)
        } else {
            None
        };
        match point {
            Some(point) => dataset.points.push(point),
            None => dataset.malformed += 1,
        }
    }
    let layout=layout.ok_or("No recognised MRTKLIB solution heading; only LLH, XYZ and ENU text with whitespace separators are supported")?;
    dataset.malformed += before_heading;
    if let Some(reference) = reference {
        let fields: Vec<_> = reference.split_whitespace().collect();
        let count = if layout.coordinates == Coordinates::Dms {
            7
        } else {
            3
        };
        // ENU reference positions can be decimal LLH or DMS, independent of the ENU rows.
        let valid_count =
            fields.len() == count || (layout.coordinates == Coordinates::Enu && fields.len() == 7);
        dataset.base = if valid_count {
            layout.position(&fields, true)
        } else {
            None
        };
        if dataset.base.is_none() {
            dataset.warnings.push("Reference position could not be read; Base Station origin is unavailable (reference separators must also be whitespace)".into());
        }
    }
    // Stable ordering retains duplicate epochs; no data is discarded or merged.
    dataset.points.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(dataset)
}

#[cfg(test)]
pub(crate) fn synthetic() -> Dataset {
    let mut text = String::from(
        "%  GPST                  latitude(deg) longitude(deg) height(m) Q ns sdn(m) sde(m) sdu(m) sdne(m) sdeu(m) sdun(m) age(s) ratio\n",
    );
    for i in 0..80 {
        text.push_str(&format!(
            "2024/08/22 11:{:02}:{:06.3} {:.9} {:.9} {:.4} {} {} .01 .02 .03 0 0 0 {:.2} {:.1}\n",
            i / 60,
            i as f64 % 60.0,
            35.0 + (i as f64 / 8.0).sin() * 0.00001,
            139.0 + (i as f64 / 8.0).cos() * 0.00002,
            40.0 + (i as f64 / 10.0).sin(),
            i % 6 + 1,
            10 + i % 12,
            i as f64 % 5.0,
            3.0 + i as f64 % 4.0
        ));
    }
    parse_with_time_layout(text.as_bytes(), Some(true)).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    const LLH: &str = "latitude(deg) longitude(deg) height(m) Q ns sdn(m) sde(m) sdu(m) sdne(m) sdeu(m) sdun(m) age(s) ratio";
    fn fixture(columns: &str, position: &str, system: &str, calendar: bool, crlf: bool) -> Dataset {
        let timestamp = if calendar {
            "2024/08/22 11:00:00.000"
        } else {
            "2328 385200.000"
        };
        let text = format!(
            "% metadata\n%  {system}                 {columns}\n{timestamp} {position} 1 15 .1 .2 .3 -.1 .2 -.3 2.50 8.0\n"
        );
        let text = if crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        };
        parse(text.as_bytes()).unwrap()
    }
    #[test]
    fn writer_layouts_and_time_systems() {
        for system in ["GPST", "UTC", "JST"] {
            for calendar in [true, false] {
                for crlf in [true, false] {
                    let d = fixture(LLH, "35.0 139.0 40.0", system, calendar, crlf);
                    assert_eq!(d.time_system, system);
                    assert_eq!(d.malformed, 0);
                    assert_eq!(d.points.len(), 1);
                    let p = &d.points[0];
                    assert_eq!(p.quality, 1);
                    assert_eq!(p.satellites, 15);
                    assert_eq!(p.age, 2.5);
                    assert_eq!(p.ratio, 8.0);
                    assert_eq!(
                        p.time,
                        crate::time::calendar("2024/08/22", "11:00:00").unwrap()
                    );
                }
            }
        }
        for (columns, position, relative) in [
            (
                "latitude(d'\") longitude(d'\") height(m) Q ns sdn(m) sde(m) sdu(m) sdne(m) sdeu(m) sdue(m) age(s) ratio",
                "-35 30 0 139 15 0 40",
                false,
            ),
            (
                "x-ecef(m) y-ecef(m) z-ecef(m) Q ns sdx(m) sdy(m) sdz(m) sdxy(m) sdyz(m) sdzx(m) age(s) ratio",
                "-3950000 3350000 3700000",
                false,
            ),
            (
                "e-baseline(m) n-baseline(m) u-baseline(m) Q ns sde(m) sdn(m) sdu(m) sden(m) sdnu(m) sdue(m) age(s) ratio",
                "1.5 -2.5 3.5",
                true,
            ),
        ] {
            let d = fixture(columns, position, "GPST", true, false);
            assert_eq!(d.malformed, 0);
            assert_eq!(d.points.len(), 1);
            assert_eq!(d.relative, relative);
            if relative {
                assert_eq!(d.points[0].position, [1.5, -2.5, 3.5]);
            }
            if position.starts_with("-35 ") {
                let llh = mrtklib_egui_plotkit::ecef_to_llh(d.points[0].position);
                assert!((llh[0] + 35.5).abs() < 1e-8);
                assert!((llh[1] - 139.25).abs() < 1e-8);
            }
        }
    }
    #[test]
    fn malformed_count_and_refused_headings() {
        let good = "2024/08/22 11:00:00.000 35 139 40 1 15 .1 .2 .3 0 0 0 1 3";
        let text = format!(
            "bad before heading\n% GPST {LLH}\n{good}\nbad\n{}\n{}\n{}\n% comment\n",
            good.replace("35 139", "NaN 139"),
            good.replace("1 15", "8 15"),
            good.replace("11:00:00", "25:00:00")
        );
        let d = parse_with_time_layout(text.as_bytes(), Some(true)).unwrap();
        assert_eq!(d.points.len(), 1);
        assert_eq!(d.malformed, 5);
        for text in [
            good.to_owned(),
            format!("% GPST {}\n{good}", LLH.replace(' ', ",")),
            format!("% GPST {LLH}\n{good}").replace("GPST ", "GPST,"),
        ] {
            assert!(parse_with_time_layout(text.as_bytes(), Some(true)).is_err());
        }
        let tab = format!("% GPST {LLH}\n{good}\n").replace(' ', "\t");
        assert_eq!(
            parse_with_time_layout(tab.as_bytes(), Some(true))
                .unwrap()
                .points
                .len(),
            1
        );
    }
    #[test]
    fn reference_and_velocity_suffix() {
        let text = format!(
            "% ref pos : 35 139 40\n% GPST {LLH} vn(m/s) ve(m/s) vu(m/s) sdvn sdve sdvu sdvne sdveu sdvun\n2024/08/22 11:00:00.000 35 139 40 1 15 .1 .2 .3 0 0 0 1 3 0 0 0 0 0 0 0 0 0\n"
        );
        let d = parse_with_time_layout(text.as_bytes(), Some(true)).unwrap();
        assert_eq!(d.points.len(), 1);
        assert_eq!(d.malformed, 0);
        assert_eq!(d.base, Some(llh_to_ecef([35.0, 139.0, 40.0])));
        let malformed = text.replace("35 139 40\n%", "35,139,40\n%");
        let d = parse_with_time_layout(malformed.as_bytes(), Some(true)).unwrap();
        assert!(d.base.is_none());
        assert_eq!(d.warnings.len(), 1);
        assert!(angle(&["-0", "30", "0"]).unwrap() < 0.0);
    }
    #[test]
    fn time_layout_resolves_once_after_counting_bad_rows() {
        let calendar = "2024/08/22 11:00:00.000 35 139 40 1 15 .1 .2 .3 0 0 0 1 3";
        let tow = "2328 385200.000 35 139 40 1 15 .1 .2 .3 0 0 0 1 3";
        for (first, second) in [(calendar, tow), (tow, calendar)] {
            let text = format!("% GPST {LLH}\nbroken\n{first}\n{second}\n{first}\n");
            let d = parse(text.as_bytes()).unwrap();
            assert_eq!(d.points.len(), 2);
            assert_eq!(d.malformed, 2);
        }
    }
    #[test]
    fn reference_layouts_and_geodetic_metadata() {
        let xyz = "x-ecef(m) y-ecef(m) z-ecef(m) Q ns sdx(m) sdy(m) sdz(m) sdxy(m) sdyz(m) sdzx(m) age(s) ratio";
        let enu = "e-baseline(m) n-baseline(m) u-baseline(m) Q ns sde(m) sdn(m) sdu(m) sden(m) sdnu(m) sdue(m) age(s) ratio";
        let dms = "latitude(d'\") longitude(d'\") height(m) Q ns sdn(m) sde(m) sdu(m) sdne(m) sdeu(m) sdue(m) age(s) ratio";
        for (columns, reference, expected) in [
            (xyz, "6378137 0 0", [6378137.0, 0.0, 0.0]),
            (enu, "35 139 40", llh_to_ecef([35.0, 139.0, 40.0])),
            (
                enu,
                "35 30 0 139 15 0 40",
                llh_to_ecef([35.5, 139.25, 40.0]),
            ),
            (
                dms,
                "35 30 0 139 15 0 40",
                llh_to_ecef([35.5, 139.25, 40.0]),
            ),
        ] {
            let text = format!("% ref pos : {reference}\n% GPST {columns}\n");
            let d = parse_with_time_layout(text.as_bytes(), Some(true)).unwrap();
            assert_eq!(d.base, Some(expected));
            assert!(d.warnings.is_empty());
        }
        let text = format!("% (lat/lon/height=WGS84/geodetic,Q=1:fix)\n% GPST {LLH}\n");
        assert_eq!(
            parse_with_time_layout(text.as_bytes(), Some(true))
                .unwrap()
                .warnings
                .len(),
            1
        );
        assert!(
            parse_with_time_layout(text.replace("WGS84", "Tokyo").as_bytes(), Some(true)).is_err()
        );
    }
    #[test]
    #[ignore = "requires MRTKLIB_EGUI_TEST_MRTKLIB_DIR"]
    fn real_mrtklib_pos_files() {
        let root = std::env::var_os("MRTKLIB_EGUI_TEST_MRTKLIB_DIR")
            .filter(|v| !v.is_empty())
            .expect("MRTKLIB_EGUI_TEST_MRTKLIB_DIR must be set for real_mrtklib_pos_files");
        for relative in [
            "tests/data/malib/MALIB_OSS_data_obsnav_240822-1100.pp.pos",
            "tests/data/madocalib/madocalib_MIZU_20250401-0000.pppar.pos",
        ] {
            let path = Path::new(&root).join(relative);
            let d = read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!d.points.is_empty());
            assert_eq!(d.malformed, 0, "{}", path.display());
            assert!(d.points.iter().all(|p| (1..=6).contains(&p.quality)));
        }
    }
}
