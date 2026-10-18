//! Display-only solution geometry, quality palette and component statistics.

pub mod leap;

/// Coordinates are metres in ECEF, or metres in ENU for a relative dataset.
#[derive(Clone, Debug)]
pub struct SolutionPoint {
    /// Continuous GPST seconds since 1980-01-06 (UTC and JST files are converted when read).
    pub time: f64,
    pub position: [f64; 3],
    pub quality: u8,
    pub satellites: u16,
    pub age: f64,
    pub ratio: f64,
}

pub fn solution_style(q: u8) -> (&'static str, [u8; 3]) {
    match q {
        0 => ("----", [192, 192, 192]),
        1 => ("FIX", [0, 128, 0]),
        2 => ("FLOAT", [255, 170, 0]),
        3 => ("SBAS", [255, 0, 255]),
        4 => ("DGPS", [0, 0, 255]),
        5 => ("SINGLE", [255, 0, 0]),
        6 => ("PPP", [0, 128, 128]),
        _ => ("DR", [128, 128, 128]),
    }
}

const A: f64 = 6_378_137.0;
const E2: f64 = (1.0 / 298.257_223_563) * (2.0 - 1.0 / 298.257_223_563);

/// WGS84 latitude/longitude in degrees and ellipsoidal height in metres.
pub fn llh_to_ecef([lat, lon, h]: [f64; 3]) -> [f64; 3] {
    let (s, c) = lat.to_radians().sin_cos();
    let (sl, cl) = lon.to_radians().sin_cos();
    let n = A / (1.0 - E2 * s * s).sqrt();
    [(n + h) * c * cl, (n + h) * c * sl, (n * (1.0 - E2) + h) * s]
}

pub fn ecef_to_llh([x, y, z]: [f64; 3]) -> [f64; 3] {
    let p = x.hypot(y);
    if p < 1e-9 {
        return [
            if z < 0.0 { -90.0 } else { 90.0 },
            0.0,
            z.abs() - A * (1.0 - E2).sqrt(),
        ];
    }
    let mut lat = z.atan2(p * (1.0 - E2));
    for _ in 0..15 {
        let n = A / (1.0 - E2 * lat.sin().powi(2)).sqrt();
        let next = (z + E2 * n * lat.sin()).atan2(p);
        if (lat - next).abs() < 1e-14 {
            lat = next;
            break;
        }
        lat = next;
    }
    let n = A / (1.0 - E2 * lat.sin().powi(2)).sqrt();
    let h = p * lat.cos() + z * lat.sin() - n * (1.0 - E2 * lat.sin().powi(2));
    [lat.to_degrees(), y.atan2(x).to_degrees(), h]
}

pub fn ecef_to_enu(position: [f64; 3], origin: [f64; 3]) -> [f64; 3] {
    let [lat, lon, _] = ecef_to_llh(origin);
    let (s, c) = lat.to_radians().sin_cos();
    let (sl, cl) = lon.to_radians().sin_cos();
    let [x, y, z] = std::array::from_fn(|i| position[i] - origin[i]);
    [
        -sl * x + cl * y,
        -s * cl * x - s * sl * y + c * z,
        c * cl * x + c * sl * y + s * z,
    ]
}

/// Inverse of [`ecef_to_enu`]: ECEF position of a local ENU offset from `origin`.
pub fn enu_to_ecef(enu: [f64; 3], origin: [f64; 3]) -> [f64; 3] {
    let [lat, lon, _] = ecef_to_llh(origin);
    let (s, c) = lat.to_radians().sin_cos();
    let (sl, cl) = lon.to_radians().sin_cos();
    let [e, n, u] = enu;
    [
        origin[0] - sl * e - s * cl * n + c * cl * u,
        origin[1] + cl * e - s * sl * n + c * sl * u,
        origin[2] + c * n + s * u,
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Statistics {
    pub average: f64,
    pub std: f64,
    pub rms: f64,
}
pub fn statistics(values: &[f64]) -> Statistics {
    if values.is_empty() {
        return Statistics::default();
    }
    let n = values.len() as f64;
    let average = values.iter().sum::<f64>() / n;
    Statistics {
        average,
        std: if values.len() > 1 {
            (values.iter().map(|v| (v - average).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        } else {
            0.0
        },
        rms: (values.iter().map(|v| v * v).sum::<f64>() / n).sqrt(),
    }
}
/// RMS of the horizontal distance sqrt(E^2+N^2) about the origin. RTKPLOT labels twice this
/// value `2D`; that name and the factor are ambiguous, so only the plain RMS is given.
pub fn horizontal_rms(values: &[[f64; 3]]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    (values
        .iter()
        .map(|v| v[0] * v[0] + v[1] * v[1])
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64, tolerance: f64) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }
    #[test]
    fn known_wgs84_and_round_trips() {
        assert_eq!(llh_to_ecef([0.0; 3]), [A, 0.0, 0.0]);
        close(llh_to_ecef([90.0, 0.0, 0.0])[2], 6_356_752.314_245, 1e-6);
        for llh in [
            [35.0, 139.0, 42.0],
            [-60.0, -179.0, 1000.0],
            [90.0, 0.0, 10.0],
            [-90.0, 0.0, 0.0],
            [0.0, 180.0, -30.0],
        ] {
            let back = ecef_to_llh(llh_to_ecef(llh));
            for i in 0..3 {
                close(back[i], llh[i], 1e-7);
            }
        }
    }
    #[test]
    fn enu_axes_at_equator() {
        assert_eq!(
            ecef_to_enu([A + 3.0, 1.0, 2.0], [A, 0.0, 0.0]),
            [1.0, 2.0, 3.0]
        );
        assert_eq!(ecef_to_enu([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), [0.0; 3]);
    }
    #[test]
    fn enu_to_ecef_inverts_ecef_to_enu() {
        let origin = llh_to_ecef([35.6, 139.7, 40.0]);
        let enu = [120.5, -33.25, 7.0];
        let back = ecef_to_enu(enu_to_ecef(enu, origin), origin);
        for i in 0..3 {
            close(back[i], enu[i], 1e-8);
        }
    }
    #[test]
    fn sample_statistics_and_horizontal_rms() {
        assert_eq!(statistics(&[]), Statistics::default());
        assert_eq!(
            statistics(&[3.0]),
            Statistics {
                average: 3.0,
                std: 0.0,
                rms: 3.0
            }
        );
        let s = statistics(&[1.0, 3.0]);
        close(s.average, 2.0, 1e-12);
        close(s.std, 2.0_f64.sqrt(), 1e-12);
        close(s.rms, 5.0_f64.sqrt(), 1e-12);
        assert_eq!(horizontal_rms(&[[3.0, 4.0, 99.0]]), 5.0);
        assert_eq!(horizontal_rms(&[]), 0.0);
    }
}
