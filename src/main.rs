use raylib::prelude::*;

mod config;
mod editor;
mod graph;

use graph::processing::*;

fn main() {
    let height = config::HEIGHT;
    let width = config::WIDTH;

    let mut editor_open = false;
    let editor_dimentions = Vector2::new(0.8, 0.8);

    // 1. Initialize the Raylib window and context
    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Raylib Nodes")
        .build();
    rl.set_exit_key(Some(KeyboardKey::KEY_NULL));
    rl.set_trace_log(TraceLogLevel::LOG_ERROR);

    rl.set_target_fps(60);

    //let nodes = NODES.read().unwrap();
    //let edges = EDGES.read().unwrap();
    //let mut dragging_node = DRAGGING_NODE.write().unwrap();

    //let mut last_click_time: f64 = 0.0;
    //let mut last_click_node: Option<usize> = None;

    generate_random_nodes();

    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Read user input

        //let mouse_pos = rl.get_mouse_position();

        //if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        //    let current_time = rl.get_time();
        //    let mut clicked_node: Option<usize> = None;

        //    for (i, node) in nodes.iter().enumerate() {
        //        let dist = (node.position - mouse_pos).length();
        //        if dist <= node.radius {
        //            clicked_node = Some(i);
        //            break;
        //        }
        //    }

        //    if let Some(i) = clicked_node {
        //        // Double click check
        //        if last_click_node == Some(i) && (current_time - last_click_time) < 0.3 {
        //            editor_open = true; // double click — editor kholo
        //            last_click_node = None; // reset
        //        } else {
        //            // Single click — drag shuru
        //            *dragging_node = Some(i);
        //            last_click_time = current_time;
        //            last_click_node = Some(i);
        //        }
        //    }
        //}

        //if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
        //    *dragging_node = None;
        //}
        //if editor_open {
        //    editor::input_handler::handle_input(&mut rl);
        //}

        //if let Some(i) = *dragging_node {
        //    nodes[i].position = mouse_pos;
        //    nodes[i].velocity = Vector2::zero();
        //}
        //if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        //    editor_open = false;
        //}
        //

        update_forces(&mut rl);

        let mut d = rl.begin_drawing(&thread);
        graph::renderer::draw(&mut d);
        editor::renderer::draw(d, editor_open, editor_dimentions);
    }
}
