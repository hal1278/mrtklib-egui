//! rtktrace rate/interval statistics and RTKPLOT baseline display (see design.md).
use crate::{controller::display_time, data::Sample, reader::Dataset, settings::Options, time};

/// rtktrace shortest interval; see the Plot rate rule in docs/design.md (>=1 ms).
pub(crate) fn rate(times: impl IntoIterator<Item = i128>) -> Option<f64> {
    let times: Vec<_> = times.into_iter().collect();
    times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|dt| *dt >= 1_000_000)
        .min()
        .map(|dt| {
            // The displayed precision is also the precision used for interval percentages.
            format!("{:.2e}", 1e9 / dt as f64)
                .parse()
                .expect("finite positive rate")
        })
}

pub(crate) fn rate_text(hz: f64) -> String {
    let decimals = 2 - hz.log10().floor() as i32;
    if (0..=6).contains(&decimals) {
        format!("{hz:.precision$}", precision = decimals as usize)
    } else {
        format!("{hz:.2e}")
    }
}

/// rtktrace src/analysis/statistics.cpp:37–65; inclusive interval, rounded then +1.
pub(crate) fn expected(start: i128, end: i128, hz: Option<f64>) -> Option<usize> {
    let hz = hz.filter(|hz| hz.is_finite() && *hz > 0.0)?;
    if end < start {
        return None;
    }
    let duration_ns = end - start;
    let intervals = (duration_ns as f64 / 1e9 * hz + 0.5).floor();
    if !intervals.is_finite() || intervals >= usize::MAX as f64 {
        return None;
    }
    Some(intervals as usize + 1)
}

pub(crate) fn percentage(count: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| count as f64 / denominator as f64 * 100.0)
}

pub(crate) fn baseline(dataset: &Dataset) -> String {
    let base = dataset
        .base
        .filter(|p| p.iter().map(|v| v * v).sum::<f64>() > 0.0);
    if !dataset.relative && base.is_none() {
        return String::new();
    }
    let lengths = dataset.points.iter().map(|p| {
        let r = if dataset.relative {
            p.position
        } else {
            std::array::from_fn(|i| p.position[i] - base.unwrap()[i])
        };
        r.iter().map(|v| v * v).sum::<f64>().sqrt()
    });
    let (min, max) = lengths.fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), b| {
        (min.min(b), max.max(b))
    });
    if !min.is_finite() {
        return String::new();
    }
    if min + 100.0 < max {
        format!("  B={:.1}–{:.1}km", min / 1000.0, max / 1000.0)
    } else {
        format!("  B={:.1}km", min / 1000.0)
    }
}

pub(crate) fn line(
    label: &str,
    samples: &[Sample],
    dataset: &Dataset,
    options: &Options,
    interval: bool,
    override_hz: Option<f64>,
) -> String {
    let mut counts = [0usize; 8];
    for s in samples {
        counts[usize::from(s.quality)] += 1;
    }
    let Some((start, end)) = samples.first().zip(samples.last()) else {
        return format!("[{label}] N=0");
    };
    // Samples may already use display time. Read GPST from their source indices;
    // difference epochs retain the Solution-1 index and use its dataset here.
    let first = display_time(dataset.points[start.index].time, options);
    let last = display_time(dataset.points[end.index].time, options);
    let date_end = options.week_time
        || time::format(first, false, true)[..10] != time::format(last, false, true)[..10];
    let hz = (interval && samples.len() > 1)
        .then(|| override_hz.or_else(|| rate(samples.iter().map(|s| s.time_ns))))
        .flatten();
    let denominator = if interval {
        expected(start.time_ns, end.time_ns, hz)
    } else {
        Some(samples.len())
    };
    // `% Interval` shows its denominator: epochs present / expected (rate).
    let n = if interval {
        format!(
            "{}/{} ({})",
            samples.len(),
            denominator.map_or("—".into(), |d| d.to_string()),
            hz.map_or("—".into(), |h| if override_hz.is_some() {
                format!("{h} Hz manual")
            } else {
                format!("{} Hz", rate_text(h))
            })
        )
    } else {
        samples.len().to_string()
    };
    let mut text = format!(
        "[{label}] {} - {} {}  N={n}{}",
        time::format(first, options.week_time, true),
        time::format(last, options.week_time, date_end),
        if options.utc && !options.week_time {
            "UTC"
        } else {
            "GPST"
        },
        baseline(dataset)
    );
    for (q, count) in counts.into_iter().enumerate().filter(|(_, n)| *n > 0) {
        text.push_str(&format!("  Q{q}={count}"));
        if let Some(pct) = denominator.and_then(|n| percentage(count, n)) {
            text.push_str(&format!(" ({pct:.1}%)"));
        }
    }
    text
}

/// Keep the complete filename; remove a middle portion of directories by measured width.
pub(crate) fn elide_path(path: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(path) <= width {
        return path.into();
    }
    let Some(split) = path.rfind(['/', '\\']) else {
        return path.into();
    };
    let (directory, file) = path.split_at(split + 1);
    let chars: Vec<char> = directory.chars().collect();
    for keep in (0..chars.len()).rev() {
        let left = keep.div_ceil(2);
        let right = keep / 2;
        let candidate = format!(
            "{}...{}{file}",
            chars[..left].iter().collect::<String>(),
            chars[chars.len() - right..].iter().collect::<String>()
        );
        if measure(&candidate) <= width {
            return candidate;
        }
    }
    format!("...{file}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rtktrace_rate_threshold_duplicates_and_gaps() {
        assert_eq!(rate([]), None);
        assert_eq!(rate([1_000_000_000]), None);
        assert_eq!(rate([0, 5_000_000, 5_000_000, 4_000_000]), Some(200.0));
        assert_eq!(rate([0, 5_000_000, 15_000_000, 1_015_000_000]), Some(200.0));
        assert_eq!(
            rate([0, 1_000_000_000, 4_000_000_000, 5_000_000_000]),
            Some(1.0)
        );
    }
    #[test]
    fn shortest_interval_estimate_includes_one_ms_and_manual_rate_overrides_it() {
        assert_eq!(rate([0, 1_000_000]), Some(1000.0));
        assert_eq!(rate([0, 999_999]), None);
        assert_eq!(rate([0, 999_999, 1_999_999]), Some(1000.0));
        assert_eq!(rate([0, 5_000_000]), Some(200.0));
        assert_eq!(rate([0, 1_000_000_000, 3_000_000_000]), Some(1.0));
        assert_eq!(rate([0, 10_000_000, 1_000_000_000]), Some(100.0));
        assert_eq!(rate([0]), None);
        let mut d = crate::reader::synthetic();
        d.points.truncate(3);
        let samples = crate::data::samples_with_origin(&d, None, 0);
        assert!(
            line("1", &samples, &d, &Options::default(), true, Some(2.0)).contains("Q1=1 (20.0%)")
        );
        assert!(line("1", &samples, &d, &Options::default(), true, None).contains("Q1=1 (33.3%)"));
        assert!(!line("1", &samples[..1], &d, &Options::default(), true, Some(2.0)).contains('%'));
    }
    #[test]
    fn estimated_rates_use_three_significant_digits_for_display_and_percentages() {
        for (interval_ns, hz, text) in [
            (999_000_000, 1.0, "1.00"),
            (30_000_000_000, 0.0333, "0.0333"),
            (50_000_000, 20.0, "20.0"),
        ] {
            let estimate = rate([0, interval_ns]).unwrap();
            assert_eq!(estimate, hz);
            assert_eq!(rate_text(estimate), text);
        }
        let d = crate::reader::synthetic();
        let template = crate::data::samples_with_origin(&d, None, 0)[0].clone();
        let mut samples: Vec<_> = (0..1000)
            .map(|i| Sample {
                time: template.time + f64::from(i),
                time_ns: template.time_ns + i128::from(i) * 1_000_000_000,
                quality: 1,
                ..template.clone()
            })
            .collect();
        samples[1].time_ns -= 1_000_000;
        assert!(
            line("1", &samples, &d, &Options::default(), true, None).contains("Q1=1000 (100.0%)")
        );
        assert!(
            line(
                "1",
                &samples,
                &d,
                &Options::default(),
                true,
                Some(1.001001001)
            )
            .contains("Q1=1000 (99.9%)")
        );
    }
    #[test]
    fn expected_interval_percentages_do_not_clamp_duplicates() {
        assert_eq!(expected(0, 4_000_000_000, Some(1.0)), Some(5));
        assert_eq!(expected(0, 1_500_000_000, Some(1.0)), Some(3));
        assert_eq!(expected(0, 0, Some(1.0)), Some(1));
        assert_eq!(expected(1_000_000_000, 0, Some(1.0)), None);
        for hz in [None, Some(0.0), Some(f64::NAN), Some(f64::INFINITY)] {
            assert_eq!(expected(0, 4_000_000_000, hz), None);
        }
        assert_eq!(percentage(3, 5), Some(60.0));
        assert_eq!(percentage(6, 5), Some(120.0));
        assert_eq!(percentage(1, 0), None);
    }
    #[test]
    fn interval_mode_shows_present_expected_and_rate() {
        let mut d = crate::reader::synthetic();
        d.points.truncate(3);
        for (p, clock) in d
            .points
            .iter_mut()
            .zip(["00:00:00", "00:00:01", "00:00:03"])
        {
            p.time = time::calendar("2024/08/22", clock).unwrap();
            p.time_ns = p.time as i128 * 1_000_000_000;
        }
        let samples = crate::data::samples_with_origin(&d, None, 0);
        let o = Options::default();
        let text = line("1", &samples, &d, &o, true, None);
        assert!(text.contains("N=3/4 (1.00 Hz)"), "{text}");
        let text = line("1", &samples, &d, &o, true, Some(2.0));
        assert!(text.contains("N=3/7 (2 Hz manual)"), "{text}");
        assert!(line("1", &samples, &d, &o, false, None).contains("N=3"));
    }

    #[test]
    fn summary_converts_gpst_before_formatting_times_and_comparing_dates() {
        let mut d = crate::reader::synthetic();
        d.points.truncate(2);
        for (p, clock) in d.points.iter_mut().zip(["00:00:05", "00:00:25"]) {
            p.time = time::calendar("2024/08/22", clock).unwrap();
            p.time_ns = p.time as i128 * 1_000_000_000;
        }
        let samples = crate::data::samples_with_origin(&d, None, 0);
        for (utc, week_time, expected) in [
            (false, false, "2024/08/22 00:00:05.000 - 00:00:25.000 GPST"),
            (
                true,
                false,
                "2024/08/21 23:59:47.000 - 2024/08/22 00:00:07.000 UTC",
            ),
            (true, true, "2328/345605.000 - 2328/345625.000 GPST"),
        ] {
            let options = Options {
                utc,
                week_time,
                ..Default::default()
            };
            let text = line("1", &samples, &d, &options, false, None);
            assert!(text.starts_with(&format!("[1] {expected}  N=2")), "{text}");
        }
        assert_eq!(
            samples[0].time, d.points[0].time,
            "summary must not mutate the stored GPST times"
        );
    }
    #[test]
    fn path_ellipsis_preserves_unicode_and_entire_filename() {
        let measure = |s: &str| s.chars().count() as f32;
        assert_eq!(
            elide_path("/one/two/file.pos", 100.0, measure),
            "/one/two/file.pos"
        );
        for width in [1.0, 12.0, 18.0] {
            let s = elide_path("/長い/ディレクトリ/測位結果.pos", width, measure);
            assert!(s.ends_with("測位結果.pos"));
            assert!(s.contains("..."));
        }
        assert_eq!(
            elide_path("long_filename.pos", 1.0, measure),
            "long_filename.pos"
        );
    }
    #[test]
    fn baseline_omits_absent_base_and_uses_strict_spread() {
        let mut d = crate::reader::synthetic();
        d.base = None;
        assert!(baseline(&d).is_empty());
        d.base = Some([0.0; 3]);
        assert!(baseline(&d).is_empty());
        d.relative = true;
        d.points.truncate(2);
        d.points[0].position = [1000.0, 0.0, 0.0];
        d.points[1].position = [1100.0, 0.0, 0.0];
        assert_eq!(baseline(&d), "  B=1.0km");
        d.points[1].position[0] += 0.1;
        assert_eq!(baseline(&d), "  B=1.0–1.1km");
    }
}
