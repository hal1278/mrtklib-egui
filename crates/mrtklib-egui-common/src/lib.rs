//! Parts shared by the mrtklib-egui applications: window setup, fonts and the classic style.

use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use std::path::PathBuf;
use std::sync::Arc;

/// Environment variable that overrides the font file used for Japanese and other CJK text.
const CJK_FONT_ENV: &str = "MRTKLIB_EGUI_CJK_FONT";

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
    if let Some(path) = std::env::var_os(CJK_FONT_ENV) {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!("cannot read the font given by {CJK_FONT_ENV} ({path:?}): {e}")
        });
        add_cjk_font(&mut fonts, bytes);
    } else {
        match system_cjk_font() {
            Some(path) => match std::fs::read(&path) {
                Ok(bytes) => add_cjk_font(&mut fonts, bytes),
                Err(e) => eprintln!("warning: cannot read CJK font {}: {e}", path.display()),
            },
            None => eprintln!(
                "warning: no CJK font found; Japanese text will not render (set {CJK_FONT_ENV})"
            ),
        }
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

fn add_cjk_font(fonts: &mut egui::FontDefinitions, bytes: Vec<u8>) {
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
    // Look for Noto Sans CJK in the font directories of the XDG data directories, without
    // starting a process (fc-match). NixOS profiles are on XDG_DATA_DIRS as well.
    const NAMES: [&str; 2] = ["NotoSansCJK-Regular.ttc", "NotoSansCJKjp-Regular.otf"];
    let mut dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("fonts"))
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/fonts"));
    }
    dirs.iter().find_map(|d| find_file(d, &NAMES, 4))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn find_file(dir: &std::path::Path, names: &[&str], depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if names.iter().any(|n| entry.file_name() == *n) {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs.iter().find_map(|d| find_file(d, names, depth - 1))
}
