use raylib::prelude::*;
use std::sync::RwLock;

// Initial window size; the window is resizable and the current size is tracked
// in SCREEN_SIZE (updated every frame by main.rs). Everything else reads the
// live size through width()/height().
pub const DEFAULT_W: i32 = 1920 * 3 / 4;
pub const DEFAULT_H: i32 = 1080 * 3 / 4;

// Floor for a persisted window size: a config file with a degenerate value
// (e.g. overscrolled to zero) is clamped back up so the window always opens
// with usable room.
pub const MIN_WINDOW_W: i32 = 800;
pub const MIN_WINDOW_H: i32 = 600;

// Current window size in pixels. Written by main.rs each frame from
// get_screen_width/height(); read by every layout function so geometry always
// matches the real window even across resizes.
pub static SCREEN_SIZE: RwLock<(i32, i32)> = RwLock::new((DEFAULT_W, DEFAULT_H));

pub fn width() -> i32 {
    SCREEN_SIZE.read().unwrap().0
}

pub fn height() -> i32 {
    SCREEN_SIZE.read().unwrap().1
}

// Global text scale (Ctrl +/-). One factor drives the editor body text and the
// graph view's UI text. In-memory only: resets to 1.0 each launch.
pub static TEXT_ZOOM: RwLock<f32> = RwLock::new(1.0);

pub const TEXT_ZOOM_MIN: f32 = 0.5;
pub const TEXT_ZOOM_MAX: f32 = 3.0;
pub const TEXT_ZOOM_STEP: f32 = 0.1;

pub fn zoom() -> f32 {
    *TEXT_ZOOM.read().unwrap()
}

// A base pixel size scaled by the current zoom, clamped to at least 1 so a
// shrunken font can never collapse a layout to zero or negative space.
pub fn scaled_size(base: i32) -> i32 {
    ((base as f32 * zoom()).round() as i32).max(1)
}

// The editor panel is a centered rectangle covering this fraction of the
// screen. Shared so the graph input handler can tell "click on/off editor".
pub const EDITOR_PANEL_FRACTION: f32 = 0.8;

// Horizontal margin (either side) kept between the screen edge and the
// editor's content in fullscreen mode, so lines and the heading buttons never
// sit flush against the display edges.
pub const FULLSCREEN_H_MARGIN: i32 = 80;

pub fn editor_panel_bounds() -> (i32, i32, i32, i32) {
    let w = (width() as f32 * EDITOR_PANEL_FRACTION) as i32;
    let h = (height() as f32 * EDITOR_PANEL_FRACTION) as i32;
    ((width() - w) / 2, (height() - h) / 2, w, h)
}

pub const EDITOR_FONT_SIZE: i32 = 20;
pub const EDITOR_HEADING_SIZE: [i32; 6] = [70, 52, 42, 33, 25, 21];
pub const EDITOR_PADDING: i32 = 12;
pub const EDITOR_LINE_SPACING: i32 = 2;
pub const EDITOR_CURSOR_HEIGHT_RATIO: f32 = 0.8;
pub const EDITOR_HEADER_HEIGHT: i32 = 40;
// Maximum height a whole-line image link is scaled down to in the editor.
pub const EDITOR_IMAGE_MAX_HEIGHT: i32 = 320;
// Collapsed frontmatter block: shown as a small pill of this height when the
// cursor is outside it, so the dead-metadata lines never scroll through.
pub const EDITOR_FRONTMATTER_HEIGHT: i32 = 16;
pub const EDITOR_FRONTMATTER_BG: Color = Color::new(60, 60, 72, 160);
pub const EDITOR_FRONTMATTER_COLOR: Color = Color::new(150, 158, 190, 255);

pub const EDITOR_FONT_COLOR: Color = Color::WHITE;
pub const EDITOR_SELECTION_COLOR: Color = Color::new(76, 128, 204, 102);
pub const EDITOR_LINK_COLOR: Color = Color::SKYBLUE;
pub const EDITOR_CODE_COLOR: Color = Color::new(240, 200, 120, 255);
pub const EDITOR_CODE_BG: Color = Color::new(50, 50, 60, 220);
pub const EDITOR_BLOCKQUOTE_COLOR: Color = Color::new(170, 190, 220, 255);
pub const EDITOR_BLOCKQUOTE_BAR: Color = Color::new(120, 150, 210, 255);
pub const EDITOR_HR_COLOR: Color = Color::new(120, 120, 120, 255);
pub const EDITOR_LIST_MARKER_COLOR: Color = Color::new(160, 160, 220, 255);

pub const AUTOCOMPLETE_MAX_VISIBLE: usize = 8;
pub const AUTOCOMPLETE_ITEM_HEIGHT: i32 = 18;
pub const AUTOCOMPLETE_BG: Color = Color::new(20, 20, 20, 235);
pub const AUTOCOMPLETE_SELECTED_BG: Color = Color::new(76, 128, 204, 160);

// Context menu
pub const CONTEXT_MENU_W: i32 = 180;
pub const CONTEXT_MENU_ITEM_H: i32 = 30;
pub const CONTEXT_MENU_SEP_COLOR: Color = Color::new(60, 60, 60, 255);

// Breadcrumb trail
pub const BREADCRUMB_Y: i32 = 10;
pub const BREADCRUMB_PAD: i32 = 8;

// Editor tab bar: pixels the horizontal tab strip scrolls per wheel notch.
pub const TAB_SCROLL_STEP: i32 = 40;

#[cfg(test)]
mod tests {
    use super::*;

    // All in one test: TEXT_ZOOM is a process-wide global shared with the
    // other config tests, so anything asserting on it must run in a single
    // thread to avoid racing the shared value.
    #[test]
    fn scaled_size_follows_zoom() {
        *TEXT_ZOOM.write().unwrap() = 1.0;
        assert_eq!(scaled_size(20), 20);
        assert_eq!(scaled_size(70), 70);
        assert_eq!(scaled_size(0), 1);

        *TEXT_ZOOM.write().unwrap() = 2.0;
        assert_eq!(scaled_size(20), 40);
        assert_eq!(scaled_size(30), 60);

        *TEXT_ZOOM.write().unwrap() = 1.5;
        assert_eq!(scaled_size(21), 32);

        *TEXT_ZOOM.write().unwrap() = TEXT_ZOOM_MIN;
        assert!(scaled_size(1) >= 1);
        assert!(scaled_size(20) >= 1);

        *TEXT_ZOOM.write().unwrap() = 1.0;
    }
}
