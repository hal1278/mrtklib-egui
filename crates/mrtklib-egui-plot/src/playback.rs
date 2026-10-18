//! Raw solution indices and repaint-clock playback, following RTKPLOT's timer.
use crate::{navigation::Viewport, settings::Options};

pub(crate) struct Playback {
    pub show_track: bool,
    pub fix_center: bool,
    pub fix_horizontal: bool,
    pub fix_vertical: bool,
    pub horizontal_offset: f64,
    pub indices: [usize; 2],
    pub animating: bool,
    last_tick: Option<f64>,
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            show_track: false,
            fix_center: true,
            fix_horizontal: false,
            fix_vertical: true,
            horizontal_offset: 0.0,
            indices: [0; 2],
            animating: false,
            last_tick: None,
        }
    }
}
impl Playback {
    pub fn set_show_track(&mut self, show: bool) {
        self.show_track = show;
        if !show {
            self.fix_horizontal = false;
            self.fix_vertical = false;
            self.set_animating(false, 0.0);
        }
    }
    pub fn set_animating(&mut self, animate: bool, now: f64) {
        self.animating = animate && self.show_track;
        self.last_tick = self.animating.then_some(now);
    }
    pub fn tick(&mut self, now: f64, options: &Options, slot: usize, count: usize) {
        if !self.show_track || count == 0 {
            self.set_animating(false, now);
        }
        if !self.animating || options.ref_cycle == 0 {
            return;
        }
        let last = self.last_tick.get_or_insert(now);
        if now - *last + 1e-9 < f64::from(options.ref_cycle) / 1000.0 {
            return;
        }
        *last = now;
        self.indices[slot] = self.indices[slot]
            .saturating_add(options.anim_cycle.max(1) as usize)
            .min(count - 1);
        if self.indices[slot] == count - 1 {
            self.set_animating(false, now);
        }
    }
    pub fn follow(
        &self,
        viewport: &mut Viewport,
        kind: usize,
        time: f64,
        values: [Option<f64>; 3],
    ) {
        if !self.show_track {
            return;
        }
        if kind == 0 && self.fix_center {
            if let (Some(east), Some(north)) = (values[0], values[1]) {
                let mut bounds = viewport.track;
                bounds.x = bounds.x.centered(east);
                bounds.y = bounds.y.centered(north);
                viewport.set_bounds(crate::navigation::Target::Track, bounds);
            }
        } else if kind != 0 {
            if self.fix_horizontal {
                viewport.time = viewport
                    .time
                    .centered(time - self.horizontal_offset * viewport.time.span() / 2.0);
            }
            if kind == 1 && self.fix_vertical {
                for (range, value) in viewport.vertical.iter_mut().zip(values) {
                    if let Some(value) = value {
                        *range = range.centered(value);
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timer_steps_raw_epochs_clamps_stops_and_does_not_rewind() {
        let mut p = Playback::default();
        let mut o = Options::default();
        p.set_show_track(true);
        p.set_animating(true, 0.0);
        p.tick(0.099, &o, 0, 24);
        assert_eq!(p.indices, [0, 0]);
        p.tick(0.1, &o, 0, 24);
        assert_eq!(p.indices, [10, 0]);
        p.tick(0.2, &o, 0, 24);
        p.tick(0.3, &o, 0, 24);
        assert_eq!(p.indices, [23, 0]);
        assert!(!p.animating);
        p.set_animating(true, 1.0);
        p.tick(1.1, &o, 0, 24);
        assert_eq!(p.indices[0], 23);
        assert!(!p.animating);
        o.anim_cycle = 0;
        p.set_animating(true, 2.0);
        p.tick(2.1, &o, 1, 24);
        assert_eq!(p.indices[1], 1);
        o.ref_cycle = 0;
        p.tick(100.0, &o, 1, 24);
        assert_eq!(p.indices[1], 1);
        p.set_show_track(false);
        assert!(!p.animating && !p.fix_horizontal && !p.fix_vertical);
        assert!(p.fix_center);
    }
    #[test]
    fn fixes_follow_only_the_applicable_axes_with_show_track() {
        let mut p = Playback::default();
        let mut v = Viewport::default();
        v.resize_track([600.0, 300.0]);
        let before = v.clone();
        p.follow(&mut v, 0, 1000.0, [Some(12.0), Some(-7.0), Some(9.0)]);
        assert_eq!(v, before);
        p.set_show_track(true);
        p.follow(&mut v, 0, 1000.0, [Some(12.0), Some(-7.0), Some(9.0)]);
        assert_eq!(v.track.x.center(), 12.0);
        assert_eq!(v.track.y.center(), -7.0);
        assert_eq!(v.track.x.span(), before.track.x.span());
        p.fix_horizontal = true;
        p.follow(&mut v, 1, 1000.0, [Some(12.0), Some(-7.0), Some(9.0)]);
        assert_eq!(v.time.center(), 1000.0);
        assert_eq!(v.vertical.map(|r| r.center()), [12.0, -7.0, 9.0]);
        let vertical = v.vertical;
        p.horizontal_offset = 0.5;
        p.follow(&mut v, 2, 2000.0, [Some(20.0); 3]);
        assert_eq!(v.time.center(), 2000.0 - v.time.span() / 4.0);
        assert_eq!(v.vertical, vertical);
    }
}
