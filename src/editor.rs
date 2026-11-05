use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
use std::sync::RwLock;

use crate::config;

// Set by the X close button to request the editor close.
pub static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

// Set by Ctrl+W (or Ctrl+Shift+T) to request closing the active tab.
// Deferred to main because tabs::close swaps the buffer globals and must run
// after the editor input handler releases its locks.
pub static CLOSE_TAB_REQUESTED: AtomicBool = AtomicBool::new(false);

// Set by Alt+1..9 to request activating the tab at that zero-based index.
// Deferred to main for the same reason as CLOSE_TAB_REQUESTED; -1 means none.
pub static ACTIVATE_TAB_REQUEST: AtomicI32 = AtomicI32::new(-1);

// Set when navigation-mode `G` asks to return to the graph. Main closes the
// editor before the graph handler runs in the same frame, so without this
// latch the graph's own `G` binding would immediately reopen the editor and
// the key would appear to do nothing. The graph handler consumes it once.
pub static G_TO_GRAPH_LATCH: AtomicBool = AtomicBool::new(false);

// Set by the input handler when a rendered [[wikilink]] is clicked. It only
// records path+name and returns: the input handler holds BUFFER's write lock
// for its whole run, so the tab is actually opened by main after it returns
// (same deferred-consumer pattern as CLOSE_REQUESTED).
pub static OPEN_NOTE_REQUESTED: RwLock<Option<(PathBuf, String)>> = RwLock::new(None);

// Fullscreen editor: the panel covers the whole window and its background is
// solid. Toggled by the heading-bar button (or the `q` key in navigation
// mode); read by main to size the panel and by the renderer to pick the
// background.
pub static FULLSCREEN: AtomicBool = AtomicBool::new(false);

// Navigation mode: the cursor is detached from the text (everything renders
// as formatted view-mode) and the line under the cursor is only *selected*,
// highlighted by a bar. Esc enters it, Enter/e re-enter edit mode, g/q/esc
// navigate away. A file opens in navigation mode; read-only preview pages stay
// there for their whole lifetime.
pub static NAV_MODE: AtomicBool = AtomicBool::new(false);

// A deferred journal open (through OPEN_NOTE_REQUESTED) that found no file
// yet sets this so main drops the freshly loaded tab straight into edit mode
// instead of the usual read-only nav mode: a brand-new entry is a writing
// intent, not a read.
pub static NAV_EDIT_REQUESTED: AtomicBool = AtomicBool::new(false);

// Set by the heading-bar sub-graph button (shown when the open note has a
// companion folder) to navigate the graph into it and close the editor.
// Consumed once by main.
pub static OPEN_SUBGRAPH_REQUESTED: AtomicBool = AtomicBool::new(false);

// Inline rename of the active note: clicking the header title starts it, the
// input handler routes keystrokes into TITLE_RENAME_DRAFT instead of the
// buffer, and Enter commits the rename (draft cleared). Renderer draws the
// draft with an editing caret while ACTIVE is set.
pub static TITLE_RENAME_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static TITLE_RENAME_DRAFT: RwLock<String> = RwLock::new(String::new());

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

// Deferred preview paging. The editor input handler runs while holding the
// BUFFER write lock, so it cannot reload a chunk there; it records -1/+1 here
// and main performs the switch once the lock is released (same pattern as
// OPEN_NOTE_REQUESTED).
pub static PREVIEW_PAGE_REQUEST: AtomicI32 = AtomicI32::new(0);

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

// Height of the read-only preview banner for the active document, in screen
// pixels. Zero for ordinary editable tabs. Renderer and input handler both use
// this so their content origins can never disagree.
pub fn preview_banner_height() -> i32 {
    if tabs::active_is_preview() {
        config::scaled_size(config::EDITOR_PREVIEW_BANNER_H)
    } else {
        0
    }
}

pub mod autocomplete;
pub mod blocks;
pub mod buffer;
pub mod command;
pub mod document;
pub mod history;
pub mod hit_test;
pub mod images;
pub mod input_handler;
pub mod markdown;
pub mod renderer;
pub mod tabs;
pub mod text;
