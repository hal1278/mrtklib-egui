//! Raw solution indices and playback driven by the GUI repaint clock.
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
/// Only enabled, visible Fix controls affect navigation; retained toggle values do not.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fixes {
    pub center: bool,
    pub horizontal: bool,
    pub horizontal_offset: f64,
    pub vertical: bool,
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
    pub fn fixes(&self, kind: usize, enabled: bool) -> Fixes {
        let enabled = enabled && self.show_track;
        Fixes {
            center: enabled && kind == 0 && self.fix_center,
            horizontal: enabled && matches!(kind, 1 | 2) && self.fix_horizontal,
            vertical: enabled && kind == 1 && self.fix_vertical,
            horizontal_offset: self.horizontal_offset,
        }
    }
}
impl Fixes {
    pub fn follow(self, viewport: &mut Viewport, time: f64, values: [Option<f64>; 3]) {
        if self.center {
            if let (Some(east), Some(north)) = (values[0], values[1]) {
                let mut bounds = viewport.track;
                bounds.x = bounds.x.centered(east);
                bounds.y = bounds.y.centered(north);
                viewport.set_bounds(crate::navigation::Target::Track, bounds);
            }
        } else {
            if self.horizontal {
                viewport.time = viewport
                    .time
                    .centered(time - self.horizontal_offset * viewport.time.span() / 2.0);
            }
            if self.vertical {
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
    fn retained_fix_toggles_have_no_effect_without_show_track_or_enabled_controls() {
        let mut p = Playback {
            fix_horizontal: true,
            ..Default::default()
        };
        for show_track in [false, true] {
            p.show_track = show_track;
            for enabled in [false, true] {
                for kind in 0..3 {
                    let fixes = p.fixes(kind, enabled);
                    assert_eq!(fixes.center, show_track && enabled && kind == 0);
                    assert_eq!(fixes.horizontal, show_track && enabled && kind != 0);
                    assert_eq!(fixes.vertical, show_track && enabled && kind == 1);
                    if !show_track || !enabled {
                        let mut viewport = Viewport::default();
                        let before = viewport.clone();
                        fixes.follow(&mut viewport, 1000.0, [Some(100.0); 3]);
                        assert_eq!(viewport, before);
                    }
                }
            }
        }
    }
    #[test]
    fn gui_timer_takes_one_step_after_a_delay_and_zero_refresh_stops_ticks() {
        let mut p = Playback::default();
        p.set_show_track(true);
        p.set_animating(true, 0.0);
        let mut options = Options::default();
        p.tick(10.0, &options, 0, 1000);
        assert_eq!(p.indices[0], 10);
        p.tick(10.0, &options, 0, 1000);
        assert_eq!(p.indices[0], 10);
        options.ref_cycle = 0;
        p.tick(100.0, &options, 0, 1000);
        assert_eq!(p.indices[0], 10);
    }
    #[test]
    fn fixes_follow_only_the_applicable_axes_with_show_track() {
        let mut p = Playback::default();
        let mut v = Viewport::default();
        v.resize_track([600.0, 300.0]);
        let before = v.clone();
        p.fixes(0, true)
            .follow(&mut v, 1000.0, [Some(12.0), Some(-7.0), Some(9.0)]);
        assert_eq!(v, before);
        p.set_show_track(true);
        p.fixes(0, true)
            .follow(&mut v, 1000.0, [Some(12.0), Some(-7.0), Some(9.0)]);
        assert_eq!(v.track.x.center(), 12.0);
        assert_eq!(v.track.y.center(), -7.0);
        assert_eq!(v.track.x.span(), before.track.x.span());
        p.fix_horizontal = true;
        p.fixes(1, true)
            .follow(&mut v, 1000.0, [Some(12.0), Some(-7.0), Some(9.0)]);
        assert_eq!(v.time.center(), 1000.0);
        assert_eq!(v.vertical.map(|r| r.center()), [12.0, -7.0, 9.0]);
        let vertical = v.vertical;
        p.horizontal_offset = 0.5;
        p.fixes(2, true).follow(&mut v, 2000.0, [Some(20.0); 3]);
        assert_eq!(v.time.center(), 2000.0 - v.time.span() / 4.0);
        assert_eq!(v.vertical, vertical);
    }
}
