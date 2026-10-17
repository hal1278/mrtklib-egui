//! RTKCONV-like frontend for the finite `mrtk convert` command.

mod command;
mod controller;
mod file_dialog;
mod logging;
mod naming;
mod settings;
mod signal_mask;
mod ui;
mod view;

fn main() -> eframe::Result {
    logging::install().map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    mrtklib_egui_common::run_with_context("mrtklib-egui-conv", ui::INITIAL_SIZE, |ctx| {
        ui::Conv::new(ctx)
    })
}
