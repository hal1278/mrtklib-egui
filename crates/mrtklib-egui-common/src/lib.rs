//! Parts shared by the mrtklib-egui applications: window setup, fonts and the classic style.

mod font;

use eframe::egui::{self, Color32, FontId, TextStyle};

/// Open the main window of an application with the shared fonts and style.
pub fn run<A: eframe::App + 'static>(
    title: &str,
    inner_size: [f32; 2],
    make_app: impl FnOnce() -> A + 'static,
) -> eframe::Result {
    run_with_context(title, inner_size, move |_| make_app())
}

/// Open a window whose constructor needs a context for worker repaint callbacks.
pub fn run_with_context<A: eframe::App + 'static>(
    title: &str,
    inner_size: [f32; 2],
    make_app: impl FnOnce(&egui::Context) -> A + 'static,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(inner_size),
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            setup(&cc.egui_ctx);
            Ok(Box::new(make_app(&cc.egui_ctx)))
        }),
    )
}

/// Install the platform's fonts (see `font`) and the dense, classic look shared by all
/// applications.
pub fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    font::add_fonts(&mut fonts);
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_gray(240);
    visuals.window_fill = Color32::from_gray(240);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        w.corner_radius = egui::CornerRadius::ZERO;
    }
    // The same light look in both themes: RTKLIB has no dark mode.
    ctx.set_visuals_of(egui::Theme::Light, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(4.0, 3.0);
        s.spacing.button_padding = egui::vec2(8.0, 2.0);
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(12.5));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(12.5));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.0));
    });
}
