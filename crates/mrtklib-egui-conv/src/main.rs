//! RTKCONV equivalent. Skeleton: an empty window.

use eframe::egui;

fn main() -> eframe::Result {
    mrtklib_egui_common::run("mrtklib-egui-conv", [360.0, 120.0], || App)
}

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("RTKCONV equivalent: not implemented yet.");
            if ui.button("Exit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}
