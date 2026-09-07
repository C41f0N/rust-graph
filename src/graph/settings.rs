use crate::config;
use std::path::PathBuf;
use std::sync::RwLock;

// Settings button (screen space, top-right of the graph view). Top-left is
// taken by the breadcrumb trail, so it lives in the opposite corner.
pub const SETTINGS_BUTTON_W: i32 = 96;
pub const SETTINGS_BUTTON_H: i32 = 32;
pub const SETTINGS_BUTTON_X: i32 = config::WIDTH - SETTINGS_BUTTON_W - 10;
pub const SETTINGS_BUTTON_Y: i32 = 10;

// Settings dialog / font picker geometry.
pub const SETTINGS_PANEL_W: i32 = 440;
pub const SETTINGS_PANEL_H: i32 = 500;
pub const SETTINGS_TITLE_H: i32 = 40;
pub const SETTINGS_ROW_H: i32 = 26;
pub const SETTINGS_VISIBLE_ROWS: usize = 16;

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
    (config::WIDTH - SETTINGS_PANEL_W) / 2
}

pub fn panel_y() -> i32 {
    70
}

// Enumerate the system's installed fonts (fontdb scans the standard font
// directories on Windows, Linux and macOS). Families are de-duplicated by
// their primary name and sorted alphabetically.
pub fn build_font_list() {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut families: Vec<FontFamily> = vec![FontFamily {
        name: "(Default)".to_string(),
        path: None,
        data: None,
    }];

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for face in db.faces() {
        let Some((name, _)) = face.families.first() else {
            continue;
        };
        if name.trim().is_empty() || !seen.insert(name.clone()) {
            continue;
        }

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

    families[1..].sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    *FONTS.write().unwrap() = families;
    *SELECTED_FONT.write().unwrap() = Some(0);
}