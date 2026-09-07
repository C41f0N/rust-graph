use std::sync::RwLock;

use crate::config;
use crate::config::HEIGHT;
use crate::config::WIDTH;
use crate::editor::text;
use crate::filesystem;
use crate::graph::processing::*;
use crate::graph::settings;
use raylib::prelude::*;

pub static CAMERA: RwLock<Camera2D> = RwLock::new(Camera2D {
    target: Vector2 {
        x: WIDTH as f32 / 2.0,
        y: HEIGHT as f32 / 2.0,
    },
    offset: Vector2 {
        x: WIDTH as f32 / 2.0,
        y: HEIGHT as f32 / 2.0,
    },
    rotation: 0.0,
    zoom: 1.0,
});

pub fn draw(d: &mut RaylibDrawHandle) {
    let dragging_node = DRAGGING_NODE.read().unwrap();
    let hover_node = HOVER_NODE.read().unwrap();
    let selected_node = SELECTED_NODE.read().unwrap();
    let delete_pending = DELETE_PENDING.read().unwrap();
    let nodes = NODES.read().unwrap();
    let edges = EDGES.read().unwrap();
    let camera = CAMERA.read().unwrap();

    let mut mode = d.begin_mode2D(*camera);

    mode.clear_background(Color::BLACK);

    for edge in edges.iter() {
        let n1 = &nodes[edge.n1];
        let n2 = &nodes[edge.n2];
        mode.draw_line_ex(n1.position, n2.position, 2., Color::new(110, 110, 110, 255));
    }

    for (i, node) in nodes.iter().enumerate() {
        let is_dragging = *dragging_node == Some(i);
        let is_hover = *hover_node == Some(i);
        let is_selected = *selected_node == Some(i);

        let draw_radius = if is_dragging {
            node.radius * 1.5
        } else {
            node.radius
        };

        // State discs are filled and drawn BEHIND the node body: the node
        // covers their centre, so only the margin reads as a ring around it.
        // Drawn largest-first so a smaller disc never overlaps a bigger one.

        // Selection: broad white halo (the strongest emphasis, still B/W).
        if is_selected {
            mode.draw_circle_v(node.position, draw_radius + 4.0, Color::WHITE);
        }
        // Sub-graph: dim gray disc, subtler than selection.
        if node.has_subgraph {
            mode.draw_circle_v(node.position, draw_radius + 2.5, Color::new(160, 160, 160, 200));
        }
        // Hover: faint near-invisible halo so it reads as "active" only.
        if is_hover {
            mode.draw_circle_v(node.position, draw_radius + 1.5, Color::new(255, 255, 255, 90));
        }

        mode.draw_circle_v(node.position, draw_radius, node.color);

        let text_w = text::measure(&mode, &node.name, 5);

        text::draw(
            &mut mode,
            &node.name,
            (node.position.x - text_w as f32 / 2.0) as i32,
            (node.position.y + node.radius + 5.0) as i32,
            5,
            Color::WHITE.alpha(((camera.zoom - 2.0) / 0.5).clamp(0.0, 1.0)),
        );
    }

    drop(mode);
    drop(camera);

    // Draw delete confirmation prompt (outside camera mode, in screen space)
    if *delete_pending {
        if let Some(idx) = *selected_node {
            let name = &nodes[idx].name;
            let prompt = format!("Delete '{}'? (Y/N)", name);
            let text_width = text::measure(d, &prompt, 20);
            let x = (WIDTH - text_width) / 2;
            d.draw_rectangle(x - 10, 10, text_width + 20, 30, Color::BLACK.alpha(0.7));
            text::draw(d, &prompt, x, 15, 20, Color::WHITE);
        }
    }

    // Draw add-note name prompt (outside camera mode, in screen space)
    let adding_note = *crate::graph::processing::ADDING_NOTE.read().unwrap();
    if adding_note {
        let adding_name = crate::graph::processing::ADDING_NAME.read().unwrap();
        let prompt_base = "Filename: ";
        let full = format!("{}{}", prompt_base, adding_name);
        let label_width = text::measure(d, prompt_base, 20);
        let text_width = text::measure(d, &full, 20);
        let x = (WIDTH - text_width) / 2 - 10;
        let y = 10;
        d.draw_rectangle(x, y, text_width + 20, 30, Color::BLACK.alpha(0.7));
        text::draw(d, prompt_base, x + 10, y + 15, 20, Color::WHITE);

        // Draw the input content (in a lighter color) plus a cursor
        text::draw(d, &adding_name, x + 10 + label_width, y + 15, 20, Color::SKYBLUE);
        let name_width = text::measure(d, &adding_name, 20);
        let cursor_x = x + 10 + label_width + name_width;
        d.draw_rectangle(cursor_x, y + 5, 2, 20, Color::WHITE);
    }

    // Right-click context menu (screen space)
    let context_node = *crate::graph::processing::CONTEXT_NODE.read().unwrap();
    let renaming = *crate::graph::processing::RENAMING.read().unwrap();
    if context_node.is_some() && !renaming {
        let (mx, my) = *crate::graph::processing::CONTEXT_POS.read().unwrap();
        let screen_mouse = d.get_mouse_position();

        // Determine sub-graph availability for this node
        let node_name = context_node
            .and_then(|i| nodes.get(i))
            .map(|n| n.name.clone())
            .unwrap_or_default();
        let dir = crate::graph::processing::DIR_PATH.read().unwrap();
        let subgraph_exists = filesystem::is_dir(&dir.join(&node_name));
        drop(dir);

        let show_create = !subgraph_exists;
        let show_open = subgraph_exists;

        // Compute menu height: Rename + Delete always present, plus one
        // of the sub-graph items when applicable.
        let mut item_count = 2i32;
        if show_create { item_count += 1; }
        if show_open { item_count += 1; }
        let menu_h = item_count * config::CONTEXT_MENU_ITEM_H;

        // Background
        d.draw_rectangle(mx, my, config::CONTEXT_MENU_W, menu_h, Color::new(20, 20, 20, 235));

        // --- Row 0: Rename ---
        let hover_rename = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + config::CONTEXT_MENU_W
            && screen_mouse.y as i32 >= my
            && screen_mouse.y as i32 <= my + config::CONTEXT_MENU_ITEM_H;
        d.draw_rectangle(
            mx, my, config::CONTEXT_MENU_W, config::CONTEXT_MENU_ITEM_H,
            if hover_rename { Color::new(76, 128, 204, 160) } else { Color::new(0, 0, 0, 0) },
        );
        text::draw(d, "Rename", mx + 10, my + 6, 18, Color::WHITE);

        let mut y_off = config::CONTEXT_MENU_ITEM_H;
        d.draw_line(mx, my + y_off, mx + config::CONTEXT_MENU_W, my + y_off, config::CONTEXT_MENU_SEP_COLOR);

        // --- Row 1: Delete ---
        let hover_delete = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + config::CONTEXT_MENU_W
            && screen_mouse.y as i32 >= my + y_off
            && screen_mouse.y as i32 <= my + y_off + config::CONTEXT_MENU_ITEM_H;
        d.draw_rectangle(
            mx, my + y_off, config::CONTEXT_MENU_W, config::CONTEXT_MENU_ITEM_H,
            if hover_delete { Color::new(200, 60, 60, 160) } else { Color::new(0, 0, 0, 0) },
        );
        text::draw(d, "Delete", mx + 10, my + y_off + 6, 18, Color::WHITE);
        y_off += config::CONTEXT_MENU_ITEM_H;

        // Separator before sub-graph items (only when at least one is shown)
        if show_create || show_open {
            d.draw_line(mx, my + y_off, mx + config::CONTEXT_MENU_W, my + y_off, config::CONTEXT_MENU_SEP_COLOR);
        }

        // --- Row 2 (conditional): Create Sub-Graph ---
        if show_create {
            let hover_create = screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + config::CONTEXT_MENU_W
                && screen_mouse.y as i32 >= my + y_off
                && screen_mouse.y as i32 <= my + y_off + config::CONTEXT_MENU_ITEM_H;
            d.draw_rectangle(
                mx, my + y_off, config::CONTEXT_MENU_W, config::CONTEXT_MENU_ITEM_H,
                if hover_create { Color::new(76, 180, 120, 160) } else { Color::new(0, 0, 0, 0) },
            );
            text::draw(d, "Create Sub-Graph", mx + 10, my + y_off + 6, 18, Color::WHITE);
            y_off += config::CONTEXT_MENU_ITEM_H;
        }

        // --- Row 2/3 (conditional): Open Sub-Graph ---
        if show_open {
            let hover_open = screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + config::CONTEXT_MENU_W
                && screen_mouse.y as i32 >= my + y_off
                && screen_mouse.y as i32 <= my + y_off + config::CONTEXT_MENU_ITEM_H;
            d.draw_rectangle(
                mx, my + y_off, config::CONTEXT_MENU_W, config::CONTEXT_MENU_ITEM_H,
                if hover_open { Color::new(76, 128, 204, 160) } else { Color::new(0, 0, 0, 0) },
            );
            text::draw(d, "Open Sub-Graph", mx + 10, my + y_off + 6, 18, Color::WHITE);
        }
    }

    // Rename-note name prompt (screen space)
    if renaming {
        let rename_name = crate::graph::processing::RENAME_NAME.read().unwrap();
        let prompt_base = "Rename to: ";
        let full = format!("{}{}", prompt_base, rename_name);
        let label_width = text::measure(d, prompt_base, 20);
        let text_width = text::measure(d, &full, 20);
        let x = (WIDTH - text_width) / 2 - 10;
        let y = 10;
        d.draw_rectangle(x, y, text_width + 20, 30, Color::BLACK.alpha(0.7));
        text::draw(d, prompt_base, x + 10, y + 15, 20, Color::WHITE);
        text::draw(d, &rename_name, x + 10 + label_width, y + 15, 20, Color::SKYBLUE);
        let name_width = text::measure(d, &rename_name, 20);
        let cursor_x = x + 10 + label_width + name_width;
        d.draw_rectangle(cursor_x, y + 5, 2, 20, Color::WHITE);
    }

    // Breadcrumb trail (only when inside a sub-graph)
    let nav_stack = crate::graph::processing::NAV_STACK.read().unwrap();
    if !nav_stack.is_empty() {
        let dir = crate::graph::processing::DIR_PATH.read().unwrap();
        let screen_mouse = d.get_mouse_position();
        let settings_open = *crate::graph::settings::SETTINGS_OPEN.read().unwrap();

        // Build path components with one entry per level: the root directory
        // (level 0) uses its own folder name rather than a separate "Root"
        // label, so the root doesn't appear twice when it also carries a name.
        let mut components: Vec<String> = Vec::new();
        if let Some(root) = nav_stack.first() {
            let label = root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Root".to_string());
            components.push(label);
        } else {
            components.push("Root".to_string());
        }
        // Intermediate levels: folders the stack pushed between root and now.
        for p in nav_stack.iter().skip(1) {
            if let Some(name) = p.file_name() {
                components.push(name.to_string_lossy().to_string());
            }
        }
        // Current folder.
        if let Some(name) = dir.file_name() {
            components.push(name.to_string_lossy().to_string());
        }
        drop(dir);

        let mut x = config::BREADCRUMB_PAD;
        let text_color = Color::new(200, 200, 200, 220);
        let total = components.len();

        for (level, comp) in components.iter().enumerate() {
            let label = if level == 0 { comp.clone() } else { format!("/{}", comp) };
            let w = text::measure(d, &label, 16);
            let is_current = level == total - 1;

            if !is_current {
                let hover = screen_mouse.x as i32 >= x
                    && screen_mouse.x as i32 <= x + w
                    && screen_mouse.y as i32 >= config::BREADCRUMB_Y
                    && screen_mouse.y as i32 <= config::BREADCRUMB_Y + 20;
                let color = if hover { Color::SKYBLUE } else { text_color };
                text::draw(d, &label, x, config::BREADCRUMB_Y, 16, color);

                // Detect click and store the level for the input handler.
                // Only when no menu/modal is active, so the value can't be
                // left stale by an input path that returns early.
                if context_node.is_none()
                    && !renaming
                    && !adding_note
                    && !settings_open
                    && hover
                    && d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
                {
                    *crate::graph::processing::BREADCRUMB_CLICK.write().unwrap() = Some(level);
                }
            } else {
                text::draw(d, &label, x, config::BREADCRUMB_Y, 16, Color::WHITE);
            }

            x += w;
        }
    }

    // Settings button (screen space, top-left)
    let settings_open = *crate::graph::settings::SETTINGS_OPEN.read().unwrap();
    let screen_mouse = d.get_mouse_position();
    let over_btn = screen_mouse.x as i32 >= settings::SETTINGS_BUTTON_X
        && screen_mouse.x as i32 <= settings::SETTINGS_BUTTON_X + settings::SETTINGS_BUTTON_W
        && screen_mouse.y as i32 >= settings::SETTINGS_BUTTON_Y
        && screen_mouse.y as i32 <= settings::SETTINGS_BUTTON_Y + settings::SETTINGS_BUTTON_H;

    let btn_bg = if settings_open {
        Color::new(76, 128, 204, 180)
    } else if over_btn {
        Color::new(76, 128, 204, 120)
    } else {
        Color::new(40, 40, 46, 200)
    };
    d.draw_rectangle(
        settings::SETTINGS_BUTTON_X,
        settings::SETTINGS_BUTTON_Y,
        settings::SETTINGS_BUTTON_W,
        settings::SETTINGS_BUTTON_H,
        btn_bg,
    );
    let btn_label = "Settings";
    let btn_w = text::measure(d, btn_label, 18);
    text::draw(
        d,
        btn_label,
        settings::SETTINGS_BUTTON_X + (settings::SETTINGS_BUTTON_W - btn_w) / 2,
        settings::SETTINGS_BUTTON_Y + (settings::SETTINGS_BUTTON_H - 18) / 2,
        18,
        Color::WHITE,
    );

    // Settings dialog / font picker
    if settings_open {
        d.draw_rectangle(0, 0, WIDTH, HEIGHT, Color::new(0, 0, 0, 120));

        let px = settings::panel_x();
        let py = settings::panel_y();
        d.draw_rectangle(
            px,
            py,
            settings::SETTINGS_PANEL_W,
            settings::SETTINGS_PANEL_H,
            Color::new(25, 25, 30, 245),
        );
        text::draw(d, "Settings", px + 12, py + 8, 24, Color::WHITE);
        d.draw_line(
            px,
            py + settings::SETTINGS_TITLE_H,
            px + settings::SETTINGS_PANEL_W,
            py + settings::SETTINGS_TITLE_H,
            Color::new(255, 255, 255, 50),
        );

        let fonts = crate::graph::settings::FONTS.read().unwrap();
        let scroll = *crate::graph::settings::SETTINGS_SCROLL.read().unwrap();
        let selected = *crate::graph::settings::SELECTED_FONT.read().unwrap();

        let list_top = py + settings::SETTINGS_TITLE_H;
        let list_h = settings::SETTINGS_PANEL_H - settings::SETTINGS_TITLE_H;

        let mut sc = d.begin_scissor_mode(px + 1, list_top, settings::SETTINGS_PANEL_W - 2, list_h - 1);
        for i in 0..settings::SETTINGS_VISIBLE_ROWS {
            let idx = scroll + i;
            let Some(fam) = fonts.get(idx) else { break };
            let row_y = list_top + (i as i32) * settings::SETTINGS_ROW_H;
            let row_hovered = screen_mouse.x as i32 >= px
                && screen_mouse.x as i32 <= px + settings::SETTINGS_PANEL_W
                && screen_mouse.y as i32 >= row_y
                && screen_mouse.y as i32 <= row_y + settings::SETTINGS_ROW_H;
            let row_selected = selected == Some(idx);

            if row_hovered {
                sc.draw_rectangle(
                    px,
                    row_y,
                    settings::SETTINGS_PANEL_W,
                    settings::SETTINGS_ROW_H,
                    Color::new(76, 128, 204, 120),
                );
            }
            if row_selected {
                sc.draw_rectangle_lines(
                    px,
                    row_y,
                    settings::SETTINGS_PANEL_W,
                    settings::SETTINGS_ROW_H,
                    Color::new(76, 128, 204, 255),
                );
            }

            let color = if row_selected {
                Color::SKYBLUE
            } else {
                Color::WHITE
            };
            text::draw(&mut sc, &fam.name, px + 12, row_y + (settings::SETTINGS_ROW_H - 16) / 2, 16, color);
        }
        drop(sc);
    }
}
