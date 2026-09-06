use crate::graph::processing::*;
use crate::graph::renderer::*;
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
    let dir_path = DIR_PATH.read().unwrap();
    let camera = CAMERA.read().unwrap();

    let mouse_pos = rl.get_screen_to_world2D(rl.get_mouse_position(), *camera);

    drop(camera);

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

    // ------------------------------------------------------------
    // Mouse click handling
    // ------------------------------------------------------------

    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
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

    // Zoom
    if wheel != 0.0 {
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
