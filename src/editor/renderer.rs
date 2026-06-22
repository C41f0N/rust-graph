use raylib::prelude::*;

use crate::config;

fn wrap_text(d: &RaylibDrawHandle, text: &str, max_width: i32, font_size: i32) -> Vec<String> {
    let mut result = Vec::new();

    for logical_line in text.lines() {
        let mut current = String::new();
        let mut width = 0;

        for word in logical_line.split_inclusive(char::is_whitespace) {
            let word_width = d.measure_text(word, font_size);

            if width > 0 && width + word_width > max_width {
                result.push(current);
                current = String::new();
                width = 0;
            }

            current.push_str(word);
            width += word_width;
        }

        result.push(current);
    }

    result
}

pub fn draw(mut d: RaylibDrawHandle, editor_open: bool, editor_dimentions: Vector2) {
    let buffer = crate::editor::buffer::BUFFER.lock().unwrap();
    let cursor_x = crate::editor::buffer::CURSOR_X.lock().unwrap();
    let cursor_y = crate::editor::buffer::CURSOR_Y.lock().unwrap();
    let font_color = config::EDITOR_FONT_COLOR;

    // Draw editor
    if editor_open {
        // The background
        d.draw_rectangle_rounded(
            Rectangle::new(
                config::WIDTH as f32 * (1. - editor_dimentions.x) * 0.5,
                config::HEIGHT as f32 * (1. - editor_dimentions.y) * 0.5,
                config::WIDTH as f32 * editor_dimentions.x,
                config::HEIGHT as f32 * editor_dimentions.y,
            ),
            0.05,
            0,
            Color::RAYWHITE.alpha(0.5),
        );

        let editor_x = ((1.0 - editor_dimentions.x) * 0.5 * config::WIDTH as f32) as i32;
        let editor_y = ((1.0 - editor_dimentions.y) * 0.5 * config::HEIGHT as f32) as i32;

        let max_width = (editor_dimentions.x * config::WIDTH as f32) as i32;

        for (line_num, line) in buffer.iter().enumerate() {
            // Draw the cursor if it's on this line
            if *cursor_y == line_num as i32 {
                let cursor_x_abs = editor_x
                    + d.measure_text(&line[0..*cursor_x as usize], config::EDITOR_FONT_SIZE);
                let cursor_y_abs = editor_y + line_num as i32 * config::EDITOR_FONT_SIZE;

                d.draw_rectangle(
                    cursor_x_abs,
                    cursor_y_abs,
                    2,
                    config::EDITOR_FONT_SIZE,
                    font_color,
                );
            }
            d.draw_text(
                line,
                editor_x,
                editor_y + line_num as i32 * config::EDITOR_FONT_SIZE,
                config::EDITOR_FONT_SIZE,
                font_color,
            );
        }
    }
}
