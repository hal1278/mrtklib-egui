//! RTKCONV-like frontend for the finite `mrtk convert` command.

mod command;
mod controller;
mod naming;
mod picker;
mod settings;
mod ui;
mod view;

fn main() -> eframe::Result {
    mrtklib_egui_common::run_with_context("mrtklib-egui-conv", [760.0, 700.0], |ctx| {
        ui::Conv::new(ctx)
    })
}
