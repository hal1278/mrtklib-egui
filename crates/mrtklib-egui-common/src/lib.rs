//! Parts shared by the mrtklib-egui applications: window setup, fonts and the classic style.

use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use std::path::PathBuf;
use std::sync::Arc;

/// Environment variable that overrides the font file used for Japanese and other CJK text.
pub const CJK_FONT_ENV: &str = "MRTKLIB_EGUI_CJK_FONT";

/// Open the main window of an application with the shared fonts and style.
pub fn run<A: eframe::App + 'static>(
    title: &str,
    inner_size: [f32; 2],
    make_app: impl FnOnce() -> A + 'static,
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
            Ok(Box::new(make_app()))
        }),
    )
}

/// Install the CJK fallback font and the dense, classic look shared by all applications.
pub fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    match find_cjk_font() {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert("cjk".into(), Arc::new(egui::FontData::from_owned(bytes)));
                for family in [FontFamily::Proportional, FontFamily::Monospace] {
                    fonts
                        .families
                        .get_mut(&family)
                        .expect("default families exist")
                        .push("cjk".into());
                }
            }
            Err(e) => eprintln!("warning: cannot read CJK font {}: {e}", path.display()),
        },
        None => eprintln!(
            "warning: no CJK font found; Japanese text will not render (set {CJK_FONT_ENV})"
        ),
    }
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

/// Find a system font file with Japanese glyphs. The environment variable wins.
fn find_cjk_font() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(CJK_FONT_ENV) {
        return Some(path.into());
    }
    system_cjk_font()
}

#[cfg(target_os = "windows")]
fn system_cjk_font() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("WINDIR")?).join("Fonts");
    ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.exists())
}

#[cfg(target_os = "macos")]
fn system_cjk_font() -> Option<PathBuf> {
    [
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn system_cjk_font() -> Option<PathBuf> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "Noto Sans CJK JP"])
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).into_owned());
    path.exists().then_some(path)
}
