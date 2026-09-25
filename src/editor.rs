use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::RwLock;

use crate::config;

// Set by the X close button to request the editor close.
pub static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

// Fullscreen editor: the panel covers the whole window and its background is
// solid. Toggled by the heading-bar button (or the `q` key in navigation
// mode); read by main to size the panel and by the renderer to pick the
// background.
pub static FULLSCREEN: AtomicBool = AtomicBool::new(false);

// Navigation mode: the cursor is detached from the text (everything renders
// as formatted view-mode) and the line under the cursor is only *selected*,
// highlighted by a bar. Esc enters it, Enter/e re-enter edit mode, g/q/esc
// navigate away. Reset whenever a file loads so a document always opens in
// edit mode.
pub static NAV_MODE: AtomicBool = AtomicBool::new(false);

// Set by the heading-bar sub-graph button (shown when the open note has a
// companion folder) to navigate the graph into it and close the editor.
// Consumed once by main.
pub static OPEN_SUBGRAPH_REQUESTED: AtomicBool = AtomicBool::new(false);

// The buffer has unsaved edits since the last save.
pub static DIRTY: AtomicBool = AtomicBool::new(false);

// The placeholder body (editor open with no node) shows a "Create New Node"
// button plus a filename prompt. CREATING_NODE, when set, makes the input
// handler swallow everything while the prompt is up.
pub static CREATING_NODE: AtomicBool = AtomicBool::new(false);
pub static NEW_NODE_NAME: RwLock<String> = RwLock::new(String::new());

// Indent-folding: source lines whose indentation sits strictly deeper than a
// fold-head line's can be folded away (like a code editor's outline). The Vec
// holds the source-line indices that are folded right now (their folded block
// is hidden). Toggled by clicking the caret on a fold-head line.
pub static FOLDED_LINES: RwLock<Vec<usize>> = RwLock::new(Vec::new());

// Screen rect of the placeholder's "Create New Node" button, written by the
// renderer each placeholder frame and read by the input handler for clicks.
pub static NEW_NODE_BUTTON: RwLock<Option<(i32, i32, i32, i32)>> = RwLock::new(None);

// Timestamp (raylib GetTime seconds, scaled to millis) of the most recent
// edit, used to autosave after a typing pause (no edits for a while).
pub static LAST_EDIT_MILLIS: AtomicU64 = AtomicU64::new(0);

// Bounds of the editor panel for the current fullscreen state. Single source
// of truth for the panel geometry: the graph handler's "click inside/outside
// the editor" test, the editor's own input geometry and main's per-frame panel
// size all use this, so they can never disagree about where the panel is.
pub fn panel_bounds() -> (i32, i32, i32, i32) {
    if FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        // Fullscreen covers the whole window: the heading bar spans edge to
        // edge, and a click anywhere on the scaled background counts as being
        // inside the editor (never dismisses it).
        (0, 0, config::width(), config::height())
    } else {
        config::editor_panel_bounds()
    }
}

// Where the text body (and the scrollbar) may go: the panel with a horizontal
// margin kept off each screen edge in fullscreen so lines never sit flush
// against the display. The heading bar is deliberately NOT inset (it uses
// panel_bounds()); only the editable text and its chrome use this.
pub fn content_bounds() -> (i32, i32, i32, i32) {
    let (x, y, w, h) = panel_bounds();
    if FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        let m = config::FULLSCREEN_H_MARGIN;
        (x + m, y, (w - 2 * m).max(1), h)
    } else {
        (x, y, w, h)
    }
}

pub mod autocomplete;
pub mod blocks;
pub mod buffer;
pub mod command;
pub mod history;
pub mod hit_test;
pub mod images;
pub mod input_handler;
pub mod markdown;
pub mod renderer;
pub mod tabs;
pub mod text;
