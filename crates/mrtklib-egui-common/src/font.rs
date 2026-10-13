//! The font for Japanese text.
//!
//! egui's built-in fonts have no CJK glyphs, so a system font is added as a fallback; no
//! font is bundled. Candidates are tried in order. On Windows the order follows the RTKLIB
//! applications (Meiryo UI, then MS Gothic), then Noto Sans JP, which recent Windows versions
//! include. Font collections (`.ttc`) hold several faces, so each candidate names the
//! family of the face to use.

use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::path::PathBuf;
use std::sync::Arc;

/// Environment variable that overrides the font file used for Japanese and other CJK text.
pub(crate) const ENV: &str = "MRTKLIB_EGUI_CJK_FONT";

/// A font file name and the family of the face to use in it.
type Candidate = (&'static str, &'static str);

#[cfg(target_os = "windows")]
const CANDIDATES: &[Candidate] = &[
    ("Meiryo.ttc", "Meiryo UI"),
    ("Msgothic.ttc", "MS Gothic"),
    // The file name of Noto Sans JP on Windows is not verified yet.
    ("NotoSansJP-VF.ttf", "Noto Sans JP"),
    ("NotoSansJP-Regular.otf", "Noto Sans JP"),
    ("NotoSansJP-Regular.ttf", "Noto Sans JP"),
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[Candidate] = &[
    // Not verified on macOS.
    ("ヒラギノ角ゴシック W3.ttc", "Hiragino Sans"),
    ("Hiragino Sans GB.ttc", "Hiragino Sans GB"),
];

#[cfg(all(unix, not(target_os = "macos")))]
const CANDIDATES: &[Candidate] = &[
    ("NotoSansCJK-Regular.ttc", "Noto Sans CJK JP"),
    ("NotoSansCJKjp-Regular.otf", "Noto Sans CJK JP"),
    ("NotoSansJP-Regular.otf", "Noto Sans JP"),
];

/// Add the CJK fallback font to all font families.
pub(crate) fn add_cjk_font(fonts: &mut FontDefinitions) {
    if let Some(path) = std::env::var_os(ENV) {
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("cannot read the font given by {ENV} ({path:?}): {e}"));
        install(fonts, bytes, 0);
        return;
    }
    match find_system_font() {
        Some((bytes, index)) => install(fonts, bytes, index),
        None => eprintln!("warning: no CJK font found; Japanese text will not render (set {ENV})"),
    }
}

fn install(fonts: &mut FontDefinitions, bytes: Vec<u8>, index: u32) {
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    fonts.font_data.insert("cjk".into(), Arc::new(data));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .get_mut(&family)
            .expect("default families exist")
            .push("cjk".into());
    }
}

/// The first candidate that exists and holds a face of the wanted family.
fn find_system_font() -> Option<(Vec<u8>, u32)> {
    for (file, family) in CANDIDATES {
        let Some(path) = locate(file) else { continue };
        match std::fs::read(&path) {
            Ok(bytes) => match face_index(&bytes, family) {
                Some(index) => return Some((bytes, index)),
                None => eprintln!("warning: {} has no face of family {family}", path.display()),
            },
            Err(e) => eprintln!("warning: cannot read font {}: {e}", path.display()),
        }
    }
    None
}

/// Index of the upright face of `family` in a font file or collection.
fn face_index(bytes: &[u8], family: &str) -> Option<u32> {
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    (0..count).find(|&index| {
        ttf_parser::Face::parse(bytes, index).is_ok_and(|face| {
            !face.is_italic()
                && face.names().into_iter().any(|name| {
                    matches!(
                        name.name_id,
                        ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                    ) && name
                        .to_string()
                        .is_some_and(|n| n.eq_ignore_ascii_case(family))
                })
        })
    })
}

#[cfg(target_os = "windows")]
fn locate(file: &str) -> Option<PathBuf> {
    // The system font directory, then fonts installed for the current user only.
    let system = std::env::var_os("WINDIR").map(|d| PathBuf::from(d).join("Fonts"));
    let user = std::env::var_os("LOCALAPPDATA").map(|d| {
        PathBuf::from(d)
            .join("Microsoft")
            .join("Windows")
            .join("Fonts")
    });
    [system, user]
        .into_iter()
        .flatten()
        .map(|d| d.join(file))
        .find(|p| p.exists())
}

#[cfg(target_os = "macos")]
fn locate(file: &str) -> Option<PathBuf> {
    ["/System/Library/Fonts", "/Library/Fonts"]
        .iter()
        .map(|d| PathBuf::from(d).join(file))
        .find(|p| p.exists())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn locate(file: &str) -> Option<PathBuf> {
    // The font directories of the XDG data directories, searched without starting a process
    // (fc-match). NixOS profiles are on XDG_DATA_DIRS as well.
    let mut dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("fonts"))
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/fonts"));
    }
    dirs.iter().find_map(|d| find_file(d, file, 4))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn find_file(dir: &std::path::Path, name: &str, depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if entry.file_name() == name {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs.iter().find_map(|d| find_file(d, name, depth - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled(name: &str) -> Vec<u8> {
        FontDefinitions::default().font_data[name].font.to_vec()
    }

    #[test]
    fn face_is_chosen_by_family() {
        let hack = bundled("Hack");
        assert_eq!(face_index(&hack, "Hack"), Some(0));
        assert_eq!(face_index(&hack, "hack"), Some(0));
        assert_eq!(face_index(&hack, "Meiryo UI"), None);
    }

    /// Faces of a collection on this machine; run with `--ignored` where Noto Sans CJK is
    /// installed.
    #[test]
    #[ignore]
    fn faces_of_a_system_collection() {
        let path = locate("NotoSansCJK-Regular.ttc").expect("Noto Sans CJK installed");
        let bytes = std::fs::read(path).unwrap();
        let jp = face_index(&bytes, "Noto Sans CJK JP").expect("JP face");
        let kr = face_index(&bytes, "Noto Sans CJK KR").expect("KR face");
        assert_ne!(jp, kr);
        println!("Noto Sans CJK JP is face {jp}, KR is face {kr}");
    }
}
