//! Origin selection and filtered plot samples; no egui dependency.
use crate::{reader::Dataset, settings::Options};
use mrtklib_egui_plotkit::{ecef_to_enu, llh_to_ecef};

#[derive(Debug)]
pub(crate) struct Origin {
    pub position: [f64; 3],
    velocity: [f64; 3],
    epoch: f64,
}
impl Origin {
    pub fn fixed(llh: [f64; 3]) -> Self {
        Self {
            position: llh_to_ecef(llh),
            velocity: [0.0; 3],
            epoch: 0.0,
        }
    }
    pub fn at(&self, time: f64) -> [f64; 3] {
        std::array::from_fn(|i| self.position[i] + self.velocity[i] * (time - self.epoch))
    }
}
pub(crate) fn origin(data: &Dataset, options: &Options) -> Result<Origin, String> {
    options.validate()?;
    let first = data.points.first().ok_or("No solution epochs")?;
    let n = data.points.len() as f64;
    let mut result = Origin {
        position: first.position,
        velocity: [0.0; 3],
        epoch: first.time,
    };
    result.position = match options.origin {
        0 => first.position,
        1 => data.points.last().unwrap().position,
        2 | 3 => {
            std::array::from_fn(|i| data.points.iter().map(|p| p.position[i]).sum::<f64>() / n)
        }
        4 => data
            .base
            .ok_or("Base Station unavailable: file has no reference position")?,
        5 => llh_to_ecef(options.llh),
        _ => unreachable!(),
    };
    if options.origin == 3 {
        let mean_t = data.points.iter().map(|p| p.time - first.time).sum::<f64>() / n;
        let denominator = data
            .points
            .iter()
            .map(|p| (p.time - first.time - mean_t).powi(2))
            .sum::<f64>();
        if denominator <= 0.0 {
            return Err("Linear Fit Pos needs at least two distinct epoch times".into());
        }
        for i in 0..3 {
            result.velocity[i] = data
                .points
                .iter()
                .map(|p| (p.time - first.time - mean_t) * (p.position[i] - result.position[i]))
                .sum::<f64>()
                / denominator;
            result.position[i] -= result.velocity[i] * mean_t;
        }
    }
    if result.position.iter().map(|v| v * v).sum::<f64>() < 1.0 {
        return Err("Origin is not a valid Earth position".into());
    }
    Ok(result)
}
#[derive(Clone, Debug)]
pub(crate) struct Sample {
    /// 0/1 are input datasets; 2 is their difference (palette 0).
    pub source: usize,
    pub index: usize,
    pub time: f64,
    pub enu: [f64; 3],
    pub quality: u8,
    pub nsat: [f64; 3],
}
#[cfg(test)]
pub(crate) fn samples(
    data: &Dataset,
    options: &Options,
    quality: u8,
) -> Result<Vec<Sample>, String> {
    let origin = if data.relative {
        None
    } else {
        Some(origin(data, options)?)
    };
    Ok(samples_with_origin(data, origin.as_ref(), quality))
}
pub(crate) fn samples_with_origin(
    data: &Dataset,
    origin: Option<&Origin>,
    quality: u8,
) -> Vec<Sample> {
    data.points
        .iter()
        .enumerate()
        .filter(|(_, p)| quality == 0 || p.quality == quality)
        .map(|(index, p)| Sample {
            source: 0,
            index,
            time: p.time,
            enu: if data.relative {
                p.position
            } else {
                origin.map_or(p.position, |o| ecef_to_enu(p.position, o.at(p.time)))
            },
            quality: p.quality,
            nsat: [f64::from(p.satellites), p.age, p.ratio],
        })
        .collect()
}
/// RTKPLOT TIMEPOS::diff: ordered, one-to-one matches within default TTOL.
pub(crate) fn difference(a: &[Sample], b: &[Sample], quality: u8) -> Vec<Sample> {
    const TTOL: f64 = 0.025;
    let (mut i, mut j) = (0, 0);
    let mut result = Vec::new();
    while i < a.len() && j < b.len() {
        let dt = a[i].time - b[j].time;
        if dt < -TTOL {
            i += 1;
        } else if dt > TTOL {
            j += 1;
        } else {
            let q = a[i].quality.max(b[j].quality);
            if quality == 0 || quality == q {
                result.push(Sample {
                    source: 2,
                    enu: std::array::from_fn(|k| a[i].enu[k] - b[j].enu[k]),
                    quality: q,
                    ..a[i].clone()
                });
            }
            i += 1;
            j += 1;
        }
    }
    result
}
/// Convert RTKPLOT's literal BGR TColor palette to RGB.
pub(crate) fn mark_color(source: usize, quality: u8) -> [u8; 3] {
    if source != 1 {
        return mrtklib_egui_plotkit::solution_style(quality).1;
    }
    const COLORS: [u32; 8] = [
        0xC0C0C0, 0x804000, 0x008080, 0xFF0080, 0xFF8000, 0x8080FF, 0xFF8080, 0x808080,
    ];
    let c = COLORS[usize::from(quality.min(7))];
    [c as u8, (c >> 8) as u8, (c >> 16) as u8]
}
/// Index ranges preserve file order and quality-filtered adjacency.
pub(crate) fn segments(samples: &[Sample], time_series: bool) -> Vec<std::ops::Range<usize>> {
    if samples.is_empty() {
        return vec![];
    }
    let mut start = 0;
    let mut ranges = vec![];
    for i in 1..samples.len() {
        if time_series && (samples[i].time - samples[i - 1].time).abs() > 300.0 {
            ranges.push(start..i);
            start = i;
        }
    }
    ranges.push(start..samples.len());
    ranges
}
#[cfg(test)]
mod tests {
    use super::*;
    use mrtklib_egui_plotkit::SolutionPoint;
    #[test]
    fn difference_matches_once_without_interpolation_and_filters_the_larger_q() {
        let mut a = samples(&dataset(), &Options::default(), 0).unwrap();
        let mut b = a.clone();
        a[0].quality = 1;
        b[0].quality = 5;
        b[0].time += 0.024;
        b[0].enu[0] -= 3.0;
        b[1].time += 0.026;
        b[2].quality = 1;
        b[2].enu[2] += 2.0;
        let d = difference(&a, &b, 0);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].index, d[0].time, d[0].quality), (0, a[0].time, 5));
        assert_eq!(d[0].enu, [3.0, 0.0, 0.0]);
        assert_eq!(d[1].quality, 3);
        assert_eq!(d[1].enu, [0.0, 0.0, -2.0]);
        assert_eq!(difference(&a, &b, 5).len(), 1);
        assert!(difference(&a, &b, 1).is_empty());
        // Duplicate timestamps are consumed one-to-one, in file order.
        a[1].time = a[0].time;
        b[1].time = b[0].time;
        assert_eq!(difference(&a, &b[..1], 0).len(), 1);
        assert_eq!(difference(&a, &b, 0).len(), 3);
        assert!(difference(&[], &b, 0).is_empty());
    }
    #[test]
    fn rtkplot_second_palette_uses_bgr_literals_and_difference_uses_first() {
        let expected = [
            [192, 192, 192],
            [0, 64, 128],
            [128, 128, 0],
            [128, 0, 255],
            [0, 128, 255],
            [255, 128, 128],
            [128, 128, 255],
            [128, 128, 128],
        ];
        for (q, rgb) in expected.into_iter().enumerate() {
            assert_eq!(mark_color(1, q as u8), rgb);
            assert_eq!(
                mark_color(0, q as u8),
                mrtklib_egui_plotkit::solution_style(q as u8).1
            );
            assert_eq!(mark_color(2, q as u8), mark_color(0, q as u8));
        }
    }
    fn dataset() -> Dataset {
        Dataset {
            points: (0..3)
                .map(|i| SolutionPoint {
                    time: f64::from(i) * 100.0,
                    position: [6_378_137.0, f64::from(i) * 10.0, 0.0],
                    quality: i as u8 + 1,
                    satellites: 12,
                    age: 1.0,
                    ratio: 3.0,
                })
                .collect(),
            base: Some([6_378_137.0, 0.0, 0.0]),
            relative: false,
            time_system: "GPST".into(),
            malformed: 0,
            warnings: vec![],
        }
    }
    #[test]
    fn all_origins_and_moving_fit() {
        let d = dataset();
        let mut o = Options::default();
        for (kind, y) in [(0, 0.0), (1, 20.0), (2, 10.0), (4, 0.0), (5, 0.0)] {
            o.origin = kind;
            assert!((origin(&d, &o).unwrap().at(0.0)[1] - y).abs() < 1e-8);
        }
        o.origin = 3;
        let fit = origin(&d, &o).unwrap();
        assert!((fit.at(50.0)[1] - 5.0).abs() < 1e-8);
        assert!(
            samples(&d, &o, 0)
                .unwrap()
                .iter()
                .all(|s| s.enu.iter().all(|v| v.abs() < 1e-8))
        );
        let mut single = dataset();
        single.points.truncate(1);
        assert!(origin(&single, &o).is_err());
        o.origin = 4;
        single.base = None;
        assert!(origin(&single, &o).is_err());
    }
    #[test]
    fn relative_pass_through_filter_and_gap_breaking() {
        let mut d = dataset();
        d.relative = true;
        d.base = None;
        let o = Options {
            origin: 4,
            ..Default::default()
        };
        let filtered = samples(&d, &o, 2).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].enu, d.points[1].position);
        let mut s = samples(&d, &o, 0).unwrap();
        s[1].time = 300.0;
        s[2].time = 601.0;
        assert_eq!(segments(&s, true), vec![0..2, 2..3]);
        assert_eq!(segments(&s, false), vec![0..3]);
        assert!(samples(&d, &o, 6).unwrap().is_empty());
    }
}
