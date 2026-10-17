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
