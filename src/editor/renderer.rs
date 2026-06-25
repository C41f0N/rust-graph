use raylib::prelude::*;

use crate::config;

pub fn draw(mut d: RaylibDrawHandle, editor_open: bool, editor_dimentions: Vector2) {
    let buffer = crate::editor::buffer::BUFFER.read().unwrap();
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

        // Generate visual lines based on the current buffer and max width
        crate::editor::buffer::generate_visual_lines(max_width, &mut d);

        for (line_num, line) in crate::editor::buffer::VISUAL_LINES
            .lock()
            .unwrap()
            .iter()
            .enumerate()
        {
            let text = &buffer[line.line][line.start..line.end];
            let text_y = editor_y + line_num as i32 * config::EDITOR_FONT_SIZE;

            // Draw the cursor if it's on this line
            if *cursor_y == line_num as i32
                && *cursor_x >= line.start as i32
                && *cursor_x <= line.end as i32
            {
                let cursor_x_abs = editor_x
                    + d.measure_text(
                        buffer[line.line as usize].as_str()[line.start..*cursor_x as usize]
                            .to_string()
                            .as_str(),
                        config::EDITOR_FONT_SIZE,
                    );
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
                buffer[line.line as usize].as_str()[line.start..line.end]
                    .to_string()
                    .as_str(),
                editor_x,
                editor_y + line_num as i32 * config::EDITOR_FONT_SIZE,
                config::EDITOR_FONT_SIZE,
                font_color,
            );
        }
    }
}
