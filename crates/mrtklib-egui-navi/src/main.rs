//! RTKNAVI-like first vertical slice, controlled through the backend RPC.

mod controller;
mod program;
mod time;
mod view;

use controller::Controller;
use eframe::egui::{self, Color32, RichText};

/// Space kept between a modal dialog and the window edges: the popup frame's inner
/// margin (6 px per side) plus a small gap to the window border.
const DIALOG_MARGIN: f32 = 32.0;

fn main() -> eframe::Result {
    mrtklib_egui_common::run_with_context("mrtklib-egui-navi", [430.0, 300.0], |ctx| {
        let ctx = ctx.clone();
        Navi {
            controller: Controller::launch(move || ctx.request_repaint()),
        }
    })
}

struct Navi {
    controller: Controller,
}

fn color(rgb: view::Color) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

impl eframe::App for Navi {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.controller.tick();
        if ctx.input(|input| input.viewport().close_requested()) {
            self.controller.begin_exit();
            if !self.controller.exit_done() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        if self.controller.closing() && self.controller.exit_done() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let view = self.controller.view();
        egui::CentralPanel::default().show(ui, |ui| {
            for time in &view.times {
                ui.label(
                    RichText::new(time)
                        .monospace()
                        .color(color(view.time_color)),
                );
            }
            ui.separator();
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 0.0, color(view.indicator));
                ui.label(&view.solution_label);
                ui.label(RichText::new(&view.solution).color(color(view.solution_color)));
            });
            egui::Grid::new("position")
                .num_columns(2)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for (label, value) in &view.rows {
                        ui.label(label);
                        ui.monospace(value);
                        ui.end_row();
                    }
                });
            ui.monospace(&view.quality);
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(view.action_enabled, egui::Button::new(&view.action_label))
                    .clicked()
                {
                    self.controller.request(view.action);
                }
                ui.add_enabled(false, egui::Button::new(&view.options_label))
                    .on_disabled_hover_text(&view.options_hint);
                if ui.button(&view.exit_label).clicked() {
                    self.controller.begin_exit();
                    ui.ctx().request_repaint();
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(&view.status);
            });
        });
        let directory = self.controller.working_directory();
        let labels: Vec<_> =
            self.controller
                .overwrite_dialog
                .as_ref()
                .map_or_else(Vec::new, |dialog| {
                    dialog
                        .paths
                        .iter()
                        .map(|row| self.controller.display(&row.path))
                        .collect()
                });
        let mut confirm = false;
        let mut cancel = false;
        if let Some(dialog) = &mut self.controller.overwrite_dialog {
            // The modal is drawn inside the compact navi window: bound it by the window,
            // wrap long paths and scroll them, so that the buttons always stay visible.
            let window = ui.ctx().content_rect().size();
            egui::Modal::new(egui::Id::new("overwrite outputs")).show(ui.ctx(), |ui| {
                ui.set_max_width((window.x - DIALOG_MARGIN).clamp(0.0, 600.0));
                ui.heading("Existing output files");
                let buttons = ui.spacing().interact_size.y + 2.0 * ui.spacing().item_spacing.y;
                let list = window.y - DIALOG_MARGIN - ui.min_rect().height() - buttons;
                egui::ScrollArea::vertical()
                    .max_height(list.max(0.0))
                    .show(ui, |ui| {
                        ui.label(format!("Working directory: {directory}"));
                        for (row, label) in dialog.paths.iter_mut().zip(labels) {
                            ui.group(|ui| {
                                ui.label(label);
                                ui.checkbox(&mut row.overwrite, "Overwrite");
                            });
                        }
                    });
                ui.horizontal(|ui| {
                    confirm = ui
                        .add_enabled(dialog.all_checked(), egui::Button::new("Start"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        }
        if cancel {
            self.controller.cancel_start();
        } else if confirm {
            self.controller.confirm_overwrite();
        }
    }

    fn on_exit(&mut self) {
        self.controller.begin_exit();
    }
}
