//! GPST/UTC conversion for displaying solutions, with a built-in leap-second table.
//!
//! Times are seconds on a civil-calendar axis from 1980-01-06 00:00 (the GPS epoch),
//! in the time system named by the caller. The table must match MRTKLIB's
//! (`src/core/mrtk_time.c`, `leaps`); an opt-in test in the plot crate compares them.
//! A new leap second needs a new row here.

/// UTC dates (year, month, day) from which GPST - UTC equals the second field.
pub const LEAP_SECONDS: [(i32, u32, u32, i32); 18] = [
    (1981, 7, 1, 1),
    (1982, 7, 1, 2),
    (1983, 7, 1, 3),
    (1985, 7, 1, 4),
    (1988, 1, 1, 5),
    (1990, 1, 1, 6),
    (1991, 1, 1, 7),
    (1992, 7, 1, 8),
    (1993, 7, 1, 9),
    (1994, 7, 1, 10),
    (1996, 1, 1, 11),
    (1997, 7, 1, 12),
    (1999, 1, 1, 13),
    (2006, 1, 1, 14),
    (2009, 1, 1, 15),
    (2012, 7, 1, 16),
    (2015, 7, 1, 17),
    (2017, 1, 1, 18),
];

fn days_from_gps_epoch(year: i32, month: u32, day: u32) -> i64 {
    // Days from 1970-01-01 (proleptic Gregorian), then shifted to 1980-01-06.
    let (y, m) = if month <= 2 {
        (i64::from(year) - 1, i64::from(month) + 9)
    } else {
        (i64::from(year), i64::from(month) - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468 - 3657
}

fn utc_start(entry: &(i32, u32, u32, i32)) -> f64 {
    days_from_gps_epoch(entry.0, entry.1, entry.2) as f64 * 86400.0
}

/// GPST - UTC in seconds at a UTC time.
pub fn leap_at_utc(utc: f64) -> i32 {
    LEAP_SECONDS
        .iter()
        .rev()
        .find(|e| utc >= utc_start(e))
        .map_or(0, |e| e.3)
}

pub fn utc_to_gpst(utc: f64) -> f64 {
    utc + f64::from(leap_at_utc(utc))
}

pub fn gpst_to_utc(gpst: f64) -> f64 {
    for e in LEAP_SECONDS.iter().rev() {
        let utc = gpst - f64::from(e.3);
        if utc >= utc_start(e) {
            return utc;
        }
    }
    gpst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gps_epoch_and_known_offsets() {
        assert_eq!(days_from_gps_epoch(1980, 1, 6), 0);
        assert_eq!(leap_at_utc(0.0), 0);
        let d2017 = days_from_gps_epoch(2017, 1, 1) as f64 * 86400.0;
        assert_eq!(leap_at_utc(d2017 - 1.0), 17);
        assert_eq!(leap_at_utc(d2017), 18);
        let t = days_from_gps_epoch(2024, 8, 22) as f64 * 86400.0 + 39600.0;
        assert_eq!(utc_to_gpst(t), t + 18.0);
        assert_eq!(gpst_to_utc(t + 18.0), t);
    }

    /// Opt-in: compare with MRTKLIB's table (`leaps` in `src/core/mrtk_time.c`).
    #[test]
    #[ignore = "needs MRTKLIB_EGUI_TEST_MRTKLIB_DIR"]
    fn real_mrtklib_leap_table_matches() {
        let dir = std::env::var("MRTKLIB_EGUI_TEST_MRTKLIB_DIR")
            .expect("set MRTKLIB_EGUI_TEST_MRTKLIB_DIR to an MRTKLIB source tree");
        let source = std::fs::read_to_string(format!("{dir}/src/core/mrtk_time.c")).unwrap();
        let start = source.find("leaps[MAXLEAPS + 1][7]").expect("leaps table");
        let body = &source[start..source[start..].find("{0}}").unwrap() + start];
        let mut rows: Vec<(i32, u32, u32, i32)> = body
            .split('{')
            .skip(2)
            .filter_map(|row| {
                let v: Vec<f64> = row
                    .split([',', '}'])
                    .filter_map(|x| x.trim().parse().ok())
                    .collect();
                (v.len() == 7).then(|| (v[0] as i32, v[1] as u32, v[2] as u32, -v[6] as i32))
            })
            .collect();
        rows.sort();
        assert_eq!(rows, LEAP_SECONDS.to_vec());
    }

    #[test]
    fn round_trip_across_every_leap_second() {
        for e in &LEAP_SECONDS {
            let start = utc_start(e);
            for utc in [start - 1.0, start, start + 0.5, start + 86400.0] {
                assert_eq!(gpst_to_utc(utc_to_gpst(utc)), utc, "{e:?} {utc}");
            }
        }
    }
}
