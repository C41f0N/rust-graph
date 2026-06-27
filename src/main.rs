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

    generate_random_nodes();

    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Read user input

        if editor_open {
            editor::input_handler::handle_input(&mut rl);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            editor_open = false;
        }

        graph::input_handler::handle_input(&mut rl, &mut editor_open);
        update_forces(&mut rl);

        let mut d = rl.begin_drawing(&thread);
        graph::renderer::draw(&mut d);
        editor::renderer::draw(d, editor_open, editor_dimentions);
    }
}
