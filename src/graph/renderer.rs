use std::sync::RwLock;

use crate::config::HEIGHT;
use crate::config::WIDTH;
use crate::editor::text;
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
        mode.draw_line_ex(n1.position, n2.position, 2., Color::LIGHTGRAY);
    }

    for (i, node) in nodes.iter().enumerate() {
        let is_dragging = *dragging_node == Some(i);
        let is_hover = *hover_node == Some(i);
        let is_selected = *selected_node == Some(i);

        let mut node_color = node.color;
        if is_hover {
            node_color = Color::LIGHTPINK;
        }

        let draw_radius = if is_dragging {
            node.radius * 1.5
        } else {
            node.radius
        };

        mode.draw_circle_v(node.position, draw_radius, node_color);

        // Draw selection ring
        if is_selected {
            mode.draw_circle_lines_v(
                node.position,
                draw_radius + 3.0,
                Color::YELLOW,
            );
        }

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
        let menu_w = 140;
        let item_h = 30;
        let screen_mouse = d.get_mouse_position();

        // Bar
        let hover_rename = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + menu_w
            && screen_mouse.y as i32 >= my
            && screen_mouse.y as i32 <= my + item_h;
        let hover_delete = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + menu_w
            && screen_mouse.y as i32 >= my + item_h
            && screen_mouse.y as i32 <= my + 2 * item_h;

        d.draw_rectangle(mx, my, menu_w, 2 * item_h, Color::new(20, 20, 20, 235));

        d.draw_rectangle(
            mx,
            my,
            menu_w,
            item_h,
            if hover_rename {
                Color::new(76, 128, 204, 160)
            } else {
                Color::new(0, 0, 0, 0)
            },
        );
        text::draw(d, "Rename", mx + 10, my + 6, 18, Color::WHITE);

        d.draw_rectangle(
            mx,
            my + item_h,
            menu_w,
            item_h,
            if hover_delete {
                Color::new(200, 60, 60, 160)
            } else {
                Color::new(0, 0, 0, 0)
            },
        );
        text::draw(d, "Delete", mx + 10, my + item_h + 6, 18, Color::WHITE);

        d.draw_line(mx, my + item_h, mx + menu_w, my + item_h, Color::new(60, 60, 60, 255));
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
