use raylib::prelude::*;
use std::path::PathBuf;

mod app_config;
mod config;
mod editor;
mod filesystem;
mod frontmatter;
mod graph;
mod sidebar;

use graph::processing::*;

// Navigate the graph into the sub-graph folder of the note currently active
// in the editor. Mirrors double-clicking a sub-graph node, so the breadcrumb
// stack, node set and camera all update identically. Keyed on the active tab's
// document name (path-derived), not a node index: node indices go stale on
// every graph regeneration, the folder on disk never lies.
fn open_editing_subgraph() {
    let name = crate::editor::tabs::active_name();
    if let Some(name) = name {
        let dir = DIR_PATH.read().unwrap();
        if crate::filesystem::is_dir(&dir.join(&name)) {
            drop(dir);
            navigate_into(&name);
            generate_nodes_from_directory(&*DIR_PATH.read().unwrap());
        }
    }
}

fn main() {
    // The global appdata config supplies the initial window size (and, once
    // the window exists, the zoom/font/force settings); missing file = the
    // defaults below.
    let startup = app_config::load();
    let height = startup.window.1.max(config::MIN_WINDOW_H);
    let width = startup.window.0.max(config::MIN_WINDOW_W);

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
    // Panel size is recomputed every frame from the fullscreen toggle; only
    // ever read after the first assignment inside the loop.
    let mut editor_dimentions;

    // 1. Initialize the Raylib window and context
    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Raylib Nodes")
        .resizable()
        .build();
    rl.set_exit_key(Some(KeyboardKey::KEY_NULL));

    rl.set_target_fps(60);

    // Capture raylib's built-in font so node labels can be drawn at subpixel
    // positions even before the user picks a custom font in the settings, and
    // compile the SDF field-text shader used by every loaded font.
    editor::text::init_text(&mut rl, &thread);

    // Store the directory and generate nodes from it
    *DIR_PATH.write().unwrap() = dir_path.clone();

    // On the first launch (no global config yet), import the legacy
    // per-folder `.graph-params` file so previously dialled-in values
    // aren't lost. The global file is written once during the import
    // and the old file is deleted from the folder.
    let cfg = if app_config::has_global_config() {
        startup
    } else {
        app_config::import_legacy_graph_params(&dir_path).unwrap_or(startup)
    };
    // Enumerate system fonts for the settings dialog's font picker.
    graph::settings::build_font_list();
    // Apply first so the (re)written global file reflects the live settings.
    // The font family rides along: apply() points the picker at the persisted
    // name and queues the load, which the loop fulfils on the first frame.
    cfg.apply();
    app_config::save();
    generate_nodes_from_directory(&dir_path);

    let mut editor_was_open = false;
    // Autosave fires when the buffer has edits and no input has happened for
    // this many milliseconds (a "typing pause").
    let autosave_pause_ms: u64 = 2000;

    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Track the live window size so every layout function reads geometry
        // that matches the real (resizable) window. When the window actually
        // reshaped, re-centre the camera offset so the world point under the
        // viewport centre stays put (target is untouched by resizes).
        let (sw, sh) = (rl.get_screen_width(), rl.get_screen_height());
        *config::SCREEN_SIZE.write().unwrap() = (sw, sh);
        if rl.is_window_resized() {
            let mut cam = graph::renderer::CAMERA.write().unwrap();
            cam.offset.x = sw as f32 / 2.0;
            cam.offset.y = sh as f32 / 2.0;
            // The gravity centre moved with the window, so a settled layout no
            // longer reflects the new size: wake the sim to re-balance.
            wake_simulation();
        }

        // Capture editor state BEFORE input handling
        let was_open = editor_was_open;

        // Read user input. A click landing on the view-switcher bar consumes
        // the frame: nothing behind it (editor caret/buttons, graph camera,
        // node drag) gets that press.
        let bar_consumed = sidebar::handle_input(&mut rl, &mut editor_open);
        if !bar_consumed {
            if editor_open {
                editor::input_handler::handle_input(&mut rl);
            }
            if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                let esc_consumed = {
                    let ac = editor::autocomplete::AUTOCOMPLETE.read().unwrap();
                    ac.esc_consumed
                        || editor::command::COMMAND_PALETTE.read().unwrap().esc_consumed
                };
                if !esc_consumed {
                    editor_open = false;
                }
            }

            // Close button in the editor heading bar
            if editor::CLOSE_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                editor_open = false;
            }

            // "Open sub-graph" button in the editor heading bar: navigate the
            // graph into the note's companion folder and close the editor.
            if editor::OPEN_SUBGRAPH_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                open_editing_subgraph();
                editor_open = false;
            }

            graph::input_handler::handle_input(&mut rl, &mut editor_open);
        }

        // Ctrl + =/+ and Ctrl + - zoom the text scale. View-independent: the
        // editor body text and the graph UI text share one factor, so the
        // shortcut works in both views. `=` covers `+` on decimal keyboards
        // (they share a key), numpad rows are handled too. When the editor is
        // open and the scale changed, force the caret to stay visible: the new
        // layout regenerates next frame, so the renderer needs a nudge to
        // re-follow it.
        let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
        if ctrl {
            let zoom_in = rl.is_key_pressed(KeyboardKey::KEY_EQUAL)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_EQUAL)
                || rl.is_key_pressed(KeyboardKey::KEY_KP_ADD)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_KP_ADD);
            let zoom_out = rl.is_key_pressed(KeyboardKey::KEY_MINUS)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_MINUS)
                || rl.is_key_pressed(KeyboardKey::KEY_KP_SUBTRACT)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_KP_SUBTRACT);
            if zoom_in {
                let mut z = config::TEXT_ZOOM.write().unwrap();
                *z = (*z + config::TEXT_ZOOM_STEP).min(config::TEXT_ZOOM_MAX);
                editor::buffer::force_follow_caret();
            } else if zoom_out {
                let mut z = config::TEXT_ZOOM.write().unwrap();
                *z = (*z - config::TEXT_ZOOM_STEP).max(config::TEXT_ZOOM_MIN);
                editor::buffer::force_follow_caret();
            }
        }

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
                editor::images::request_full(&dst);
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

        // A finished asset-import dialog (background thread) left a result: copy the
// chosen file into the project's assets/ folder and hand the relative target
// to the editor, which inserts `[[assets/...]]` as a new line below the caret.
if let Some(src) = editor::command::ASSET_PICK_RESULT.write().unwrap().take() {
    let root = project_root();
    if let Some(dst) = filesystem::copy_file_unique(&src, &root.join("assets")) {
        if let Ok(rel) = dst.strip_prefix(&root) {
            *editor::command::ASSET_INSERT.write().unwrap() =
                Some(rel.to_string_lossy().to_string());
        }
    }
}

// An "Add asset" command in the editor: spawn the (blocking) native file
// dialog on a background thread so the app keeps redrawing; the chosen path
// comes back through ASSET_PICK_RESULT above. Any file type is allowed.
if editor::command::ASSET_PICK_REQUEST.swap(false, std::sync::atomic::Ordering::Relaxed) {
    if !editor::command::ASSET_PICK_ACTIVE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        std::thread::spawn(move || {
            let chosen = rfd::FileDialog::new()
                .set_title("Select asset")
                .pick_file();
            *editor::command::ASSET_PICK_RESULT.write().unwrap() = chosen;
            editor::command::ASSET_PICK_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }
}

        // Ctrl+S = save the open editor buffer immediately
        if editor_open
            && (rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
                || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL))
            && rl.is_key_pressed(KeyboardKey::KEY_S)
        {
            if let Some(path) = editor::tabs::active_path() {
                editor::buffer::save_to_file(&path);
                refresh_saved_node(&path);
            }
        }

        // Autosave: after a typing pause, save if the buffer is dirty.
        if editor_open && editor::DIRTY.load(std::sync::atomic::Ordering::Relaxed) {
            let now_ms = (rl.get_time() * 1000.0) as u64;
            let last_edit =
                editor::LAST_EDIT_MILLIS.load(std::sync::atomic::Ordering::Relaxed);
            if last_edit > 0 && now_ms.saturating_sub(last_edit) >= autosave_pause_ms {
                if let Some(path) = editor::tabs::active_path() {
                    editor::buffer::save_to_file(&path);
                    refresh_saved_node(&path);
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
                    // Build an SDF field-font for the family's upright, bold
                    // and italic cuts so markdown emphasis renders with real
                    // glyph shapes. Any cut the family doesn't ship stays empty
                    // and the editor falls back to the upright font. Rasterizing
                    // the fields takes a moment on the first pick of a family.
                    let load = |src: &Option<graph::settings::FontStyleSource>| {
                        if let Some(src) = src {
                            if let Some(path) = &src.path {
                                std::fs::read(path)
                                    .ok()
                                    .and_then(|b| editor::text::load_sdf_font(&b))
                            } else if let Some(data) = &src.data {
                                editor::text::load_sdf_font(data)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    let regular = load(&Some(graph::settings::FontStyleSource {
                        path: fam.path.clone(),
                        data: fam.data.clone(),
                    }));
                    let bold = load(&fam.bold);
                    let italic = load(&fam.italic);
                    editor::text::set_active_fonts(editor::text::LoadedFontSet {
                        regular,
                        bold,
                        italic,
                    });
                }
                *graph::settings::SELECTED_FONT.write().unwrap() = Some(idx);
            }
            *graph::settings::REQUEST_LOAD_FONT.write().unwrap() = None;
        }

        // Detect transitions AFTER all input has been processed. The editor's tab
        // system loads documents itself (on open/activate), so the open
        // transition needs nothing else here.

        if !editor_open && was_open {
            // Leaving the editor: snapshot the active tab (saving it if it
            // was edited) so reopen resumes exactly where we left off.
            editor::tabs::deactivate_current();
            // A placeholder "Create New Node" prompt must not survive the
            // editor closing (its state has no node behind it anymore).
            editor::CREATING_NODE.store(false, std::sync::atomic::Ordering::Relaxed);
            *editor::NEW_NODE_NAME.write().unwrap() = String::new();
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
                                editor::images::request_full(&path);
                            }
                        }
                    }
                }
            }
        }

        // Preload node header images (frontmatter `header:` targets) for the
        // graph, but only for nodes inside the camera's viewport plus a small
        // margin, so panning through a large graph never decodes offscreen
        // images. Requests are non-blocking: the background loader decodes and
        // the worker hands back ready images (thumbnail or full-resolution)
        // which the sync() below uploads on this thread. Headers render from
        // small thumbnails; once generated this is just a 256px PNG decode.
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
                            editor::images::request_thumb(&path);
                        }
                    }
                }
            }
        }

        // Upload whatever the background loader finished since the last frame.
        editor::images::sync(&mut rl, &thread);

        // The fullscreen toggle resizes the editor panel; recompute it here so
        // the renderer and its heading bar always agree with the flag.
        let (_, _, panel_w, panel_h) = editor::panel_bounds();
        editor_dimentions = Vector2::new(
            panel_w as f32 / config::width() as f32,
            panel_h as f32 / config::height() as f32,
        );

        let mut d = rl.begin_drawing(&thread);

        graph::renderer::draw(&mut d);

        editor::renderer::draw(&mut d, editor_open, editor_dimentions);

        // The view-switcher bar is drawn last so it stays on top of both views.
        sidebar::draw(&mut d, editor_open);
    }

    // Persist the current settings (window size, zoom, font, force values) to
    // the global appdata config so everything survives a restart.
    app_config::save();
}

