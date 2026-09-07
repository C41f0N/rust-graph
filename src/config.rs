use raylib::prelude::*;

pub const HEIGHT: i32 = 1080 * 3 / 4;
pub const WIDTH: i32 = 1920 * 3 / 4;

// The editor panel is a centered rectangle covering this fraction of the
// screen. Shared so the graph input handler can tell "click on/off editor".
pub const EDITOR_PANEL_FRACTION: f32 = 0.8;

pub fn editor_panel_bounds() -> (i32, i32, i32, i32) {
    let w = (WIDTH as f32 * EDITOR_PANEL_FRACTION) as i32;
    let h = (HEIGHT as f32 * EDITOR_PANEL_FRACTION) as i32;
    ((WIDTH - w) / 2, (HEIGHT - h) / 2, w, h)
}

pub const EDITOR_FONT_SIZE: i32 = 20;
pub const EDITOR_HEADING_SIZE: [i32; 6] = [70, 52, 42, 33, 25, 21];
pub const EDITOR_PADDING: i32 = 12;
pub const EDITOR_LINE_SPACING: i32 = 2;
pub const EDITOR_CURSOR_HEIGHT_RATIO: f32 = 0.8;
pub const EDITOR_HEADER_HEIGHT: i32 = 40;
// Maximum height a whole-line image link is scaled down to in the editor.
pub const EDITOR_IMAGE_MAX_HEIGHT: i32 = 320;
// Collapsed frontmatter block: shown as a single bar of this height when the
// cursor is outside it, so the dead-metadata lines never scroll through.
pub const EDITOR_FRONTMATTER_HEIGHT: i32 = 22;
pub const EDITOR_FRONTMATTER_BG: Color = Color::new(60, 60, 72, 160);
pub const EDITOR_FRONTMATTER_COLOR: Color = Color::new(150, 158, 190, 255);

pub const EDITOR_FONT_COLOR: Color = Color::WHITE;
pub const EDITOR_SELECTION_COLOR: Color = Color::new(76, 128, 204, 102);
pub const EDITOR_LINK_COLOR: Color = Color::SKYBLUE;
pub const EDITOR_CODE_COLOR: Color = Color::new(240, 200, 120, 255);
pub const EDITOR_CODE_BG: Color = Color::new(50, 50, 60, 220);
pub const EDITOR_FENCE_COLOR: Color = Color::new(120, 140, 170, 255);
pub const EDITOR_BLOCKQUOTE_COLOR: Color = Color::new(170, 190, 220, 255);
pub const EDITOR_BLOCKQUOTE_BAR: Color = Color::new(120, 150, 210, 255);
pub const EDITOR_HR_COLOR: Color = Color::new(120, 120, 120, 255);
pub const EDITOR_LIST_MARKER_COLOR: Color = Color::new(160, 160, 220, 255);

pub const AUTOCOMPLETE_MAX_VISIBLE: usize = 8;
pub const AUTOCOMPLETE_ITEM_HEIGHT: i32 = 18;
pub const AUTOCOMPLETE_BG: Color = Color::new(20, 20, 20, 235);
pub const AUTOCOMPLETE_SELECTED_BG: Color = Color::new(76, 128, 204, 160);
