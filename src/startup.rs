// First-launch picker: when the app starts without a folder (or the folder it
// was handed does not exist), the main loop shows this standalone screen
// instead of the graph. It offers either browsing to an existing notes folder
// or creating a brand-new one seeded with a "Fold me" tour: notes that link
// each other (so the graph is alive on load), a formatting showcase, and a
// folding demo that ships unfolded - fold the hub and its children pack into
// a sub-graph folder right in front of you.
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
// must name one of these stems so the graph draws real edges on first load;
// image targets (`[[assets/...]]`) are exempt - they paint pictures, not
// edges.
fn tutorial_notes() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Formatting.md", formatting_note()),
        ("Shortcuts.md", shortcuts_note()),
        ("Try this.md", try_this_note()),
        // The fold demo: a hub note plus four children it references. Each
        // child is referenced by exactly one note (the hub) and nothing
        // links back - shared or cyclic references would trip the fold
        // guards. It ships unfolded so the children are visible on load;
        // the Fold menu row packs them into `Fold me/` and Unwrap spills
        // them back out.
        ("Fold me.md", fold_hub_note()),
        ("Inside.md", inside_fold_note()),
        ("Deeper.md", deeper_note()),
        ("Leaf.md", leaf_note()),
        ("Core.md", core_note()),
    ]
}

fn fold_hub_note() -> &'static str {
    "# Fold me\n\
     \n\
     This starter vault is a live tour of the app: notes that link each\n\
     other, so the graph is alive the moment it loads. This note is a\n\
     working fold demo all on its own - these four notes are its children:\n\
     \n\
     - [[Inside]] - the first level packed away\n\
     - [[Deeper]] - one more level down\n\
     - [[Leaf]] - the end of the branch\n\
     - [[Core]] - where the fold stops\n\
     \n\
     Each child is referenced by exactly one note (me), and none of them\n\
     points back at anything. That single-owner rule is what keeps a fold\n\
     legal: shared children or back-links block it.\n\
     \n\
     Right-click this node and choose Fold: the children pack into a\n\
     folder named `Fold me/` and disappear, and this node becomes a disc.\n\
     Unwrap spills them back out; V (or double-click) dives into the\n\
     folded folder.\n\
     \n\
     From the editor, Ctrl+K jumps to any note by name, N creates one,\n\
     and J is today's journal. When the tour is over, delete these notes\n\
     (select, Del, confirm Y) and make the vault yours."
}

fn formatting_note() -> &'static str {
    concat!(
        "---\n",
        "title: Formatting\n",
        "header: [[assets/swatch.bmp]]\n",
        "---\n",
        "\n",
        "# Formatting\n",
        "\n",
        "Every markup style the editor renders, in one place. The image on\n",
        "this note's graph node is its header - `header:` in the frontmatter\n",
        "above - and a whole-line image link draws inline as well:\n",
        "\n",
        "[[assets/swatch.bmp]]\n",
        "\n",
        "## Emphasis\n",
        "\n",
        "- **bold** with two asterisks or two underscores\n",
        "- *italic* with one asterisk or one underscore\n",
        "- __bold__ and _italic_ - the underscore spellings work too\n",
        "\n",
        "## Code\n",
        "\n",
        "Short spans go in `inline backticks`, longer samples in a fence:\n",
        "\n",
        "```\n",
        "rust\n",
        "fn fold(block: &Block) {\n",
        "    block.collapsed()\n",
        "}\n",
        "```\n",
        "\n",
        "## Links\n",
        "\n",
        "- [[Fold me]] - a plain wikilink; every link becomes a graph edge\n",
        "- [custom label]([[Try this]]) - the markdown form\n",
        "- Type two opening brackets in the editor to pick a target from the palette\n",
        "\n",
        "## Lists\n",
        "\n",
        "Indented bullets build a foldable block: the head line gets a caret.\n",
        "\n",
        "- level one\n",
        "    - level two\n",
        "        - level three\n",
        "\n",
        "1. Numbered lists work too\n",
        "2. And they nest the same way\n",
        "\n",
        "## Blocks fold\n",
        "\n",
        "Whole blocks collapse behind their head line, like a code editor's\n",
        "outline: click the fold caret on a list item (or a heading that owns\n",
        "indented content) and everything beneath it folds away.\n",
        "\n",
        "- A fold head owns the nested block below it\n",
        "    - deeper indents stack more levels\n",
        "- A heading folds whatever sits indented under it\n",
        "        this line hides when the heading above folds\n",
        "\n",
        "## Quotes and rules\n",
        "\n",
        "> Blockquotes draw a trailing rail beside the text.\n",
        "\n",
        "---\n",
        "\n",
        "A horizontal rule splits sections, and the frontmatter block at the\n",
        "top holds the `title:` and `header:` fields the editor and graph\n",
        "both read. Notes fold into sub-graphs too - [[Fold me]] on the main\n",
        "level is one ready to collapse.\n",
    )
}

fn shortcuts_note() -> &'static str {
    "# Shortcuts\n\
     \n\
     The bar at the bottom of the screen always shows the keys that do\n\
     something right now; here is the whole map. [[Fold me]] ties it all\n\
     together.\n\
     \n\
     **Graph**\n\
     - WASD - move; C / Shift+C - zoom out / in (hold to glide)\n\
     - Space - open the hovered node\n\
     - V - dive into the hovered node's sub-graph\n\
     - R - node context menu\n\
     - N - new note; Del - delete (Y confirms)\n\
     - J - today's journal note\n\
     - F - force-simulation tweaks\n\
     - Alt+Left - climb out of a sub-graph\n\
     \n\
     **Editor, navigation mode**\n\
     - Enter / E - start editing the selected line\n\
     - Esc - back to navigation mode\n\
     - G - close the editor and return to the graph\n\
     - Q - toggle editor fullscreen\n\
     - Click a fold caret to fold or unfold the block under it\n\
     - J - today's journal; [ and ] - previous / next day\n\
     \n\
     **Editor, edit mode**\n\
     - Ctrl+S - save (autosave also fires after a typing pause)\n\
     - Ctrl+Z / Ctrl+Y - undo / redo\n\
     - Esc - back to navigation mode\n\
     \n\
     **Everywhere** - Ctrl+K opens any note by name; Ctrl+= and Ctrl+-\n\
     resize the UI text."
}

fn try_this_note() -> &'static str {
    "# Try this\n\
     \n\
     A five-minute tour, in order. Everything links out for the details:\n\
     [[Formatting]] for markup, [[Shortcuts]] for keys, and [[Fold me]]\n\
     for the fold demo.\n\
     \n\
     1. Open [[Fold me]] and read how its demo works.\n\
     2. Hold C, then Shift+C: the view zooms out, then in, smoothly.\n\
     3. Right-click [[Fold me]] and choose Fold: its children vanish into\n\
        a disc right in front of you. Unwrap spills them back out, then\n\
        Fold again - the round trip is the whole demo in ten seconds.\n\
     4. Dive into the folded [[Fold me]] with V, then climb back out\n\
        with Alt+Left or the breadcrumb.\n\
     5. Right-click any node: rename, delete, fold, set a header image.\n\
     6. Press N, type a title, press Enter: a note joins the graph on save.\n\
     7. Press J for today's journal note.\n\
     8. Open [[Formatting]] and click the fold caret on its demo block.\n\
     9. Spend a minute in the Ctrl+K note palette, then delete these\n\
        notes (select, Del, confirm Y) and start your own vault.\n\
     \n\
     You have now touched every major surface of the app."
}

fn inside_fold_note() -> &'static str {
    "# Inside\n\
     \n\
     The first level of the fold. This note is a plain leaf: it references\n\
     nothing, and only Fold me references it - so when Fold me folds,\n\
     this note folds away with it and nothing gets left behind."
}

fn deeper_note() -> &'static str {
    "# Deeper\n\
     \n\
     One level further down. A leaf like this one is what keeps a fold\n\
     honest: no back-links, no shared references. Fold packs a folder\n\
     around it, and it stays tucked in until Unwrap."
}

fn leaf_note() -> &'static str {
    "# Leaf\n\
     \n\
     The end of the branch - the fold stops here. Nothing beyond this\n\
     note exists in the demo, so the whole set (Fold me, Inside, Deeper,\n\
     this note) packs and unpacks as one unit."
}

fn core_note() -> &'static str {
    "# Core\n\
     \n\
     The innermost child. To fold your own graph, create a note (N), have\n\
     it reference a couple of fresh notes, then right-click it and choose\n\
     Fold - the menu row appears whenever the graph agrees the fold is\n\
     legal, and the children disappear exactly like the four here."
}

// Create a fresh tutorial vault under `parent` named `name` (sanitized and
// de-duplicated). All tutorial notes are written flat: the fold demo ships
// unfolded, so its children start as ordinary root notes and the folder
// appears only when the user folds the hub. A generated header-image asset
// is written too. Returns the created path.
pub fn create_tutorial_vault(parent: &Path, name: &str) -> PathBuf {
    let cleaned = sanitize_name(name);
    let dir = unique_dir(parent, &cleaned);
    crate::filesystem::create_dir(&dir);
    for (file, content) in tutorial_notes() {
        crate::filesystem::write_file(&dir.join(file), content);
    }
    crate::filesystem::create_dir(&dir.join("assets"));
    std::fs::write(dir.join("assets").join("swatch.bmp"), swatch_bmp()).unwrap();
    dir
}

// A tiny 24-bit bitmap the starter vault uses as Formatting.md's header
// image, generated at seed time so no binary asset ships with the source.
// A blue-to-mint gradient reads clearly as a node disc and as the note's
// inline image.
fn swatch_bmp() -> Vec<u8> {
    const W: u32 = 64;
    const H: u32 = 64;
    let row_bytes = W * 3;
    let pad = (4 - row_bytes % 4) % 4;
    let mut out = Vec::with_capacity(54 + (H * (row_bytes + pad)) as usize);
    // BITMAPFILEHEADER
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + H * (row_bytes + pad)).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    // BITMAPINFOHEADER (24-bit, no compression)
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&W.to_le_bytes());
    out.extend_from_slice(&(H as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&24u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // compression
    out.extend_from_slice(&(H * (row_bytes + pad)).to_le_bytes()); // image size
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // palette colours
    out.extend_from_slice(&0u32.to_le_bytes());
    // Pixels, rows bottom-up in BGR order.
    for row in (0..H).rev() {
        for col in 0..W {
            let t = (row + col) as f32 / ((W + H - 2) as f32);
            let blue = (150.0 - 90.0 * t).clamp(0.0, 255.0) as u8;
            let green = (120.0 + 60.0 * t).clamp(0.0, 255.0) as u8;
            let red = (60.0 + 100.0 * t).clamp(0.0, 255.0) as u8;
            out.push(blue);
            out.push(green);
            out.push(red);
        }
        for _ in 0..pad {
            out.push(0);
        }
    }
    out
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
        assert!(files.len() >= 8, "tutorial vault should have >= 8 notes");

        // The fold demo ships unfolded: a loose hub note plus four children
        // as plain root notes, and no `Fold me/` folder yet. Folding
        // generates that folder at runtime; the header swatch always ships
        // under assets/ as a generated bitmap.
        assert!(dir.join("Fold me.md").is_file());
        for child in ["Inside.md", "Deeper.md", "Leaf.md", "Core.md"] {
            assert!(dir.join(child).is_file(), "{child} should be a root note");
        }
        assert!(!dir.join("Fold me").is_dir(), "fold demo must ship unfolded");
        let bmp = std::fs::read(dir.join("assets").join("swatch.bmp")).unwrap();
        assert_eq!(&bmp[..2], b"BM", "swatch should be a bitmap");

        // Every wikilink target resolves to a sibling note file. Image targets
        // ([[assets/...]]) draw pictures, not edges, so they are exempt - the
        // graph's edge builder treats them the same way.
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
                if crate::editor::images::is_image_target(&target) {
                    continue;
                }
                assert!(
                    stems.iter().any(|s| s == &target),
                    "{:?} links to [[{}]] which has no note here",
                    f.file_name().unwrap(),
                    target
                );
            }
        }
        // The hub references its four children (so folding packs exactly them),
        // and a tour note links back into it, so real edges exist on first
        // load.
        let hub = std::fs::read_to_string(dir.join("Fold me.md")).unwrap();
        assert!(hub.contains("[[Inside]]") && hub.contains("[[Core]]"));
        let formatting = std::fs::read_to_string(dir.join("Formatting.md")).unwrap();
        assert!(formatting.contains("[[Fold me]]"));
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
        assert!(crate::graph::processing::NODES.read().unwrap().len() >= 5);

        VAULT_READY.store(false, Ordering::Relaxed);
    }

    #[test]
    fn fold_me_demo_round_trips_on_disk() {
        let _guard = crate::graph::processing::TEST_NAV_LOCK.lock().unwrap();
        let parent = temp_parent();
        let dir = create_tutorial_vault(&parent, "Vault");
        VAULT_READY.store(false, Ordering::Relaxed);
        set_vault(&dir);
        crate::graph::processing::rebuild_edges();

        // The hub ships unfolded and folds cleanly (its four children are
        // dead-end leaves with a single owner).
        let idx = {
            let nodes = crate::graph::processing::NODES.read().unwrap();
            let i = nodes
                .iter()
                .position(|n| {
                    n.path
                        .file_stem()
                        .map(|s| s.to_string_lossy() == "Fold me")
                        .unwrap_or(false)
                })
                .unwrap();
            let rows = crate::graph::processing::context_menu_rows(Some(i), &nodes);
            assert!(
                rows.contains(&crate::graph::processing::ContextRow::FoldSubGraph),
                "hub must offer Fold"
            );
            i
        };
        assert!(crate::graph::processing::fold_node(&dir, idx));
        assert!(dir.join("Fold me").join("Fold me.md").is_file());
        for child in ["Inside.md", "Deeper.md", "Leaf.md", "Core.md"] {
            assert!(
                !dir.join(child).exists(),
                "{child} should be packed inside the fold"
            );
            assert!(dir.join("Fold me").join(child).is_file());
        }

        // After folding, the hub is a folder-backed disc: it offers Unwrap and
        // the round trip restores every note to the root exactly as seeded.
        crate::graph::processing::generate_nodes_from_directory(&dir);
        let hub = {
            let nodes = crate::graph::processing::NODES.read().unwrap();
            let i = nodes
                .iter()
                .position(|n| {
                    n.path
                        .file_stem()
                        .map(|s| s.to_string_lossy() == "Fold me")
                        .unwrap_or(false)
                })
                .unwrap();
            assert!(nodes[i].folder_backed);
            let rows = crate::graph::processing::context_menu_rows(Some(i), &nodes);
            assert!(
                rows.contains(&crate::graph::processing::ContextRow::UnwrapSubGraph),
                "folded hub must offer Unwrap"
            );
            i
        };
        assert!(crate::graph::processing::unwrap_node(&dir, hub));
        assert!(dir.join("Fold me.md").is_file());
        for child in ["Inside.md", "Deeper.md", "Leaf.md", "Core.md"] {
            assert!(dir.join(child).is_file(), "{child} should be back at root");
        }

        VAULT_READY.store(false, Ordering::Relaxed);
    }
}