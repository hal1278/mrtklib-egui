//! The native app and these tests share Conv::draw/update_logic. No visible window.

use super::*;
use crate::settings::Settings;
use egui_kittest::{Harness, kittest::Queryable};
use std::path::PathBuf;

struct Fixture {
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "conv-ui-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        Self { directory }
    }
    fn harness(&self) -> Harness<'static, Conv> {
        let settings = Settings {
            input: "receiver_20080526.ubx".into(),
            ..Default::default()
        };
        let app = Conv::with_controller(Controller::for_ui_tests(settings, self.directory.clone()));
        let mut styled = false;
        let mut harness = Harness::builder()
            .with_max_steps(20)
            .with_size(egui::vec2(INITIAL_SIZE[0], INITIAL_SIZE[1]))
            .build_ui_state(
                move |ui, app: &mut Conv| {
                    if !styled {
                        mrtklib_egui_common::setup(ui.ctx());
                        styled = true;
                    }
                    app.update_logic(ui.ctx());
                    app.draw(ui);
                },
                app,
            );
        harness.run();
        harness
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn click(harness: &mut Harness<'_, Conv>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn click_combo(harness: &mut Harness<'_, Conv>, value: &str) {
    harness
        .get(
            egui_kittest::kittest::By::new()
                .role(egui::accesskit::Role::ComboBox)
                .value(value),
        )
        .click();
    harness.run();
}

fn snapshot_path(name: &str) -> PathBuf {
    let directory = std::env::var_os("MRTKLIB_EGUI_SNAPSHOT_DIR")
        .filter(|v| !v.is_empty())
        .expect("MRTKLIB_EGUI_SNAPSHOT_DIR must be set for snapshot_ tests");
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap_or_else(|e| {
        panic!(
            "cannot create snapshot directory {}: {e}",
            directory.display()
        )
    });
    directory.join(format!("{name}.png"))
}
fn save(harness: &mut Harness<'_, Conv>, path: PathBuf) {
    let image: image::RgbaImage = harness
        .render()
        .expect("snapshot rendering requires a GPU adapter and its runtime libraries");
    assert_eq!(
        image.dimensions(),
        (INITIAL_SIZE[0] as u32, INITIAL_SIZE[1] as u32)
    );
    image
        .save(&path)
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
}

#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and a GPU adapter"]
fn snapshot_main() {
    let path = snapshot_path("snapshot_main");
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    save(&mut harness, path);
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and a GPU adapter"]
fn snapshot_options() {
    let path = snapshot_path("snapshot_options");
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    click(&mut harness, "Options...");
    save(&mut harness, path);
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and a GPU adapter"]
fn snapshot_mask() {
    let path = snapshot_path("snapshot_mask");
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    click(&mut harness, "Options...");
    click(&mut harness, "Mask...");
    save(&mut harness, path);
}
#[test]
#[ignore = "requires MRTKLIB_EGUI_SNAPSHOT_DIR and a GPU adapter"]
fn snapshot_overwrite() {
    let path = snapshot_path("snapshot_overwrite");
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    std::fs::write(
        fixture.directory.join("receiver_20080526.obs"),
        b"existing output",
    )
    .unwrap();
    click(&mut harness, "Convert");
    assert!(harness.state().controller.overwrite.is_some());
    save(&mut harness, path);
}

#[test]
fn mask_cancel_empty_rejection_and_options_cancel_keep_their_drafts_separate() {
    let fixture = Fixture::new();
    let mut h = fixture.harness();
    click(&mut h, "Options...");
    click(&mut h, "Mask...");
    click(&mut h, "Unset All");
    assert!(
        h.state()
            .mask
            .as_ref()
            .unwrap()
            .0
            .iter()
            .flatten()
            .all(|value| !value),
        "Unset All did not clear the mask"
    );
    click(&mut h, "OK");
    assert!(h.state().mask.is_some());
    assert!(
        h.state()
            .mask_error
            .as_ref()
            .unwrap()
            .contains("at least one")
    );
    click(&mut h, "Cancel");
    assert_eq!(
        h.state().options.as_ref().unwrap().signal_mask,
        SignalMask::default()
    );
    click(&mut h, "Mask...");
    h.state_mut().mask.as_mut().unwrap().0[2][16] = false;
    click(&mut h, "OK");
    assert!(!h.state().options.as_ref().unwrap().signal_mask.0[2][16]);
    click(&mut h, "Cancel");
    assert_eq!(
        h.state().controller.settings.options.signal_mask,
        SignalMask::default()
    );
    click(&mut h, "Options...");
    click(&mut h, "Mask...");
    h.state_mut().mask.as_mut().unwrap().0[0][0] = false;
    click(&mut h, "OK");
    click(&mut h, "OK");
    assert!(!h.state().controller.settings.options.signal_mask.0[0][0]);
}

#[test]
fn every_combo_can_select_its_last_item_without_scrolling() {
    let fixture = Fixture::new();
    let mut h = fixture.harness();
    click_combo(&mut h, "Auto");
    click(&mut h, "RINEX");
    assert_eq!(h.state().controller.settings.format, 14);
    click(&mut h, "Interval");
    click_combo(&mut h, "s");
    click(&mut h, "60");
    assert_eq!(h.state().controller.settings.interval, "60");
    click(&mut h, "Options...");
    click_combo(&mut h, "3.04");
    click(&mut h, "2.10");
    click_combo(&mut h, "2.10");
    click(&mut h, "3.04");
    assert_eq!(h.state().options.as_ref().unwrap().version, "3.04");
    click_combo(&mut h, "5");
    click(&mut h, "1");
    click_combo(&mut h, "1");
    click(&mut h, "5");
    assert_eq!(h.state().options.as_ref().unwrap().frequencies, 5);
}
