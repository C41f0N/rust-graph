use crate::config;
use raylib::ffi::Rectangle;
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
// Tall enough for the editor option section (label + two rows) plus the full
// 16-row font list below it.
pub const SETTINGS_PANEL_H: i32 = 580;
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

// --- Editor option section (the modifiable values above the font list) -----
// One row per setting: label on the left, the live value, then a minus and a
// plus button. Every coordinate lives here so the renderer and the input
// handler always agree on where the controls are.

// The settings carried by the section, in display order. A row's index is its
// identity for both painting and hit-testing.
pub const OPTION_ROWS: usize = 2;
pub const OPTION_LINE_SPACING: usize = 1;

// Small square stepper button, and the gap between the two.
const STEP_BTN: i32 = 22;
const STEP_GAP: i32 = 6;
// Vertical gap between the section label and the first row, and between the
// last option row and the font list below it.
const SECTION_LABEL_H: i32 = 18;
const SECTION_GAP: i32 = 8;

pub fn options_label_h() -> i32 {
    config::scaled_size(SECTION_LABEL_H)
}

pub fn options_top() -> i32 {
    // Directly under the title divider.
    panel_y() + title_h()
}

pub fn option_row_y(row: usize) -> i32 {
    options_top() + options_label_h() + (row as i32) * row_h()
}

pub fn options_bottom() -> i32 {
    option_row_y(OPTION_ROWS)
}

pub fn font_list_top() -> i32 {
    options_bottom() + config::scaled_size(SECTION_GAP)
}

// Minus / plus button rects of one option row, in that order.
pub fn stepper_rects(row: usize) -> (Rectangle, Rectangle) {
    let y = option_row_y(row) + (row_h() - STEP_BTN) / 2;
    let right = panel_x() + panel_w() - 12;
    let plus = Rectangle::new(
        (right - STEP_BTN) as f32,
        y as f32,
        STEP_BTN as f32,
        STEP_BTN as f32,
    );
    let minus = Rectangle::new(
        (right - 2 * STEP_BTN - STEP_GAP) as f32,
        y as f32,
        STEP_BTN as f32,
        STEP_BTN as f32,
    );
    (minus, plus)
}

// The step one press moves each setting. Line spacing is a fine pixel glue so
// it moves by 1; the padding is a coarse margin, 4 at a time.
pub fn option_step(row: usize) -> i32 {
    match row {
        OPTION_LINE_SPACING => 1,
        _ => config::EDITOR_PADDING_STEP,
    }
}

// Live value and bounds of one option row, so the dialog shows the same number
// the editor lays out with.
pub fn option_value(row: usize) -> (i32, i32, i32) {
    match row {
        OPTION_LINE_SPACING => (
            config::line_spacing(),
            config::EDITOR_LINE_SPACING_MIN,
            config::EDITOR_LINE_SPACING_MAX,
        ),
        _ => (
            config::editor_padding(),
            config::EDITOR_PADDING_MIN,
            config::EDITOR_PADDING_MAX,
        ),
    }
}

// Human label of one option row.
pub fn option_label(row: usize) -> &'static str {
    match row {
        OPTION_LINE_SPACING => "Line Spacing",
        _ => "Horizontal Padding",
    }
}

// Apply a stepper press: move the setting one step, clamp it, persist. Returns
// true when the value actually moved (a press at a limit is a no-op).
pub fn nudge_option(row: usize, up: bool) -> bool {
    let next = nudged_value(row, up);
    match next {
        Some(next) => {
            match row {
                OPTION_LINE_SPACING => config::set_line_spacing(next),
                _ => config::set_editor_padding(next),
            }
            crate::app_config::save();
            true
        }
        None => false,
    }
}

// Pure preview of what a press would do, so the renderer can grey out a
// stepper that is already at its limit without touching the value.
pub fn nudge_preview(row: usize, up: bool) -> bool {
    nudged_value(row, up).is_some()
}

fn nudged_value(row: usize, up: bool) -> Option<i32> {
    let (value, min, max) = option_value(row);
    let step = option_step(row);
    let next = if up {
        (value + step).min(max)
    } else {
        (value - step).max(min)
    };
    (next != value).then_some(next)
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

    // The option rows, their steppers and the font list must all sit inside the
    // dialog, in order and without overlapping: render and hit-test read these
    // same numbers, so a drift here is a visibly broken control.
    #[test]
    fn option_section_geometry_fits_the_dialog() {
        let (px, py) = (panel_x(), panel_y());
        let (pw, ph) = (panel_w(), panel_h());
        let right = px + pw;

        // Label, then every option row in display order.
        assert!(options_top() >= py + title_h());
        for row in 0..OPTION_ROWS {
            let y = option_row_y(row);
            assert!(y >= options_top() + options_label_h());
            let (minus, plus) = stepper_rects(row);
            for r in [minus, plus] {
                assert!(
                    r.x >= px as f32 && r.x + r.width <= right as f32,
                    "stepper outside panel"
                );
                assert!(
                    r.y >= y as f32 && r.y + r.height <= (y + row_h()) as f32,
                    "stepper off its row"
                );
            }
            // Plus sits right of minus, with the documented gap.
            assert!(plus.x > minus.x + minus.width);
        }
        // Rows never overlap each other.
        assert!(option_row_y(1) >= option_row_y(0) + row_h());

        // The font list starts below the last row and the whole 16-row list
        // still fits inside the dialog.
        assert!(font_list_top() > options_bottom());
        assert!(
            font_list_top() + (SETTINGS_VISIBLE_ROWS as i32) * row_h() <= py + ph,
            "font list overflows the dialog"
        );
    }

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
