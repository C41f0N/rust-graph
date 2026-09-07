use raylib::prelude::*;
use std::path::PathBuf;

mod config;
mod editor;
mod filesystem;
mod frontmatter;
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
    let editor_dimentions = Vector2::new(
        config::EDITOR_PANEL_FRACTION,
        config::EDITOR_PANEL_FRACTION,
    );

    // 1. Initialize the Raylib window and context
    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Raylib Nodes")
        .build();
    rl.set_exit_key(Some(KeyboardKey::KEY_NULL));

    rl.set_target_fps(60);

    // Capture raylib's built-in font so node labels can be drawn at subpixel
    // positions even before the user picks a custom font in the settings.
    editor::text::capture_default_font(rl.get_font_default());

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

        // A finished header-picker dialog (background thread) left a result: copy
// the chosen file into the project's assets/ folder and write it into the
// note's frontmatter header. Runs here, on the main thread, because both the
// file ops and the GL texture load must not race the render loop.
if let Some((idx, src)) = HEADER_PICK_RESULT.write().unwrap().take() {
    let root = project_root();
    if let Some(dst) = filesystem::copy_file_unique(&src, &root.join("assets")) {
        if let Ok(rel) = dst.strip_prefix(&root) {
            let header_rel = rel.to_string_lossy().to_string();
            if attach_header(idx, &format!("[[{}]]", header_rel)) {
                editor::images::ensure_loaded(&mut rl, &thread, &dst);
            }
        }
    }
}

// A "Set Header Image" click: spawn the (blocking) native file dialog on a
// background thread so the app keeps redrawing; the chosen path comes back
// through HEADER_PICK_RESULT above.
if let Some(idx) = HEADER_PICK_REQUEST.write().unwrap().take() {
    if !HEADER_PICK_ACTIVE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        std::thread::spawn(move || {
            let chosen = rfd::FileDialog::new()
                .set_title("Select header file")
                .add_filter(
                    "Images",
                    &["png", "jpg", "jpeg", "gif", "bmp", "tga", "ico"],
                )
                .pick_file();
            *HEADER_PICK_RESULT.write().unwrap() = chosen.map(|p| (idx, p));
            HEADER_PICK_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }
}

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

        // Preload images referenced by the note the editor is showing, but
        // only on lines the user can actually see (the renderer records which
        // source lines it drew last frame). A two-line margin keeps images
        // decoded before they scroll into view. Needs the raylib handle, so it
        // happens here before drawing on the GL context thread.
        if editor_open {
            let range = *editor::buffer::DRAW_LINE_RANGE.read().unwrap();
            let buf = editor::buffer::BUFFER.read().unwrap();
            if let Some((start, end)) = editor::buffer::clamp_line_range(
                range.0.saturating_sub(2),
                range.1.saturating_add(2),
                buf.len(),
            ) {
                for line in &buf[start..=end] {
                    for target in filesystem::parse_links(line) {
                        if editor::images::is_image_target(&target) {
                            if let Some(path) = editor::images::resolve_path(&target) {
                                editor::images::ensure_loaded(&mut rl, &thread, &path);
                            }
                        }
                    }
                }
            }
        }

        // Preload node header images (frontmatter `header:` targets) for the
        // graph, but only for nodes inside the camera's viewport plus a small
        // margin, so panning through a large graph never decodes offscreen
        // images. Headers render from small circular thumbnails (persisted
        // under assets/thumbnails/), so after the first visit this is just a
        // cheap 256x256 PNG load; the thumbnail cache short-circuits once a
        // texture is loaded.
        if !editor_open {
            let bounds = graph::renderer::world_view_bounds();
            let nodes = NODES.read().unwrap();
            for node in nodes.iter() {
                if !graph::renderer::circle_intersects_rect(node.position, node.radius, bounds) {
                    continue;
                }
                if let Some(header) = &node.header {
                    if editor::images::is_image_target(header) {
                        if let Some(path) = resolve_header_path(header) {
                            editor::images::ensure_thumb_loaded(&mut rl, &thread, &path);
                        }
                    }
                }
            }
        }

        let mut d = rl.begin_drawing(&thread);

        graph::renderer::draw(&mut d);

        editor::renderer::draw(d, editor_open, editor_dimentions);
    }
}

