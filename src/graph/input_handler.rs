use crate::graph::processing::*;
use crate::graph::renderer::*;
use raylib::prelude::*;
use std::cell::Cell;

// Thread-local variables persist across calls on the same thread safely!
thread_local! {
    static LAST_CLICK_TIME: Cell<f64> = Cell::new(0.0);
    static LAST_CLICK_NODE: Cell<Option<usize>> = Cell::new(None);
}
pub fn handle_input(rl: &mut RaylibHandle, editor_open: &mut bool) {
    let mut nodes = NODES.write().unwrap();
    let mut dragging_node = DRAGGING_NODE.write().unwrap();
    let mut hover_node = HOVER_NODE.write().unwrap();
    let camera = CAMERA.read().unwrap();

    let mouse_pos = rl.get_screen_to_world2D(rl.get_mouse_position(), *camera);

    drop(camera);

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
            // Read values using .get()
            let last_node = LAST_CLICK_NODE.get();
            let last_time = LAST_CLICK_TIME.get();

            if last_node == Some(i) && (current_time - last_time) < 0.3 {
                *editor_open = true;
                LAST_CLICK_NODE.set(None); // Update using .set()
            } else {
                *dragging_node = Some(i);
                LAST_CLICK_TIME.set(current_time);
                LAST_CLICK_NODE.set(Some(i));
            }
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
