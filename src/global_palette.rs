// Global Ctrl+K node palette. Unlike the graph's per-folder node list, this
// indexes the whole project tree (the note's [[ links may point anywhere), so
// the search is recursive, from the project root down. It doubles as a quick
// "go to note" launcher and a "type the name of a note that doesn't exist yet"
// creator: Enter on the last `+ Create` row creates and opens the note, and
// Enter with zero matches does the same.
//
// Matching mirrors the command palette's loosened prefix rule: a note whose
// name starts with the filter comes first, then any substring of a word in
// the name, then a bare substring anywhere. Ties keep tree order.
//
// The palette is modal and reachable from both the graph and the editor view;
// main.rs runs its input handler before the view handlers so an open palette
// swallows the keystrokes that would otherwise edit the note or pan the graph.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use raylib::prelude::*;

use crate::config;
use crate::editor::text;

#[derive(Debug, Clone, PartialEq)]
pub struct NoteEntry {
    pub name: String,
    pub path: PathBuf,
    // The name, or `parent/name` when the project holds two notes with the
    // same stem so Ctrl+K can tell them apart.
    pub display: String,
}

pub struct PaletteState {
    pub open: bool,
    pub filter: String,
    pub selected: usize,
    pub index: Vec<NoteEntry>,
    // Sorted-match indices into `index`, recomputed on every keystroke.
    pub matches: Vec<usize>,
    // Whether the trailing `+ Create 'filter'` row is offered: the filter is
    // non-empty and no index note has that exact name.
    pub show_create: bool,
}

pub static STATE: RwLock<PaletteState> = RwLock::new(PaletteState {
    open: false,
    filter: String::new(),
    selected: 0,
    index: Vec::new(),
    matches: Vec::new(),
    show_create: false,
});

fn stem(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().trim_end_matches(".md").to_string())
        .unwrap_or_default()
}

fn build_index(root: &Path) -> Vec<NoteEntry> {
    let paths = crate::filesystem::scan_tree(root);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in &paths {
        *counts.entry(stem(p)).or_insert(0) += 1;
    }
    paths
        .into_iter()
        .map(|p| {
            let name = stem(&p);
            let duplicated = counts.get(&name).copied().unwrap_or(0) > 1;
            let display = if duplicated {
                let rel = p
                    .parent()
                    .and_then(|par| par.strip_prefix(root).ok())
                    .map(|r| r.to_string_lossy())
                    .unwrap_or_default();
                if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", rel, name)
                }
            } else {
                name.clone()
            };
            NoteEntry {
                name,
                path: p,
                display,
            }
        })
        .collect()
}

// Re-derive `matches`/`show_create` from `filter`. Loosened-prefix buckets in
// tree order, plus a create row whenever the exact (case-insensitive) name is
// absent. The trailing ".md" of the filter is ignored for the exact check so
// typing either `alpha` or `alpha.md` lands on the real note.
fn recompute(st: &mut PaletteState) {
    // Stems drop the ".md", so the filter's trailing ".md" is ignored too:
    // typing `beta.md` matches the `beta` note just like `beta`.
    let target = st.filter.trim().trim_end_matches(".md");
    let f = target.to_lowercase();
    let mut prefix: Vec<usize> = Vec::new();
    let mut loose: Vec<usize> = Vec::new();
    if f.is_empty() {
        prefix = (0..st.index.len()).collect();
    } else {
        for (i, e) in st.index.iter().enumerate() {
            let lower = e.name.to_lowercase();
            let hit = lower.starts_with(&f);
            let word_hit = !hit && lower.split_whitespace().any(|w| w.starts_with(&f));
            let contains_hit = !hit && !word_hit && lower.contains(&f);
            if hit {
                prefix.push(i);
            } else if word_hit || contains_hit {
                loose.push(i);
            }
        }
    }
    prefix.extend(loose);
    st.matches = prefix;

    let exact = target.to_lowercase();
    st.show_create = !st.filter.trim().is_empty()
        && !st
            .index
            .iter()
            .any(|e| e.name.to_lowercase() == exact);

    let total = st.matches.len() + st.show_create as usize;
    st.selected = if total == 0 { 0 } else { st.selected.min(total - 1) };
}

pub fn open() {
    let root = crate::graph::processing::project_root();
    let index = build_index(&root);
    let mut st = STATE.write().unwrap();
    st.open = true;
    st.filter.clear();
    st.selected = 0;
    st.index = index;
    recompute(&mut st);
}

pub fn close() {
    STATE.write().unwrap().open = false;
}

fn is_open() -> bool {
    STATE.read().unwrap().open
}

enum Action {
    Open(PathBuf, String),
    Create(String),
}

// Returns true when the frame was consumed by the palette (Ctrl+K toggle, or
// any keystroke while it is open). Called before the sidebar/editor/graph
// handlers so typing lands in the filter instead of the note.
pub fn handle_input(rl: &mut RaylibHandle, editor_open: &mut bool) -> bool {
    let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
    if ctrl && rl.is_key_pressed(KeyboardKey::KEY_K) {
        if is_open() {
            close();
        } else {
            open();
        }
        return true;
    }
    if !is_open() {
        return false;
    }

    // Modal while open: everything that follows belongs to the palette.
    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        close();
        return true;
    }

    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE) {
        let mut st = STATE.write().unwrap();
        if !st.filter.is_empty() {
            st.filter.pop();
            recompute(&mut st);
        }
        return true;
    }

    if rl.is_key_pressed(KeyboardKey::KEY_DOWN) || rl.is_key_pressed(KeyboardKey::KEY_TAB) {
        let mut st = STATE.write().unwrap();
        let total = st.matches.len() + st.show_create as usize;
        if total > 0 {
            st.selected = (st.selected + 1) % total;
        }
        return true;
    }

    if rl.is_key_pressed(KeyboardKey::KEY_UP) {
        let mut st = STATE.write().unwrap();
        let total = st.matches.len() + st.show_create as usize;
        if total > 0 {
            st.selected = (st.selected + total - 1) % total;
        }
        return true;
    }

    while let Some(code) = rl.get_char_pressed() {
        let ch = char::from_u32(code as u32).unwrap_or(' ');
        if !ch.is_control() {
            let mut st = STATE.write().unwrap();
            st.filter.push(ch);
            recompute(&mut st);
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) || rl.is_key_pressed(KeyboardKey::KEY_KP_ENTER) {
        let action = {
            let st = STATE.read().unwrap();
            let total = st.matches.len() + st.show_create as usize;
            if total == 0 {
                None
            } else if st.selected < st.matches.len() {
                let idx = st.matches[st.selected];
                st.index
                    .get(idx)
                    .map(|e| Action::Open(e.path.clone(), e.name.clone()))
            } else if st.show_create {
                let name = st.filter.trim().to_string();
                if name.is_empty() {
                    None
                } else {
                    Some(Action::Create(name))
                }
            } else {
                None
            }
        };
        let acted = action.is_some();
        match action {
            Some(Action::Open(path, name)) => {
                crate::editor::tabs::open(&path, &name);
            }
            Some(Action::Create(name)) => {
                let dir = crate::graph::processing::DIR_PATH.read().unwrap().clone();
                crate::editor::tabs::create_and_open(&dir, &name);
            }
            None => {}
        }
        if acted {
            *editor_open = true;
        }
        close();
        return true;
    }

    true
}

// Screen-centred overlay: a dim backdrop with the note list (and the trailing
// create row) boxed in the middle. Drawn by main.rs after the editor and
// sidebar so it always sits on top.
pub fn draw(d: &mut RaylibDrawHandle) {
    let st = STATE.read().unwrap();
    if !st.open {
        return;
    }

    let (sw, sh) = (config::width(), config::height());
    d.draw_rectangle(0, 0, sw, sh, Color::new(0, 0, 0, 120));

    let font_size = config::scaled_size(config::EDITOR_FONT_SIZE);
    let item_h = config::scaled_size(config::AUTOCOMPLETE_ITEM_HEIGHT).max(2);
    let cap = config::AUTOCOMPLETE_MAX_VISIBLE;
    let mcap = cap.saturating_sub(st.show_create as usize);

    // Keep the selected row in a scrolling window the size of one box-worth;
    // the create row is pinned to the bottom and never displaced.
    let total = st.matches.len() + st.show_create as usize;
    let sel = if total == 0 {
        0
    } else {
        st.selected.min(total - 1)
    };
    let off = if mcap == 0 {
        0
    } else if sel < mcap {
        0
    } else {
        (sel - mcap + 1).min(st.matches.len().saturating_sub(mcap))
    };
    // The highlighted visual row: for a match that is `off` shifted in the
    // window, for the create row the last row.
    let shown = st.matches.len().saturating_sub(off).min(mcap) + st.show_create as usize;
    let hl = if sel < st.matches.len() {
        sel.saturating_sub(off)
    } else {
        shown.saturating_sub(1)
    };

    let mut max_w = 0;
    for i in off..off + st.matches.len().min(mcap) {
        if let Some(&idx) = st.matches.get(i) {
            if let Some(e) = st.index.get(idx) {
                max_w = max_w.max(text::measure(d, &e.display, font_size));
            }
        }
    }
    if st.show_create {
        let label = format!("+ Create '{}'", st.filter.trim());
        max_w = max_w.max(text::measure(d, &label, font_size));
    }

    let pad_x = 24;
    let box_w = max_w + pad_x * 2 + 40;
    let shown_i32 = shown as i32;
    let box_h = shown_i32 * item_h + 12;
    let cx = sw / 2 - box_w / 2;
    let cy = sh / 2 - box_h / 2;

    d.draw_rectangle(cx, cy, box_w, box_h, config::AUTOCOMPLETE_BG);
    d.draw_rectangle_lines(cx, cy, box_w, box_h, Color::new(110, 120, 150, 255));

    for r in 0..shown {
        let row_y = cy + 6 + r as i32 * item_h;
        let create_row = r >= shown - st.show_create as usize && st.show_create;
        if r == hl {
            d.draw_rectangle(
                cx + 2,
                row_y,
                box_w - 4,
                item_h - 2,
                config::AUTOCOMPLETE_SELECTED_BG,
            );
        }
        let (label, color) = if create_row {
            (
                format!("+ Create '{}'", st.filter.trim()),
                Color::new(150, 220, 160, 255),
            )
        } else {
            let idx = st.matches[off + r];
            let e = &st.index[idx];
            (e.display.clone(), Color::WHITE)
        };
        text::draw(
            d,
            &label,
            cx + pad_x,
            row_y + (item_h - font_size) / 2,
            font_size,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(filter: &str, names: &[&str]) -> PaletteState {
        let index = names
            .iter()
            .enumerate()
            .map(|(i, n)| NoteEntry {
                name: n.to_string(),
                path: PathBuf::from(format!("/n{i}.md")),
                display: n.to_string(),
            })
            .collect();
        let mut st = PaletteState {
            open: false,
            filter: filter.to_string(),
            selected: 0,
            index,
            matches: Vec::new(),
            show_create: false,
        };
        recompute(&mut st);
        st
    }

    #[test]
    fn prefix_before_loose_substring() {
        let st = state("pro", &["graph project alpha", "projector", "opromo"]);
        // Prefix wins, then any substring hit (`opromo`), tree order kept.
        assert_eq!(st.matches, vec![1, 0, 2]);
    }

    #[test]
    fn empty_filter_lists_everything_in_order() {
        let st = state("", &["zeta", "alpha", "mid"]);
        assert_eq!(st.matches, vec![0, 1, 2]);
        assert!(!st.show_create);
    }

    #[test]
    fn show_create_only_when_exact_absent() {
        let st = state("beta", &["beta", "beta gamma"]);
        assert!(!st.show_create, "exact name exists");
        assert_eq!(st.matches, vec![0, 1]);

        let st = state("bta", &["beta"]);
        assert!(st.show_create, "no exact `bta` note");
    }

    #[test]
    fn trailing_md_counts_as_exact() {
        let st = state("beta.md", &["beta"]);
        assert!(!st.show_create);
        assert_eq!(st.matches, vec![0]);
    }

    #[test]
    fn duplicate_stems_get_parent_prefix() {
        let root = std::env::temp_dir().join("rg_idx_test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("alpha.md"), "x").unwrap();
        std::fs::write(root.join("sub").join("alpha.md"), "x").unwrap();
        std::fs::write(root.join("sub").join("solo.md"), "x").unwrap();

        let index = build_index(&root);
        let dup: Vec<&str> = index.iter().map(|e| e.display.as_str()).collect();
        assert!(dup.contains(&"sub/alpha"), "dup gets parent: {dup:?}");
        assert_eq!(index.iter().filter(|e| e.name == "alpha").count(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }
}