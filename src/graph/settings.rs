use crate::config;
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

// One selectable entry in the font list, carrying serialized SDF atlas bytes
// for a single style cut. Loaded straight from the packed catalog (no device
// font scanning), so every machine shows the same list.
#[derive(Clone)]
pub struct FontStyleSource {
    pub data: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct FontFamily {
    pub name: String,
    pub data: Option<Vec<u8>>,
    // Sibling cuts of the same family for markdown emphasis. Loaded lazily by
    // main.rs next to the upright roster; None when the family ships no such
    // cut (the rendering then falls back to the upright glyphs).
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

// Enumerate the fonts packed into the binary (see crate::fonts). Each family
// carries its pre-baked SDF atlas bytes for the cuts it ships; the interaction
// is instant — there is no font scanning or rasterization on the user's
// machine, so the picker is identical on every system. Row 0 is the default.
pub fn build_font_list() {
    let families: Vec<FontFamily> = crate::fonts::PACKED_FONTS
        .iter()
        .map(|f| FontFamily {
            name: f.name.to_string(),
            data: Some(f.regular.to_vec()),
            bold: f
                .bold
                .map(|b| FontStyleSource {
                    data: Some(b.to_vec()),
                }),
            italic: f
                .italic
                .map(|i| FontStyleSource {
                    data: Some(i.to_vec()),
                }),
        })
        .collect();
    *FONTS.write().unwrap() = families;
    *SELECTED_FONT.write().unwrap() = Some(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every atlas embedded in the packed catalog must pass the SDF-cache header
    // check (headless, no GL) so a bad bake or cut never reaches the picker.
    #[test]
    fn packed_fonts_are_valid_sdf_caches() {
        for f in crate::fonts::PACKED_FONTS {
            assert!(!f.name.is_empty());
            assert!(
                crate::editor::text::sdf_cache_ok(&f.regular),
                "regular atlas invalid for {}",
                f.name
            );
            if let Some(b) = f.bold {
                assert!(
                    crate::editor::text::sdf_cache_ok(b),
                    "bold atlas invalid for {}",
                    f.name
                );
            }
            if let Some(i) = f.italic {
                assert!(
                    crate::editor::text::sdf_cache_ok(i),
                    "italic atlas invalid for {}",
                    f.name
                );
            }
        }
    }
}
