//! Civil calendar arithmetic only; no time-system or leap-second conversion.

/// rtktrace src/analysis/gps_time.cpp:82–104: retain nine digits, round on digit ten.
pub(crate) fn fractional_ns(digits: &str) -> Option<i128> {
    let mut ns = 0;
    let retained = digits.len().min(9);
    for (index, digit) in digits.bytes().enumerate() {
        if !digit.is_ascii_digit() {
            return None;
        }
        if index < retained {
            ns = ns * 10 + i128::from(digit - b'0');
        }
    }
    for _ in retained..9 {
        ns *= 10;
    }
    if digits.len() > 9 && digits.as_bytes()[9] >= b'5' {
        ns += 1;
    }
    Some(ns)
}

fn leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 => {
            if leap(year) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}
fn days_before_year(year: i32) -> i64 {
    let y = i64::from(year - 1);
    365 * y + y / 4 - y / 100 + y / 400
}
fn epoch_days() -> i64 {
    days_before_year(1980) + 5
}
pub(crate) fn calendar(date: &str, clock: &str) -> Option<f64> {
    let d: Vec<_> = date.split('/').collect();
    let t: Vec<_> = clock.split(':').collect();
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    let year: i32 = d[0].parse().ok()?;
    let month: u32 = d[1].parse().ok()?;
    let day: u32 = d[2].parse().ok()?;
    let hour: u32 = t[0].parse().ok()?;
    let minute: u32 = t[1].parse().ok()?;
    let second: f64 = t[2].parse().ok()?;
    if !(1..=9999).contains(&year)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || !second.is_finite()
        || !(0.0..60.0).contains(&second)
    {
        return None;
    }
    let days = days_before_year(year)
        + (1..month)
            .map(|m| i64::from(days_in_month(year, m)))
            .sum::<i64>()
        + i64::from(day - 1)
        - epoch_days();
    Some(days as f64 * 86400.0 + f64::from(hour * 3600 + minute * 60) + second)
}
pub(crate) fn format(seconds: f64, week: bool, date: bool) -> String {
    // Round before splitting, so labels carry into the next minute/day/week.
    let seconds = (seconds * 1000.0).round() / 1000.0;
    if week {
        return format!(
            "{:04}/{:010.3}",
            (seconds / 604800.0).floor() as i64,
            seconds.rem_euclid(604800.0)
        );
    }
    let mut days = (seconds / 86400.0).floor() as i64 + epoch_days();
    let mut year = (days / 366 + 1).clamp(1, 9999) as i32;
    while year < 9999 && days_before_year(year + 1) <= days {
        year += 1;
    }
    days -= days_before_year(year);
    let mut month = 1;
    while month < 12 && days >= i64::from(days_in_month(year, month)) {
        days -= i64::from(days_in_month(year, month));
        month += 1;
    }
    let s = seconds.rem_euclid(86400.0);
    let clock = format!(
        "{:02}:{:02}:{:06.3}",
        (s / 3600.0) as u32,
        (s % 3600.0 / 60.0) as u32,
        s % 60.0
    );
    if date {
        format!("{year:04}/{month:02}/{:02} {clock}", days + 1)
    } else {
        clock
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rtktrace_fraction_rounding_retains_nine_digits_and_carries() {
        for (digits, expected) in [
            ("", 0),
            ("005", 5_000_000),
            ("1234567894", 123_456_789),
            ("1234567895", 123_456_790),
            ("9999999995", 1_000_000_000),
        ] {
            assert_eq!(fractional_ns(digits), Some(expected));
        }
        assert_eq!(fractional_ns("00x"), None);
    }
    #[test]
    fn calendar_and_week_share_axis_without_system_conversion() {
        assert_eq!(calendar("1980/01/06", "00:00:00"), Some(0.0));
        for d in ["1980/01/05", "2000/02/29", "2024/08/22", "2025/04/01"] {
            let seconds = calendar(d, "12:34:56.789").unwrap();
            assert_eq!(format(seconds, false, true), format!("{d} 12:34:56.789"));
        }
        assert_eq!(calendar("2023/02/29", "00:00:00"), None);
        assert_eq!(calendar("2024/01/01", "24:00:00"), None);
        assert_eq!(format(604799.9999, true, true), "0001/000000.000");
    }
}
