//! RTKNAVI equivalent. Skeleton: the main window layout without a backend yet.

use eframe::egui::{self, Color32, RichText};

const NOT_CONNECTED: &str = "Not available yet: the RPC client is not implemented";

fn main() -> eframe::Result {
    mrtklib_egui_common::run("mrtklib-egui-navi", [430.0, 300.0], || Navi)
}

struct Navi;

impl eframe::App for Navi {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for name in ["Rover", "Base", "Corr"] {
                    ui.label(RichText::new("■").color(Color32::GRAY));
                    ui.label(name);
                }
                ui.separator();
                ui.monospace("----/--/-- --:--:--.- GPST");
            });
            ui.separator();
            egui::Grid::new("solution")
                .num_columns(2)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for label in [
                        "Solution:",
                        "N:",
                        "E:",
                        "H:",
                        "σ N/E/U:",
                        "Age / Ratio / #Sat:",
                    ] {
                        ui.label(label);
                        ui.monospace("----");
                        ui.end_row();
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                for name in ["Start", "Stop", "Options..."] {
                    ui.add_enabled(false, egui::Button::new(name))
                        .on_disabled_hover_text(NOT_CONNECTED);
                }
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}
