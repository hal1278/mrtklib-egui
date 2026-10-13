//! Simulated solutions: a rover circling 100 m east of a fixed base.
//!
//! The values only need to be plausible and consistent with each other (ECEF and
//! latitude/longitude/height, ENU and ECEF covariance), so that a GUI shows the same
//! point in every representation. The conversions are the textbook WGS84 formulas; this is
//! test data, not positioning.

use std::f64::consts::PI;
use std::time::Duration;

use serde_json::{Value, json};

const WGS84_A: f64 = 6_378_137.0;
const WGS84_F: f64 = 1.0 / 298.257_223_563;

/// Base station: latitude and longitude (deg), ellipsoidal height (m).
const BASE: [f64; 3] = [35.6658, 139.7920, 60.0];
const START_WEEK: u64 = 2440;
const START_TOW: f64 = 345_600.0;
const LEAP_SECONDS: i64 = 18;
const GEOID_SEPARATION: f64 = 36.7;

/// The solution of epoch `epoch` of a run. Epochs 0–2 are single, 3–9 float, later fix,
/// and every 40th epoch has no solution.
pub(crate) fn at(epoch: u64, interval: Duration) -> Value {
    let elapsed = epoch as f64 * interval.as_secs_f64();
    let week_seconds = 604_800.0;
    let total = START_TOW + elapsed;
    let time = json!({
        "week": START_WEEK + (total / week_seconds).floor() as u64,
        "tow": total % week_seconds,
    });
    if epoch % 40 == 39 {
        return json!({ "time": time, "leapSeconds": LEAP_SECONDS, "status": "none" });
    }
    let (status, sigma, ratio) = match epoch {
        0..=2 => ("single", [1.5, 1.5, 3.0], 0.0),
        3..=9 => ("float", [0.3, 0.3, 0.6], 1.8),
        _ => ("fix", [0.005, 0.005, 0.010], 15.0),
    };
    let angle = 2.0 * PI * (epoch % 60) as f64 / 60.0;
    let rover = offset(BASE, [100.0 + 3.0 * angle.cos(), 3.0 * angle.sin(), 0.0]);
    json!({
        "time": time,
        "leapSeconds": LEAP_SECONDS,
        "status": status,
        "position": position(rover),
        "covariance": covariance(rover, sigma),
        "geoid": { "model": "internal", "separation": GEOID_SEPARATION },
        "validSatellites": 18,
        "age": if status == "single" { 0.0 } else { 0.6 },
        "ratio": ratio,
        "base": position(BASE),
    })
}

fn position(llh: [f64; 3]) -> Value {
    let [x, y, z] = ecef(llh);
    json!({
        "ecef": { "x": x, "y": y, "z": z },
        "llh": { "lat": llh[0], "lon": llh[1], "height": llh[2] },
    })
}

/// Covariance with standard deviations `sigma` (east, north, up) and no correlation.
fn covariance(llh: [f64; 3], sigma: [f64; 3]) -> Value {
    let [ee, nn, uu] = sigma.map(|s| s * s);
    // ECEF covariance = E^T diag(ee, nn, uu) E, with E the rows of the ENU unit vectors.
    let e = enu_basis(llh);
    let q = |i: usize, j: usize| {
        ee * e[0][i] * e[0][j] + nn * e[1][i] * e[1][j] + uu * e[2][i] * e[2][j]
    };
    json!({
        "ecef": { "xx": q(0, 0), "yy": q(1, 1), "zz": q(2, 2), "xy": q(0, 1), "yz": q(1, 2), "zx": q(2, 0) },
        "enu": { "ee": ee, "nn": nn, "uu": uu, "en": 0.0, "nu": 0.0, "ue": 0.0 },
    })
}

/// Move `llh` by east, north, up metres (small offsets).
fn offset(llh: [f64; 3], enu: [f64; 3]) -> [f64; 3] {
    let lat = llh[0].to_radians();
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let w = (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let meridian = WGS84_A * (1.0 - e2) / w.powi(3);
    let prime_vertical = WGS84_A / w;
    [
        llh[0] + (enu[1] / meridian).to_degrees(),
        llh[1] + (enu[0] / (prime_vertical * lat.cos())).to_degrees(),
        llh[2] + enu[2],
    ]
}

fn ecef(llh: [f64; 3]) -> [f64; 3] {
    let (lat, lon, h) = (llh[0].to_radians(), llh[1].to_radians(), llh[2]);
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let n = WGS84_A / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    [
        (n + h) * lat.cos() * lon.cos(),
        (n + h) * lat.cos() * lon.sin(),
        (n * (1.0 - e2) + h) * lat.sin(),
    ]
}

/// Unit vectors east, north, up in ECEF.
fn enu_basis(llh: [f64; 3]) -> [[f64; 3]; 3] {
    let (lat, lon) = (llh[0].to_radians(), llh[1].to_radians());
    [
        [-lon.sin(), lon.cos(), 0.0],
        [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()],
        [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecef_of_a_known_point() {
        // Equator, prime meridian, on the ellipsoid.
        let [x, y, z] = ecef([0.0, 0.0, 0.0]);
        assert!((x - WGS84_A).abs() < 1e-6 && y.abs() < 1e-6 && z.abs() < 1e-6);
    }

    #[test]
    fn ecef_covariance_keeps_the_trace() {
        let cov = covariance(BASE, [0.3, 0.4, 0.5]);
        let ecef = &cov["ecef"];
        let trace = ecef["xx"].as_f64().unwrap()
            + ecef["yy"].as_f64().unwrap()
            + ecef["zz"].as_f64().unwrap();
        assert!((trace - (0.09 + 0.16 + 0.25)).abs() < 1e-12);
    }

    #[test]
    fn status_sequence() {
        let interval = Duration::from_secs(1);
        assert_eq!(at(0, interval)["status"], "single");
        assert_eq!(at(5, interval)["status"], "float");
        assert_eq!(at(20, interval)["status"], "fix");
        let none = at(39, interval);
        assert_eq!(none["status"], "none");
        assert!(none.get("position").is_none());
    }
}
