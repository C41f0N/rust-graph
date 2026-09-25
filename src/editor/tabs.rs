use crate::config;
use crate::editor::buffer;
use crate::editor::history;
use crate::editor::text;
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
    if tab.dirty && !tab.path.as_os_str().is_empty() {
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
fn load_active_globals(tab: &mut Tab) {
    let path = tab.path.clone();
    let cursor = std::mem::replace(&mut tab.cursor, (-1, -1));
    let anchor = std::mem::replace(&mut tab.anchor, (-1, -1));
    let scroll = std::mem::replace(&mut tab.scroll, 0);
    let dirty = std::mem::replace(&mut tab.dirty, false);
    let last_edit_ms = std::mem::replace(&mut tab.last_edit_ms, 0);
    let undo = std::mem::replace(&mut tab.undo, VecDeque::new());
    let redo = std::mem::replace(&mut tab.redo, VecDeque::new());
    let run = tab.run;

    buffer::load_from_file(&path);
    // A brand-new tab carries the (-1,-1) sentinel: keep the end-of-file
    // caret that load_from_file just placed instead of overwriting it with
    // zeroes. Every visited tab restores its stored position verbatim.
    if cursor != (-1, -1) {
        *buffer::CURSOR_X.write().unwrap() = cursor.0;
        *buffer::CURSOR_Y.write().unwrap() = cursor.1;
        *buffer::ANCHOR_X.write().unwrap() = anchor.0;
        *buffer::ANCHOR_Y.write().unwrap() = anchor.1;
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
        load_active_globals(t);
    }
    *ACTIVE.write().unwrap() = Some(idx);
    drop(tabs);
    reveal_tab(idx, view_w);
}

// Open a document as a tab (deduplicated by path), activating it.
pub fn open(path: &Path, name: &str) -> usize {
    let idx = {
        let mut tabs = TABS.write().unwrap();
        if let Some(existing) = find_by_path(&tabs, path) {
            existing
        } else {
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
            });
            tabs.len() - 1
        }
    };
    activate(idx, panel_width());
    idx
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
            load_active_globals(t);
        }
        drop(tabs);
        reveal_tab(next, width);
        return false;
    }
    false
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
// also appears as a graph node.
pub fn create_and_open(dir: &Path, stem: &str) -> usize {
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
    let tab = open(&path, &name);
    crate::graph::processing::rebuild_edges();
    tab
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
    d.draw_rectangle(bar_x, bar_y, bar_w, TABS_H, Color::new(10, 10, 12, 255));
    d.draw_line(bar_x, bar_y + TABS_H, bar_x + bar_w, bar_y + TABS_H, Color::new(255, 255, 255, 40));

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

            // Background tint: strongest on the active tab, faintest on a
            // plain hover. Black/white theme keeps shapes, not color.
            let bg = if is_active {
                Color::new(255, 255, 255, 10)
            } else if over {
                Color::new(255, 255, 255, 5)
            } else {
                Color::new(0, 0, 0, 0)
            };
            if bg.a != 0 {
                s.draw_rectangle(x, y, w - 1, h, bg);
            }
            if is_active {
                // Bottom accent bar marks the open document.
                s.draw_rectangle(x, y + h - 2, w - 1, 2, Color::new(255, 255, 255, 215));
            }

            // Name, pushed left by the close button room.
            let tw = text::measure(&s, &t.name, 14);
            let name_x = x + TAB_NAME_LEFT_PAD + if t.dirty { 10 } else { 0 };
            let max_name = (w - CLOSE_W - CLOSE_RIGHT_PAD - TAB_NAME_LEFT_PAD * 2).max(8);
            let shown = if tw > max_name {
                // Truncate with a trailing ellipsis.
                let mut cut = t.name.clone();
                loop {
                    cut.pop();
                    if text::measure(&s, &format!("{}\u{2026}", cut), 14) <= max_name || cut.is_empty() {
                        break;
                    }
                }
                format!("{}\u{2026}", cut)
            } else {
                t.name.clone()
            };
            let color = if is_active {
                Color::WHITE
            } else {
                Color::new(255, 255, 255, 140)
            };
            text::draw(&mut s, &shown, name_x, y + (h - 14) / 2, 14, color);

            // Dirty dot: a small filled disc left of the name.
            if t.dirty {
                s.draw_circle_v(
                    Vector2::new((x + TAB_NAME_LEFT_PAD) as f32, (y + h / 2) as f32),
                    2.5,
                    Color::new(255, 255, 255, 180),
                );
            }

            // Close glyph: two crossing lines, drawn when hovered (or always
            // on the active tab so its close affordance is discoverable).
            if over_close || is_active {
                let (cx, cy, _, _) = close_rect(i, bar_x, bar_y, scroll);
                let inset = 4;
                let gc = if over_close { Color::WHITE } else { Color::new(255, 255, 255, 100) };
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
        reset();
        let a = open(Path::new("/x/a.md"), "a");
        let b = open(Path::new("/x/b.md"), "b");
        let a2 = open(Path::new("/x/a.md"), "a");
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(a2, a);
        assert_eq!(TABS.read().unwrap().len(), 2);
    }

    #[test]
    fn close_shifts_active_index_down() {
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
        reset();
        *ACTIVE.write().unwrap() = Some(0);
        *TABS.write().unwrap() = vec![tab("/a")];
        assert!(close(0));
        assert_eq!(*ACTIVE.read().unwrap(), None);
        assert!(!has_any());
    }

    #[test]
    fn rename_active_updates_tab_node_and_file() {
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
        crate::editor::tabs::open(&path, &name);
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