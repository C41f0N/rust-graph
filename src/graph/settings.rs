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
pub struct FontFamily {
    pub name: String,
    pub path: Option<PathBuf>,
    pub data: Option<Vec<u8>>,
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

fn regular_score(face: &fontdb::FaceInfo) -> RegularScore {
    let style = match face.style {
        fontdb::Style::Normal => 0,
        fontdb::Style::Italic => 1,
        fontdb::Style::Oblique => 2,
    };
    (style, face.weight.0.abs_diff(400), face.stretch.to_number().abs_diff(5))
}

// Enumerate the system's installed fonts (fontdb scans the standard font
// directories on Windows, Linux and macOS). One face per family is kept -
// whichever sits closest to the regular cut - so the editor never inherits a
// sibling style (italic/oblique/black) just because it happened to be the
// first face fontdb enumerated. Families are sorted alphabetically.
pub fn build_font_list() {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let faces: Vec<fontdb::FaceInfo> = db.faces().cloned().collect();

    let mut best: std::collections::HashMap<String, (usize, RegularScore)> =
        std::collections::HashMap::new();
    for (i, face) in faces.iter().enumerate() {
        let Some((name, _)) = face.families.first() else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let score = regular_score(face);
        match best.get(name) {
            Some(&(_, current)) if current <= score => continue,
            _ => {
                best.insert(name.to_string(), (i, score));
            }
        }
    }

    let mut indices: Vec<usize> = best.into_values().map(|(i, _)| i).collect();
    indices.sort_by(|&a, &b| {
        let (na, _) = faces[a].families.first().unwrap();
        let (nb, _) = faces[b].families.first().unwrap();
        na.to_lowercase().cmp(&nb.to_lowercase())
    });

    let mut families: Vec<FontFamily> = vec![FontFamily {
        name: "(Default)".to_string(),
        path: None,
        data: None,
    }];

    for i in indices {
        let face = &faces[i];
        let (name, _) = face.families.first().unwrap();
        let (path, data) = match &face.source {
            fontdb::Source::File(path) => (Some(path.clone()), None),
            fontdb::Source::SharedFile(path, _) => (Some(path.clone()), None),
            fontdb::Source::Binary(bytes) => (None, Some(bytes.as_ref().as_ref().to_vec())),
        };
        families.push(FontFamily {
            name: name.clone(),
            path,
            data,
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