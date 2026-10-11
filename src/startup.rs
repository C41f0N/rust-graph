// First-launch picker: when the app starts without a folder (or the folder it
// was handed does not exist), the main loop shows this standalone screen
// instead of the graph. It offers either browsing to an existing notes folder
// or creating a brand-new one seeded with a handful of tutorial notes that
// link to each other, so the graph is alive the moment it loads.
//
// While VAULT_READY is false the rest of the app (graph/editor/sidebar/palette
// input and rendering) is gated off in main.rs; choosing a folder calls
// set_vault(), which installs DIR_PATH, regenerates nodes and flips the flag,
// and the normal loop takes over next frame.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use raylib::prelude::*;

use crate::config;
use crate::editor::text;

pub static VAULT_READY: AtomicBool = AtomicBool::new(false);

const BG: Color = Color::new(18, 18, 22, 255);
const PANEL_BG: Color = Color::new(28, 28, 34, 250);
const PANEL_EDGE: Color = Color::new(255, 255, 255, 40);
const BTN_BG: Color = Color::new(52, 56, 68, 255);
const BTN_HOVER: Color = Color::new(76, 128, 204, 110);
const BTN_TEXT: Color = Color::new(233, 236, 243, 255);
const TITLE_COLOR: Color = Color::new(245, 246, 250, 255);
const SUBTITLE_COLOR: Color = Color::new(168, 173, 186, 255);
const ROW_HOVER: Color = Color::new(255, 255, 255, 22);
const CARET: Color = Color::WHITE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Choose,
    Browse,
    Create,
}

static SCREEN: RwLock<Screen> = RwLock::new(Screen::Choose);
// Browser position: entries are re-listed whenever the directory changes.
static BROWSE_DIR: RwLock<PathBuf> = RwLock::new(PathBuf::new());
static BROWSE_ENTRIES: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());
static BROWSE_SCROLL: RwLock<i32> = RwLock::new(0);
static CREATE_NAME: RwLock<String> = RwLock::new(String::new());

// Prime the first-launch screen before the loop starts (defaults only; the
// static initializers already cover every other field).
pub fn init() {
    *BROWSE_DIR.write().unwrap() = home_dir();
    refresh_entries();
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

// Everything that can be hit-test failures on stale state: re-list the current
// browse directory under its lock so draw and input always agree.
fn refresh_entries() {
    let dir = BROWSE_DIR.read().unwrap().clone();
    *BROWSE_ENTRIES.write().unwrap() = list_subdirs(&dir);
    *BROWSE_SCROLL.write().unwrap() = 0;
}

// Sub-directories of `dir`, sorted and with dot-folders skipped (the Ctrl+K
// palette's scan tree treats `.*` the same way).
pub fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let hidden = p
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false);
                if !hidden {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

// Install a folder as the live vault and hand control back to the main loop.
pub fn set_vault(dir: &Path) {
    let dir = dir.to_path_buf();
    *crate::graph::processing::DIR_PATH.write().unwrap() = dir.clone();
    crate::graph::processing::generate_nodes_from_directory(&dir);
    VAULT_READY.store(true, Ordering::Relaxed);
}

// ---------- Tutorial vault ----------

fn sanitize_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "My Notes".to_string();
    }
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('-').to_string();
    if cleaned.is_empty() {
        "My Notes".to_string()
    } else {
        cleaned
    }
}

fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let mut dir = parent.join(name);
    let mut n = 2;
    while dir.exists() {
        dir = parent.join(format!("{name} ({n})"));
        n += 1;
    }
    dir
}

// The note files a fresh tutorial vault is seeded with. Every [[wikilink]]
// must name one of these stems so the graph draws real edges on first load.
fn tutorial_notes() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Welcome.md",
            "# Welcome to the Graph\n\
             \n\
             This is your new notes vault. The starter notes below already\n\
             link to each other with wikilinks, so the graph view shows a\n\
             real network the moment it loads.\n\
             \n\
             - [[The Graph]]\n\
             - [[The Editor]]\n\
             - [[Shortcuts]]\n\
             - [[Wikilinks]]\n\
             - [[Sub-graphs]]\n\
             \n\
             When you are ready, delete these notes (select one, press Delete,\n\
             confirm with Y) and write your own. Press N to add a new one.\n",
        ),
        (
            "The Graph.md",
            "# The Graph\n\
             \n\
             Every note is a node and a link to another note draws an edge, so a\n\
             web of notes becomes a web of connected dots.\n\
             \n\
             - Double-click a node (or hover it and press E) to open it.\n\
             - Press N to create a new note here.\n\
             - Press F for the force-simulation tweaks panel.\n\
             - Drag a node to move it; the simulation finds a new balance.\n\
             - Wheel zooms, middle-drag pans, right-click opens a menu.\n\
             - Sub-graph folders nest a mini-graph inside a note — see\n\
             [[Sub-graphs]].\n\
             \n\
             The layout is a force simulation (d3-style); tick budgets keep it\n\
             smooth. The camera follows the mouse when you zoom or pan.\n",
        ),
        (
            "The Editor.md",
            "# The Editor\n\
             \n\
             Opening a note lands you in **navigation mode**: the caret is\n\
             detached and the document renders as formatted view-source.\n\
             \n\
             - Enter or E — start editing on the selected line\n\
             - Esc — back to navigation mode\n\
             - G — close the editor and return to the graph\n\
             - Q — toggle editor fullscreen\n\
             - Ctrl+S saves (autosave also fires after a typing pause)\n\
             - Ctrl+Z / Ctrl+Y undo and redo\n\
             \n\
             Click [[Welcome]] to open the linked note, or type [[ and pick from the\n\
             popup to start a new one.\n",
        ),
        (
            "Shortcuts.md",
            "# Shortcuts\n\
             \n\
             The bar at the bottom of the screen always shows the keys that do\n\
             something in the current state; here is the whole map.\n\
             \n\
             **Graph**\n\
             - E — open the hovered node\n\
             - N — new note\n\
             - Del — delete the selected node (Y confirms)\n\
             - F — force-panel toggle\n\
             - Alt+Left — leave a sub-graph\n\
             - Ctrl+K — note palette\n\
             \n\
             **Editor**\n\
             - Enter / E — edit; Esc — nav mode; G — back to graph; Q — fullscreen\n\
             - Ctrl+S save; Ctrl+Z undo; Ctrl+Y redo\n\
             \n\
             **Anywhere** — Ctrl + = and Ctrl + - zoom the UI text.\n",
        ),
        (
            "Wikilinks.md",
            "# Wikilinks\n\
             \n\
             Links are what make the graph: every link from one note to another\n\
             becomes an edge connecting their nodes.\n\
             \n\
             - [[Welcome]] renders as a clickable link to the welcome note.\n\
             - Click a link to open the note in a new tab.\n\
             - Renaming a note rewrites every link that points at it.\n\
             - Targets match the note's file stem, so [[The Graph]] finds\n\
             The Graph.md.\n\
             \n\
             To make a new link, open the editor and type two opening brackets:\n\
             the autocomplete popup lists existing notes to pick from.\n",
        ),
        (
            "Sub-graphs.md",
            "# Sub-graphs\n\
             \n\
             A note can own a folder of its own: when `Project.md` sits next to\n\
             a `Project/` folder, the notes inside that folder form a sub-graph\n\
             nested under the Project note.\n\
             \n\
             - Double-click the folder disc to dive into the sub-graph.\n\
             - The breadcrumb across the top shows where you are.\n\
             - Alt+Left climbs back to the parent level.\n\
             - The editor's \"Open sub-graph\" button jumps straight there.\n\
             \n\
             Sub-graphs keep companion material (assets, daily notes, project\n\
             scratch) together with the note that owns them.\n",
        ),
    ]
}

// Create a fresh tutorial vault under `parent` named `name` (sanitized and
// de-duplicated), written with the tutorial notes. Returns the created path.
pub fn create_tutorial_vault(parent: &Path, name: &str) -> PathBuf {
    let cleaned = sanitize_name(name);
    let dir = unique_dir(parent, &cleaned);
    crate::filesystem::create_dir(&dir);
    for (file, content) in tutorial_notes() {
        crate::filesystem::write_file(&dir.join(file), content);
    }
    dir
}

// ---------- Input ----------

pub fn handle_input(rl: &mut RaylibHandle) {
    // Copy the screen out of the lock before dispatching: a temporary in a
    // match scrutinee lives until the whole match ends, so `match *SCREEN
    // .read()` would keep the read guard alive inside the arms — and the arms
    // (via handle_choose/handle_browse/handle_create) write SCREEN to switch
    // screens, which then deadlocks against that very guard.
    let screen = *SCREEN.read().unwrap();
    match screen {
        Screen::Choose => handle_choose(rl),
        Screen::Browse => handle_browse(rl),
        Screen::Create => handle_create(rl),
    }
}

fn handle_choose(rl: &mut RaylibHandle) {
    if !rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        return;
    }
    let m = rl.get_mouse_position();
    for id in 0..2 {
        let (x, y, w, h) = choose_button_rect(id);
        let hit = m.x as i32 >= x
            && m.x as i32 <= x + w
            && m.y as i32 >= y
            && m.y as i32 <= y + h;
        if hit {
            match id {
                0 => {
                    *BROWSE_DIR.write().unwrap() = home_dir();
                    refresh_entries();
                    *SCREEN.write().unwrap() = Screen::Browse;
                }
                _ => {
                    *CREATE_NAME.write().unwrap() = String::new();
                    *SCREEN.write().unwrap() = Screen::Create;
                }
            }
            return;
        }
    }
}

fn handle_browse(rl: &mut RaylibHandle) {
    let wheel = rl.get_mouse_wheel_move();
    if wheel != 0.0 {
        let (_, _, _panel_w, panel_h) = browse_panel_rect();
        let list_top = config::scaled_size(96);
        let row_h = config::scaled_size(36);
        let visible = ((panel_h - list_top - config::scaled_size(110)) / row_h).max(0);
        let n = BROWSE_ENTRIES.read().unwrap().len();
        let max_scroll = n.saturating_sub(visible as usize) as i32;
        let mut scroll = BROWSE_SCROLL.write().unwrap();
        *scroll = (*scroll as i32 - wheel as i32).clamp(0, max_scroll);
        return;
    }

    if !rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        return;
    }
    let m = rl.get_mouse_position();

    // Footer buttons: Cancel / Up / Open this folder.
    if hit_button(m, browse_button_rect(0)) {
        *SCREEN.write().unwrap() = Screen::Choose;
        return;
    }
    if hit_button(m, browse_button_rect(2)) {
        // Up to the parent; at the root the button just stays put.
        let parent = BROWSE_DIR
            .read()
            .unwrap()
            .parent()
            .map(|p| p.to_path_buf());
        if let Some(p) = parent {
            *BROWSE_DIR.write().unwrap() = p;
            refresh_entries();
        }
        return;
    }
    if hit_button(m, browse_button_rect(1)) {
        let dir = BROWSE_DIR.read().unwrap().clone();
        set_vault(&dir);
        return;
    }

    // A row click descends into that sub-directory. The read guard above must
    // not outlive this scope: refresh_entries takes the write lock on the same
    // RwLock, so holding the read guard across it would deadlock the thread.
    let (px, py, panel_w, panel_h) = browse_panel_rect();
    let list_top = py + config::scaled_size(96);
    let row_h = config::scaled_size(36);
    let rows_bottom = py + panel_h - config::scaled_size(110);
    if m.y as i32 >= list_top && m.y as i32 <= rows_bottom {
        let row = (m.y as i32 - list_top) / row_h;
        if row >= 0 && m.x as i32 >= px && m.x as i32 <= px + panel_w {
            let row = row + *BROWSE_SCROLL.read().unwrap();
            let entry = BROWSE_ENTRIES.read().unwrap().get(row as usize).cloned();
            if let Some(entry) = entry {
                *BROWSE_DIR.write().unwrap() = entry;
                refresh_entries();
            }
        }
    }
}

fn handle_create(rl: &mut RaylibHandle) {
    while let Some(ch) = rl.get_char_pressed() {
        let c = char::from_u32(ch as u32).unwrap();
        if !c.is_control() {
            CREATE_NAME.write().unwrap().push(c);
        }
    }
    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
    {
        CREATE_NAME.write().unwrap().pop();
    }
    if rl.is_key_pressed(KeyboardKey::KEY_ENTER)
        || rl.is_key_pressed(KeyboardKey::KEY_KP_ENTER)
    {
        create_vault_and_go();
        return;
    }
    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        *CREATE_NAME.write().unwrap() = String::new();
        *SCREEN.write().unwrap() = Screen::Choose;
        return;
    }
    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        let m = rl.get_mouse_position();
        if hit_button(m, create_button_rect(0)) {
            create_vault_and_go();
        } else if hit_button(m, create_button_rect(1)) {
            *CREATE_NAME.write().unwrap() = String::new();
            *SCREEN.write().unwrap() = Screen::Choose;
        }
    }
}

fn create_vault_and_go() {
    let name = CREATE_NAME.read().unwrap().clone();
    let dir = create_tutorial_vault(&home_dir(), &name);
    set_vault(&dir);
}

// ---------- Shared geometry (draw + hit-test agree) ----------

fn choose_button_rect(id: usize) -> (i32, i32, i32, i32) {
    let bw = config::scaled_size(280);
    let bh = config::scaled_size(64);
    let gap = config::scaled_size(28);
    let total = bw * 2 + gap;
    let x0 = (config::width() - total) / 2;
    let y = (config::height() as f32 * 0.52) as i32;
    let x = if id == 0 { x0 } else { x0 + bw + gap };
    (x, y, bw, bh)
}

fn browse_panel_rect() -> (i32, i32, i32, i32) {
    let pw = config::scaled_size(760);
    let ph = config::scaled_size(500);
    (
        (config::width() - pw) / 2,
        (config::height() - ph) / 2,
        pw,
        ph,
    )
}

// Footer button row inside the browse panel: [Cancel] [Open this folder] [Up].
fn browse_button_rect(id: usize) -> (i32, i32, i32, i32) {
    let (px, py, panel_w, panel_h) = browse_panel_rect();
    let bw = config::scaled_size(200);
    let bh = config::scaled_size(44);
    let gap = config::scaled_size(16);
    // Buttons are right-aligned; id 0 = Cancel (leftmost of the three).
    let total = bw * 3 + gap * 2;
    let x0 = px + panel_w - config::scaled_size(24) - total;
    let y = py + panel_h - config::scaled_size(58);
    let x = x0 + id as i32 * (bw + gap);
    (x, y, bw, bh)
}

fn create_button_rect(id: usize) -> (i32, i32, i32, i32) {
    // Row above id order: [Create & Start] [Cancel].
    let (px, py, panel_w, panel_h) = create_panel_rect();
    let bw = config::scaled_size(200);
    let bh = config::scaled_size(44);
    let gap = config::scaled_size(16);
    let total = bw * 2 + gap;
    let x0 = px + (panel_w - total) / 2;
    let y = py + panel_h - config::scaled_size(58);
    let x = if id == 0 { x0 } else { x0 + bw + gap };
    (x, y, bw, bh)
}

fn create_panel_rect() -> (i32, i32, i32, i32) {
    let pw = config::scaled_size(560);
    let ph = config::scaled_size(300);
    (
        (config::width() - pw) / 2,
        (config::height() - ph) / 2,
        pw,
        ph,
    )
}

fn hit_button(m: Vector2, (x, y, w, h): (i32, i32, i32, i32)) -> bool {
    m.x as i32 >= x && m.x as i32 <= x + w && m.y as i32 >= y && m.y as i32 <= y + h
}

// ---------- Draw ----------

pub fn draw(d: &mut RaylibDrawHandle) {
    let w = config::width();
    let h = config::height();
    d.draw_rectangle(0, 0, w, h, BG);
    // Same scrutinee-copy as handle_input: the arms never write SCREEN today,
    // but reading the lock into a plain value keeps this structurally unable
    // to deadlock if one ever does.
    let screen = *SCREEN.read().unwrap();
    match screen {
        Screen::Choose => draw_choose(d),
        Screen::Browse => draw_browse(d),
        Screen::Create => draw_create(d),
    }
}

fn draw_choose(d: &mut RaylibDrawHandle) {
    let w = config::width();
    let h = config::height();
    let title_fs = config::scaled_size(46);
    let sub_fs = config::scaled_size(20);
    let hint_fs = config::scaled_size(15);

    let title = "Welcome to the Graph";
    let title_w = text::measure(d, title, title_fs);
    let ty = (h as f32 * 0.24) as i32;
    text::draw(d, title, (w - title_w) / 2, ty, title_fs, TITLE_COLOR);

    let sub = "Open a notes folder, or create a new one with a starter tutorial";
    let sub_w = text::measure(d, sub, sub_fs);
    text::draw(d, sub, (w - sub_w) / 2, ty + title_fs + config::scaled_size(14), sub_fs, SUBTITLE_COLOR);

    // Two action buttons; the hover ring responds to the cursor.
    for (id, label) in [(0, "Open Folder..."), (1, "Create New")] {
        let (x, y, bw, bh) = choose_button_rect(id);
        let m = d.get_mouse_position();
        let hover = hit_button(m, (x, y, bw, bh));
        d.draw_rectangle(x, y, bw, bh, if hover { BTN_HOVER } else { BTN_BG });
        let fs = config::scaled_size(22);
        let lw = text::measure(d, label, fs);
        text::draw(d, label, x + (bw - lw) / 2, y + (bh - fs) / 2, fs, BTN_TEXT);
    }

    let hint = "Notes are .md files; [[wikilinks]] between them draw the graph.";
    let hint_w = text::measure(d, hint, hint_fs);
    text::draw(d, hint, (w - hint_w) / 2, h - config::scaled_size(56), hint_fs, SUBTITLE_COLOR);
}

fn draw_browse(d: &mut RaylibDrawHandle) {
    let (px, py, pw, ph) = browse_panel_rect();
    d.draw_rectangle(px, py, pw, ph, PANEL_BG);
    d.draw_rectangle_lines_ex(Rectangle::new(px as f32, py as f32, pw as f32, ph as f32), 1.0, PANEL_EDGE);

    let title_fs = config::scaled_size(24);
    text::draw(d, "Open a notes folder", px + config::scaled_size(24), py + config::scaled_size(20), title_fs, TITLE_COLOR);

    // Current path, truncated to fit from the right end (keep the tail: that
    // is the part the user just navigated to).
    let path_fs = config::scaled_size(14);
    let path_text = BROWSE_DIR.read().unwrap().to_string_lossy().to_string();
    let max_w = pw - config::scaled_size(48);
    let shown = truncate_middle(&path_text, path_fs, max_w, d);
    text::draw(d, &shown, px + config::scaled_size(24), py + config::scaled_size(58), path_fs, SUBTITLE_COLOR);

    // Row list (clipped by the panel; the wheel scrolls it).
    let list_top = py + config::scaled_size(96);
    let row_h = config::scaled_size(36);
    let rows_bottom = py + ph - config::scaled_size(110);
    let scroll = *BROWSE_SCROLL.read().unwrap();
    let entries = BROWSE_ENTRIES.read().unwrap();

    if entries.is_empty() {
        let fs = config::scaled_size(16);
        let msg = "No sub-folders here (press N in the graph to create notes in it)";
        let mw = text::measure(d, msg, fs);
        text::draw(d, msg, px + (pw - mw) / 2, list_top + config::scaled_size(40), fs, SUBTITLE_COLOR);
    }

    for (i, entry) in entries.iter().enumerate() {
        let row = i as i32 - scroll;
        let y = list_top + row * row_h;
        if y + row_h <= list_top || y >= rows_bottom {
            continue;
        }
        let hover = {
            let m = d.get_mouse_position();
            m.y as i32 >= y && m.y as i32 <= y + row_h && m.x as i32 >= px && m.x as i32 <= px + pw
        };
        if hover {
            d.draw_rectangle(px + config::scaled_size(8), y, pw - config::scaled_size(16), row_h, ROW_HOVER);
        }
        let name = entry.file_name().unwrap_or_default().to_string_lossy().to_string();
        let fs = config::scaled_size(16);
        text::draw(d, &name, px + config::scaled_size(22), y + (row_h - fs) / 2, fs, BTN_TEXT);
    }

    // Footer buttons.
    let buttons = ["Cancel", "Open this folder", "Up one level"];
    for (id, label) in buttons.iter().enumerate() {
        let (x, y, bw, bh) = browse_button_rect(id);
        let m = d.get_mouse_position();
        let hover = hit_button(m, (x, y, bw, bh));
        d.draw_rectangle(x, y, bw, bh, if hover { BTN_HOVER } else { BTN_BG });
        let fs = config::scaled_size(16);
        let lw = text::measure(d, label, fs);
        text::draw(d, label, x + (bw - lw) / 2, y + (bh - fs) / 2, fs, BTN_TEXT);
    }
}

fn draw_create(d: &mut RaylibDrawHandle) {
    let (px, py, pw, ph) = create_panel_rect();
    d.draw_rectangle(px, py, pw, ph, PANEL_BG);
    d.draw_rectangle_lines_ex(Rectangle::new(px as f32, py as f32, pw as f32, ph as f32), 1.0, PANEL_EDGE);

    let title_fs = config::scaled_size(22);
    text::draw(d, "Create a new notes folder", px + config::scaled_size(24), py + config::scaled_size(22), title_fs, TITLE_COLOR);

    let label_fs = config::scaled_size(15);
    text::draw(d, "Name", px + config::scaled_size(24), py + config::scaled_size(74), label_fs, SUBTITLE_COLOR);

    // Name input box with a caret.
    let (bx, by, bw, bh) = {
        let bw = pw - config::scaled_size(48);
        let bh = config::scaled_size(40);
        (px + config::scaled_size(24), py + config::scaled_size(98), bw, bh)
    };
    d.draw_rectangle(bx, by, bw, bh, Color::new(40, 44, 54, 255));
    let fs = config::scaled_size(18);
    let name = CREATE_NAME.read().unwrap();
    let pad = config::scaled_size(10);
    text::draw(d, &name, bx + pad, by + (bh - fs) / 2, fs, BTN_TEXT);
    let caret_x = bx + pad + text::measure(d, &name, fs);
    d.draw_rectangle(caret_x, by + (bh - fs) / 2, 2, fs, CARET);

    let home = home_dir();
    let hint = format!("Created at {}", home.join(&*name).display());
    let hint_fs = config::scaled_size(13);
    text::draw(d, &hint, bx, by + bh + config::scaled_size(10), hint_fs, SUBTITLE_COLOR);

    let buttons = ["Create & Start", "Cancel"];
    for (id, label) in buttons.iter().enumerate() {
        let (x, y, bw, bh) = create_button_rect(id);
        let m = d.get_mouse_position();
        let hover = hit_button(m, (x, y, bw, bh));
        d.draw_rectangle(x, y, bw, bh, if hover { BTN_HOVER } else { BTN_BG });
        let fs = config::scaled_size(16);
        let lw = text::measure(d, label, fs);
        text::draw(d, label, x + (bw - lw) / 2, y + (bh - fs) / 2, fs, BTN_TEXT);
    }
}

// Truncate a path to `max_w` at the font size, keeping the tail (the head is
// "/..."). Full paths almost never fit a narrow panel without this.
fn truncate_middle(path: &str, fs: i32, max_w: i32, d: &mut RaylibDrawHandle) -> String {
    if text::measure(d, path, fs) <= max_w {
        return path.to_string();
    }
    const HEAD: &str = "/.../";
    let head_w = text::measure(d, HEAD, fs);
    let mut tail = String::new();
    for ch in path.chars().rev() {
        tail.insert(0, ch);
        if head_w + text::measure(d, &tail, fs) > max_w {
            tail.remove(0);
            break;
        }
    }
    format!("{HEAD}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_parent() -> PathBuf {
        let p = std::env::temp_dir().join("rg_startup_test");
        let _ = std::fs::remove_dir_all(&p);
        crate::filesystem::create_dir(&p);
        p
    }

    #[test]
    fn sanitize_cleans_and_defaults() {
        assert_eq!(sanitize_name(""), "My Notes");
        assert_eq!(sanitize_name("  My Vault  "), "My Vault");
        assert_eq!(sanitize_name("a/b:c*"), "a-b-c");
        assert_eq!(sanitize_name("///"), "My Notes");
        // Unicode letters survive (they are valid in filenames).
        assert_eq!(sanitize_name("Neue Notizen"), "Neue Notizen");
    }

    #[test]
    fn tutorial_vault_is_populated_and_consistent() {
        let parent = temp_parent();
        let dir = create_tutorial_vault(&parent, "My Notes");
        assert!(dir.is_dir());

        let files = crate::filesystem::scan_tree(&dir);
        assert!(files.len() >= 6, "tutorial vault should have >= 6 notes");

        // Every wikilink target resolves to a sibling note file.
        let stems: Vec<String> = files
            .iter()
            .filter_map(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        for f in &files {
            let content = crate::filesystem::read_file(f);
            assert!(
                !content.contains('\\'),
                "{:?} has a stray backslash in it",
                f.file_name().unwrap()
            );
            for target in crate::filesystem::parse_links(&content) {
                assert!(
                    stems.iter().any(|s| s == &target),
                    "{:?} links to [[{}]] which has no note here",
                    f.file_name().unwrap(),
                    target
                );
            }
        }
        // The welcome note links the hub notes, so edges exist.
        let welcome = std::fs::read_to_string(dir.join("Welcome.md")).unwrap();
        assert!(welcome.contains("[[Wikilinks]]"));
    }

    #[test]
    fn duplicate_tutorial_name_gets_suffix() {
        let parent = temp_parent();
        let first = create_tutorial_vault(&parent, "Notes");
        let second = create_tutorial_vault(&parent, "Notes");
        assert_ne!(first, second);
        assert!(second.file_name().unwrap().to_string_lossy().contains("(2)"));
        assert!(second.is_dir());
    }

    #[test]
    fn lists_subdirs_sorted_and_skips_hidden() {
        let parent = temp_parent();
        crate::filesystem::create_dir(&parent.join("zeta"));
        crate::filesystem::create_dir(&parent.join("alpha"));
        crate::filesystem::create_dir(&parent.join(".hidden"));
        std::fs::write(parent.join("file.md"), "x").unwrap();

        let dirs = list_subdirs(&parent);
        let names: Vec<String> = dirs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    #[test]
    fn set_vault_installs_path_and_nodes() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        let parent = temp_parent();
        let dir = create_tutorial_vault(&parent, "Vault");
        VAULT_READY.store(false, Ordering::Relaxed);
        set_vault(&dir);

        assert!(VAULT_READY.load(Ordering::Relaxed));
        assert_eq!(*crate::graph::processing::DIR_PATH.read().unwrap(), dir);
        assert!(crate::graph::processing::NODES.read().unwrap().len() >= 6);

        VAULT_READY.store(false, Ordering::Relaxed);
    }
}