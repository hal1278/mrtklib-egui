//! Review images use the native drawing path, shared fonts and style; no processes.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::path::PathBuf;
fn harness(plot: usize) -> Harness<'static, PlotApp> {
    let dataset = std::env::var_os("MRTKLIB_EGUI_TEST_MRTKLIB_DIR")
        .map(PathBuf::from)
        .map(|root| root.join("tests/data/malib/MALIB_OSS_data_obsnav_240822-1100.pp.pos"))
        .filter(|p| p.is_file())
        .map(|p| crate::reader::read(&p).unwrap())
        .unwrap_or_else(crate::reader::synthetic);
    let mut controller = Controller::fixture(dataset);
    controller.settings.plot = plot;
    harness_with_controller(controller)
}
fn harness_with_controller(controller: Controller) -> Harness<'static, PlotApp> {
    let mut styled = false;
    let mut h = Harness::builder()
        .with_max_steps(20)
        .with_size(egui::vec2(INITIAL_SIZE[0], INITIAL_SIZE[1]))
        .build_ui_state(
            move |ui, app: &mut PlotApp| {
                if !styled {
                    mrtklib_egui_common::setup(ui.ctx());
                    styled = true;
                }
                app.update_logic(ui.ctx());
                app.draw(ui);
            },
            PlotApp::with_controller(controller),
        );
    h.run();
    h
}
fn save(h: &mut Harness<'_, PlotApp>, name: &str) {
    let directory = std::env::var_os("MRTKLIB_EGUI_SNAPSHOT_DIR")
        .filter(|v| !v.is_empty())
        .expect("MRTKLIB_EGUI_SNAPSHOT_DIR must be set for snapshot_ tests");
    let path = PathBuf::from(directory);
    std::fs::create_dir_all(&path).unwrap();
    let image: image::RgbaImage = h
        .render()
        .expect("snapshot rendering requires a GPU adapter and its runtime libraries");
    assert_eq!(
        image.dimensions(),
        (INITIAL_SIZE[0] as u32, INITIAL_SIZE[1] as u32)
    );
    image.save(path.join(format!("{name}.png"))).unwrap();
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_gnd_trk() {
    save(&mut harness(0), "snapshot_plot_gnd_trk");
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_position() {
    save(&mut harness(1), "snapshot_plot_position");
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_nsat() {
    save(&mut harness(2), "snapshot_plot_nsat");
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_options() {
    let mut h = harness(0);
    h.get_by_label("Options").click();
    h.run();
    h.get_by_label("Options...").click();
    h.run();
    save(&mut h, "snapshot_plot_options");
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_origin_range_list() {
    let mut h = harness(0);
    h.get_by_label("5m...").click();
    h.run();
    save(&mut h, "snapshot_plot_origin_range_list");
}
#[test]
fn options_cancel_and_validation_preserve_settings() {
    let mut h = harness(0);
    h.get_by_label("Options").click();
    h.run();
    h.get_by_label("Options...").click();
    h.run();
    h.state_mut().options.as_mut().unwrap().y_range = 0.0;
    h.get_by_label("OK").click();
    h.run();
    assert!(h.state().options_error.is_some());
    assert_eq!(h.state().controller.settings.options.y_range, 5.0);
    h.get_by_label("Cancel").click();
    h.run();
    assert!(h.state().options.is_none());
}
#[test]
fn quality_components_and_epoch_selection() {
    let mut h = harness(1);
    h.get_by_label("Plot2").click();
    h.run();
    assert!(!h.state().controller.settings.components[1]);
    h.state_mut().controller.settings.qualities.0 =
        [false, false, false, false, false, false, true, false];
    h.state_mut().controller.rebuild();
    h.run();
    assert!(h.state().controller.samples.iter().all(|s| s.quality == 6));
    assert!(h.state().summary().contains("Q1="));
}

fn only_ppp(h: &mut Harness<'_, PlotApp>) {
    for label in ["FIX", "FLOAT", "SBAS", "DGPS", "SINGLE"] {
        h.get_by_label(label).click();
        h.run();
    }
}
fn hover_epoch(h: &mut Harness<'_, PlotApp>, source: usize) {
    let app = h.state();
    let sample = app
        .controller
        .samples
        .iter()
        .filter(|s| s.source == source)
        .nth(5)
        .unwrap();
    let frame = app.geometry.unwrap().frames[0].unwrap();
    let bounds = if app.controller.settings.plot == 0 {
        app.viewport.track
    } else {
        app.viewport.bounds(ViewTarget::Both(0))
    };
    let [x, y] = coordinates(sample, app.controller.settings.plot, 0);
    let point = egui::pos2(
        frame.left() + ((x - bounds.x.min) / bounds.x.span()) as f32 * frame.width(),
        frame.bottom() - ((y - bounds.y.min) / bounds.y.span()) as f32 * frame.height(),
    );
    h.event(egui::Event::PointerMoved(point));
    h.run();
}

fn select(h: &mut Harness<'_, PlotApp>, current: &str, next: &str) {
    h.get(
        egui_kittest::kittest::By::new()
            .role(egui::accesskit::Role::ComboBox)
            .value(current),
    )
    .click();
    h.run();
    h.get_by_label(next).click();
    h.run();
}
#[test]
fn every_combo_exposes_its_last_choice_without_scrolling() {
    let mut h = harness(0);
    select(&mut h, "Gnd Trk", "NSat");
    assert_eq!(h.state().controller.settings.plot, 2);
    only_ppp(&mut h);
    assert!(h.state().controller.settings.qualities.visible(6));
    h.get_by_label("Options").click();
    h.run();
    h.get_by_label("Options...").click();
    h.run();
    select(&mut h, "h:m:s GPST", "www/ssss");
    select(&mut h, "www/ssss", "h:m:s UTC");
    select(&mut h, "Degrees", "DMS");
    select(&mut h, "Mark/Line", "Line");
    select(&mut h, "2", "20");
    select(&mut h, "Average Pos", "Lat/Lon/Hgt");
    let o = h.state().options.as_ref().unwrap();
    assert!(o.dms);
    assert_eq!(o.style, 2);
    assert_eq!(o.mark_size, 20);
    assert_eq!(o.origin, 5);
}

fn select_range(h: &mut Harness<'_, PlotApp>, current: &str, next: &str) {
    h.get_by_label(&format!("{current}...")).click();
    h.run();
    h.get_by_label(next).click();
    h.run();
}
#[test]
fn range_list_centres_immediately_and_shares_the_options_value() {
    for kind in [0, 1] {
        let mut h = harness(kind);
        let time = h.state().viewport.time;
        select_range(&mut h, "5m", "1000km");
        assert_eq!(h.state().controller.settings.options.y_range, 1_000_000.0);
        select_range(&mut h, "1000km", "5cm");
        assert_eq!(h.state().controller.settings.options.y_range, 0.05);
        assert_eq!(h.state().viewport.time, time);
        if kind == 0 {
            let bounds = h.state().viewport.track;
            let frame = h.state().geometry.unwrap().frames[0].unwrap();
            assert_eq!(bounds.x.center(), 0.0);
            assert_eq!(bounds.y.center(), 0.0);
            assert!((bounds.y.span() - 0.1).abs() < 1e-9);
            assert!(
                (bounds.x.span() / f64::from(frame.width())
                    - bounds.y.span() / f64::from(frame.height()))
                .abs()
                    < 1e-9
            );
        } else {
            assert_eq!(h.state().viewport.vertical, [Range::new(-0.05, 0.05); 3]);
        }
        h.get_by_label("Options").click();
        h.run();
        h.get_by_label("Options...").click();
        h.run();
        assert_eq!(h.state().options.as_ref().unwrap().y_range, 0.05);
        h.state_mut().options.as_mut().unwrap().y_range = 0.075;
        h.get_by_label("OK").click();
        h.run();
        // A custom Options value remains exact; the list doesn't snap it to a preset.
        select_range(&mut h, "0.075m", "2m");
        assert_eq!(h.state().controller.settings.options.y_range, 2.0);
        // Choosing the already-selected item still centres the view.
        if kind == 0 {
            h.state_mut().viewport.track.x = Range::new(10.0, 20.0);
        } else {
            h.state_mut().viewport.vertical = [Range::new(10.0, 20.0); 3];
        }
        select_range(&mut h, "2m", "2m");
        if kind == 0 {
            assert_eq!(h.state().viewport.track.x.center(), 0.0);
        } else {
            assert_eq!(h.state().viewport.vertical, [Range::new(-2.0, 2.0); 3]);
        }
    }
    let h = harness(2);
    assert!(h.get_by_label("5m...").accesskit_node().is_disabled());
}

#[test]
fn wheel_hit_testing_recognises_each_time_label_band() {
    let geometry = Geometry {
        kind: 1,
        canvas: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(300.0, 600.0)),
        frames: std::array::from_fn(|i| {
            let top = 20.0 + i as f32 * 200.0;
            Some(egui::Rect::from_min_max(
                egui::pos2(50.0, top),
                egui::pos2(290.0, top + 130.0),
            ))
        }),
    };
    for i in 0..3 {
        let frame = geometry.frames[i].unwrap();
        let label = egui::pos2(frame.center().x, frame.bottom() + 20.0);
        assert_eq!(geometry.wheel_area(label), Area::Time);
        assert_eq!(geometry.wheel_area(frame.center()), Area::Graph(i));
        assert_eq!(
            geometry.wheel_area(egui::pos2(10.0, frame.center().y)),
            Area::Left(i)
        );
        assert_eq!(
            geometry.wheel_area(egui::pos2(10.0, label.y)),
            Area::Outside
        );
        // The requested deviation concerns the wheel, not drag/double-click mapping.
        assert_eq!(
            geometry.area(label),
            if i == 2 { Area::Time } else { Area::Outside }
        );
    }
    assert_eq!(geometry.wheel_area(egui::pos2(150.0, 10.0)), Area::Outside);
    assert_eq!(geometry.wheel_area(egui::pos2(150.0, 610.0)), Area::Outside);
    let hidden_middle = Geometry {
        frames: [geometry.frames[0], None, geometry.frames[2]],
        ..geometry
    };
    assert_eq!(
        hidden_middle.wheel_area(egui::pos2(150.0, 170.0)),
        Area::Time
    );
    let track = Geometry {
        kind: 0,
        ..geometry
    };
    assert_eq!(track.wheel_area(egui::pos2(150.0, 170.0)), Area::Graph(0));
}

#[test]
fn quality_change_keeps_manual_range_for_each_plot() {
    for kind in 0..=2 {
        let mut h = harness(kind);
        {
            let app = h.state_mut();
            if kind == 0 {
                app.viewport.track = Bounds {
                    x: Range::new(100.0, 120.0),
                    y: Range::new(-10.0, 10.0),
                }
                .equal_aspect([
                    f64::from(app.geometry.unwrap().frames[0].unwrap().width()),
                    f64::from(app.geometry.unwrap().frames[0].unwrap().height()),
                ]);
            } else {
                app.viewport.time = Range::new(100.0, 200.0);
                app.viewport.vertical = [Range::new(7.0, 11.0); 3];
            }
        }
        h.run();
        let before = h.state().viewport.clone();
        only_ppp(&mut h);
        assert_eq!(
            h.state().viewport,
            before,
            "quality change fitted plot {kind}"
        );
        // An empty filter still retains its axes and range.
        for point in &mut h.state_mut().controller.dataset.as_mut().unwrap().points {
            point.quality = 1;
        }
        h.state_mut().controller.rebuild();
        h.run();
        assert!(h.state().controller.samples.is_empty());
        assert_eq!(h.state().viewport, before);
    }
}
fn wheel_at(h: &mut Harness<'_, PlotApp>, point: egui::Pos2, modifiers: egui::Modifiers) {
    h.event(egui::Event::PointerMoved(point));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers,
    });
    h.run();
}
#[test]
fn wheel_zooms_track_about_its_centre_and_time_panels_by_area_with_shared_time() {
    let mut h = harness(0);
    let frame = h.state().geometry.unwrap().frames[0].unwrap();
    let point = frame.left_top() + egui::vec2(frame.width() * 0.25, frame.height() * 0.75);
    let before = h.state().viewport.track;
    wheel_at(&mut h, point, egui::Modifiers::NONE);
    let after = h.state().viewport.track;
    let factor = navigation::wheel_factor(1.0);
    assert!((after.x.span() / before.x.span() - factor).abs() < 1e-8);
    assert!((after.y.span() / before.y.span() - factor).abs() < 1e-8);
    // RTKPLOT keeps the track centre wherever the cursor is (`plotmain.cpp:1716-1718`).
    assert!((after.x.center() - before.x.center()).abs() < 1e-8);
    assert!((after.y.center() - before.y.center()).abs() < 1e-8);
    let mut h = harness(1);
    let frame = h.state().geometry.unwrap().frames[1].unwrap();
    let before = h.state().viewport.clone();
    wheel_at(&mut h, frame.center(), egui::Modifiers::CTRL);
    assert_eq!(h.state().viewport.time, before.time);
    assert_eq!(h.state().viewport.vertical[0], before.vertical[0]);
    assert!(
        (h.state().viewport.vertical[1].span() / before.vertical[1].span() - factor).abs() < 1e-8
    );
    for kind in [1, 2] {
        let mut h = harness(kind);
        for components in [[true; 3], [true, false, true], [false, true, false]] {
            h.state_mut().controller.settings.components = components;
            h.run();
            for i in 0..3 {
                let g = h.state().geometry.unwrap();
                let Some(frame) = g.frames[i] else { continue };
                let bottom = g.frames[i + 1..]
                    .iter()
                    .flatten()
                    .next()
                    .map_or(g.canvas.bottom(), |next| next.top());
                let point = egui::pos2(frame.center().x, (frame.bottom() + bottom) / 2.0);
                let before = h.state().viewport.clone();
                wheel_at(&mut h, point, egui::Modifiers::NONE);
                assert!(
                    (h.state().viewport.time.span() / before.time.span() - factor).abs() < 1e-8,
                    "time labels of panel {i} in plot {kind} did not zoom shared time"
                );
                assert!((h.state().viewport.time.center() - before.time.center()).abs() < 1e-8);
                assert_eq!(h.state().viewport.vertical, before.vertical);
            }
        }
    }
}

#[test]
fn fit_time_uses_all_epochs_and_fit_vertical_uses_filtered_components() {
    let mut h = harness(1);
    only_ppp(&mut h);
    let extent = h.state().time_extent().unwrap();
    h.state_mut().viewport.time = Range::new(1.0, 2.0);
    h.get_by_label("Fit Horizontal").click();
    h.run();
    assert_eq!(h.state().viewport.time, extent);
    h.state_mut().viewport.vertical = [Range::new(-99.0, 99.0); 3];
    h.get_by_label("Fit Vertical").click();
    h.run();
    for i in 0..3 {
        let expected =
            navigation::fit_component(h.state().controller.samples.iter().map(|s| s.enu[i]))
                .unwrap();
        assert_eq!(h.state().viewport.vertical[i], expected);
    }
}
#[test]
fn auto_fit_option_does_not_immediately_reset_the_view() {
    let mut h = harness(1);
    h.state_mut().viewport.time = Range::new(1.0, 2.0);
    h.state_mut().viewport.vertical = [Range::new(7.0, 8.0); 3];
    h.run();
    let before = h.state().viewport.clone();
    h.get_by_label("Options").click();
    h.run();
    h.get_by_label("Options...").click();
    h.run();
    select(&mut h, "ON", "OFF");
    h.get_by_label("OK").click();
    h.run();
    assert!(!h.state().controller.settings.options.auto_fit);
    assert_eq!(h.state().viewport, before);
    select(&mut h, "Position", "NSat");
    select(&mut h, "NSat", "Position");
    assert_eq!(h.state().viewport.time, before.time);
    assert_eq!(h.state().viewport.vertical, [Range::new(-5.0, 5.0); 3]);
}
fn drag(
    h: &mut Harness<'_, PlotApp>,
    start: egui::Pos2,
    delta: egui::Vec2,
    button: egui::PointerButton,
) {
    h.event(egui::Event::PointerMoved(start));
    h.event(egui::Event::PointerButton {
        pos: start,
        button,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::PointerMoved(start + delta));
    h.event(egui::Event::PointerButton {
        pos: start + delta,
        button,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
}
#[test]
fn primary_drag_pans_and_secondary_drag_scales_the_source_axes() {
    let mut h = harness(0);
    let frame = h.state().geometry.unwrap().frames[0].unwrap();
    let before = h.state().viewport.track;
    drag(
        &mut h,
        frame.center(),
        egui::vec2(20.0, 30.0),
        egui::PointerButton::Primary,
    );
    let after = h.state().viewport.track;
    assert!((after.x.span() - before.x.span()).abs() < 1e-8);
    assert!(
        (after.x.center()
            - (before.x.center() - 20.0 * before.x.span() / f64::from(frame.width())))
        .abs()
            < 1e-8
    );
    assert!(
        (after.y.center()
            - (before.y.center() + 30.0 * before.y.span() / f64::from(frame.height())))
        .abs()
            < 1e-8
    );
    let mut h = harness(1);
    let frame = h.state().geometry.unwrap().frames[1].unwrap();
    let before = h.state().viewport.clone();
    drag(
        &mut h,
        frame.center(),
        egui::vec2(100.0, 20.0),
        egui::PointerButton::Secondary,
    );
    assert!((h.state().viewport.time.span() / before.time.span() - 0.5).abs() < 1e-8);
    assert_eq!(h.state().viewport.vertical, before.vertical);
    let before = h.state().viewport.clone();
    drag(
        &mut h,
        egui::pos2(frame.left() - 10.0, frame.center().y),
        egui::vec2(20.0, 100.0),
        egui::PointerButton::Secondary,
    );
    assert_eq!(h.state().viewport.time, before.time);
    assert!((h.state().viewport.vertical[1].span() / before.vertical[1].span() - 2.0).abs() < 1e-8);
}

#[test]
fn track_fit_uses_the_settled_frame_and_contains_every_sample() {
    let mut h = harness(0);
    for button in [None, Some("Fit Horizontal"), Some("Fit Vertical")] {
        if let Some(button) = button {
            h.state_mut().viewport.track.x = Range::new(1000.0, 2000.0);
            h.get_by_label(button).click();
            h.run();
        }
        let app = h.state();
        let b = app.viewport.track;
        for sample in &app.controller.samples {
            assert!((b.x.min..=b.x.max).contains(&sample.enu[0]));
            assert!((b.y.min..=b.y.max).contains(&sample.enu[1]));
        }
        let frame = app.geometry.unwrap().frames[0].unwrap();
        assert!(
            (b.x.span() / f64::from(frame.width()) - b.y.span() / f64::from(frame.height())).abs()
                < 1e-9
        );
    }
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_TEST_MRTKLIB_DIR, MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_solution2_overlay_and_difference() {
    let root = PathBuf::from(
        std::env::var_os("MRTKLIB_EGUI_TEST_MRTKLIB_DIR")
            .expect("MRTKLIB_EGUI_TEST_MRTKLIB_DIR is required for the two-file snapshot"),
    );
    let read = |suffix| {
        crate::reader::read(&root.join(format!(
            "tests/data/malib/MALIB_OSS_data_obsnav_240822-1100.{suffix}.pos"
        )))
        .unwrap()
    };
    let mut controller = Controller::fixture(read("pp"));
    controller.dataset2 = Some(read("rt"));
    controller.toggle(1);
    assert!(controller.error.is_none());
    assert!(controller.samples.iter().any(|s| s.source == 0));
    assert!(controller.samples.iter().any(|s| s.source == 1));
    let mut h = harness_with_controller(controller);
    save(&mut h, "snapshot_plot_solution2_overlay");
    h.get_by_label("1-2").click();
    h.run();
    h.get_by_label("Fit Vertical").click();
    h.run();
    assert!(!h.state().controller.samples.is_empty());
    assert!(h.state().controller.samples.iter().all(|s| s.source == 2));
    save(&mut h, "snapshot_plot_solution2_difference");
    select(&mut h, "Gnd Trk", "Position");
    save(&mut h, "snapshot_plot_solution2_difference_position");
    h.get_by_label("1").click();
    h.run();
    h.get_by_label("2").click();
    h.run();
    h.get_by_label("Fit Vertical").click();
    h.run();
    save(&mut h, "snapshot_plot_solution2_overlay_position");
}

fn two_solutions(plot: usize) -> Controller {
    let mut controller = Controller::fixture(crate::reader::synthetic());
    let mut second = crate::reader::synthetic();
    for p in &mut second.points {
        p.position[0] += 50.0;
        p.time += 0.01;
        p.time_ns += 10_000_000;
        p.quality = 5;
    }
    controller.dataset2 = Some(second);
    controller.settings.plot = plot;
    controller.rebuild();
    controller
}

#[test]
fn dataset_buttons_status_and_line_styles_follow_rtkplot() {
    let mut h = harness_with_controller(two_solutions(1));
    assert_eq!(line_style(0), LineStyle::Solid);
    assert_eq!(line_style(1), LineStyle::Dashed { length: 1.0 });
    assert_eq!(line_style(2), LineStyle::Solid);
    h.get_by_label("2").click();
    h.run();
    assert_eq!(h.state().controller.selection.data, [true, true]);
    hover_epoch(&mut h, 0);
    assert!(h.state().epoch_text().starts_with("[1]"));
    assert!(h.state().summary().starts_with("[1]"));
    h.get_by_label("1").click();
    h.run();
    assert_eq!(h.state().controller.selection.data, [false, true]);
    h.get_by_label("Fit Vertical").click();
    h.run();
    hover_epoch(&mut h, 1);
    assert!(h.state().epoch_text().starts_with("[2]"));
    assert!(h.state().summary().starts_with("[2]"));
    h.get_by_label("1-2").click();
    h.run();
    assert_eq!(h.state().controller.selection.data, [false, false]);
    assert!(h.state().controller.selection.difference);
    assert!(h.state().summary().contains("Q5="));
    h.get_by_label("Fit Vertical").click();
    h.run();
    hover_epoch(&mut h, 2);
    assert!(h.state().epoch_text().starts_with("[1-2]"));
    select(&mut h, "Position", "NSat");
    assert!(h.get_by_label("1-2").accesskit_node().is_disabled());
    assert!(h.state().controller.samples.is_empty());
    h.get_by_label("2").click();
    h.run();
    assert!(!h.state().controller.selection.difference);
    assert!(h.state().controller.samples.iter().all(|s| s.source == 1));
    h.get_by_label("2").click();
    h.run();
    assert!(h.state().controller.samples.is_empty());
    assert!(h.state().summary().is_empty());
}

#[test]
fn range_fit_includes_selected_series_or_the_difference() {
    for plot in [0, 1] {
        let mut h = harness_with_controller(two_solutions(plot));
        h.get_by_label("2").click();
        h.run();
        h.get_by_label("Fit Vertical").click();
        h.run();
        let fits = |h: &Harness<'_, PlotApp>| {
            let app = h.state();
            for s in &app.controller.samples {
                if plot == 0 {
                    assert!(
                        (app.viewport.track.x.min..=app.viewport.track.x.max).contains(&s.enu[0])
                    );
                    assert!(
                        (app.viewport.track.y.min..=app.viewport.track.y.max).contains(&s.enu[1])
                    );
                } else {
                    for (i, range) in app.viewport.vertical.iter().enumerate() {
                        assert!((range.min..=range.max).contains(&s.enu[i]));
                    }
                }
            }
        };
        fits(&h);
        h.get_by_label("1-2").click();
        h.run();
        h.get_by_label("Fit Vertical").click();
        h.run();
        fits(&h);
        assert!(!h.state().controller.samples.is_empty());
    }
}

#[test]
fn status_has_one_summary_per_shown_dataset_and_switches_percentages() {
    let mut c = two_solutions(1);
    // A missing epoch makes the two denominators observably different.
    c.dataset.as_mut().unwrap().points.remove(1);
    c.toggle(1);
    let mut h = harness_with_controller(c);
    let summary = h.state().summary();
    assert_eq!(summary.lines().count(), 2);
    assert!(summary.lines().nth(1).unwrap().starts_with("[2]"));
    assert!(!summary.contains("Malformed"));
    assert!(!summary.contains("FIX"));
    h.get_by_label("% Epochs").click();
    h.run();
    assert_ne!(h.state().summary(), summary);
    assert!(h.state().controller.settings.interval_percentages);
    let count_text = |text: &str| {
        text.split("Q1=")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(count_text(&summary), count_text(&h.state().summary()));
    h.get_by_label("1-2").click();
    h.run();
    assert_eq!(h.state().summary().lines().count(), 1);
    assert!(h.state().summary().starts_with("[1-2]"));
}

#[test]
fn cursor_uses_epoch_only_near_a_point_and_reports_the_hovered_dataset() {
    let mut c = two_solutions(1);
    c.toggle(1);
    let mut h = harness_with_controller(c);
    hover_epoch(&mut h, 1);
    assert!(h.state().epoch_text().starts_with("[2]"));
    assert!(h.state().epoch_text().contains("Q=5"));
    let frame = h.state().geometry.unwrap().frames[1].unwrap();
    h.event(egui::Event::PointerMoved(
        frame.left_top() + egui::vec2(70.0, 10.0),
    ));
    h.run();
    assert!(h.state().hovered.is_none());
    let text = h.state().epoch_text();
    assert!(text.contains("GPST  N="), "{text}");
    assert!(!text.contains("Q="));
    assert!(!text.contains('['));
}

#[test]
fn difference_height_cursor_reports_the_plotted_height_difference() {
    let mut c = two_solutions(1);
    c.settings.height = true;
    c.toggle(2);
    let mut h = harness_with_controller(c);
    hover_epoch(&mut h, 2);
    assert!(h.state().epoch_text().contains("dH="));
}

#[test]
fn status_coordinates_use_measured_width_and_the_llh_enu_choice() {
    let mut h = harness(1);
    hover_epoch(&mut h, 0);
    h.state_mut().status_width = 10000.0;
    let both = h.state().epoch_text();
    assert!(both.contains("°") && both.contains("E="), "{both}");
    h.state_mut().status_width = 1.0;
    let enu = h.state().epoch_text();
    assert!(enu.contains("E=") && !enu.contains("°"));
    h.state_mut().controller.settings.cursor_llh = true;
    let llh = h.state().epoch_text();
    assert!(llh.contains("°") && !llh.contains("E="));
}

#[test]
fn nsat_panels_follow_the_plot_buttons_and_height_stays_position_only() {
    let mut h = harness(2);
    assert!(h.query_by_label("Plot2").is_some());
    assert!(h.query_by_label("U").is_none() && h.query_by_label("Height").is_none());
    h.get_by_label("Plot2").click();
    h.run();
    let frames = h.state().geometry.unwrap().frames;
    assert!(frames[0].is_some() && frames[1].is_none() && frames[2].is_some());
}

#[test]
fn track_cursor_off_the_points_shows_its_latitude_and_longitude() {
    let mut h = harness(0);
    h.state_mut().hovered = None;
    h.state_mut().cursor = Some((0, [1.0, 2.0]));
    h.state_mut().status_width = 10000.0;
    let both = h.state().epoch_text();
    assert!(
        both.contains("°") && both.contains("E=1.0000 N=2.0000"),
        "{both}"
    );
    assert!(!both.contains("H="), "{both}");
    h.state_mut().status_width = 1.0;
    h.state_mut().controller.settings.cursor_llh = true;
    let llh = h.state().epoch_text();
    assert!(llh.contains("°") && !llh.contains("E="), "{llh}");
    h.state_mut().controller.dataset.as_mut().unwrap().relative = true;
    assert!(!h.state().epoch_text().contains("°"));
}

#[test]
fn coordinate_button_is_hidden_when_both_llh_and_enu_fit() {
    let mut h = harness(1);
    hover_epoch(&mut h, 0);
    assert!(h.state().epoch_text().contains("°") && h.state().epoch_text().contains("E="));
    assert!(h.query_by_label("ENU").is_none());
    h.set_size(egui::vec2(650.0, INITIAL_SIZE[1]));
    h.run();
    hover_epoch(&mut h, 0);
    assert!(!h.state().epoch_text().contains("°"));
    assert!(h.query_by_label("ENU").is_some());
}

#[test]
fn info_indicator_retains_each_dataset_diagnostics_when_window_closes() {
    let mut c = two_solutions(0);
    c.dataset.as_mut().unwrap().malformed = 3;
    c.dataset2
        .as_mut()
        .unwrap()
        .warnings
        .push("Height is geodetic".into());
    c.file_errors[1].push("cannot open solution /missing.pos".into());
    c.error = Some("settings could not be saved".into());
    let mut h = harness_with_controller(c);
    assert_eq!(h.state().info_items().len(), 4);
    h.get_by_label("⚠ 4").click();
    h.run();
    assert!(h.state().info_open);
    assert!(
        h.state()
            .info_items()
            .iter()
            .any(|(error, text)| *error && text.starts_with("[2]"))
    );
    h.state_mut().info_open = false;
    h.run();
    h.get_by_label("1").click();
    h.run();
    assert_eq!(h.state().info_items().len(), 4);
    assert!(!h.state().info_open);
    assert!(h.query_by_label("⚠ 4").is_some());
}

#[test]
fn component_controls_only_show_for_time_plots_and_height_changes_only_third_range() {
    let mut h = harness(0);
    assert!(h.query_by_label("Plot1").is_none());
    select(&mut h, "Gnd Trk", "Position");
    let before = h.state().viewport.clone();
    let button_rect = h.get_by_label("U").rect();
    let toolbar_rect = h.get_by_label("FIX").rect();
    let panel_heights = h
        .state()
        .geometry
        .unwrap()
        .frames
        .map(|frame| frame.unwrap().height());
    h.get_by_label("U").click();
    h.run();
    assert!(h.state().controller.settings.height);
    assert_eq!(h.get_by_label("Height").rect(), button_rect);
    assert_eq!(h.get_by_label("FIX").rect(), toolbar_rect);
    assert_eq!(
        h.state()
            .geometry
            .unwrap()
            .frames
            .map(|frame| frame.unwrap().height()),
        panel_heights
    );
    assert_eq!(h.state().viewport.time, before.time);
    assert_eq!(h.state().viewport.vertical[..2], before.vertical[..2]);
    for s in &h.state().controller.samples {
        assert_eq!(s.vertical, s.height);
    }
    select(&mut h, "Position", "NSat");
    assert!(h.query_by_label("Plot1").is_some());
    assert!(h.query_by_label("Height").is_none());
}

#[test]
fn all_quality_button_restores_each_quality_and_every_control_fits_default_width() {
    let mut h = harness(1);
    let before = h.state().viewport.clone();
    h.get_by_label("All").click();
    h.run();
    assert_eq!(h.state().controller.settings.qualities.0, [false; 8]);
    assert!(h.state().controller.samples.is_empty());
    h.get_by_label("All").click();
    h.run();
    assert_eq!(h.state().controller.settings.qualities.0, [true; 8]);
    assert_eq!(h.state().viewport, before);
    only_ppp(&mut h);
    h.get_by_label("PPP").click();
    h.run();
    assert!(h.state().controller.samples.is_empty());
    h.get_by_label("All").click();
    h.run();
    assert_eq!(h.state().controller.settings.qualities.0, [true; 8]);
    for label in [
        "Plot1",
        "Plot2",
        "Plot3",
        "U",
        "FIX",
        "FLOAT",
        "SBAS",
        "DGPS",
        "SINGLE",
        "PPP",
        "Fit Horizontal",
        "Fit Vertical",
        "Center Origin",
        "5m...",
    ] {
        let node = h.get_by_label(label);
        let rect = node.rect();
        assert!(
            rect.left() >= 0.0 && rect.right() <= INITIAL_SIZE[0],
            "{label} outside default window: {rect:?}"
        );
    }
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_two_dataset_status() {
    let mut c = two_solutions(1);
    c.toggle(1);
    c.settings.last_file = Some(
        "/a/long/directory/for/the/first/solution/complete-first-solution-filename.pos".into(),
    );
    c.settings.last_file2 = Some("/another/long/directory/second-solution-filename.pos".into());
    c.set_override_hz(0, Some("1")).unwrap();
    let mut h = harness_with_controller(c);
    hover_epoch(&mut h, 1);
    save(&mut h, "snapshot_plot_two_dataset_status");
    h.get_by_label("Rate 1: 1 Hz (manual)").click();
    h.run();
    save(&mut h, "snapshot_plot_rate_override");
    h.state_mut().rate_drafts[0] = "2".into();
    h.run();
    h.get_by_label("Apply override").click();
    h.run();
    assert_eq!(h.state().controller.override_hz(0), Some(2.0));
    h.get_by_label("Rate 1: 2 Hz (manual)").click();
    h.run();
    h.get_by_label("Use estimated").click();
    h.run();
    assert_eq!(h.state().controller.override_hz(0), None);
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_info_window() {
    let mut c = two_solutions(0);
    c.dataset.as_mut().unwrap().malformed = 12;
    c.dataset.as_mut().unwrap().duplicates_removed = 2;
    c.dataset
        .as_mut()
        .unwrap()
        .warnings
        .push("Geodetic height: displayed as supplied in the solution file".into());
    c.dataset2
        .as_mut()
        .unwrap()
        .warnings
        .push("UTC converted to GPST using the built-in leap-second table".into());
    c.file_errors[1].push("cannot open solution /example/missing.pos: file does not exist".into());
    let mut h = harness_with_controller(c);
    h.get_by_label("⚠ 5").click();
    h.run();
    save(&mut h, "snapshot_plot_info_window");
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and GPU adapter"]
fn snapshot_position_height() {
    let mut h = harness(1);
    h.get_by_label("U").click();
    h.run();
    save(&mut h, "snapshot_plot_position_height");
}

#[test]
fn status_times_match_the_plot_for_each_time_option_and_dataset() {
    for (utc, week_time, first, epoch, second) in [
        (
            false,
            false,
            "2024/08/22 11:00:00.000 GPST",
            "2024/08/22 11:00:05.000 GPST",
            "2024/08/22 11:00:00.010",
        ),
        (
            true,
            false,
            "2024/08/22 10:59:42.000 UTC",
            "2024/08/22 10:59:47.000 UTC",
            "2024/08/22 10:59:42.010",
        ),
        (
            true,
            true,
            "2328/385200.000 GPST",
            "2328/385205.000 GPST",
            "2328/385200.010",
        ),
    ] {
        let mut c = two_solutions(1);
        c.selection.data = [true; 2];
        c.apply_options(Options {
            utc,
            week_time,
            ..Default::default()
        })
        .unwrap();
        let mut h = harness_with_controller(c);
        let (first_time, system) = first.rsplit_once(' ').unwrap();
        let summary = h.state().summary();
        assert!(
            summary.starts_with(&format!("[1] {first_time} - ")),
            "{summary}"
        );
        assert!(summary.lines().all(|line| line.contains(system)));
        assert!(
            summary
                .lines()
                .nth(1)
                .unwrap()
                .starts_with(&format!("[2] {second} - "))
        );
        let plot_time = h.state().controller.samples[0].time;
        assert_eq!(
            format!("{} {system}", time::format(plot_time, week_time, true)),
            first
        );
        hover_epoch(&mut h, 0);
        assert!(
            h.state()
                .epoch_text()
                .starts_with(&format!("[1] {epoch}  "))
        );
        hover_epoch(&mut h, 1);
        assert!(h.state().epoch_text().starts_with(&format!(
            "[2] {} ",
            time::format(plot_time + 5.01, week_time, true)
        )));
        // Plot coordinates already use display time; cursor text must not convert again.
        h.state_mut().hovered = None;
        h.state_mut().cursor = Some((1, [plot_time, 0.125]));
        assert_eq!(h.state().epoch_text(), format!("{first}  N=0.1250 m"));
        h.get_by_label("1-2").click();
        h.run();
        assert!(
            h.state()
                .summary()
                .starts_with(&format!("[1-2] {first_time} - "))
        );
        h.get_by_label("Fit Vertical").click();
        h.run();
        hover_epoch(&mut h, 2);
        assert!(
            h.state()
                .epoch_text()
                .starts_with(&format!("[1-2] {epoch}  "))
        );
    }
}

#[test]
fn hidden_dataset_origin_error_is_reported_without_removing_the_visible_dataset() {
    let mut c = two_solutions(0);
    c.dataset.as_mut().unwrap().relative = true;
    for (i, point) in c.dataset.as_mut().unwrap().points.iter_mut().enumerate() {
        point.position = [i as f64 * 0.02, 1.0, 0.0];
    }
    c.rebuild();
    let mut h = harness_with_controller(c);
    assert_eq!(
        h.state().controller.samples.len(),
        h.state().controller.dataset.as_ref().unwrap().points.len()
    );
    let items = h.state().info_items();
    assert_eq!(items.len(), 1);
    assert!(items[0].0);
    assert!(items[0].1.starts_with("[2]"));
    assert!(items[0].1.contains("absolute Solution-1"));
    h.get_by_label("⚠ 1").click();
    h.run();
    assert!(h.state().info_open);
    h.state_mut().info_open = false;
    // NSat needs no coordinate origin, so the conversion error resolves.
    select(&mut h, "Gnd Trk", "NSat");
    assert!(h.state().info_items().is_empty());
    assert!(h.query_by_label("⚠ 1").is_none());
    assert!(!h.state().controller.samples.is_empty());
}

#[test]
fn absolute_height_statistics_omit_rms_about_zero() {
    assert_eq!(
        component_statistics("1", &[1.0, 3.0], true),
        "[1] AVE=2.0000  STD=1.4142  RMS=2.2361 m"
    );
    assert_eq!(
        component_statistics("1", &[1.0, 3.0], false),
        "[1] AVE=2.0000  STD=1.4142 m"
    );
}

#[test]
fn data2_mark_follows_the_selected_data1_epoch_within_twice_dttol() {
    let mut c = two_solutions(0);
    c.selection.data = [true, true];
    c.rebuild();
    let samples = &c.samples;
    let data1: Vec<_> = samples.iter().filter(|s| s.source == 0).collect();
    for s in [data1[0], data1[data1.len() - 1]] {
        let marks = selected_marks(samples, Some((0, s.index)));
        assert_eq!(marks.len(), 2);
        let companion = &samples[marks[1]];
        assert_eq!(companion.source, 1);
        assert!((companion.time - s.time).abs() < 0.05);
    }
    let first2 = samples.iter().find(|s| s.source == 1).unwrap().index;
    assert_eq!(selected_marks(samples, Some((1, first2))).len(), 1);
    let mut far = two_solutions(0);
    for p in &mut far.dataset2.as_mut().unwrap().points {
        p.time += 0.05;
        p.time_ns += 50_000_000;
    }
    far.selection.data = [true, true];
    far.rebuild();
    let first1 = far.samples.iter().find(|s| s.source == 0).unwrap().index;
    assert_eq!(selected_marks(&far.samples, Some((0, first1))).len(), 1);
}
