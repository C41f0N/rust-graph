use raylib::prelude::*;
use std::path::PathBuf;

mod config;
mod editor;
mod filesystem;
mod graph;

use graph::processing::*;

fn main() {
    let height = config::HEIGHT;
    let width = config::WIDTH;

    // Parse CLI arg for the directory
    let args: Vec<String> = std::env::args().collect();
    let dir_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        eprintln!("Usage: {} <directory>", args[0]);
        std::process::exit(1);
    };

    if !dir_path.is_dir() {
        eprintln!("Error: '{}' is not a directory", dir_path.display());
        std::process::exit(1);
    }

    let mut editor_open = false;
    let editor_dimentions = Vector2::new(0.8, 0.8);

    // 1. Initialize the Raylib window and context
    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Raylib Nodes")
        .build();
    rl.set_exit_key(Some(KeyboardKey::KEY_NULL));

    rl.set_target_fps(60);

    // Store the directory and generate nodes from it
    *DIR_PATH.write().unwrap() = dir_path.clone();
    generate_nodes_from_directory(&dir_path);

    let mut editor_was_open = false;

    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Capture editor state BEFORE input handling
        let was_open = editor_was_open;

        // Read user input
        if editor_open {
            editor::input_handler::handle_input(&mut rl);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            editor_open = false;
        }

        graph::input_handler::handle_input(&mut rl, &mut editor_open);
        update_forces(&mut rl);

        // Detect transitions AFTER all input has been processed
        if editor_open && !was_open {
            if let Some(idx) = *EDITING_NODE.read().unwrap() {
                let nodes = NODES.read().unwrap();
                if idx < nodes.len() {
                    let path = nodes[idx].path.clone();
                    drop(nodes);
                    editor::buffer::load_from_file(&path);
                }
            }
        }

        if !editor_open && was_open {
            if let Some(idx) = *EDITING_NODE.read().unwrap() {
                let nodes = NODES.read().unwrap();
                if idx < nodes.len() {
                    let path = nodes[idx].path.clone();
                    drop(nodes);
                    editor::buffer::save_to_file(&path);
                    rebuild_edges();
                }
            }
            *EDITING_NODE.write().unwrap() = None;
        }

        editor_was_open = editor_open;

        let mut d = rl.begin_drawing(&thread);

        graph::renderer::draw(&mut d);

        editor::renderer::draw(d, editor_open, editor_dimentions);
    }
}

