use raylib::prelude::*;

use crate::config;

pub fn draw(mut d: RaylibDrawHandle, editor_open: bool, editor_dimentions: Vector2, buffer: &str) {
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

        // Draw the text on editor
        let mut i = 0;
        let max_w = (editor_dimentions.x * config::WIDTH as f32) as i32;
        let mut w = 0;

        for word in buffer.split_inclusive(char::is_whitespace) {
            let new_w = w + d.measure_text(word, config::EDITOR_FONT_SIZE);

            if new_w > max_w {
                w = 0;
                i += 1;
            }

            d.draw_text(
                word,
                ((1. - editor_dimentions.x) * 0.5 * config::WIDTH as f32) as i32 + w,
                ((1. - editor_dimentions.y) * 0.5 * config::HEIGHT as f32) as i32
                    + config::EDITOR_FONT_SIZE * i,
                config::EDITOR_FONT_SIZE,
                Color::YELLOW,
            );

            w += d.measure_text(word, config::EDITOR_FONT_SIZE);
        }
    }
}
