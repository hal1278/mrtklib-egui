//! The fonts: on Windows the ones RTKLIB shows, elsewhere egui's fonts plus a CJK font.
//!
//! egui draws text itself and has no OS font fallback; it uses an ordered list of fonts per
//! family. RTKLIB's forms use Tahoma, and on Japanese Windows the first font linked to Tahoma
//! (`FontLink\SystemLink` in the registry) is MS UI Gothic. On Windows both are taken as they
//! are, without further candidates: a Japanese Windows environment is assumed to have them.
//! Elsewhere egui's built-in fonts stay first and a CJK font is added for Japanese text. No
//! font is bundled. Font collections (`.ttc`) hold several faces, so each font names the
//! family of its face.

use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::path::PathBuf;
use std::sync::Arc;

/// Environment variable that overrides the font file used for Japanese and other CJK text.
pub(crate) const ENV: &str = "MRTKLIB_EGUI_CJK_FONT";

/// A font file name and the family of the face to use in it.
type Candidate = (&'static str, &'static str);

/// Font for Latin text, placed before egui's built-in fonts.
#[cfg(target_os = "windows")]
const LATIN: &[Candidate] = &[("Tahoma.ttf", "Tahoma")];
#[cfg(not(target_os = "windows"))]
const LATIN: &[Candidate] = &[];

/// Font for Japanese text. Several entries are alternative files of the same font.
#[cfg(target_os = "windows")]
const CJK: &[Candidate] = &[("Msgothic.ttc", "MS UI Gothic")];

#[cfg(target_os = "macos")]
const CJK: &[Candidate] = &[
    // Not verified on macOS.
    ("ヒラギノ角ゴシック W3.ttc", "Hiragino Sans"),
    ("Hiragino Sans GB.ttc", "Hiragino Sans GB"),
];

#[cfg(all(unix, not(target_os = "macos")))]
const CJK: &[Candidate] = &[
    ("NotoSansCJK-Regular.ttc", "Noto Sans CJK JP"),
    ("NotoSansCJKjp-Regular.otf", "Noto Sans CJK JP"),
];

/// Add the Latin and CJK fonts of this platform to egui's font families.
pub(crate) fn add_fonts(fonts: &mut FontDefinitions) {
    let latin = if LATIN.is_empty() {
        None
    } else {
        let found = load(LATIN);
        if found.is_none() {
            eprintln!(
                "warning: {} not found; Latin text uses the built-in font",
                LATIN[0].1
            );
        }
        found
    };
    let cjk = match std::env::var_os(ENV) {
        Some(path) => {
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("cannot read the font given by {ENV} ({path:?}): {e}"));
            Some(font_data(bytes, 0))
        }
        None => {
            let found = load(CJK);
            if found.is_none() {
                eprintln!("warning: no CJK font found; Japanese text will not render (set {ENV})");
            }
            found
        }
    };
    install(fonts, latin, cjk);
}

/// Proportional: the Latin font, then the CJK font, then egui's fonts; without a Latin font
/// the CJK font goes last, so that egui's fonts keep drawing Latin text. Monospace: egui's
/// fonts, then the CJK font.
fn install(fonts: &mut FontDefinitions, latin: Option<FontData>, cjk: Option<FontData>) {
    let has_latin = latin.is_some();
    let has_cjk = cjk.is_some();
    for (name, data) in [("latin", latin), ("cjk", cjk)] {
        if let Some(data) = data {
            fonts.font_data.insert(name.into(), Arc::new(data));
        }
    }
    let proportional = family_mut(fonts, FontFamily::Proportional);
    match (has_latin, has_cjk) {
        (true, true) => {
            proportional.insert(0, "cjk".into());
            proportional.insert(0, "latin".into());
        }
        (true, false) => proportional.insert(0, "latin".into()),
        (false, true) => proportional.push("cjk".into()),
        (false, false) => {}
    }
    if has_cjk {
        family_mut(fonts, FontFamily::Monospace).push("cjk".into());
    }
}

fn family_mut(fonts: &mut FontDefinitions, family: FontFamily) -> &mut Vec<String> {
    fonts
        .families
        .get_mut(&family)
        .expect("default families exist")
}

fn font_data(bytes: Vec<u8>, index: u32) -> FontData {
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    data
}

/// The first candidate that exists and holds a face of the wanted family.
fn load(candidates: &[Candidate]) -> Option<FontData> {
    for (file, family) in candidates {
        let Some(path) = locate(file) else { continue };
        match std::fs::read(&path) {
            Ok(bytes) => match face_index(&bytes, family) {
                Some(index) => return Some(font_data(bytes, index)),
                None => eprintln!("warning: {} has no face of family {family}", path.display()),
            },
            Err(e) => eprintln!("warning: cannot read font {}: {e}", path.display()),
        }
    }
    None
}

/// Index of the first upright face of `family` in a font file or collection. The weight is
/// not checked: every candidate names a file that holds the regular weight.
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

    fn names(fonts: &FontDefinitions, family: FontFamily) -> Vec<String> {
        fonts.families[&family].clone()
    }

    #[test]
    fn latin_font_goes_first_and_cjk_right_after_it() {
        let mut fonts = FontDefinitions::default();
        let defaults = names(&fonts, FontFamily::Proportional);
        install(
            &mut fonts,
            Some(font_data(bundled("Hack"), 0)),
            Some(font_data(bundled("Hack"), 0)),
        );
        let proportional = names(&fonts, FontFamily::Proportional);
        assert_eq!(proportional[..2], ["latin", "cjk"]);
        assert_eq!(proportional[2..], defaults[..]);
        assert_eq!(names(&fonts, FontFamily::Monospace).last().unwrap(), "cjk");
    }

    #[test]
    fn without_a_latin_font_cjk_goes_last() {
        let mut fonts = FontDefinitions::default();
        let defaults = names(&fonts, FontFamily::Proportional);
        install(&mut fonts, None, Some(font_data(bundled("Hack"), 0)));
        let proportional = names(&fonts, FontFamily::Proportional);
        assert_eq!(proportional[..defaults.len()], defaults[..]);
        assert_eq!(proportional.last().unwrap(), "cjk");
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
