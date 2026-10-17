//! RTKPLOT-like viewer for two MRTKLIB text solution files.
mod controller;
mod data;
mod grid;
mod navigation;
mod reader;
mod settings;
mod time;
mod ui;

fn main() -> eframe::Result {
    mrtklib_egui_common::logging::install().map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    mrtklib_egui_common::run_with_context("mrtklib-egui-plot", ui::INITIAL_SIZE, ui::PlotApp::new)
}
