use crate::config;
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
    let mut editing_node = EDITING_NODE.write().unwrap();
    let mut adding_note = ADDING_NOTE.write().unwrap();
    let mut adding_name = ADDING_NAME.write().unwrap();
    let mut context_node = CONTEXT_NODE.write().unwrap();
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
            let (ex, ey, ew, eh) = config::editor_panel_bounds();
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
            let inside = mouse.x as i32 >= px
                && mouse.x as i32 <= px + settings::SETTINGS_PANEL_W
                && mouse.y as i32 >= py
                && mouse.y as i32 <= py + settings::SETTINGS_PANEL_H;
            if inside {
                // Click on a row picks that font; main.rs loads it next frame.
                let list_top = py + settings::SETTINGS_TITLE_H;
                let row = (mouse.y as i32 - list_top) / settings::SETTINGS_ROW_H;
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

    // Escape dismisses an open context menu
    if context_node.is_some() && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        *context_node = None;
        return;
    }

    // ------------------------------------------------------------
    // Mouse click handling
    // ------------------------------------------------------------

    let screen_mouse = rl.get_mouse_position();

    // Right-click: open the context menu for a node under the cursor.
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
        } else {
            *context_node = None;
        }
    }

    // While the context menu is open, a left click either picks an item or
    // dismisses the menu (anything that isn't the menu closes it).
    if context_node.is_some() && rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        let menu_w = 140;
        let item_h = 30;
        let (mx, my) = *context_pos;
        let clicked_rename = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + menu_w
            && screen_mouse.y as i32 >= my
            && screen_mouse.y as i32 <= my + item_h;
        let clicked_delete = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + menu_w
            && screen_mouse.y as i32 >= my + item_h
            && screen_mouse.y as i32 <= my + 2 * item_h;

        if clicked_rename {
            // Start rename prompt pre-filled with the current stem. Keep
            // context_node set so the prompt knows which node to rename.
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
        } else {
            // Click anywhere else dismisses the menu.
            *context_node = None;
            return;
        }
    }

    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        let click = rl.get_mouse_position();

        // Settings button (screen space, top-left) toggles the dialog.
        if click.x as i32 >= settings::SETTINGS_BUTTON_X
            && click.x as i32 <= settings::SETTINGS_BUTTON_X + settings::SETTINGS_BUTTON_W
            && click.y as i32 >= settings::SETTINGS_BUTTON_Y
            && click.y as i32 <= settings::SETTINGS_BUTTON_Y + settings::SETTINGS_BUTTON_H
        {
            let mut open = settings::SETTINGS_OPEN.write().unwrap();
            *open = !*open;
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
                // Double-click: open editor
                *editing_node = Some(i);
                *editor_open = true;
                *context_node = None;
                LAST_CLICK_NODE.set(None);
            } else {
                // Single click: select + start drag
                *selected_node = Some(i);
                *delete_pending = false;
                *dragging_node = Some(i);
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
