use crate::config;
use crate::graph::kb_nav::{
    reticle_world, target_for_center, zoomed_continuously, PAN_PX_PER_SEC, RETICLE_ACTIVE,
    RETICLE_HANDOFF_PX,
};
use crate::graph::processing::*;
use crate::graph::renderer::*;
use crate::graph::settings;
use raylib::prelude::*;
use std::cell::Cell;

thread_local! {
    static LAST_CLICK_TIME: Cell<f64> = Cell::new(0.0);
    static LAST_CLICK_NODE: Cell<Option<usize>> = Cell::new(None);
    // A press on a node arms a possible drag; it only becomes a real drag once
    // the pointer moves past DRAG_THRESHOLD screen pixels, so a plain click
    // (select/double-click-open) never wakes the settled layout.
    static PRESS_NODE: Cell<Option<usize>> = Cell::new(None);
    static PRESS_SCREEN_POS: Cell<Vector2> = Cell::new(Vector2::zero());
}

// Screen-pixel distance the pointer must travel after pressing a node before
// the press is treated as a drag (and the force sim is reheated).
const DRAG_THRESHOLD: f32 = 4.0;

pub fn handle_input(rl: &mut RaylibHandle, editor_open: &mut bool) {
    let mut nodes = NODES.write().unwrap();
    let mut dragging_node = DRAGGING_NODE.write().unwrap();
    let mut hover_node = HOVER_NODE.write().unwrap();
    let mut selected_node = SELECTED_NODE.write().unwrap();
    let mut delete_pending = DELETE_PENDING.write().unwrap();
    let editing_node = EDITING_NODE.write().unwrap();
    let mut adding_note = ADDING_NOTE.write().unwrap();
    let mut adding_name = ADDING_NAME.write().unwrap();
    let mut context_node = CONTEXT_NODE.write().unwrap();
    let mut context_empty = CONTEXT_EMPTY.write().unwrap();
    let mut context_pos = CONTEXT_POS.write().unwrap();
    let mut renaming = RENAMING.write().unwrap();
    let mut rename_name = RENAME_NAME.write().unwrap();
    let dir_path = DIR_PATH.read().unwrap();
    let camera = CAMERA.read().unwrap();

    let mouse_pos = rl.get_screen_to_world2D(rl.get_mouse_position(), *camera);

    drop(camera);

    // ------------------------------------------------------------
    // While the editor is open the graph is inert: the editor owns all
    // keyboard, wheel and click input, so nothing behind it can be selected,
    // dragged or toggled. The one exception is a left click OUTSIDE the
    // editor panel, which dismisses the editor (main.rs saves the buffer on
    // the resulting open->closed transition). Clicks inside the panel are
    // for the editor and never reach the graph.
    // ------------------------------------------------------------

    if *editor_open {
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            let (ex, ey, ew, eh) = crate::editor::panel_bounds();
            let m = rl.get_mouse_position();
            let inside = m.x as i32 >= ex
                && m.x as i32 <= ex + ew
                && m.y as i32 >= ey
                && m.y as i32 <= ey + eh;
            if !inside {
                *editor_open = false;
            }
        }
        // The zoom scale belongs to the graph: never linger over the editor.
        *ZOOM_SCALE_HOVER.write().unwrap() = false;
        *ZOOM_SCALE_DRAGGING.write().unwrap() = false;
        // The keyboard reticle belongs to the graph too.
        *RETICLE_ACTIVE.write().unwrap() = false;
        return;
    }

    // ------------------------------------------------------------
    // Vertical zoom scale: hover state is refreshed every frame (including
    // modal overlays like delete prompts) so the fade-out can never strand it
    // on screen; the drag itself is driven at the end of the frame.
    // ------------------------------------------------------------
    *ZOOM_SCALE_HOVER.write().unwrap() = {
        let (zx, ztop, zw, zh) = zoom_scale_band();
        let hover_m = rl.get_mouse_position();
        hover_m.x as i32 >= zx
            && hover_m.x as i32 <= zx + zw
            && hover_m.y as i32 >= ztop
            && hover_m.y as i32 <= ztop + zh
    };

    // ------------------------------------------------------------
    // Rename-note name prompt mode
    // ------------------------------------------------------------

    if *renaming && !*editor_open {
        // Char input appends to the name
        while let Some(ch) = rl.get_char_pressed() {
            let c = char::from_u32(ch as u32).unwrap();
            if !c.is_control() {
                rename_name.push(c);
            }
        }

        // Backspace removes last char
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
        {
            rename_name.pop();
        }

        // Enter confirms and renames the note
        if rl.is_key_pressed(KeyboardKey::KEY_ENTER)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER)
        {
            let mut index = None;
            if let Some(idx) = *context_node {
                index = Some(idx);
            }
            let name = rename_name.clone();
            if let Some(idx) = index {
                drop(nodes);
                drop(dragging_node);
                drop(hover_node);
                drop(selected_node);
                drop(delete_pending);
                drop(editing_node);
                drop(adding_note);
                drop(adding_name);
                drop(context_node);
                drop(context_pos);
                drop(renaming);
                drop(rename_name);
                drop(dir_path);
                drop(context_empty);
                if rename_node(idx, &name) {
                    *SELECTED_NODE.write().unwrap() = Some(idx);
                }
                // Reset menu state
                *CONTEXT_NODE.write().unwrap() = None;
                *RENAMING.write().unwrap() = false;
                *RENAME_NAME.write().unwrap() = String::new();
                return;
            }
            *renaming = false;
            *rename_name = String::new();
            *context_node = None;
        }

        // Escape cancels the rename
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            *renaming = false;
            *rename_name = String::new();
            *context_node = None;
        }

        return;
    }

    // ------------------------------------------------------------
    // Add-note name prompt mode
    // ------------------------------------------------------------

    if *adding_note && !*editor_open {
        // Char input appends to the name
        while let Some(ch) = rl.get_char_pressed() {
            let c = char::from_u32(ch as u32).unwrap();
            if !c.is_control() {
                adding_name.push(c);
            }
        }

        // Backspace removes last char
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
        {
            adding_name.pop();
        }

        // Enter confirms and creates the note
        if rl.is_key_pressed(KeyboardKey::KEY_ENTER)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER)
        {
            let name = adding_name.clone();
            let name = if name.trim().is_empty() {
                "untitled".to_string()
            } else {
                name.trim().to_string()
            };

            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            let idx = add_node(&dir_path, &name);
            let mut selected_node = SELECTED_NODE.write().unwrap();
            let mut adding_note = ADDING_NOTE.write().unwrap();
            let mut adding_name = ADDING_NAME.write().unwrap();
            *adding_name = String::new();
            *adding_note = false;
            *selected_node = Some(idx);
            return;
        }

        // Escape cancels the prompt
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            *adding_name = String::new();
            *adding_note = false;
        }

        return;
    }

    // ------------------------------------------------------------
    // Settings dialog mode (modal, blocks graph interaction)
    // ------------------------------------------------------------

    let settings_open = *settings::SETTINGS_OPEN.read().unwrap();
    if settings_open && !*editor_open {
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            *settings::SETTINGS_OPEN.write().unwrap() = false;
            return;
        }

        let fonts_len = settings::FONTS.read().unwrap().len();

        // Mouse wheel scrolls the font list.
        let wheel = rl.get_mouse_wheel_move();
        if wheel != 0.0 {
            let max_scroll = fonts_len.saturating_sub(settings::SETTINGS_VISIBLE_ROWS);
            let mut scroll = settings::SETTINGS_SCROLL.write().unwrap();
            *scroll = (*scroll as i32 - wheel as i32).clamp(0, max_scroll as i32) as usize;
            return;
        }

        let mouse = rl.get_mouse_position();
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            let (px, py) = (settings::panel_x(), settings::panel_y());
            let (pw, ph) = (settings::panel_w(), settings::panel_h());
            let rh = settings::row_h();
            let inside = mouse.x as i32 >= px
                && mouse.x as i32 <= px + pw
                && mouse.y as i32 >= py
                && mouse.y as i32 <= py + ph;
            if inside {
                // A press on an option stepper adjusts that setting (and
                // persists it) instead of picking a font. Checked before the
                // font rows, which start lower down anyway.
                let mut handled = false;
                for row in 0..settings::OPTION_ROWS {
                    let (minus, plus) = settings::stepper_rects(row);
                    for (rect, up) in [(minus, true), (plus, false)] {
                        let hit = mouse.x as i32 >= rect.x as i32
                            && mouse.x as i32 <= (rect.x + rect.width) as i32
                            && mouse.y as i32 >= rect.y as i32
                            && mouse.y as i32 <= (rect.y + rect.height) as i32;
                        if hit {
                            settings::nudge_option(row, up);
                            handled = true;
                        }
                    }
                }
                if handled {
                    return;
                }

                // Click on a row picks that font; main.rs loads it next frame.
                let list_top = settings::font_list_top();
                let row = (mouse.y as i32 - list_top) / rh;
                let idx = *settings::SETTINGS_SCROLL.read().unwrap() as i32 + row;
                if row >= 0 && idx >= 0 && (idx as usize) < fonts_len {
                    *settings::SELECTED_FONT.write().unwrap() = Some(idx as usize);
                    *settings::REQUEST_LOAD_FONT.write().unwrap() = Some(idx as usize);
                }
            } else {
                *settings::SETTINGS_OPEN.write().unwrap() = false;
            }
            return;
        }

        return;
    }

    // ------------------------------------------------------------
    // Force debug panel: F toggles it; slider/toggle interaction
    // ------------------------------------------------------------

    // F key shows or hides the panel. Runs even when the panel is behind other
    // overlays so it can always be dismissed.
    if rl.is_key_pressed(KeyboardKey::KEY_F) {
        let mut show = SHOW_FORCE_PANEL.write().unwrap();
        *show = !*show;
        if !*show {
            *ACTIVE_SLIDER.write().unwrap() = None;
        }
    }

    // Escape dismisses the panel (when no other modal owns Escape).
    if *SHOW_FORCE_PANEL.read().unwrap() && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        *SHOW_FORCE_PANEL.write().unwrap() = false;
        *ACTIVE_SLIDER.write().unwrap() = None;
        return;
    }

    // While the panel is open, clicks/drags on its controls are consumed here
    // and never reach node selection below. Graphic clicks outside the panel
    // fall through untouched.
    if *SHOW_FORCE_PANEL.read().unwrap() {
        let mx = rl.get_mouse_position().x;
        let my = rl.get_mouse_position().y;

        // Stray release clears an active drag.
        if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
            *ACTIVE_SLIDER.write().unwrap() = None;
        }

        // Drag: while the left button is held on an active slider, keep
        // tracking the pointer and consume the frame.
        if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
            let active = *ACTIVE_SLIDER.read().unwrap();
            if let Some(idx) = active {
                update_slider_from_mouse(idx, mx, &mut nodes);
                return;
            }
        }

        // Press: start a slider drag or flip the alpha-cooldown switch.
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            if let Some(idx) = hit_test_slider(mx, my) {
                *ACTIVE_SLIDER.write().unwrap() = Some(idx);
                update_slider_from_mouse(idx, mx, &mut nodes);
                return;
            } else if hit_test_alpha_toggle(mx, my) {
                let mut enabled = ALPHA_COOLING_ENABLED.write().unwrap();
                *enabled = !*enabled;
                drop(enabled);
                // Reheat so a graph that was frozen mid-cooldown (or running
                // at full steam) picks up the new setting immediately.
                wake_simulation();
                return;
            } else if hit_test_reset_button(mx, my) {
                // Restore the d3 defaults in place (no respawn): force values,
                // cooldown switch and disc radii change, sim reheats. Needs no
                // guard drops - it only writes the param statics plus the
                // already-locked node slice.
                reset_graph_tweaks(&mut nodes);
                *ACTIVE_SLIDER.write().unwrap() = None;
                return;
            } else if hit_test_respawn_button(mx, my) {
                // Regenerate lays nodes out from scratch and re-locks every
                // graph static, so release the guards held at the top of this
                // function before calling it.
                drop(nodes);
                drop(dragging_node);
                drop(hover_node);
                drop(selected_node);
                drop(delete_pending);
                drop(editing_node);
                drop(adding_note);
                drop(adding_name);
                drop(context_node);
                drop(context_empty);
                drop(context_pos);
                drop(renaming);
                drop(rename_name);
                drop(dir_path);
                respawn_graph();
                return;
            }
        }
    }

    // ------------------------------------------------------------
    // Delete confirmation keys
    // ------------------------------------------------------------

    if *delete_pending && !*editor_open {
        if rl.is_key_pressed(KeyboardKey::KEY_Y) {
            if let Some(idx) = *selected_node {
                drop(nodes);
                drop(dragging_node);
                drop(hover_node);
                drop(selected_node);
                drop(delete_pending);
                drop(editing_node);
                drop(dir_path);
                drop(context_empty);
                remove_node(idx);
                // Re-acquire to clean up
                let mut selected_node = SELECTED_NODE.write().unwrap();
                let mut delete_pending = DELETE_PENDING.write().unwrap();
                *selected_node = None;
                *delete_pending = false;
                return;
            }
            *selected_node = None;
            *delete_pending = false;
        } else if rl.is_key_pressed(KeyboardKey::KEY_N)
            || rl.is_key_pressed(KeyboardKey::KEY_ESCAPE)
        {
            *delete_pending = false;
        }
        return;
    }

    // ------------------------------------------------------------
    // Delete key = start delete confirmation
    // ------------------------------------------------------------

    if !*editor_open
        && selected_node.is_some()
        && (rl.is_key_pressed(KeyboardKey::KEY_DELETE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_DELETE))
    {
        *delete_pending = true;
        return;
    }

    // ------------------------------------------------------------
    // N key = start add-note name prompt
    // ------------------------------------------------------------

    if !*editor_open && rl.is_key_pressed(KeyboardKey::KEY_N) {
        *adding_name = String::new();
        *adding_note = true;
        return;
    }

    // ------------------------------------------------------------
    // J key = open (or create) today's journal note. The journal flow writes
    // NODES (ensure_node_path) and swaps the editor's buffer on the returned
    // frame, so every guard held here must go first.
    // ------------------------------------------------------------

    if !*editor_open && rl.is_key_pressed(KeyboardKey::KEY_J) {
        drop(nodes);
        drop(dragging_node);
        drop(hover_node);
        drop(selected_node);
        drop(delete_pending);
        drop(editing_node);
        drop(adding_note);
        drop(adding_name);
        drop(context_node);
        drop(context_pos);
        drop(renaming);
        drop(rename_name);
        drop(dir_path);
        drop(context_empty);
        let pending = crate::journal::open_today();
        // A brand-new day has no file yet: leave the read/nav default and
        // start typing at once, like creating a note.
        if pending {
            crate::editor::NAV_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        *editor_open = true;
        *crate::graph::processing::CONTEXT_NODE.write().unwrap() = None;
        return;
    }

    // ------------------------------------------------------------
    // Ctrl+C / Ctrl+X / Ctrl+V — copy, cut and paste notes. Copy/Cut grab the
    // selected (else hovered) note into the node clipboard; paste drops it
    // into the folder this graph level is showing (e.g. the sub-graph you
    // navigated into). Both clipboard fns lock NODES, so every guard drops
    // first, exactly like the J journal block above.
    // ------------------------------------------------------------

    let ctrl =
        rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
    if !*editor_open && ctrl {
        let grab_target = (*selected_node).or(*hover_node);
        let wants_grab = rl.is_key_pressed(KeyboardKey::KEY_C) || rl.is_key_pressed(KeyboardKey::KEY_X);
        if wants_grab && grab_target.is_some() {
            let idx = grab_target.unwrap();
            let kind = if rl.is_key_pressed(KeyboardKey::KEY_C) {
                ClipKind::Copy
            } else {
                ClipKind::Cut
            };
            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            drop(context_node);
            drop(context_pos);
            drop(renaming);
            drop(rename_name);
            drop(dir_path);
            drop(context_empty);
            crate::graph::processing::clipboard_grab(idx, kind);
            return;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_V) {
            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            drop(context_node);
            drop(context_pos);
            drop(renaming);
            drop(rename_name);
            drop(dir_path);
            drop(context_empty);
            if let Some(idx) = crate::graph::processing::clipboard_paste() {
                *SELECTED_NODE.write().unwrap() = Some(idx);
            }
            return;
        }
    }

    // Escape dismisses an open context menu
    if (context_node.is_some() || *context_empty) && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        *context_node = None;
        *context_empty = false;
        return;
    }

    // Alt + left arrow leaves the current sub-graph for its parent, mirroring
    // the Alt + left-click breadcrumb shortcut below for keyboard users. At
    // the root the stack is empty, so this is a no-op.
    if (rl.is_key_down(KeyboardKey::KEY_LEFT_ALT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT))
        && rl.is_key_pressed(KeyboardKey::KEY_LEFT)
    {
        let parent = {
            let stack = NAV_STACK.read().unwrap();
            stack.len().checked_sub(1)
        };
        if let Some(level) = parent {
            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            drop(context_node);
            drop(context_pos);
            drop(renaming);
            drop(rename_name);
            drop(dir_path);
            drop(context_empty);
            navigate_to_level(level);
            generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
        }
        return;
    }

    // ------------------------------------------------------------
    // Mouse click handling
    // ------------------------------------------------------------

    let screen_mouse = rl.get_mouse_position();

    // Right-click: open the context menu for a node under the cursor, or an
    // empty-space menu (Add Node) when the cursor is over nothing.
    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT) {
        let mut clicked: Option<usize> = None;
        for (i, node) in nodes.iter().enumerate() {
            if (node.position - mouse_pos).length() <= node.radius {
                clicked = Some(i);
                break;
            }
        }
        if let Some(i) = clicked {
            *selected_node = Some(i);
            *context_node = Some(i);
            *context_pos = (screen_mouse.x as i32, screen_mouse.y as i32);
            *renaming = false;
            *context_empty = false;
        } else {
            *context_node = None;
            *context_empty = true;
            *context_pos = (screen_mouse.x as i32, screen_mouse.y as i32);
        }
    }

    // Middle-click on a node that owns a sub-graph navigates into it.
    if context_node.is_none()
        && !*context_empty
        && rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_MIDDLE)
    {
        let mut nav_name: Option<String> = None;
        for node in nodes.iter() {
            if (node.position - mouse_pos).length() <= node.radius {
                if node.has_subgraph {
                    nav_name = Some(node.name.clone());
                }
                break;
            }
        }
        if let Some(name) = nav_name {
            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            drop(context_node);
            drop(context_pos);
            drop(renaming);
            drop(rename_name);
            drop(dir_path);
            drop(context_empty);
            navigate_into(&name);
            // Regenerate nodes/edges (also resets camera + zoom).
            generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
            return;
        }
    }

    // Alt + left click leaves the current sub-graph for its parent.
    if (rl.is_key_down(KeyboardKey::KEY_LEFT_ALT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT))
        && rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
    {
        let parent = {
            let stack = NAV_STACK.read().unwrap();
            stack.len().checked_sub(1)
        };
        if let Some(level) = parent {
            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            drop(context_node);
            drop(context_pos);
            drop(renaming);
            drop(rename_name);
            drop(dir_path);
            drop(context_empty);
            navigate_to_level(level);
            generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
        }
        return;
    }

    // While a context menu is open, a left click either picks an item or
    // dismisses the menu (anything that isn't the menu closes it).
    if (context_node.is_some() || *context_empty)
        && rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
    {
        let (mx, my) = *context_pos;
        let mw = config::scaled_size(config::CONTEXT_MENU_W);
        let mh = config::scaled_size(config::CONTEXT_MENU_ITEM_H);

        if *context_empty {
            // Empty-space menu: Add Node, plus Paste when the clipboard holds
            // a note (both rows mirror the renderer's empty menu exactly).
            let has_clip = crate::graph::processing::CLIPBOARD.read().unwrap().is_some();
            let click = rl.get_mouse_position();
            let clicked = |r: i32| {
                click.x as i32 >= mx
                    && click.x as i32 <= mx + mw
                    && click.y as i32 >= my + r * mh
                    && click.y as i32 <= my + r * mh + mh
            };
            if clicked(0) {
                *adding_name = String::new();
                *adding_note = true;
            } else if has_clip && clicked(1) {
                drop(nodes);
                drop(dragging_node);
                drop(hover_node);
                drop(selected_node);
                drop(delete_pending);
                drop(editing_node);
                drop(adding_note);
                drop(adding_name);
                drop(context_node);
                drop(context_pos);
                drop(renaming);
                drop(rename_name);
                drop(dir_path);
                drop(context_empty);
                *CONTEXT_NODE.write().unwrap() = None;
                *CONTEXT_EMPTY.write().unwrap() = false;
                if let Some(idx) = crate::graph::processing::clipboard_paste() {
                    *SELECTED_NODE.write().unwrap() = Some(idx);
                }
                return;
            }
            *context_node = None;
            *context_empty = false;
            return;
        }

        // Hit-test against the same row list the renderer draws, so a click
        // always targets the exact row under the cursor.
        let rows: Vec<ContextRow> = context_menu_rows(*context_node, &nodes);
        let mut clicked_row: Option<ContextRow> = None;
        for (i, row) in rows.iter().copied().enumerate() {
            let y = my + i as i32 * mh;
            if screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + mw
                && screen_mouse.y as i32 >= y
                && screen_mouse.y as i32 <= y + mh
            {
                clicked_row = Some(row);
                break;
            }
        }

        match clicked_row {
            Some(ContextRow::Rename) => {
                let idx = *context_node;
                if let Some(idx) = idx {
                    if let Some(node) = nodes.get(idx) {
                        *rename_name = node.name.clone();
                    }
                }
                *renaming = true;
                return;
            }
            Some(ContextRow::Delete) => {
                if let Some(idx) = *context_node {
                    *selected_node = Some(idx);
                    *delete_pending = true;
                }
                *context_node = None;
                return;
            }
            Some(ContextRow::Copy) => {
                let idx = *context_node;
                *context_node = None;
                if let Some(idx) = idx {
                    drop(nodes);
                    drop(dragging_node);
                    drop(hover_node);
                    drop(selected_node);
                    drop(delete_pending);
                    drop(editing_node);
                    drop(adding_note);
                    drop(adding_name);
                    drop(context_node);
                    drop(context_pos);
                    drop(renaming);
                    drop(rename_name);
                    drop(dir_path);
                    drop(context_empty);
                    crate::graph::processing::clipboard_grab(idx, ClipKind::Copy);
                }
                return;
            }
            Some(ContextRow::Cut) => {
                let idx = *context_node;
                *context_node = None;
                if let Some(idx) = idx {
                    drop(nodes);
                    drop(dragging_node);
                    drop(hover_node);
                    drop(selected_node);
                    drop(delete_pending);
                    drop(editing_node);
                    drop(adding_note);
                    drop(adding_name);
                    drop(context_node);
                    drop(context_pos);
                    drop(renaming);
                    drop(rename_name);
                    drop(dir_path);
                    drop(context_empty);
                    crate::graph::processing::clipboard_grab(idx, ClipKind::Cut);
                }
                return;
            }
            Some(ContextRow::OpenSubGraph) => {
                // Navigate into the sub-graph folder immediately.
                let idx = *context_node;
                *context_node = None;
                if let Some(idx) = idx {
                    let name = nodes.get(idx).map(|n| n.name.clone()).unwrap_or_default();
                    drop(nodes);
                    drop(dragging_node);
                    drop(hover_node);
                    drop(selected_node);
                    drop(delete_pending);
                    drop(editing_node);
                    drop(adding_note);
                    drop(adding_name);
                    drop(context_node);
                    drop(context_pos);
                    drop(renaming);
                    drop(rename_name);
                    drop(dir_path);
                    drop(context_empty);
                    if !name.is_empty() {
                        navigate_into(&name);
                        // Regenerate nodes/edges (also resets camera + zoom).
                        generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
                    }
                }
                return;
            }
            Some(ContextRow::FoldSubGraph) => {
                // Pack the node's reachable chain into its companion folder
                // (see processing::fold_node); the in-place refresh the fold
                // performs removes the packed notes without respawning the
                // graph.
                let idx = *context_node;
                *context_node = None;
                if let Some(idx) = idx {
                    drop(nodes);
                    drop(dragging_node);
                    drop(hover_node);
                    drop(selected_node);
                    drop(delete_pending);
                    drop(editing_node);
                    drop(adding_note);
                    drop(adding_name);
                    drop(context_node);
                    drop(context_pos);
                    drop(renaming);
                    drop(rename_name);
                    drop(dir_path);
                    drop(context_empty);
                    let dir = DIR_PATH.read().unwrap().clone();
                    fold_node(&dir, idx);
                }
                return;
            }
            Some(ContextRow::UnwrapSubGraph) => {
                // Expand a fold-created sub-graph back into this directory; the
                // in-place refresh re-seeds only the returning children.
                let idx = *context_node;
                *context_node = None;
                if let Some(idx) = idx {
                    drop(nodes);
                    drop(dragging_node);
                    drop(hover_node);
                    drop(selected_node);
                    drop(delete_pending);
                    drop(editing_node);
                    drop(adding_note);
                    drop(adding_name);
                    drop(context_node);
                    drop(context_pos);
                    drop(renaming);
                    drop(rename_name);
                    drop(dir_path);
                    drop(context_empty);
                    let dir = DIR_PATH.read().unwrap().clone();
                    unwrap_node(&dir, idx);
                }
                return;
            }
            Some(ContextRow::SetHeader) => {
                // Defer to main.rs: it spawns the native file dialog on a worker
                // thread, then copies the pick into assets/ and attaches it as
                // the note header. Skip if a dialog is already in flight.
                if let Some(idx) = *context_node {
                    *selected_node = Some(idx);
                    if !HEADER_PICK_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                        *HEADER_PICK_REQUEST.write().unwrap() = Some(idx);
                    }
                }
                *context_node = None;
                return;
            }
            None => {
                // Click anywhere else dismisses the menu.
                *context_node = None;
                return;
            }
        }
    }

    // A left press on the vertical zoom scale belongs to it alone: don't let
    // it select nodes, arm drags or toggle anything. The drag block at the end
    // of the frame drives the camera from the pointer instead.
    let zoom_band_press = {
        let (zx, ztop, zw, zh) = zoom_scale_band();
        let click = rl.get_mouse_position();
        click.x as i32 >= zx
            && click.x as i32 <= zx + zw
            && click.y as i32 >= ztop
            && click.y as i32 <= ztop + zh
    };

    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) && !zoom_band_press {
        let click = rl.get_mouse_position();

        // Settings button (screen space, top-left) toggles the dialog.
        if click.x as i32 >= settings::settings_button_x()
            && click.x as i32 <= settings::settings_button_x() + config::scaled_size(settings::SETTINGS_BUTTON_W)
            && click.y as i32 >= settings::SETTINGS_BUTTON_Y
            && click.y as i32 <= settings::SETTINGS_BUTTON_Y + config::scaled_size(settings::SETTINGS_BUTTON_H)
        {
            let mut open = settings::SETTINGS_OPEN.write().unwrap();
            *open = !*open;
            return;
        }

        // Breadcrumb trail click: set by the renderer in the same frame
        // where it draws the breadcrumb (it has text::measure access).
        let bc_click = BREADCRUMB_CLICK.write().unwrap().take();
        if let Some(level) = bc_click {
            drop(nodes);
            drop(dragging_node);
            drop(hover_node);
            drop(selected_node);
            drop(delete_pending);
            drop(editing_node);
            drop(adding_note);
            drop(adding_name);
            drop(context_node);
            drop(context_pos);
            drop(renaming);
            drop(rename_name);
            drop(dir_path);
                drop(context_empty);
            navigate_to_level(level);
            // Regenerate nodes/edges (also resets camera + zoom).
            generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
            return;
        }

        let current_time = rl.get_time();
        let mut clicked_node: Option<usize> = None;

        for (i, node) in nodes.iter().enumerate() {
            let dist = (node.position - mouse_pos).length();
            if dist <= node.radius {
                clicked_node = Some(i);
                break;
            }
        }

        if let Some(i) = clicked_node {
            let last_node = LAST_CLICK_NODE.get();
            let last_time = LAST_CLICK_TIME.get();

            if last_node == Some(i) && (current_time - last_time) < 0.3 {
                // Double-click: open the node in the editor as a tab
                // (deduplicated by path).
                let (path, name) = {
                    let n = &nodes[i];
                    (n.path.clone(), n.name.clone())
                };
                *editor_open = true;
                crate::editor::tabs::open(&path, &name);
                *context_node = None;
                LAST_CLICK_NODE.set(None);
            } else {
                // Single click: select + arm a drag. The drag (and the sim
                // wake) is held until the pointer actually moves past
                // DRAG_THRESHOLD, so merely clicking to select or open a note
                // leaves the settled layout alone instead of relaying it out.
                *selected_node = Some(i);
                *delete_pending = false;
                PRESS_NODE.set(Some(i));
                PRESS_SCREEN_POS.set(rl.get_mouse_position());
                LAST_CLICK_TIME.set(current_time);
                LAST_CLICK_NODE.set(Some(i));
            }
        } else {
            // Click on empty space: deselect
            *selected_node = None;
            *delete_pending = false;
        }
    }

    // A press on a node armed a possible drag; promote it (and heat the sim to
    // the gentle drag level) only once the pointer has really moved. Tracked in
    // screen pixels, so the threshold behaves the same at every zoom.
    if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
        if let Some(i) = PRESS_NODE.get() {
            let m = rl.get_mouse_position();
            if (m - PRESS_SCREEN_POS.get()).length() >= DRAG_THRESHOLD {
                PRESS_NODE.set(None);
                *dragging_node = Some(i);
                crate::graph::processing::reheat_drag();
            }
        }
    }

    if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
        *dragging_node = None;
        PRESS_NODE.set(None);
    }

    if let Some(i) = *dragging_node {
        nodes[i].position = mouse_pos;
        nodes[i].velocity = Vector2::zero();
    }

    // ------------------------------------------------------------
    // Keyboard navigation: a reticle pinned to the centre of the view stands
    // in for the pointer. W/A/S/D pan the graph under it, holding C zooms in
    // and Shift+C zooms out (continuous, anchored on the reticle), and Space /
    // V / R act on the node under it. Esc or any real mouse motion hands the
    // pointer back and hides the reticle.
    // ------------------------------------------------------------

    let nav_x =
        rl.is_key_down(KeyboardKey::KEY_D) as i32 - rl.is_key_down(KeyboardKey::KEY_A) as i32;
    let nav_y =
        rl.is_key_down(KeyboardKey::KEY_S) as i32 - rl.is_key_down(KeyboardKey::KEY_W) as i32;
    let zoom_held = !ctrl && rl.is_key_down(KeyboardKey::KEY_C);
    let shift_held = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);

    {
        let mut reticle = RETICLE_ACTIVE.write().unwrap();
        // WASD movement or holding a zoom key engages keyboard navigation.
        if nav_x != 0 || nav_y != 0 || zoom_held {
            *reticle = true;
        }
        // Esc, or any real mouse motion/press, hands the pointer back.
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE)
            || rl.get_mouse_delta().length() > RETICLE_HANDOFF_PX
            || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
            || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT)
            || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_MIDDLE)
        {
            *reticle = false;
        }
    }
    let reticle_on = *RETICLE_ACTIVE.read().unwrap();

    // Hover node code. With keyboard navigation active the reticle (screen
    // centre) is the pointer; otherwise the mouse is.
    *hover_node = None;
    let hover_point = if reticle_on {
        let cam = CAMERA.read().unwrap();
        reticle_world(&cam)
    } else {
        mouse_pos
    };

    for (i, node) in nodes.iter().enumerate() {
        let dist = (node.position - hover_point).length();
        if dist <= node.radius {
            *hover_node = Some(i);
            break;
        }
    }

    // Reticle actions, aimed at the hovered note (else the selected one).
    // Skipped while a context menu is open — that menu owns the keyboard.
    let menu_open = context_node.is_some() || *context_empty;
    let action_target = (*hover_node).or(*selected_node);
    if !*editor_open && !menu_open {
        // Space opens the note in the editor, like a double-click.
        if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
            if let Some(i) = action_target {
                let (path, name) = {
                    let n = &nodes[i];
                    (n.path.clone(), n.name.clone())
                };
                *editor_open = true;
                crate::editor::tabs::open(&path, &name);
                *context_node = None;
            }
            return;
        }

        // V enters the note's sub-graph, like a middle-click (folder nodes
        // only). Ctrl+V stays paste, so require no modifier.
        if rl.is_key_pressed(KeyboardKey::KEY_V) && !ctrl {
            let mut nav_name: Option<String> = None;
            if let Some(i) = action_target {
                if nodes[i].has_subgraph {
                    nav_name = Some(nodes[i].name.clone());
                }
            }
            if let Some(name) = nav_name {
                drop(nodes);
                drop(dragging_node);
                drop(hover_node);
                drop(selected_node);
                drop(delete_pending);
                drop(editing_node);
                drop(adding_note);
                drop(adding_name);
                drop(context_node);
                drop(context_pos);
                drop(renaming);
                drop(rename_name);
                drop(dir_path);
                drop(context_empty);
                navigate_into(&name);
                // Regenerate nodes/edges (also resets camera + zoom).
                generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
                return;
            }
        }

        // R opens the note's context menu at the reticle (the mouse then takes
        // over to pick an item). With no target it opens the empty-space menu,
        // mirroring a right-click on blank canvas.
        if rl.is_key_pressed(KeyboardKey::KEY_R) && !ctrl {
            let (mw, mh) = (
                config::scaled_size(config::CONTEXT_MENU_W),
                config::scaled_size(config::CONTEXT_MENU_ITEM_H),
            );
            let rows = match action_target {
                Some(i) => context_menu_rows(Some(i), &nodes).len() as i32,
                None => {
                    let has_clip = CLIPBOARD.read().unwrap().is_some();
                    if has_clip {
                        2
                    } else {
                        1
                    }
                }
            };
            match action_target {
                Some(i) => {
                    *selected_node = Some(i);
                    *context_node = Some(i);
                    *renaming = false;
                    *context_empty = false;
                }
                None => {
                    *context_node = None;
                    *context_empty = true;
                }
            }
            *context_pos = (
                config::width() / 2 - mw / 2,
                config::height() / 2 - rows * mh / 2,
            );
            return;
        }
    }

    let wheel = rl.get_mouse_wheel_move();

    let mut camera = CAMERA.write().unwrap();

    // Zoom (suppressed while the editor is open: the wheel belongs to the
    // editor's scrollview then).
    if wheel != 0.0 && !*editor_open {
        camera.zoom = (camera.zoom + wheel * 0.1).clamp(0.1, 10.0);
    }

    // Keyboard pan + zoom, anchored on the reticle so the node under it stays
    // put while the camera moves or rescales.
    if reticle_on && !*editor_open {
        if nav_x != 0 || nav_y != 0 {
            let step = PAN_PX_PER_SEC * rl.get_frame_time() / camera.zoom;
            camera.target.x += nav_x as f32 * step;
            camera.target.y += nav_y as f32 * step;
        }
        if zoom_held {
            // Continuous: advance the log band per frame of frame time while
            // the key is held, re-anchoring so the reticle's node stays put.
            let world = reticle_world(&camera);
            let dir = if shift_held { -1.0 } else { 1.0 };
            camera.zoom = zoomed_continuously(camera.zoom, dir, rl.get_frame_time());
            camera.target = target_for_center(world, &camera);
        }
    }

    // Pan
    if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE) {
        let delta = rl.get_mouse_delta();

        camera.target.x -= delta.x / camera.zoom;
        camera.target.y -= delta.y / camera.zoom;
    }

    // Vertical zoom scale drag: pressing the band grabs it (hover was set up
    // top), the pointer then drives the zoom clamped to the band, and a
    // release drops it. Same 0.1..10 camera range as the wheel.
    {
        let (_zx, ztop, _zw, zh) = zoom_scale_band();
        let m = rl.get_mouse_position();
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) && *ZOOM_SCALE_HOVER.read().unwrap() {
            *ZOOM_SCALE_DRAGGING.write().unwrap() = true;
        }
        if !rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
            *ZOOM_SCALE_DRAGGING.write().unwrap() = false;
        }
        if *ZOOM_SCALE_DRAGGING.read().unwrap() {
            let t = 1.0 - ((m.y as i32 - ztop) as f32 / zh as f32).clamp(0.0, 1.0);
            camera.zoom = t_to_zoom(t);
        }
    }

    drop(camera);
}
