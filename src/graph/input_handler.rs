use crate::config;
use crate::filesystem;
use crate::graph::processing::*;
use crate::graph::renderer::*;
use crate::graph::settings;
use raylib::prelude::*;
use std::cell::Cell;

thread_local! {
    static LAST_CLICK_TIME: Cell<f64> = Cell::new(0.0);
    static LAST_CLICK_NODE: Cell<Option<usize>> = Cell::new(None);
}

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
        return;
    }

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
            let (th, rh) = (settings::title_h(), settings::row_h());
            let inside = mouse.x as i32 >= px
                && mouse.x as i32 <= px + pw
                && mouse.y as i32 >= py
                && mouse.y as i32 <= py + ph;
            if inside {
                // Click on a row picks that font; main.rs loads it next frame.
                let list_top = py + th;
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
    // E key = open the hovered (or failing that, the selected) node in the
    // editor as a tab, like a double-click.
    // ------------------------------------------------------------

    if !*editor_open && rl.is_key_pressed(KeyboardKey::KEY_E) {
        let mut target = *hover_node;
        if target.is_none() {
            target = *selected_node;
        }
        if let Some(i) = target {
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

    // Escape dismisses an open context menu
    if (context_node.is_some() || *context_empty) && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        *context_node = None;
        *context_empty = false;
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
            // Empty-space menu: Add Node for now; more items plug in here.
            let clicked_add = screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + mw
                && screen_mouse.y as i32 >= my
                && screen_mouse.y as i32 <= my + mh;
            if clicked_add {
                *adding_name = String::new();
                *adding_note = true;
            }
            *context_node = None;
            *context_empty = false;
            return;
        }

        // Compute which items are visible (mirrors the renderer logic)
        let node_name = context_node
            .and_then(|i| nodes.get(i))
            .map(|n| n.name.clone())
            .unwrap_or_default();
        let subgraph_exists = filesystem::is_dir(&dir_path.join(&node_name));
        let show_create = !subgraph_exists;
        let show_open = subgraph_exists;

        // Row 0: Rename (always present)
        let clicked_rename = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= my
            && screen_mouse.y as i32 <= my + mh;

        // Row 1: Delete (always present)
        let y_delete = my + mh;
        let clicked_delete = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= y_delete
            && screen_mouse.y as i32 <= y_delete + mh;

        // Row 2 (conditional): Create Sub-Graph
        let y_subgraph = y_delete + mh;
        let clicked_create = show_create
            && screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= y_subgraph
            && screen_mouse.y as i32 <= y_subgraph + mh;

        // Row 2/3 (conditional): Open Sub-Graph
        let y_open = if show_create { y_subgraph + mh } else { y_subgraph };
        let clicked_open = show_open
            && screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= y_open
            && screen_mouse.y as i32 <= y_open + mh;

        // Row (last): Set Header (always at 3*ITEM_H; exactly one of the
        // create/open sub-graph rows is shown, so the layout is fixed).
        let y_header = my + 3 * mh;
        let clicked_header = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= y_header
            && screen_mouse.y as i32 <= y_header + mh;

        if clicked_rename {
            let idx = *context_node;
            if let Some(idx) = idx {
                if let Some(node) = nodes.get(idx) {
                    *rename_name = node.name.clone();
                }
            }
            *renaming = true;
            return;
        } else if clicked_delete {
            if let Some(idx) = *context_node {
                *selected_node = Some(idx);
                *delete_pending = true;
            }
            *context_node = None;
            return;
        } else if clicked_create {
            // Create the sub-graph folder, then navigate into it immediately.
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
                    let dir = DIR_PATH.read().unwrap().clone();
                    filesystem::create_dir(&dir.join(&name));
                    drop(dir);
                    navigate_into(&name);
                    // Regenerate nodes/edges (also resets camera + zoom).
                    generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
                }
                return;
            }
            return;
        } else if clicked_open {
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
                return;
            }
            return;
        } else if clicked_header {
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
        } else {
            // Click anywhere else dismisses the menu.
            *context_node = None;
            return;
        }
    }

    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
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
                // Single click: select + start drag
                *selected_node = Some(i);
                *delete_pending = false;
                *dragging_node = Some(i);
                // Nudging a node must wake a settled layout so its neighbours
                // respond to the drag.
                crate::graph::processing::wake_simulation();
                LAST_CLICK_TIME.set(current_time);
                LAST_CLICK_NODE.set(Some(i));
            }
        } else {
            // Click on empty space: deselect
            *selected_node = None;
            *delete_pending = false;
        }
    }

    if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
        *dragging_node = None;
    }

    if let Some(i) = *dragging_node {
        nodes[i].position = mouse_pos;
        nodes[i].velocity = Vector2::zero();
    }

    // Hover node code.
    *hover_node = None;

    for (i, node) in nodes.iter().enumerate() {
        let dist = (node.position - mouse_pos).length();
        if dist <= node.radius {
            *hover_node = Some(i);
            break;
        }
    }

    let wheel = rl.get_mouse_wheel_move();

    let mut camera = CAMERA.write().unwrap();

    // Zoom (suppressed while the editor is open: the wheel belongs to the
    // editor's scrollview then).
    if wheel != 0.0 && !*editor_open {
        camera.zoom = (camera.zoom + wheel * 0.1).clamp(0.1, 10.0);
    }

    // Pan
    if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE) {
        let delta = rl.get_mouse_delta();

        camera.target.x -= delta.x / camera.zoom;
        camera.target.y -= delta.y / camera.zoom;
    }

    drop(camera);
}
