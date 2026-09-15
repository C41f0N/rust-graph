use crate::config;
use std::path::PathBuf;
use std::sync::RwLock;

// Settings button (screen space, top-right of the graph view). Top-left is
// taken by the breadcrumb trail, so it lives in the opposite corner.
pub const SETTINGS_BUTTON_W: i32 = 96;
pub const SETTINGS_BUTTON_H: i32 = 32;
pub const SETTINGS_BUTTON_Y: i32 = 10;
pub fn settings_button_x() -> i32 {
    config::width() - config::scaled_size(SETTINGS_BUTTON_W) - 10
}

// Settings dialog / font picker geometry.
pub const SETTINGS_PANEL_W: i32 = 440;
pub const SETTINGS_PANEL_H: i32 = 500;
pub const SETTINGS_TITLE_H: i32 = 40;
pub const SETTINGS_ROW_H: i32 = 26;
pub const SETTINGS_VISIBLE_ROWS: usize = 16;

// The whole dialog scales with the global text zoom so the list rows, title
// and their hit-testing stay in lockstep.
pub fn panel_w() -> i32 {
    config::scaled_size(SETTINGS_PANEL_W)
}

pub fn panel_h() -> i32 {
    config::scaled_size(SETTINGS_PANEL_H)
}

pub fn title_h() -> i32 {
    config::scaled_size(SETTINGS_TITLE_H)
}

pub fn row_h() -> i32 {
    config::scaled_size(SETTINGS_ROW_H)
}

pub static SETTINGS_OPEN: RwLock<bool> = RwLock::new(false);
pub static SETTINGS_SCROLL: RwLock<usize> = RwLock::new(0);
pub static SELECTED_FONT: RwLock<Option<usize>> = RwLock::new(None);

// Set by the dialog when the user picks a font; read by main.rs which owns
// the raylib handle and can actually load the font.
pub static REQUEST_LOAD_FONT: RwLock<Option<usize>> = RwLock::new(None);

// One selectable entry in the font list. Entry 0 is always the "(Default)"
// sentinel meaning "use raylib's built-in font".
#[derive(Clone)]
pub struct FontStyleSource {
    pub path: Option<PathBuf>,
    pub data: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct FontFamily {
    pub name: String,
    pub path: Option<PathBuf>,
    pub data: Option<Vec<u8>>,
    // Sibling cuts of the same family for markdown emphasis. Loaded lazily by
    // main.rs next to the upright roster; None when the family ships no such
    // file (the rendering then falls back to the upright glyphs).
    pub bold: Option<FontStyleSource>,
    pub italic: Option<FontStyleSource>,
}

pub static FONTS: RwLock<Vec<FontFamily>> = RwLock::new(Vec::new());

pub fn panel_x() -> i32 {
    (config::width() - panel_w()) / 2
}

pub fn panel_y() -> i32 {
    70
}

// How far a face is from the "regular" cut of a family: upright style outranks
// weight and width so an Italic/Oblique sibling can never win the picker slot
// when a Normal face exists. Lower is better.
type RegularScore = (u8, u16, u16);

fn style_rank(style: fontdb::Style) -> u8 {
    match style {
        fontdb::Style::Normal => 0,
        fontdb::Style::Italic => 1,
        fontdb::Style::Oblique => 2,
    }
}

fn regular_score(face: &fontdb::FaceInfo) -> RegularScore {
    (
        style_rank(face.style),
        face.weight.0.abs_diff(400),
        face.stretch.to_number().abs_diff(5),
    )
}

// Bold cut: still upright, but as close to weight 700 as the family gets.
type BoldScore = (u8, u16, u16);

fn bold_score(face: &fontdb::FaceInfo) -> BoldScore {
    (
        style_rank(face.style),
        face.weight.0.abs_diff(700),
        face.stretch.to_number().abs_diff(5),
    )
}

// Italic cut: a slanted face (italic preferred over oblique) at weight 400.
type ItalicScore = (u8, u16, u16);

fn italic_score(face: &fontdb::FaceInfo) -> ItalicScore {
    let slant = match face.style {
        fontdb::Style::Italic => 0,
        fontdb::Style::Oblique => 1,
        fontdb::Style::Normal => 2,
    };
    (
        slant,
        face.weight.0.abs_diff(400),
        face.stretch.to_number().abs_diff(5),
    )
}

fn insert_best<S: Ord>(
    map: &mut std::collections::HashMap<String, (usize, S)>,
    name: &str,
    i: usize,
    score: S,
) {
    match map.get(name) {
        Some(&(_, ref cur)) if *cur <= score => {}
        _ => {
            map.insert(name.to_string(), (i, score));
        }
    }
}

fn style_source(face: &fontdb::FaceInfo) -> FontStyleSource {
    match &face.source {
        fontdb::Source::File(path) => FontStyleSource {
            path: Some(path.clone()),
            data: None,
        },
        fontdb::Source::SharedFile(path, _) => FontStyleSource {
            path: Some(path.clone()),
            data: None,
        },
        fontdb::Source::Binary(bytes) => FontStyleSource {
            path: None,
            data: Some(bytes.as_ref().as_ref().to_vec()),
        },
    }
}

// Enumerate the system's installed fonts (fontdb scans the standard font
// directories on Windows, Linux and macOS). One entry per family is kept,
// carrying the upright, bold and italic cuts the family actually ships (a
// missing cut stays None and rendering falls back to upright glyphs). The
// upright cut is picked so an Italic/Oblique sibling can never win the slot
// just because it was the first face fontdb enumerated. Families are sorted
// alphabetically.
pub fn build_font_list() {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let faces: Vec<fontdb::FaceInfo> = db.faces().cloned().collect();

    let mut reg: std::collections::HashMap<String, (usize, RegularScore)> =
        std::collections::HashMap::new();
    let mut bold: std::collections::HashMap<String, (usize, BoldScore)> =
        std::collections::HashMap::new();
    let mut ital: std::collections::HashMap<String, (usize, ItalicScore)> =
        std::collections::HashMap::new();

    for (i, face) in faces.iter().enumerate() {
        let Some((name, _)) = face.families.first() else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        insert_best(&mut reg, name, i, regular_score(face));
        insert_best(&mut bold, name, i, bold_score(face));
        insert_best(&mut ital, name, i, italic_score(face));
    }

    let mut names: Vec<&str> = reg.keys().map(|n| n.as_str()).collect();
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

    let mut families: Vec<FontFamily> = vec![FontFamily {
        name: "(Default)".to_string(),
        path: None,
        data: None,
        bold: None,
        italic: None,
    }];

    for name in names {
        let (fi, _) = reg[name];
        let src = style_source(&faces[fi]);
        // A family with no real bold/italic cut (e.g. Fira Code) scores its
        // regular face for those slots; drop the duplicate so we don't raster
        // the same file three times. Rendering then falls back to upright.
        let distinct = |s: &FontStyleSource| s.path != src.path || s.data != src.data;
        let bold_src = bold
            .get(name)
            .map(|(j, _)| style_source(&faces[*j]))
            .filter(&distinct);
        let italic_src = ital
            .get(name)
            .map(|(j, _)| style_source(&faces[*j]))
            .filter(&distinct);
        families.push(FontFamily {
            name: name.to_string(),
            path: src.path.clone(),
            data: src.data.clone(),
            bold: bold_src,
            italic: italic_src,
        });
    }

    *FONTS.write().unwrap() = families;
    *SELECTED_FONT.write().unwrap() = Some(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(style: fontdb::Style, weight: u16, stretch: u16) -> fontdb::FaceInfo {
        let stretch = match stretch {
            1 => fontdb::Stretch::UltraCondensed,
            2 => fontdb::Stretch::ExtraCondensed,
            3 => fontdb::Stretch::Condensed,
            4 => fontdb::Stretch::SemiCondensed,
            5 => fontdb::Stretch::Normal,
            _ => fontdb::Stretch::Normal,
        };
        fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: fontdb::Source::Binary(
                std::sync::Arc::new([0u8; 4]) as std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>
            ),
            index: 0,
            families: vec![("Fixture".to_string(), fontdb::Language::English_UnitedStates)],
            post_script_name: "Fixture".to_string(),
            style,
            weight: fontdb::Weight(weight),
            stretch,
            monospaced: false,
        }
    }

    #[test]
    fn upright_regular_wins_over_slanted_siblings() {
        let italic = regular_score(&face(fontdb::Style::Italic, 400, 5));
        let oblique = regular_score(&face(fontdb::Style::Oblique, 400, 5));
        let normal = regular_score(&face(fontdb::Style::Normal, 400, 5));
        assert!(normal < italic && normal < oblique, "upright must beat slants");
    }

    #[test]
    fn weight_and_width_break_style_ties() {
        let bold = regular_score(&face(fontdb::Style::Normal, 700, 5));
        let regular = regular_score(&face(fontdb::Style::Normal, 400, 5));
        assert!(regular < bold, "weight 400 preferred over 700");

        let condensed = regular_score(&face(fontdb::Style::Normal, 400, 3));
        assert!(regular < condensed, "normal width preferred over condensed");
    }
}
