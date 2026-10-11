// Bottom shortcut-helper bar: a slim strip along the bottom edge of the
// window that shows the keys that will actually fire in the current state.
// "Current state" is resolved from the existing globals (no new state
// machinery), mirroring the input-dispatch order exactly — the palette runs
// first and swallows everything, then the graph handler's modals in the order
// they early-return, then the editor's own modals. The bar therefore always
// answers "what can I press right now" with keys that genuinely work.
//
// The bar is view-only: it is drawn last (under the palette's dimming) and
// never consumes input or offers hit targets.

use raylib::prelude::*;

use crate::config;
use crate::editor::text;

type Chip = (&'static str, &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpContext {
    GlobalPalette,
    GraphRename,
    GraphAddNote,
    Settings,
    ForcePanel,
    GraphDeleteConfirm,
    GraphContextMenu,
    GraphBase,
    EditorCreatingNode,
    EditorPlaceholder,
    EditorTitleRename,
    EditorNav,
    CommandPalette,
    Autocomplete,
    EditorEdit,
}

// The flags a frame exposes to the helper. Everything is pulled from the
// existing statics; the struct exists so resolution can be a pure function
// (and therefore unit-testable without touching process globals).
#[derive(Default)]
pub struct StateFlags {
    pub palette_open: bool,
    pub editor_open: bool,
    pub has_node: bool,
    pub creating_node: bool,
    pub title_rename_active: bool,
    pub nav_mode: bool,
    pub command_palette_active: bool,
    pub autocomplete_active: bool,
    pub renaming: bool,
    pub adding_note: bool,
    pub settings_open: bool,
    pub force_panel_open: bool,
    pub delete_pending: bool,
    pub context_menu_open: bool,
}

fn current(editor_open: bool) -> StateFlags {
    StateFlags {
        palette_open: crate::global_palette::STATE.read().unwrap().open,
        editor_open,
        has_node: crate::editor::tabs::has_any(),
        creating_node: crate::editor::CREATING_NODE.load(std::sync::atomic::Ordering::Relaxed),
        title_rename_active: crate::editor::TITLE_RENAME_ACTIVE
            .load(std::sync::atomic::Ordering::Relaxed),
        nav_mode: crate::editor::NAV_MODE.load(std::sync::atomic::Ordering::Relaxed),
        command_palette_active: crate::editor::command::COMMAND_PALETTE
            .read()
            .unwrap()
            .active,
        autocomplete_active: crate::editor::autocomplete::AUTOCOMPLETE
            .read()
            .unwrap()
            .active,
        renaming: *crate::graph::processing::RENAMING.read().unwrap(),
        adding_note: *crate::graph::processing::ADDING_NOTE.read().unwrap(),
        settings_open: *crate::graph::settings::SETTINGS_OPEN.read().unwrap(),
        force_panel_open: *crate::graph::processing::SHOW_FORCE_PANEL.read().unwrap(),
        delete_pending: *crate::graph::processing::DELETE_PENDING.read().unwrap(),
        context_menu_open: crate::graph::processing::CONTEXT_NODE
            .read()
            .unwrap()
            .is_some()
            || *crate::graph::processing::CONTEXT_EMPTY.read().unwrap(),
    }
}

// Priority chain, highest first. It mirrors the input handlers' early-return
// order so the shown keys always match the keys that dispatch.
pub fn resolve(editor_open: bool) -> HelpContext {
    resolve_from(&current(editor_open))
}

fn resolve_from(f: &StateFlags) -> HelpContext {
    // The Ctrl+K palette runs before every other handler and swallows input.
    if f.palette_open {
        return HelpContext::GlobalPalette;
    }
    if !f.editor_open {
        // Graph-handler order: rename / add-note prompts first, then the
        // settings dialog, then the force panel, delete confirm and menus.
        if f.renaming {
            return HelpContext::GraphRename;
        }
        if f.adding_note {
            return HelpContext::GraphAddNote;
        }
        if f.settings_open {
            return HelpContext::Settings;
        }
        if f.force_panel_open {
            return HelpContext::ForcePanel;
        }
        if f.delete_pending {
            return HelpContext::GraphDeleteConfirm;
        }
        if f.context_menu_open {
            return HelpContext::GraphContextMenu;
        }
        return HelpContext::GraphBase;
    }
    // Editor-handler order: no-node placeholder, then title rename, then nav
    // mode, then the command/autocomplete popups, then plain edit mode.
    if !f.has_node {
        if f.creating_node {
            return HelpContext::EditorCreatingNode;
        }
        return HelpContext::EditorPlaceholder;
    }
    if f.title_rename_active {
        return HelpContext::EditorTitleRename;
    }
    if f.nav_mode {
        return HelpContext::EditorNav;
    }
    if f.command_palette_active {
        return HelpContext::CommandPalette;
    }
    if f.autocomplete_active {
        return HelpContext::Autocomplete;
    }
    HelpContext::EditorEdit
}

// The default (no sub-graph stack) graph view: "up level" is a no-op at the
// root, so it only appears once a stack exists (GRAPH_BASE_FULL).
const GRAPH_BASE: &[Chip] = &[
    ("E", "open"),
    ("N", "new"),
    ("Del", "delete"),
    ("J", "journal"),
    ("F", "forces"),
    ("Ctrl+K", "open note"),
];

const GRAPH_BASE_FULL: &[Chip] = &[
    ("E", "open"),
    ("N", "new"),
    ("Del", "delete"),
    ("J", "journal"),
    ("F", "forces"),
    ("Alt+Left", "up level"),
    ("Ctrl+K", "open note"),
];

// Nav mode with a dated journal note open: the day-step keys only exist
// there, so they appear only then (swap-in replaces the plain EditorNav bar).
const EDITOR_NAV_JOURNAL: &[Chip] = &[
    ("Enter/E", "edit"),
    ("G", "to graph"),
    ("Q", "fullscreen"),
    ("J", "today"),
    ("[ / ]", "prev/next"),
    ("Up/Down/Home/End", "move"),
];

// The keycap/action rows for each state. Keys are ASCII: the SDF fonts the UI
// draws with are not guaranteed to carry the arrows/⌫ glyphs.
pub fn chips(ctx: HelpContext) -> &'static [Chip] {
    match ctx {
        HelpContext::GlobalPalette => &[
            ("Up/Down/Tab", "next"),
            ("Enter", "open"),
            ("Esc", "close"),
            ("Ctrl+K", "close"),
        ],
        HelpContext::GraphRename => &[
            ("Enter", "rename"),
            ("Backspace", "delete char"),
            ("Esc", "cancel"),
        ],
        HelpContext::GraphAddNote => &[
            ("Enter", "create"),
            ("Backspace", "delete char"),
            ("Esc", "cancel"),
        ],
        HelpContext::Settings => &[("Esc", "close")],
        HelpContext::ForcePanel => &[("F", "hide"), ("Esc", "dismiss")],
        HelpContext::GraphDeleteConfirm => &[
            ("Y", "delete"),
            ("N", "cancel"),
            ("Esc", "cancel"),
        ],
        HelpContext::GraphContextMenu => &[("Esc", "dismiss")],
        HelpContext::GraphBase => GRAPH_BASE,
        HelpContext::EditorCreatingNode => &[
            ("Enter", "create"),
            ("Backspace", "delete char"),
            ("Esc", "cancel"),
        ],
        HelpContext::EditorPlaceholder => &[("Ctrl+K", "open note")],
        HelpContext::EditorTitleRename => &[("Enter", "commit"), ("Esc", "cancel")],
        HelpContext::EditorNav => &[
            ("Enter/E", "edit"),
            ("G", "to graph"),
            ("Q", "fullscreen"),
            ("J", "journal"),
            ("Up/Down/Home/End", "move"),
        ],
        HelpContext::CommandPalette => &[
            ("Up/Down/Tab", "select"),
            ("Enter", "run"),
            ("Esc", "dismiss"),
        ],
        HelpContext::Autocomplete => &[
            ("Up/Down/Tab", "select"),
            ("Enter", "accept"),
            ("Esc", "dismiss"),
        ],
        HelpContext::EditorEdit => &[
            ("Esc", "nav mode"),
            ("Ctrl+S", "save"),
            ("Ctrl+Z", "undo"),
            ("Ctrl+Y", "redo"),
        ],
    }
}

pub fn draw(d: &mut RaylibDrawHandle, editor_open: bool) {
    let ctx = resolve(editor_open);
    let mut chip_list = chips(ctx);
    if ctx == HelpContext::GraphBase && !crate::graph::processing::NAV_STACK.read().unwrap().is_empty()
    {
        chip_list = GRAPH_BASE_FULL;
    }
    if ctx == HelpContext::EditorNav
        && crate::editor::tabs::active_path()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
            .is_some_and(|n| crate::journal::is_journal_file_name(&n))
    {
        chip_list = EDITOR_NAV_JOURNAL;
    }
    draw_chips(d, chip_list);
}

fn draw_chips(d: &mut RaylibDrawHandle, chip_list: &[Chip]) {
    let w = config::width();
    let h = config::scaled_size(config::HELPBAR_H);
    let y = config::height() - h;

    d.draw_rectangle(0, y, w, h, config::HELPBAR_BG);
    d.draw_line(0, y, w, y, config::HELPBAR_SEP);

    let fs = config::scaled_size(13);
    let inset = config::scaled_size(12);
    let gap = config::scaled_size(8); // keycap -> action label
    let entry_gap = config::scaled_size(20); // chip -> chip
    let pad_x = config::scaled_size(6); // keycap horizontal padding
    let key_h = fs + config::scaled_size(2);
    let key_y = y + (h - key_h) / 2;
    let max_x = w - inset;

    let mut x = inset;
    for (key, action) in chip_list {
        let key_w = text::measure(d, key, fs);
        let pill_w = key_w + pad_x * 2;
        let total = pill_w + gap + text::measure(d, action, fs);
        if x + total > max_x {
            // Narrow window: drop the trailing chips rather than wrapping.
            break;
        }
        d.draw_rectangle(x, key_y, pill_w, key_h, config::HELPBAR_KEY_BG);
        text::draw(d, key, x + pad_x, key_y, fs, config::HELPBAR_KEY_TEXT);
        text::draw(d, action, x + pill_w + gap, key_y, fs, config::HELPBAR_ACTION_TEXT);
        x += total + entry_gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> StateFlags {
        StateFlags::default()
    }

    #[test]
    fn palette_outranks_every_state() {
        // All editor modals up at once: palette still wins.
        let mut f = flags();
        f.palette_open = true;
        f.editor_open = true;
        f.has_node = true;
        f.title_rename_active = true;
        f.nav_mode = true;
        f.command_palette_active = true;
        f.autocomplete_active = true;
        assert_eq!(resolve_from(&f), HelpContext::GlobalPalette);

        // All graph modals up at once: palette still wins.
        let mut g = flags();
        g.palette_open = true;
        g.renaming = true;
        g.adding_note = true;
        g.settings_open = true;
        g.force_panel_open = true;
        g.delete_pending = true;
        g.context_menu_open = true;
        assert_eq!(resolve_from(&g), HelpContext::GlobalPalette);
    }

    // Walk down the graph handler's early-return order with every modal set
    // at once, clearing one at a time so each step proves the next-winner.
    #[test]
    fn graph_modals_resolve_in_dispatch_order() {
        let mut f = flags(); // editor closed by default
        f.renaming = true;
        f.adding_note = true;
        f.settings_open = true;
        f.force_panel_open = true;
        f.delete_pending = true;
        f.context_menu_open = true;
        assert_eq!(resolve_from(&f), HelpContext::GraphRename);

        f.renaming = false;
        assert_eq!(resolve_from(&f), HelpContext::GraphAddNote);

        f.adding_note = false;
        assert_eq!(resolve_from(&f), HelpContext::Settings);

        f.settings_open = false;
        assert_eq!(resolve_from(&f), HelpContext::ForcePanel);

        f.force_panel_open = false;
        assert_eq!(resolve_from(&f), HelpContext::GraphDeleteConfirm);

        f.delete_pending = false;
        assert_eq!(resolve_from(&f), HelpContext::GraphContextMenu);

        f.context_menu_open = false;
        assert_eq!(resolve_from(&f), HelpContext::GraphBase);
    }

    // Same walk for the editor modals (a tab is open, editor modals active).
    #[test]
    fn editor_modals_resolve_in_dispatch_order() {
        let mut f = flags();
        f.editor_open = true;
        f.has_node = true;
        f.title_rename_active = true;
        f.nav_mode = true;
        f.command_palette_active = true;
        f.autocomplete_active = true;
        assert_eq!(resolve_from(&f), HelpContext::EditorTitleRename);

        f.title_rename_active = false;
        assert_eq!(resolve_from(&f), HelpContext::EditorNav);

        f.nav_mode = false;
        assert_eq!(resolve_from(&f), HelpContext::CommandPalette);

        f.command_palette_active = false;
        assert_eq!(resolve_from(&f), HelpContext::Autocomplete);

        f.autocomplete_active = false;
        assert_eq!(resolve_from(&f), HelpContext::EditorEdit);
    }

    #[test]
    fn no_node_shows_placeholder_or_create_prompt() {
        // Even with stray editor modals set, a missing tab wins: nothing else
        // can be bound with no document behind the buffer.
        let mut f = flags();
        f.editor_open = true;
        f.has_node = false;
        f.nav_mode = true;
        f.autocomplete_active = true;
        f.creating_node = true;
        assert_eq!(resolve_from(&f), HelpContext::EditorCreatingNode);

        f.creating_node = false;
        assert_eq!(resolve_from(&f), HelpContext::EditorPlaceholder);

        // And the open-editor branch never leaks into the graph modals.
        f.editor_open = false;
        assert_eq!(resolve_from(&f), HelpContext::GraphBase);
    }

    #[test]
    fn graph_base_hides_up_level_without_a_stack() {
        // chips() is the no-stack default: no Alt+Left. The full variant adds
        // it only while the breadcrumb stack exists.
        assert!(
            chips(HelpContext::GraphBase)
                .iter()
                .all(|(key, _)| *key != "Alt+Left")
        );
        assert!(GRAPH_BASE_FULL.iter().any(|(key, _)| *key == "Alt+Left"));
    }

    // No state may show an empty bar: draw() breaks on an empty chip list
    // only after the fact, so every context must carry at least one chip.
    #[test]
    fn every_context_has_at_least_one_chip() {
        let all = [
            HelpContext::GlobalPalette,
            HelpContext::GraphRename,
            HelpContext::GraphAddNote,
            HelpContext::Settings,
            HelpContext::ForcePanel,
            HelpContext::GraphDeleteConfirm,
            HelpContext::GraphContextMenu,
            HelpContext::GraphBase,
            HelpContext::EditorCreatingNode,
            HelpContext::EditorPlaceholder,
            HelpContext::EditorTitleRename,
            HelpContext::EditorNav,
            HelpContext::CommandPalette,
            HelpContext::Autocomplete,
            HelpContext::EditorEdit,
        ];
        for ctx in all {
            assert!(!chips(ctx).is_empty(), "{ctx:?} must show helpers");
        }
    }
}