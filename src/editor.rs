use std::sync::atomic::{AtomicBool, AtomicU64};

use crate::config;

// Set by the X close button to request the editor close.
pub static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

// Fullscreen editor: the panel covers the whole window and its background is
// solid. Toggled by the heading-bar button; read by main to size the panel and
// by the renderer to pick the background.
pub static FULLSCREEN: AtomicBool = AtomicBool::new(false);

// Set by the heading-bar sub-graph button (shown when the open note has a
// companion folder) to navigate the graph into it and close the editor.
// Consumed once by main.
pub static OPEN_SUBGRAPH_REQUESTED: AtomicBool = AtomicBool::new(false);

// The buffer has unsaved edits since the last save.
pub static DIRTY: AtomicBool = AtomicBool::new(false);

// Timestamp (raylib GetTime seconds, scaled to millis) of the most recent
// edit, used to autosave after a typing pause (no edits for a while).
pub static LAST_EDIT_MILLIS: AtomicU64 = AtomicU64::new(0);

// Bounds of the editor panel for the current fullscreen state. Single source
// of truth for the panel geometry: the graph handler's "click inside/outside
// the editor" test, the editor's own input geometry and main's per-frame panel
// size all use this, so they can never disagree about where the panel is.
pub fn panel_bounds() -> (i32, i32, i32, i32) {
    if FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        (0, 0, config::WIDTH, config::HEIGHT)
    } else {
        config::editor_panel_bounds()
    }
}

pub mod autocomplete;
pub mod blocks;
pub mod buffer;
pub mod history;
pub mod hit_test;
pub mod images;
pub mod input_handler;
pub mod markdown;
pub mod renderer;
pub mod text;
