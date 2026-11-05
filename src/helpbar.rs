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
use crate::ui;

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
    EditorPreview,
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
    pub preview_mode: bool,
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
        preview_mode: crate::editor::tabs::active_is_preview(),
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
    if f.preview_mode {
        return HelpContext::EditorPreview;
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
//
// Per-node actions (Space open, V subgraph, R menu, Del delete) are deliberately
// absent: the hover popup already lists them next to the node, so repeating
// them here would just crowd out the global bindings.
const GRAPH_BASE: &[Chip] = &[
    ("WASD/Arrows", "move"),
    ("Q/E", "zoom in/out"),
    ("G", "to editor"),
    ("N", "new"),
    ("Ctrl+C/V/X", "copy/cut/paste"),
    ("J", "journal"),
    ("F", "forces"),
    ("Ctrl+K", "open note"),
];

const GRAPH_BASE_FULL: &[Chip] = &[
    ("WASD/Arrows", "move"),
    ("Q/E", "zoom in/out"),
    ("G", "to editor"),
    ("N", "new"),
    ("Ctrl+C/V/X", "copy/cut/paste"),
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
    ("W/S/Up/Down", "move"),
    ("Ctrl+W", "close tab"),
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
            ("Enter", "delete"),
            ("Esc", "cancel"),
        ],
        HelpContext::GraphContextMenu => &[
            ("W/S", "move"),
            ("Enter", "select"),
            ("C/X/V", "copy/cut/paste"),
            ("Esc", "dismiss"),
        ],
        HelpContext::GraphBase => GRAPH_BASE,
        HelpContext::EditorCreatingNode => &[
            ("Enter", "create"),
            ("Backspace", "delete char"),
            ("Esc", "cancel"),
        ],
        HelpContext::EditorPlaceholder => &[("Ctrl+K", "open note")],
        HelpContext::EditorTitleRename => &[("Enter", "commit"), ("Esc", "cancel")],
        HelpContext::EditorPreview => &[
            ("PgUp/PgDn", "page"),
            ("W/S/Up/Down", "move"),
            ("G", "to graph"),
            ("Q", "fullscreen"),
            ("Ctrl+W", "close tab"),
            ("Esc", "close"),
        ],
        HelpContext::EditorNav => &[
            ("Enter/E", "edit"),
            ("G", "to graph"),
            ("Q", "fullscreen"),
            ("J", "journal"),
            ("W/S/Up/Down", "move"),
            ("Ctrl+W", "close tab"),
            ("Ctrl+Tab", "cycle tabs"),
            ("Alt+1-9", "switch tab"),
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
            ("Ctrl+W", "close tab"),
            ("Ctrl+Tab", "cycle tabs"),
            ("Alt+1-9", "switch tab"),
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
    let inset = config::scaled_size(14);
    let gap = config::scaled_size(8); // keycap -> action label
    let entry_gap = config::scaled_size(22); // chip -> chip
    let pad_x = config::scaled_size(7); // keycap horizontal padding
    let key_h = fs + config::scaled_size(6);
    let key_y = y + (h - key_h) / 2;

    // Measure the whole row first so it can be centred in the bar. On a narrow
    // window the trailing chips are dropped rather than wrapped.
    let mut laid_out = Vec::new();
    let mut total_w = 0;
    for &(key, action) in chip_list {
        let label_w = text::measure(d, action, fs);
        let pill_w = text::measure(d, key, fs) + pad_x * 2;
        let item_w = pill_w + gap + label_w;
        let next = if laid_out.is_empty() {
            item_w
        } else {
            total_w + entry_gap + item_w
        };
        if next + 2 * inset > w {
            break;
        }
        laid_out.push((key, action, pill_w, label_w));
        total_w = next;
    }

    let mut x = ((w - total_w) / 2).max(inset);
    for (key, action, pill_w, label_w) in laid_out {
        ui::keycap(
            d,
            Rectangle::new(x as f32, key_y as f32, pill_w as f32, key_h as f32),
            config::HELPBAR_KEY_BG,
        );
        text::draw(d, key, x + pad_x, key_y + (key_h - fs) / 2, fs, config::HELPBAR_KEY_TEXT);
        text::draw(
            d,
            action,
            x + pill_w + gap,
            key_y + (key_h - fs) / 2,
            fs,
            config::HELPBAR_ACTION_TEXT,
        );
        x += pill_w + gap + label_w + entry_gap;
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
        f.preview_mode = true;
        assert_eq!(resolve_from(&f), HelpContext::EditorPreview);

        f.preview_mode = false;
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

    #[test]
    fn graph_base_leaves_node_actions_to_the_hover_popup() {
        // The hover popup already lists Space/V/R/Del next to the node, so the
        // bottom bar must not repeat them.
        for list in [GRAPH_BASE, GRAPH_BASE_FULL] {
            for key in ["Space", "V", "R", "Del"] {
                assert!(
                    list.iter().all(|(k, _)| *k != key),
                    "{key} belongs to the hover popup, not the helper bar"
                );
            }
        }
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