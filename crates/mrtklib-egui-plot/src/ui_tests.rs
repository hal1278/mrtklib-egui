//! Review images use the native drawing path, shared fonts and style; no processes.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
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
    h.state_mut().controller.settings.quality = 6;
    h.state_mut().controller.rebuild();
    h.run();
    assert!(h.state().controller.samples.iter().all(|s| s.quality == 6));
    assert!(h.state().epoch_text().contains("PPP"));
    assert!(h.state().summary().contains("Q1 FIX:"));
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
    select(&mut h, "ALL", "6 PPP");
    assert_eq!(h.state().controller.settings.quality, 6);
    h.get_by_label("Options").click();
    h.run();
    h.get_by_label("Options...").click();
    h.run();
    select(&mut h, "h:m:s", "www/ssss");
    select(&mut h, "www/ssss", "h:m:s");
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
        select(&mut h, "ALL", "6 PPP");
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
    let g = h.state().geometry.unwrap();
    let frame = g.frames[2].unwrap();
    let point = egui::pos2(frame.center().x, (frame.bottom() + g.canvas.bottom()) / 2.0);
    let before = h.state().viewport.clone();
    wheel_at(&mut h, point, egui::Modifiers::NONE);
    assert!((h.state().viewport.time.span() / before.time.span() - factor).abs() < 1e-8);
    assert_eq!(h.state().viewport.vertical, before.vertical);
}

#[test]
fn fit_time_uses_all_epochs_and_fit_vertical_uses_filtered_components() {
    let mut h = harness(1);
    select(&mut h, "ALL", "6 PPP");
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
