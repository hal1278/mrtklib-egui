//! RTKPLOT viewport rules, independent of egui. References live in docs/design.md.
use crate::data::Sample;

const MIN_SCALE: f64 = 2e-5;
const MAX_SCALE: f64 = 1e7;
const DEFAULT_TIME_SPAN: f64 = 600.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Range {
    pub min: f64,
    pub max: f64,
}
impl Range {
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }
    pub fn span(self) -> f64 {
        self.max - self.min
    }
    pub fn center(self) -> f64 {
        self.min + self.span() / 2.0
    }
    pub fn centered(self, center: f64) -> Self {
        Self::new(center - self.span() / 2.0, center + self.span() / 2.0)
    }
    pub fn translated(self, offset: f64) -> Self {
        Self::new(self.min + offset, self.max + offset)
    }
    pub fn scaled(self, anchor: f64, factor: f64, pixels: f64) -> Self {
        let scale = (self.span() * factor / pixels).clamp(MIN_SCALE, MAX_SCALE);
        let factor = scale * pixels / self.span();
        Self::new(
            anchor + (self.min - anchor) * factor,
            anchor + (self.max - anchor) * factor,
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub x: Range,
    pub y: Range,
}
impl Bounds {
    pub fn equal_aspect(self, pixels: [f64; 2]) -> Self {
        let scale = (self.x.span() / pixels[0])
            .max(self.y.span() / pixels[1])
            .clamp(MIN_SCALE, MAX_SCALE);
        Self {
            x: Range::new(-scale * pixels[0] / 2.0, scale * pixels[0] / 2.0)
                .translated(self.x.center()),
            y: Range::new(-scale * pixels[1] / 2.0, scale * pixels[1] / 2.0)
                .translated(self.y.center()),
        }
    }
}

/// FitTime has no padding. Retain its asymmetric singleton case as written in RTKPLOT.
pub(crate) fn fit_time(extent: Option<Range>) -> Range {
    match extent {
        Some(r) if r.min < r.max => r,
        Some(r) if r.min == r.max => Range::new(r.min - DEFAULT_TIME_SPAN / 2.0, r.max),
        _ => Range::new(-DEFAULT_TIME_SPAN / 2.0, DEFAULT_TIME_SPAN / 2.0),
    }
}
/// FitRange adds a fixed 0.05 m on each side, independently of the data span.
pub(crate) fn fit_component(values: impl Iterator<Item = f64>) -> Option<Range> {
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for value in values {
        min = min.min(value);
        max = max.max(value);
    }
    (min <= max).then_some(Range::new(min - 0.05, max + 0.05))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Area {
    Graph(usize),
    Left(usize),
    Right(usize),
    Time,
    Outside,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Track,
    Both(usize),
    Vertical(usize),
    Time,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DragMode {
    Pan,
    Scale,
}

pub(crate) fn drag_target(track: bool, area: Area, mode: DragMode) -> Option<Target> {
    if area == Area::Outside {
        return None;
    }
    if track {
        return Some(Target::Track);
    }
    match (area, mode) {
        (Area::Graph(i), DragMode::Pan) => Some(Target::Both(i)),
        (Area::Graph(_) | Area::Time, _) => Some(Target::Time),
        (Area::Left(i), _) => Some(Target::Vertical(i)),
        _ => None,
    }
}
pub(crate) fn wheel_target(track: bool, area: Area) -> Option<Target> {
    if track {
        return (area != Area::Outside).then_some(Target::Track);
    }
    match area {
        Area::Graph(i) | Area::Left(i) | Area::Right(i) => Some(Target::Vertical(i)),
        Area::Time => Some(Target::Time),
        Area::Outside => None,
    }
}
/// One wheel notch is 120 Windows wheel units; RTKPLOT uses 2^(-delta/1200).
pub(crate) fn wheel_factor(notches: f64) -> f64 {
    2.0_f64.powf(-notches / 10.0)
}
pub(crate) fn drag_factors(delta: [f64; 2]) -> [f64; 2] {
    [
        2.0_f64.powf(-delta[0] / 100.0),
        2.0_f64.powf(delta[1] / 100.0),
    ]
}
pub(crate) fn drag_bounds(
    start: Bounds,
    target: Target,
    mode: DragMode,
    delta: [f64; 2],
    pixels: [f64; 2],
) -> Bounds {
    let mut result = start;
    match mode {
        DragMode::Pan => {
            if !matches!(target, Target::Vertical(_)) {
                result.x = start.x.translated(-delta[0] * start.x.span() / pixels[0]);
            }
            if !matches!(target, Target::Time) {
                result.y = start.y.translated(delta[1] * start.y.span() / pixels[1]);
            }
        }
        DragMode::Scale => {
            let factors = drag_factors(delta);
            if !matches!(target, Target::Vertical(_)) {
                result.x = start.x.scaled(
                    start.x.center(),
                    if target == Target::Track {
                        factors[1]
                    } else {
                        factors[0]
                    },
                    pixels[0],
                );
            }
            if !matches!(target, Target::Time) {
                result.y = start.y.scaled(start.y.center(), factors[1], pixels[1]);
            }
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Viewport {
    pub track: Bounds,
    pub time: Range,
    pub vertical: [Range; 3],
    pending_track: Option<Bounds>,
    track_pixels: Option<[f64; 2]>,
    fit_pixels: Option<[f64; 2]>,
}
impl Default for Viewport {
    fn default() -> Self {
        let range = Range::new(-5.0, 5.0);
        let track = Bounds { x: range, y: range };
        Self {
            track,
            time: fit_time(None),
            vertical: [range; 3],
            pending_track: Some(track),
            track_pixels: None,
            fit_pixels: None,
        }
    }
}
impl Viewport {
    pub fn loaded(
        &mut self,
        kind: usize,
        auto_fit: bool,
        y_range: f64,
        extent: Option<Range>,
        samples: &[Sample],
    ) {
        self.time = fit_time(extent);
        if auto_fit && kind <= 1 {
            self.fit_range(0, samples);
            self.fit_range(1, samples);
        } else {
            self.center_origin(0, y_range);
            self.center_origin(kind.max(1), y_range);
        }
    }
    pub fn plot_type_changed(
        &mut self,
        kind: usize,
        auto_fit: bool,
        y_range: f64,
        samples: &[Sample],
    ) {
        if auto_fit && kind <= 1 {
            self.fit_range(kind, samples);
        } else {
            self.center_origin(kind, y_range);
        }
    }
    pub fn fit_horizontal(&mut self, kind: usize, extent: Option<Range>, samples: &[Sample]) {
        if kind == 0 {
            self.fit_range(kind, samples);
        } else {
            self.time = fit_time(extent);
        }
    }
    pub fn fit_range(&mut self, kind: usize, samples: &[Sample]) {
        if kind == 2 {
            return;
        }
        let ranges: [Option<Range>; 3] =
            std::array::from_fn(|i| fit_component(samples.iter().map(|s| s.enu[i])));
        if kind == 0 {
            if let (Some(x), Some(y)) = (ranges[0], ranges[1]) {
                self.pending_track = Some(Bounds { x, y });
                self.fit_pixels = None;
            }
        } else {
            for (i, range) in ranges.into_iter().enumerate() {
                if let Some(range) = range {
                    self.vertical[i] = range;
                }
            }
        }
    }
    pub fn center_origin(&mut self, kind: usize, range: f64) {
        let range = Range::new(-range, range);
        match kind {
            0 => {
                self.pending_track = Some(Bounds { x: range, y: range });
                self.fit_pixels = None;
            }
            1 => self.vertical = [range; 3],
            _ => {
                self.vertical = [
                    Range::new(0.0, 30.0),
                    Range::new(0.0, 10.0),
                    Range::new(0.0, 20.0),
                ]
            }
        }
    }
    /// Track preserves metres per pixel on resize (GraphT Fit=0); time plots preserve limits.
    pub fn resize_track(&mut self, pixels: [f64; 2]) -> bool {
        if let Some(bounds) = self.pending_track {
            self.track = bounds.equal_aspect(pixels);
            // egui measures axis labels over successive layout passes. Finish an
            // explicit fit only after the data frame has settled, so all data fit.
            if self.fit_pixels == Some(pixels) {
                self.pending_track = None;
                self.fit_pixels = None;
            } else {
                self.fit_pixels = Some(pixels);
            }
        } else if let Some(old) = self.track_pixels {
            if old == pixels {
                return false;
            }
            let scale = self.track.y.span() / old[1];
            self.track = Bounds {
                x: Range::new(-pixels[0] * scale / 2.0, pixels[0] * scale / 2.0)
                    .translated(self.track.x.center()),
                y: Range::new(-pixels[1] * scale / 2.0, pixels[1] * scale / 2.0)
                    .translated(self.track.y.center()),
            };
        }
        self.track_pixels = Some(pixels);
        self.pending_track.is_some()
    }
    pub fn bounds(&self, target: Target) -> Bounds {
        match target {
            Target::Track => self.track,
            Target::Both(i) | Target::Vertical(i) => Bounds {
                x: self.time,
                y: self.vertical[i],
            },
            Target::Time => Bounds {
                x: self.time,
                y: self.vertical[0],
            },
        }
    }
    pub fn set_bounds(&mut self, target: Target, bounds: Bounds) {
        match target {
            Target::Track => {
                self.track = bounds;
                self.pending_track = None;
                self.fit_pixels = None;
            }
            Target::Both(i) => {
                self.time = bounds.x;
                self.vertical[i] = bounds.y;
            }
            Target::Vertical(i) => self.vertical[i] = bounds.y,
            Target::Time => self.time = bounds.x,
        }
    }
    pub fn wheel(&mut self, target: Target, notches: f64, pixels: [f64; 2]) {
        let mut bounds = self.bounds(target);
        let factor = wheel_factor(notches);
        match target {
            // RTKPLOT scales the track plot about its centre (`plotmain.cpp:1716-1718`).
            Target::Track => {
                bounds.x = bounds.x.scaled(bounds.x.center(), factor, pixels[0]);
                bounds.y = bounds.y.scaled(bounds.y.center(), factor, pixels[1]);
            }
            Target::Time => bounds.x = bounds.x.scaled(bounds.x.center(), factor, pixels[0]),
            Target::Vertical(_) => bounds.y = bounds.y.scaled(bounds.y.center(), factor, pixels[1]),
            Target::Both(_) => unreachable!(),
        }
        self.set_bounds(target, bounds);
    }
    pub fn double_click(&mut self, track: bool, point: [f64; 2]) {
        if track {
            self.pending_track = None;
            self.fit_pixels = None;
            self.track.x = self.track.x.centered(point[0]);
            self.track.y = self.track.y.centered(point[1]);
        } else {
            self.time = self.time.centered(point[0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-8, "{a} != {b}");
    }
    #[test]
    fn fit_time_has_no_margin_and_keeps_singleton_source_rule() {
        assert_eq!(
            fit_time(Some(Range::new(1000.0, 2000.0))),
            Range::new(1000.0, 2000.0)
        );
        assert_eq!(
            fit_time(Some(Range::new(1000.0, 1000.0))),
            Range::new(700.0, 1000.0)
        );
        assert_eq!(fit_time(None), Range::new(-300.0, 300.0));
    }
    #[test]
    fn fit_range_has_fixed_margin_and_track_fits_both_dimensions() {
        assert_eq!(
            fit_component([1.0, 101.0].into_iter()),
            Some(Range::new(0.95, 101.05))
        );
        assert_eq!(
            fit_component([3.0].into_iter()),
            Some(Range::new(2.95, 3.05))
        );
        assert_eq!(fit_component([].into_iter()), None);
        let original = Bounds {
            x: Range::new(-2.05, 4.05),
            y: Range::new(-1.05, 1.05),
        };
        for pixels in [[900.0, 300.0], [300.0, 900.0]] {
            let b = original.equal_aspect(pixels);
            close(b.x.span() / pixels[0], b.y.span() / pixels[1]);
            close(b.x.center(), 1.0);
            close(b.y.center(), 0.0);
            assert!(b.x.min <= original.x.min && b.x.max >= original.x.max);
            assert!(b.y.min <= original.y.min && b.y.max >= original.y.max);
        }
    }
    #[test]
    fn scaling_keeps_anchor_at_same_pixel_and_uses_rtkplot_factors() {
        close(wheel_factor(10.0), 0.5);
        close(wheel_factor(-10.0), 2.0);
        assert_eq!(drag_factors([100.0, 100.0]), [0.5, 2.0]);
        let original = Range::new(-10.0, 30.0);
        let anchor = 20.0;
        let scaled = original.scaled(anchor, 0.5, 400.0);
        close(scaled.span(), 20.0);
        close(
            (anchor - original.min) / original.span(),
            (anchor - scaled.min) / scaled.span(),
        );
        close(
            original.scaled(anchor, 0.0, 400.0).span(),
            MIN_SCALE * 400.0,
        );
    }
    #[test]
    fn cursor_area_selects_pan_and_scale_axes() {
        assert_eq!(
            drag_target(false, Area::Graph(1), DragMode::Pan),
            Some(Target::Both(1))
        );
        assert_eq!(
            drag_target(false, Area::Graph(1), DragMode::Scale),
            Some(Target::Time)
        );
        assert_eq!(
            drag_target(false, Area::Left(2), DragMode::Scale),
            Some(Target::Vertical(2))
        );
        assert_eq!(drag_target(false, Area::Right(1), DragMode::Pan), None);
        for area in [Area::Graph(1), Area::Left(1), Area::Right(1)] {
            assert_eq!(wheel_target(false, area), Some(Target::Vertical(1)));
        }
        assert_eq!(wheel_target(false, Area::Time), Some(Target::Time));
        let start = Bounds {
            x: Range::new(0.0, 100.0),
            y: Range::new(-10.0, 10.0),
        };
        let b = drag_bounds(
            start,
            Target::Both(1),
            DragMode::Pan,
            [10.0, 20.0],
            [100.0, 100.0],
        );
        assert_eq!(b.x, Range::new(-10.0, 90.0));
        assert_eq!(b.y, Range::new(-6.0, 14.0));
        let b = drag_bounds(
            start,
            Target::Time,
            DragMode::Scale,
            [100.0, 100.0],
            [100.0, 100.0],
        );
        assert_eq!(b.x, Range::new(25.0, 75.0));
        assert_eq!(b.y, start.y);
    }
    #[test]
    fn double_click_recenters_without_changing_scale() {
        let mut v = Viewport::default();
        v.resize_track([600.0, 300.0]);
        let before = v.clone();
        v.double_click(true, [12.0, -7.0]);
        close(v.track.x.center(), 12.0);
        close(v.track.y.center(), -7.0);
        close(v.track.x.span(), before.track.x.span());
        close(v.track.y.span(), before.track.y.span());
        v.double_click(false, [1234.0, 9999.0]);
        close(v.time.center(), 1234.0);
        assert_eq!(v.vertical, before.vertical);
    }
    #[test]
    fn auto_fit_and_type_change_follow_source_without_refitting_time() {
        let samples = crate::data::samples(
            &crate::reader::synthetic(),
            &crate::settings::Options::default(),
            0,
        )
        .unwrap();
        let mut v = Viewport::default();
        v.loaded(1, true, 5.0, Some(Range::new(1000.0, 2000.0)), &samples);
        assert_eq!(
            v.vertical[0],
            fit_component(samples.iter().map(|s| s.enu[0])).unwrap()
        );
        v.time = Range::new(1100.0, 1200.0);
        v.plot_type_changed(1, false, 2.0, &samples);
        assert_eq!(v.time, Range::new(1100.0, 1200.0));
        assert_eq!(v.vertical, [Range::new(-2.0, 2.0); 3]);
        v.plot_type_changed(2, true, 5.0, &samples);
        assert_eq!(
            v.vertical,
            [
                Range::new(0.0, 30.0),
                Range::new(0.0, 10.0),
                Range::new(0.0, 20.0)
            ]
        );
        let before = v.clone();
        v.fit_range(2, &samples);
        assert_eq!(v, before);
    }
}
