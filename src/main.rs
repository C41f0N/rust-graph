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

    // Enumerate system fonts for the settings dialog's font picker.
    graph::settings::build_font_list();

    let mut editor_was_open = false;
    // Autosave fires when the buffer has edits and no input has happened for
    // this many milliseconds (a "typing pause").
    let autosave_pause_ms: u64 = 2000;

    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Capture editor state BEFORE input handling
        let was_open = editor_was_open;

        // Read user input
        if editor_open {
            editor::input_handler::handle_input(&mut rl);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            let esc_consumed = {
                let ac = editor::autocomplete::AUTOCOMPLETE.read().unwrap();
                ac.esc_consumed
            };
            if !esc_consumed {
                editor_open = false;
            }
        }

        // Close button in the editor heading bar
        if editor::CLOSE_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed) {
            editor_open = false;
        }

        graph::input_handler::handle_input(&mut rl, &mut editor_open);

        // Ctrl+S = save the open editor buffer immediately
        if editor_open
            && (rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
                || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL))
            && rl.is_key_pressed(KeyboardKey::KEY_S)
        {
            if let Some(idx) = *EDITING_NODE.read().unwrap() {
                let nodes = NODES.read().unwrap();
                if idx < nodes.len() {
                    let path = nodes[idx].path.clone();
                    drop(nodes);
                    editor::buffer::save_to_file(&path);
                    rebuild_edges();
                }
            }
        }

        // Autosave: after a typing pause, save if the buffer is dirty.
        if editor_open && editor::DIRTY.load(std::sync::atomic::Ordering::Relaxed) {
            let now_ms = (rl.get_time() * 1000.0) as u64;
            let last_edit =
                editor::LAST_EDIT_MILLIS.load(std::sync::atomic::Ordering::Relaxed);
            if last_edit > 0 && now_ms.saturating_sub(last_edit) >= autosave_pause_ms {
                if let Some(idx) = *EDITING_NODE.read().unwrap() {
                    let nodes = NODES.read().unwrap();
                    if idx < nodes.len() {
                        let path = nodes[idx].path.clone();
                        drop(nodes);
                        editor::buffer::save_to_file(&path);
                        rebuild_edges();
                    }
                }
            }
        }

        update_forces(&mut rl);

        // Process a font selection from the settings dialog: load (or clear)
        // the active custom font. Needs the raylib handle, so it happens here
        // between input and drawing on the GL context thread.
        let font_request = *graph::settings::REQUEST_LOAD_FONT.read().unwrap();
        if let Some(idx) = font_request {
            let fonts = graph::settings::FONTS.read().unwrap();
            if let Some(fam) = fonts.get(idx) {
                if idx == 0 {
                    editor::text::clear_active_font();
                } else {
                    let loaded = if let Some(path) = &fam.path {
                        editor::text::load_font_set(&mut rl, &thread, &path.to_string_lossy())
                    } else if let Some(data) = &fam.data {
                        editor::text::load_font_set_from_memory(&mut rl, &thread, ".ttf", data)
                    } else {
                        Vec::new()
                    };
                    editor::text::set_active_font(loaded);
                }
                *graph::settings::SELECTED_FONT.write().unwrap() = Some(idx);
            }
            *graph::settings::REQUEST_LOAD_FONT.write().unwrap() = None;
        }

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

