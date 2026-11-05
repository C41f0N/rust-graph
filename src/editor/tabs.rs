use crate::config;
use crate::editor::buffer;
use crate::editor::document::{self, EditableLoad, OpenClass};
use crate::editor::history;
use crate::editor::text;
use crate::ui;
use raylib::prelude::*;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::RwLock;

// Multi-file editor: a list of open documents ("tabs"), one of which is
// "active". The editor's buffer globals (BUFFER, cursor, scroll) always belong
// to the ACTIVE tab; switching saves the old tab (file becomes current) and
// loads the new one, so each open document keeps its view state and its own
// undo/redo stacks. Tabs live for the whole application run and survive the
// editor closing (switching to the graph view and back).

pub const TAB_W: i32 = 130;
pub const TABS_H: i32 = 30;
const TAB_GAP: i32 = 2;
const TAB_NAME_LEFT_PAD: i32 = 8;
const CLOSE_W: i32 = 14;
const CLOSE_RIGHT_PAD: i32 = 6;

pub struct Tab {
    pub name: String,
    pub path: PathBuf,
    pub cursor: (i32, i32),
    pub anchor: (i32, i32),
    pub scroll: i32,
    pub dirty: bool,
    pub last_edit_ms: u64,
    pub undo: VecDeque<history::HistEntry>,
    pub redo: VecDeque<history::HistEntry>,
    pub run: Option<(i32, i32, u64)>,
    pub mode: DocMode,
}

// Whether the open document is a normal, editable buffer or a bounded
// read-only preview page.
#[derive(Clone, Debug)]
pub enum DocMode {
    Editable,
    Preview(PreviewState),
}

// A preview tab stores just enough state to re-read its current page and to
// walk backwards through pages already visited. `start_byte` is always a line
// boundary, and `line_count` is how many source lines the current page holds.
#[derive(Clone, Debug)]
pub struct PreviewState {
    pub start_byte: u64,
    pub next_byte: u64,
    pub first_line: usize,
    pub line_count: usize,
    pub file_bytes: u64,
    pub eof: bool,
    pub truncated: bool,
    // (start_byte, first_line) of pages before this one.
    pub history: Vec<(u64, usize)>,
}

// The small, copyable view the renderer/input need to draw the banner and
// decide whether paging is possible. Deliberately excludes the history vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewView {
    pub first_line: usize,
    pub line_count: usize,
    pub file_bytes: u64,
    pub has_prev: bool,
    pub has_next: bool,
    pub truncated: bool,
}

static TABS: RwLock<Vec<Tab>> = RwLock::new(Vec::new());
static ACTIVE: RwLock<Option<usize>> = RwLock::new(None);

// Horizontal scroll offset (px) of the tab bar.
static TAB_SCROLL: RwLock<i32> = RwLock::new(0);

// --------------------------------------------------------------------------
// Pure helpers
// --------------------------------------------------------------------------

// Left edge of a tab relative to the bar's left edge (after scroll).
pub fn tab_left(index: usize, scroll: i32) -> i32 {
    (index as i32) * (TAB_W + TAB_GAP) - scroll
}

// Clamp a tab-bar scroll offset so the last tab's right edge and the bar's
// viewport align. Pure so input and renderer agree and tests can drive it.
pub fn clamp_scroll(scroll: i32, count: usize, view_w: i32) -> i32 {
    if count == 0 {
        return 0;
    }
    let total = (count as i32) * (TAB_W + TAB_GAP) - TAB_GAP;
    let max_scroll = (total - view_w).max(0);
    scroll.clamp(0, max_scroll)
}

// The rectangle of tab `i` in screen space.
pub fn tab_rect(index: usize, bar_x: i32, bar_y: i32, scroll: i32) -> (i32, i32, i32, i32) {
    (bar_x + tab_left(index, scroll), bar_y, TAB_W, TABS_H)
}

// The close-box rectangle within tab `i`, in screen space.
pub fn close_rect(index: usize, bar_x: i32, bar_y: i32, scroll: i32) -> (i32, i32, i32, i32) {
    let (x, _, w, _) = tab_rect(index, bar_x, bar_y, scroll);
    let cx = x + w - CLOSE_W - CLOSE_RIGHT_PAD;
    let cy = bar_y + (TABS_H - CLOSE_W) / 2;
    (cx, cy, CLOSE_W, CLOSE_W)
}

fn panel_width() -> i32 {
    crate::editor::panel_bounds().2
}

// --------------------------------------------------------------------------
// Queries
// --------------------------------------------------------------------------

pub fn has_any() -> bool {
    !TABS.read().unwrap().is_empty()
}

pub fn active_name() -> Option<String> {
    let tabs = TABS.read().unwrap();
    let idx = match *ACTIVE.read().unwrap() {
        Some(i) => i,
        None => return None,
    };
    tabs.get(idx).map(|t| t.name.clone())
}

pub fn active_path() -> Option<PathBuf> {
    let tabs = TABS.read().unwrap();
    let idx = match *ACTIVE.read().unwrap() {
        Some(i) => i,
        None => return None,
    };
    tabs.get(idx).map(|t| t.path.clone())
}

// True when the active document is a bounded read-only preview page.
pub fn active_is_preview() -> bool {
    active_preview_view().is_some()
}

// Banner/paging details for the active preview document, if any.
pub fn active_preview_view() -> Option<PreviewView> {
    let tabs = TABS.read().unwrap();
    let idx = (*ACTIVE.read().unwrap())?;
    match tabs.get(idx)?.mode {
        DocMode::Preview(ref state) => Some(PreviewView {
            first_line: state.first_line,
            line_count: state.line_count,
            file_bytes: state.file_bytes,
            has_prev: !state.history.is_empty(),
            has_next: !state.eof,
            truncated: state.truncated,
        }),
        DocMode::Editable => None,
    }
}

// Save every open tab that holds unsaved edits and clear its dirty state,
// returning the paths written so the caller can refresh their graph nodes.
// Autosave runs through here so a typing pause flushes every tab, not just the
// one on screen. The active document's text lives in the shared buffer globals
// (not in its Tab), so its live dirty flag is folded back in before the write;
// every other tab is flushed the moment it is switched away from
// (store_active_globals), which is why only the active tab is normally dirty.
pub fn save_all_dirty() -> Vec<PathBuf> {
    let active = *ACTIVE.read().unwrap();
    let mut tabs = TABS.write().unwrap();
    let mut saved = Vec::new();

    for (i, tab) in tabs.iter_mut().enumerate() {
        if matches!(tab.mode, DocMode::Preview(_)) {
            tab.dirty = false;
            tab.last_edit_ms = 0;
            continue;
        }
        if Some(i) == active {
            tab.dirty = crate::editor::DIRTY.load(std::sync::atomic::Ordering::Relaxed);
        }
        if tab.dirty && !tab.path.as_os_str().is_empty() {
            buffer::save_to_file(&tab.path);
            saved.push(tab.path.clone());
        }
        tab.dirty = false;
        tab.last_edit_ms = 0;
    }

    crate::editor::DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
    crate::editor::LAST_EDIT_MILLIS.store(0, std::sync::atomic::Ordering::Relaxed);
    saved
}

fn find_by_path(tabs: &[Tab], path: &Path) -> Option<usize> {
    tabs.iter().position(|t| t.path == path)
}

// --------------------------------------------------------------------------
// State swap
// --------------------------------------------------------------------------

// Copy the live buffer globals (which describe the ACTIVE tab) into `tab`.
// Caller holds the TABS write lock and makes sure this is the active tab.
fn store_active_globals(tab: &mut Tab) {
    tab.cursor = (*buffer::CURSOR_X.read().unwrap(), *buffer::CURSOR_Y.read().unwrap());
    tab.anchor = (*buffer::ANCHOR_X.read().unwrap(), *buffer::ANCHOR_Y.read().unwrap());
    tab.scroll = *buffer::SCROLL_Y.read().unwrap();
    tab.dirty = crate::editor::DIRTY.load(std::sync::atomic::Ordering::Relaxed);
    tab.last_edit_ms = crate::editor::LAST_EDIT_MILLIS.load(std::sync::atomic::Ordering::Relaxed);
    let (undo, redo, run) = history::take();
    tab.undo = undo;
    tab.redo = redo;
    tab.run = run;
    // The active tab's globals are authoritative again after we stop editing;
    // leaving dirty content un-saved here would be a data-loss bug, so flush.
    // Preview tabs are read-only by construction and must never write.
    if tab.dirty
        && !tab.path.as_os_str().is_empty()
        && !matches!(tab.mode, DocMode::Preview(_))
    {
        buffer::save_to_file(&tab.path);
    }
    // The tab is now clean and the globals belong to the next document: reset
    // them so nothing stale triggers an autosave against the wrong file.
    tab.dirty = false;
    tab.last_edit_ms = 0;
    crate::editor::DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
    crate::editor::LAST_EDIT_MILLIS.store(0, std::sync::atomic::Ordering::Relaxed);
}

// Push the stored state of `tab` back into the live globals, making it the
// active document. Caller holds no locks.
fn load_active_globals(tab: &mut Tab, preloaded: Option<Vec<String>>) {
    let path = tab.path.clone();
    let cursor = std::mem::replace(&mut tab.cursor, (-1, -1));
    let anchor = std::mem::replace(&mut tab.anchor, (-1, -1));
    let scroll = std::mem::replace(&mut tab.scroll, 0);
    let dirty = std::mem::replace(&mut tab.dirty, false);
    let last_edit_ms = std::mem::replace(&mut tab.last_edit_ms, 0);
    let undo = std::mem::replace(&mut tab.undo, VecDeque::new());
    let redo = std::mem::replace(&mut tab.redo, VecDeque::new());
    let run = tab.run;

    match &mut tab.mode {
        DocMode::Editable => match preloaded {
            Some(lines) => buffer::load_lines(lines),
            None => buffer::load_from_file(&path),
        },
        DocMode::Preview(state) => {
            match document::read_chunk(&path, state.start_byte) {
                Ok(chunk) => {
                    state.line_count = chunk.lines.len();
                    state.next_byte = chunk.next_byte;
                    state.eof = chunk.eof;
                    state.truncated = chunk.truncated;
                    buffer::load_preview_lines(chunk.lines, false);
                }
                Err(error) => {
                    crate::notice::push("Cannot reload preview", error.describe(&path));
                    state.line_count = 0;
                    state.eof = true;
                    state.truncated = false;
                    buffer::load_preview_lines(Vec::new(), false);
                }
            }
        }
    }
    // A brand-new tab carries the (-1,-1) sentinel: keep the end-of-file
    // caret that load_from_file just placed instead of overwriting it with
    // zeroes. Every visited tab restores its stored position verbatim, but a
    // preview page or an externally edited file can be shorter than when the
    // position was saved, so clamp both ends to the live buffer before the
    // input handler indexes with them.
    {
        let buffer = buffer::BUFFER.read().unwrap();
        let last_line = buffer.len().saturating_sub(1) as i32;
        let mut cursor_x = buffer::CURSOR_X.write().unwrap();
        let mut cursor_y = buffer::CURSOR_Y.write().unwrap();
        if cursor != (-1, -1) {
            *cursor_x = cursor.0;
            *cursor_y = cursor.1;
        }
        *cursor_y = (*cursor_y).clamp(0, last_line);
        *cursor_x = (*cursor_x).clamp(0, buffer[*cursor_y as usize].len() as i32);

        let mut anchor_x = buffer::ANCHOR_X.write().unwrap();
        let mut anchor_y = buffer::ANCHOR_Y.write().unwrap();
        if cursor != (-1, -1) {
            *anchor_x = anchor.0;
            *anchor_y = anchor.1;
        }
        *anchor_y = (*anchor_y).clamp(0, last_line);
        *anchor_x = (*anchor_x).clamp(0, buffer[*anchor_y as usize].len() as i32);
    }
    *buffer::SCROLL_Y.write().unwrap() = scroll;
    crate::editor::DIRTY.store(dirty, std::sync::atomic::Ordering::Relaxed);
    crate::editor::LAST_EDIT_MILLIS.store(last_edit_ms, std::sync::atomic::Ordering::Relaxed);
    history::put(undo, redo, run);
}

// Make tab `idx` the active document. Saves the old tab first if it was
// edited, preserves per-tab view + undo state. `view_w` is the tab bar width,
// used to keep the freshly activated tab visible.
pub fn activate(idx: usize, view_w: i32) {
    activate_with(idx, view_w, None);
}

// Activation with an optional already-read document. Used by `open` so a file
// that was read during preflight is not read a second time.
fn activate_with(idx: usize, view_w: i32, preloaded: Option<Vec<String>>) {
    let mut tabs = TABS.write().unwrap();
    if idx >= tabs.len() {
        return;
    }
    let prev = *ACTIVE.read().unwrap();
    if let Some(p) = prev {
        if p != idx {
            store_active_globals(&mut tabs[p]);
        }
    }
    // Take the target's state out (its history stacks in particular move
    // through the transition), then restore globals from it.
    {
        let t = &mut tabs[idx];
        load_active_globals(t, preloaded);
    }
    *ACTIVE.write().unwrap() = Some(idx);
    drop(tabs);
    reveal_tab(idx, view_w);
}

// Step the active preview document one page forward (+1) or backward (-1).
// Reads the target page from disk first, so a failed read leaves the current
// page untouched. Called from main after the editor input lock is released.
pub fn preview_step(delta: i32) -> bool {
    if delta == 0 {
        return false;
    }
    let mut tabs = TABS.write().unwrap();
    let Some(idx) = *ACTIVE.read().unwrap() else {
        return false;
    };
    if idx >= tabs.len() {
        return false;
    }
    let path = tabs[idx].path.clone();
    let (target_byte, target_line) = match &tabs[idx].mode {
        DocMode::Preview(state) => {
            if delta > 0 {
                if state.eof {
                    return false;
                }
                (state.next_byte, state.first_line + state.line_count)
            } else {
                match state.history.last().copied() {
                    Some(target) => target,
                    None => return false,
                }
            }
        }
        DocMode::Editable => return false,
    };

    let chunk = match document::read_chunk(&path, target_byte) {
        Ok(chunk) => chunk,
        Err(error) => {
            crate::notice::push("Cannot page preview", error.describe(&path));
            return false;
        }
    };
    let line_count = chunk.lines.len();
    let anchor_end = delta < 0;

    if let DocMode::Preview(state) = &mut tabs[idx].mode {
        if delta > 0 {
            state.history.push((state.start_byte, state.first_line));
        } else {
            state.history.pop();
        }
        state.start_byte = target_byte;
        state.first_line = target_line;
        state.line_count = line_count;
        state.next_byte = chunk.next_byte;
        state.eof = chunk.eof;
        state.truncated = chunk.truncated;
    }

    buffer::load_preview_lines(chunk.lines, anchor_end);
    {
        let buffer = buffer::BUFFER.read().unwrap();
        let last = buffer.len().saturating_sub(1);
        let (cursor_x, cursor_y) = if anchor_end {
            (buffer[last].len() as i32, last as i32)
        } else {
            (0, 0)
        };
        let tab = &mut tabs[idx];
        tab.cursor = (cursor_x, cursor_y);
        tab.anchor = (cursor_x, cursor_y);
        tab.scroll = if anchor_end { i32::MAX } else { 0 };
    }
    true
}

// Open a document as a tab (deduplicated by path), activating it. Files above
// the editable budget are opened as read-only preview pages; files above the
// preview budget are refused before anything is read. A refusal never creates
// a tab or disturbs the active document.
pub fn open(path: &Path, name: &str) -> Result<usize, document::OpenError> {
    {
        let tabs = TABS.read().unwrap();
        if let Some(existing) = find_by_path(&tabs, path) {
            drop(tabs);
            activate(existing, panel_width());
            return Ok(existing);
        }
    }

    let class = document::classify(path).map_err(|error| report_open_error(path, error))?;
    let mut preloaded = None;
    let mode = match class {
        OpenClass::Editable { bytes } => match document::read_editable(path)
            .map_err(|error| report_open_error(path, error))?
        {
            EditableLoad::Lines(lines) => {
                preloaded = Some(lines);
                DocMode::Editable
            }
            EditableLoad::NeedsPreview => preview_mode(path, bytes)?,
        },
        OpenClass::Preview { bytes } => preview_mode(path, bytes)?,
    };

    let idx = {
        let mut tabs = TABS.write().unwrap();
        // cursor/anchor (-1,-1) mark "never positioned": load_active_globals
        // keeps the end-of-file caret placed by load_from_file instead of
        // clamping a fresh document to (0,0).
        tabs.push(Tab {
            name: name.to_string(),
            path: path.to_path_buf(),
            cursor: (-1, -1),
            anchor: (-1, -1),
            scroll: 0,
            dirty: false,
            last_edit_ms: 0,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            run: None,
            mode,
        });
        tabs.len() - 1
    };
    activate_with(idx, panel_width(), preloaded);
    Ok(idx)
}

fn report_open_error(path: &Path, error: document::OpenError) -> document::OpenError {
    crate::notice::push("Cannot open note", error.describe(path));
    error
}

// Read the first preview page and build the tab's mode. The page is not kept
// here: activation reads it again through the same bounded path, which keeps
// the policy in one place and is cheap for a 128 KiB page.
fn preview_mode(path: &Path, file_bytes: u64) -> Result<DocMode, document::OpenError> {
    let chunk = document::read_chunk(path, 0).map_err(|error| report_open_error(path, error))?;
    Ok(DocMode::Preview(PreviewState {
        start_byte: 0,
        next_byte: chunk.next_byte,
        first_line: 0,
        line_count: chunk.lines.len(),
        file_bytes,
        eof: chunk.eof,
        truncated: chunk.truncated,
        history: Vec::new(),
    }))
}

fn reveal_tab(idx: usize, view_w: i32) {
    let mut scroll = TAB_SCROLL.write().unwrap();
    let count = TABS.read().unwrap().len();
    let left = tab_left(idx, *scroll);
    let right = left + TAB_W;
    if left < 0 {
        *scroll = clamp_scroll(*scroll + left, count, view_w);
    } else if right > view_w.max(0) {
        *scroll = clamp_scroll(*scroll + (right - view_w), count, view_w);
    } else {
        *scroll = clamp_scroll(*scroll, count, view_w);
    }
}

// Close tab `idx`, saving its unsaved edits first. Activating a neighbour
// (rightmost-preferred) if the active tab was closed. Returns true when the
// last tab was closed (editor should close).
pub fn close(idx: usize) -> bool {
    let mut tabs = TABS.write().unwrap();
    if idx >= tabs.len() {
        return false;
    }
    let active = *ACTIVE.read().unwrap();
    let was_active = active == Some(idx);

    if was_active {
        store_active_globals(&mut tabs[idx]);
    } else if let Some(a) = active {
        if a > idx {
            *ACTIVE.write().unwrap() = Some(a - 1);
        }
    }

    tabs.remove(idx);

    if was_active {
        if tabs.is_empty() {
            *ACTIVE.write().unwrap() = None;
            return true;
        }
        let next = idx.min(tabs.len() - 1);
        *ACTIVE.write().unwrap() = Some(next);
        let width = panel_width();
        {
            let t = &mut tabs[next];
            load_active_globals(t, None);
        }
        drop(tabs);
        reveal_tab(next, width);
        return false;
    }
    false
}

// Close whichever tab is active. Returns true when the last tab was closed
// (editor should close), mirroring `close`.
pub fn close_active() -> bool {
    let active = *ACTIVE.read().unwrap();
    match active {
        Some(idx) => close(idx),
        None => false,
    }
}

// Save + unload the active tab's working state without switching (editor
// closing). The tab list stays intact and ACTIVE keeps pointing at the same
// tab, so reopening the editor resumes exactly where it left off: the buffer
// globals still describe it and autosave/Ctrl+S keep working.
pub fn deactivate_current() {
    let mut tabs = TABS.write().unwrap();
    if let Some(idx) = *ACTIVE.read().unwrap() {
        if idx < tabs.len() {
            store_active_globals(&mut tabs[idx]);
        }
    }
}

// Rename the active tab's note: flush any unsaved edits, rename the .md file
// (and companion sub-graph folder) through processing::rename_node, which also
// rewrites every [[old]] reference elsewhere, then rebind the tab to the new
// path. Returns false when the name is empty/unchanged or the rename failed.
pub fn rename_active(new_name: &str) -> bool {
    // A preview page is read-only: its title must not start a rename.
    if active_is_preview() {
        return false;
    }
    let Some(path) = active_path() else {
        return false;
    };
    let old_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let clean = new_name.trim().trim_end_matches(".md").to_string();
    if clean.is_empty() || clean == old_stem {
        return false;
    }

    // The node index is found by path so the editor never has to know how the
    // graph bookkeeps; missing node (e.g. orphan tab) just bails.
    let idx = {
        let nodes = crate::graph::processing::NODES.read().unwrap();
        nodes.iter().position(|n| n.path == path)
    };
    let Some(idx) = idx else {
        return false;
    };

    // Flush in-flight edits to the old path before the file moves under it.
    if crate::editor::DIRTY.load(std::sync::atomic::Ordering::Relaxed) {
        buffer::save_to_file(&path);
    }
    if !crate::graph::processing::rename_node(idx, &clean) {
        return false;
    }

    let new_path = path.with_file_name(format!("{clean}.md"));
    {
        let mut tabs = TABS.write().unwrap();
        if let Some(p) = *ACTIVE.read().unwrap() {
            if let Some(t) = tabs.get_mut(p) {
                t.name = clean.clone();
                t.path = new_path;
            }
        }
    }
    crate::editor::DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
    crate::editor::LAST_EDIT_MILLIS.store(0, std::sync::atomic::Ordering::Relaxed);
    true
}

// Create a new note file (unique name) and open it as the active tab. Used by
// the placeholder screen; mirrors the graph's add-node flow so the new note
// also appears as a graph node. Returns None only if the freshly created file
// could not be opened (open already reports the refusal).
pub fn create_and_open(dir: &Path, stem: &str) -> Option<usize> {
    let stem = if stem.trim().is_empty() {
        "untitled".to_string()
    } else {
        stem.trim().to_string()
    };
    let node_idx = crate::graph::processing::add_node(dir, &stem);
    let (path, name) = {
        let nodes = crate::graph::processing::NODES.read().unwrap();
        let node = &nodes[node_idx];
        (node.path.clone(), node.name.clone())
    };
    let tab = open(&path, &name).ok()?;
    // Creating a note is a writing intent, not "opening to read": the new
    // blank document starts with a live caret so the user can type at once,
    // unlike a freshly loaded existing note which lands in nav mode.
    crate::editor::NAV_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
    crate::graph::processing::rebuild_edges();
    Some(tab)
}

// --------------------------------------------------------------------------
// Input
// --------------------------------------------------------------------------

// Handle a left press over the tab bar: close-box click closes that tab,
// otherwise click activates. Returns true when the press was consumed by the
// bar.
pub fn handle_bar_click(rl: &mut RaylibHandle, bar_x: i32, bar_y: i32, bar_w: i32) -> bool {
    if !rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        return false;
    }
    let m = rl.get_mouse_position();
    let mx = m.x as i32;
    let my = m.y as i32;
    if !(my >= bar_y && my < bar_y + TABS_H && mx >= bar_x && mx <= bar_x + bar_w) {
        return false;
    }
    let count = TABS.read().unwrap().len();
    if count == 0 {
        return true;
    }
    let scroll = *TAB_SCROLL.read().unwrap();

    // Close-boxes first: they take precedence over activating the tab.
    for i in 0..count {
        let (cx, cy, cw, ch) = close_rect(i, bar_x, bar_y, scroll);
        if m.x as i32 >= cx
            && m.x as i32 <= cx + cw
            && m.y as i32 >= cy
            && m.y as i32 <= cy + ch
        {
            if close(i) {
                crate::editor::CLOSE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            return true;
        }
    }

    for i in 0..count {
        let (x, _, w, _) = tab_rect(i, bar_x, bar_y, scroll);
        if x + w <= bar_x || x >= bar_x + bar_w {
            continue;
        }
        if m.x as i32 >= x && m.x as i32 <= x + w {
            activate(i, bar_w);
            break;
        }
    }
    true
}

// Scroll the tab bar horizontally with the wheel while the cursor is over it.
pub fn handle_bar_wheel(rl: &mut RaylibHandle, bar_x: i32, bar_y: i32, bar_w: i32) -> bool {
    let wheel = rl.get_mouse_wheel_move();
    if wheel == 0.0 {
        return false;
    }
    let m = rl.get_mouse_position();
    let mx = m.x as i32;
    let my = m.y as i32;
    if !(my >= bar_y && my < bar_y + TABS_H && mx >= bar_x && mx <= bar_x + bar_w) {
        return false;
    }
    let count = TABS.read().unwrap().len();
    let mut scroll = TAB_SCROLL.write().unwrap();
    *scroll = clamp_scroll(
        *scroll - (wheel as i32) * config::TAB_SCROLL_STEP,
        count,
        bar_w,
    );
    true
}

// --------------------------------------------------------------------------
// Rendering
// --------------------------------------------------------------------------

pub fn draw_tab_bar(d: &mut RaylibDrawHandle, bar_x: i32, bar_y: i32, bar_w: i32) {
    let tabs = TABS.read().unwrap();
    let count = tabs.len();
    if count == 0 {
        return;
    }
    let active = *ACTIVE.read().unwrap();
    let scroll = *TAB_SCROLL.read().unwrap();

    // Bar background + bottom separator, then tabs clipped to the row so the
    // horizontally scrolled content never bleeds outside it.
    d.draw_rectangle(bar_x, bar_y, bar_w, TABS_H, ui::PANEL_SOLID);
    d.draw_line(bar_x, bar_y + TABS_H, bar_x + bar_w, bar_y + TABS_H, ui::BORDER);

    let mouse = d.get_mouse_position();
    d.draw_scissor_mode(bar_x, bar_y, bar_w, TABS_H, |mut s| {
        for (i, t) in tabs.iter().enumerate() {
            let (x, y, w, h) = tab_rect(i, bar_x, bar_y, scroll);
            if x + w <= bar_x || x >= bar_x + bar_w {
                continue;
            }
            let is_active = active == Some(i);
            let over = mouse.x as i32 >= x && mouse.x as i32 <= x + w;
            let over_close = {
                let (cx, cy, cw, ch) = close_rect(i, bar_x, bar_y, scroll);
                mouse.x as i32 >= cx
                    && mouse.x as i32 <= cx + cw
                    && mouse.y as i32 >= cy
                    && mouse.y as i32 <= cy + ch
            };

            // Background tint: a rounded pill on the active tab and a faint
            // wash on hover, so the strip reads as chrome rather than a row of
            // boxes.
            let bg = if is_active {
                ui::SURFACE_RAISED
            } else if over {
                ui::SURFACE_HOVER
            } else {
                Color::new(0, 0, 0, 0)
            };
            if bg.a != 0 {
                s.draw_rectangle_rounded(
                    Rectangle::new(
                        (x + 1) as f32,
                        (y + 2) as f32,
                        (w - 2) as f32,
                        (h - 4) as f32,
                    ),
                    0.24,
                    6,
                    bg,
                );
            }
            if is_active {
                // Bottom accent bar marks the open document.
                s.draw_rectangle(x + 7, y + h - 2, w - 15, 2, ui::ACCENT);
            }

            // Name, pushed left by the close button room. A dated journal tab's
            // title renders as "19th Aug 2025"; the stored name stays the file
            // stem so activation/saving keeps working on the raw path.
            let display = crate::journal::pretty_name(&t.name);
            let tw = text::measure(&s, &display, 14);
            let name_x = x + TAB_NAME_LEFT_PAD + if t.dirty { 10 } else { 0 };
            let max_name = (w - CLOSE_W - CLOSE_RIGHT_PAD - TAB_NAME_LEFT_PAD * 2).max(8);
            let shown = if tw > max_name {
                // Truncate with a trailing ellipsis.
                let mut cut = display.clone();
                loop {
                    cut.pop();
                    if text::measure(&s, &format!("{}\u{2026}", cut), 14) <= max_name || cut.is_empty() {
                        break;
                    }
                }
                format!("{}\u{2026}", cut)
            } else {
                display
            };
            let color = if is_active {
                ui::TEXT
            } else {
                ui::TEXT_SECONDARY
            };
            text::draw(&mut s, &shown, name_x, y + (h - 14) / 2, 14, color);

            // Dirty dot: a small filled disc left of the name.
            if t.dirty {
                s.draw_circle_v(
                    Vector2::new((x + TAB_NAME_LEFT_PAD) as f32, (y + h / 2) as f32),
                    2.5,
                    ui::ACCENT,
                );
            }

            // Close glyph: two crossing lines, drawn when hovered (or always
            // on the active tab so its close affordance is discoverable).
            if over_close || is_active {
                let (cx, cy, _, _) = close_rect(i, bar_x, bar_y, scroll);
                let inset = 4;
                let gc = if over_close { ui::TEXT } else { ui::TEXT_MUTED };
                s.draw_line(cx + inset, cy + inset, cx + CLOSE_W - inset, cy + CLOSE_W - inset, gc);
                s.draw_line(cx + CLOSE_W - inset, cy + inset, cx + inset, cy + CLOSE_W - inset, gc);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        *TABS.write().unwrap() = Vec::new();
        *ACTIVE.write().unwrap() = None;
        *TAB_SCROLL.write().unwrap() = 0;
    }

    fn tab(path: &str) -> Tab {
        Tab {
            name: path.to_string(),
            path: PathBuf::from(path),
            cursor: (0, 0),
            anchor: (0, 0),
            scroll: 0,
            dirty: false,
            last_edit_ms: 0,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            run: None,
            mode: DocMode::Editable,
        }
    }

    #[test]
    fn clamp_scroll_tracks_total_width() {
        // Fewer tabs than fit the viewport clamp to 0.
        assert_eq!(clamp_scroll(500, 2, 600), 0);
        // Wrapping: last tab's right edge aligns with the bar's right edge.
        let count = 10;
        let view_w = 500;
        let total = (count as i32) * (TAB_W + TAB_GAP) - TAB_GAP;
        assert_eq!(clamp_scroll(9999, count, view_w), total - view_w);
        assert_eq!(clamp_scroll(-50, count, view_w), 0);
        assert_eq!(clamp_scroll(0, 0, 100), 0);
    }

    #[test]
    fn tab_layout_is_contiguous_left_to_right() {
        for i in 0..5 {
            let left = tab_left(i, 0);
            assert_eq!(left, (i as i32) * (TAB_W + TAB_GAP));
        }
        // Scrolling shifts every tab by the same amount.
        assert_eq!(tab_left(3, 100), 3 * (TAB_W + TAB_GAP) - 100);
    }

    #[test]
    fn open_dedupes_by_path() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        let a = open(Path::new("/x/a.md"), "a").unwrap();
        let b = open(Path::new("/x/b.md"), "b").unwrap();
        let a2 = open(Path::new("/x/a.md"), "a").unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(a2, a);
        assert_eq!(TABS.read().unwrap().len(), 2);
    }

    #[test]
    fn close_shifts_active_index_down() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        *ACTIVE.write().unwrap() = Some(2);
        *TABS.write().unwrap() = vec![tab("/a"), tab("/b"), tab("/c"), tab("/d")];
        // Closing a tab below the active one leaves the active index alone.
        close(0);
        assert_eq!(*ACTIVE.read().unwrap(), Some(1));
        // Closing a tab above the active one shifts it down.
        close(1);
        // The tab that slid into the closed slot becomes active.
        assert_eq!(*ACTIVE.read().unwrap(), Some(1));
    }

    #[test]
    fn close_last_tab_reports_empty() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        *ACTIVE.write().unwrap() = Some(0);
        *TABS.write().unwrap() = vec![tab("/a")];
        assert!(close(0));
        assert_eq!(*ACTIVE.read().unwrap(), None);
        assert!(!has_any());
    }

    #[test]
    fn close_active_closes_the_active_tab() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        *ACTIVE.write().unwrap() = Some(1);
        *TABS.write().unwrap() = vec![tab("/a"), tab("/b"), tab("/c")];

        assert!(!close_active());
        assert_eq!(TABS.read().unwrap().len(), 2);
        assert_eq!(*ACTIVE.read().unwrap(), Some(1));
        assert_eq!(active_path().as_deref(), Some(Path::new("/c")));

        // No active tab: nothing to close.
        *ACTIVE.write().unwrap() = None;
        assert!(!close_active());
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rg_tabs_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_preview_file(path: &Path, lines: usize, line_len: usize) {
        let mut content = String::new();
        for i in 0..lines {
            content.push_str(&format!("{i:04} "));
            content.push_str(&"x".repeat(line_len.saturating_sub(5)));
            content.push('\n');
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn refused_file_adds_no_tab_and_keeps_active() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        let _notice_guard = crate::notice::TEST_LOCK.lock().unwrap();
        reset();
        let root = temp_root("refused");
        let small = root.join("small.md");
        std::fs::write(&small, "hello\n").unwrap();
        let huge = root.join("huge.md");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(config::EDITOR_MAX_PREVIEW_BYTES + 1)
            .unwrap();

        let small_idx = open(&small, "small").unwrap();
        assert_eq!(small_idx, 0);
        let result = open(&huge, "huge");
        assert!(matches!(result, Err(document::OpenError::TooLarge { .. })));
        assert_eq!(TABS.read().unwrap().len(), 1);
        assert_eq!(*ACTIVE.read().unwrap(), Some(0));
        assert_eq!(active_path().as_deref(), Some(small.as_path()));
        assert_eq!(buffer::BUFFER.read().unwrap().join("\n"), "hello\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn invalid_utf8_is_refused_without_a_tab() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        let _notice_guard = crate::notice::TEST_LOCK.lock().unwrap();
        reset();
        let root = temp_root("invalid_utf8");
        let path = root.join("bad.md");
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(matches!(
            open(&path, "bad"),
            Err(document::OpenError::NotText { .. })
        ));
        assert!(!has_any());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overlong_line_in_small_file_downgrades_to_preview() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        let root = temp_root("long_line");
        let path = root.join("one-line.md");
        std::fs::write(&path, "x".repeat(config::EDITOR_MAX_EDIT_LINE_BYTES + 100)).unwrap();
        open(&path, "one-line").unwrap();
        assert!(active_is_preview());
        let view = active_preview_view().unwrap();
        assert_eq!(view.first_line, 0);
        assert!(view.truncated);
        assert!(!view.has_next);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preview_page_error_leaves_current_page_intact() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        let _notice_guard = crate::notice::TEST_LOCK.lock().unwrap();
        reset();
        let root = temp_root("preview_bad_chunk");
        let path = root.join("bad-chunk.md");

        // ~600 KiB of lines makes the first page valid UTF-8, while the
        // invalid line at index 150 lands in the second page. Paging must fail
        // without replacing the page already on screen.
        let mut bytes = Vec::new();
        for i in 0..600 {
            if i == 150 {
                bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
            } else {
                bytes.extend_from_slice(format!("{i:04} ").as_bytes());
                bytes.extend_from_slice(&[b'x'; 995]);
                bytes.push(b'\n');
            }
        }
        std::fs::write(&path, &bytes).unwrap();

        open(&path, "bad-chunk").unwrap();
        assert!(active_is_preview());
        let first_buffer_line = buffer::BUFFER.read().unwrap()[0].clone();
        assert!(!preview_step(1));
        let view = active_preview_view().unwrap();
        assert_eq!(view.first_line, 0);
        assert_eq!(buffer::BUFFER.read().unwrap()[0], first_buffer_line);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn large_file_opens_as_preview_and_never_saves() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        let root = temp_root("preview_save");
        let path = root.join("big.md");
        write_preview_file(&path, 1_200, 600);
        let original = std::fs::read(&path).unwrap();

        open(&path, "big").unwrap();
        assert!(active_is_preview());
        assert!(matches!(TABS.read().unwrap()[0].mode, DocMode::Preview(_)));
        let view = active_preview_view().unwrap();
        assert_eq!(view.first_line, 0);
        assert!(view.line_count > 0);
        assert!(buffer::BUFFER.read().unwrap().len() < 600);

        // A preview is read-only even if a stray dirty flag is raised: the
        // buffer only contains one page, so a write here would truncate the
        // file. save_all_dirty must skip it entirely.
        crate::editor::DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
        TABS.write().unwrap()[0].dirty = true;
        let saved = save_all_dirty();
        assert!(saved.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), original);

        // Switching away stores the preview's page state; store_active_globals
        // must likewise not flush the chunk to disk.
        TABS.write().unwrap()[0].dirty = true;
        crate::editor::DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
        let small = root.join("small.md");
        std::fs::write(&small, "small\n").unwrap();
        open(&small, "small").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!active_is_preview());
        activate(0, 500);
        assert!(active_is_preview());
        assert!(!rename_active("changed"), "preview title is read-only");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preview_steps_both_ways_and_restores_page_on_tab_switch() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        let root = temp_root("preview_paging");
        let path = root.join("big.md");
        write_preview_file(&path, 1_200, 600);

        let preview_idx = open(&path, "big").unwrap();
        assert!(preview_step(1), "first page has a successor");
        let page1 = active_preview_view().unwrap();
        assert!(page1.first_line > 0);
        assert!(page1.has_prev);
        let page1_first_buffer_line = buffer::BUFFER.read().unwrap()[0].clone();

        assert!(preview_step(-1), "back to page 0");
        let page0 = active_preview_view().unwrap();
        assert_eq!(page0.first_line, 0);
        assert!(!page0.has_prev);

        assert!(preview_step(1));
        assert_eq!(active_preview_view().unwrap().first_line, page1.first_line);
        *buffer::SCROLL_Y.write().unwrap() = 42;

        let small = root.join("small.md");
        std::fs::write(&small, "small\n").unwrap();
        open(&small, "small").unwrap();
        activate(preview_idx, 500);
        assert!(active_is_preview());
        let restored = active_preview_view().unwrap();
        assert_eq!(restored.first_line, page1.first_line);
        assert_eq!(buffer::BUFFER.read().unwrap()[0], page1_first_buffer_line);
        assert_eq!(*buffer::SCROLL_Y.read().unwrap(), 42);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_active_updates_tab_node_and_file() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        reset();
        let root = std::env::temp_dir().join("rg_tab_rename_test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        crate::graph::processing::DIR_PATH
            .write()
            .unwrap()
            .clone_from(&root);

        let idx = crate::graph::processing::add_node(&root, "alpha");
        let (path, name) = {
            let nodes = crate::graph::processing::NODES.read().unwrap();
            let n = &nodes[idx];
            (n.path.clone(), n.name.clone())
        };
        crate::editor::tabs::open(&path, &name).unwrap();
        assert_eq!(active_name().as_deref(), Some("alpha"));

        assert!(rename_active("beta"));
        {
            let tabs = TABS.read().unwrap();
            let t = &tabs[0];
            assert_eq!(t.name, "beta");
            assert_eq!(t.path, root.join("beta.md"));
        }
        assert!(root.join("beta.md").exists());
        assert!(!root.join("alpha.md").exists());
        let nodes = crate::graph::processing::NODES.read().unwrap();
        let n = &nodes[idx];
        assert_eq!(n.name, "beta");
        assert_eq!(n.path, root.join("beta.md"));
        drop(nodes);

        assert!(!rename_active(" "), "blank name rejected");
        assert!(!rename_active("beta"), "unchanged name rejected");

        let _ = std::fs::remove_dir_all(&root);
    }
}