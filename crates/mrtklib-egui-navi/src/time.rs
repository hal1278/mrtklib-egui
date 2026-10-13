//! Display conversion only: GPST epoch plus the server's UTC offset, no leap table.

use mrtklib_egui_rpc::GpsTime;

const WEEK_MS: i64 = 604_800_000;
const DAY_MS: i64 = 86_400_000;
const GPS_EPOCH_UNIX_DAYS: i64 = 3657;

pub(crate) fn format(time: GpsTime, leap_seconds: i32) -> [String; 2] {
    checked_format(time, leap_seconds)
        .unwrap_or_else(|| ["GPST invalid time".into(), "UTC invalid time".into()])
}

fn checked_format(time: GpsTime, leap_seconds: i32) -> Option<[String; 2]> {
    // Validate the unrounded contract value: a valid final millisecond may carry
    // into the next week, but 604800 itself is not a valid time of week.
    if !time.tow.is_finite() || !(0.0..604_800.0).contains(&time.tow) {
        return None;
    }
    // Round before splitting fields so carries cross seconds, days and GPS weeks.
    // The validated range bounds the rounded float to 0..=WEEK_MS, so its cast
    // cannot saturate. Keep the accumulated epoch arithmetic checked as well.
    let gps_ms = i64::from(time.week)
        .checked_mul(WEEK_MS)?
        .checked_add((time.tow * 1000.0).round() as i64)?;
    let week = gps_ms.div_euclid(WEEK_MS);
    let tow = gps_ms.rem_euclid(WEEK_MS);
    let utc_ms = gps_ms.checked_sub(i64::from(leap_seconds).checked_mul(1000)?)?;
    let (year, month, day) =
        civil_date(utc_ms.div_euclid(DAY_MS).checked_add(GPS_EPOCH_UNIX_DAYS)?);
    let clock = utc_ms.rem_euclid(DAY_MS);
    let seconds = clock / 1000;
    Some([
        format!("GPST {week} {}.{:03}", tow / 1000, tow % 1000),
        format!(
            "UTC {year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60,
            clock % 1000
        ),
    ])
}

/// Gregorian civil date from days since 1970-01-01. Split into 400-year eras,
/// counting from March so leap days are at the end of each year.
fn civil_date(unix_days: i64) -> (i64, i64, i64) {
    let days = unix_days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = march_month + if march_month < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(week: u32, tow: f64, leap: i32) -> [String; 2] {
        format(GpsTime { week, tow }, leap)
    }

    #[test]
    fn epoch_and_offset_crossing_previous_day() {
        assert_eq!(
            at(0, 0.0, 0),
            ["GPST 0 0.000", "UTC 1980-01-06T00:00:00.000Z"]
        );
        assert_eq!(at(0, 0.123, 18)[1], "UTC 1980-01-05T23:59:42.123Z");
    }

    #[test]
    fn invalid_time_of_week_shows_placeholders_without_overflow() {
        for tow in [
            f64::MAX,
            1.0e20,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            -f64::MAX,
            -1.0,
            -f64::MIN_POSITIVE,
            604_800.0,
            604_800.001,
        ] {
            for week in [0, u32::MAX] {
                for leap in [i32::MIN, 0, i32::MAX] {
                    assert_eq!(
                        at(week, tow, leap),
                        ["GPST invalid time", "UTC invalid time"],
                        "week={week}, tow={tow}, leap={leap}"
                    );
                }
            }
        }
    }

    #[test]
    fn valid_boundaries_preserve_week_and_leap_seconds() {
        let before_week_end = f64::from_bits(604_800.0_f64.to_bits() - 1);
        assert_eq!(at(0, before_week_end, 0)[0], "GPST 1 0.000");
        assert_eq!(at(u32::MAX, 604_799.999_6, 0)[0], "GPST 4294967296 0.000");
        assert_eq!(at(0, f64::MIN_POSITIVE, 0)[0], "GPST 0 0.000");
        for week in [0, u32::MAX] {
            for tow in [0.0, before_week_end] {
                let baseline = at(week, tow, 0);
                for leap in [i32::MIN, i32::MAX] {
                    let offset = at(week, tow, leap);
                    assert_eq!(offset[0], baseline[0]);
                    assert_ne!(offset[1], "UTC invalid time");
                    assert_ne!(offset[1], baseline[1]);
                }
            }
        }
    }

    #[test]
    fn rounding_carries_through_second_day_and_week() {
        assert_eq!(at(0, 59.9996, 0)[1], "UTC 1980-01-06T00:01:00.000Z");
        assert_eq!(at(0, 86_399.999_6, 0)[1], "UTC 1980-01-07T00:00:00.000Z");
        assert_eq!(
            at(0, 604_799.999_6, 0),
            ["GPST 1 0.000", "UTC 1980-01-13T00:00:00.000Z"]
        );
        assert_eq!(at(1, 604_799.999_4, 0)[0], "GPST 1 604799.999");
        assert_eq!(at(0, 0.0005, 0)[0], "GPST 0 0.001");
    }

    #[test]
    fn year_and_leap_day_boundaries() {
        assert_eq!(
            at(1042, 518_412.999_6, 13)[1],
            "UTC 2000-01-01T00:00:00.000Z"
        );
        assert_eq!(at(1051, 172_813.0, 13)[1], "UTC 2000-02-29T00:00:00.000Z");
        assert_eq!(at(1051, 259_213.0, 13)[1], "UTC 2000-03-01T00:00:00.000Z");
        // 2100 is divisible by 100 but not 400: it has no February 29.
        assert_eq!(civil_date(47_541), (2100, 3, 1));
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(-1), (1969, 12, 31));
    }
}
