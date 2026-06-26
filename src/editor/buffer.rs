use crate::config;
use raylib::prelude::*;
use std::sync::Mutex;
use std::sync::RwLock;

pub struct VisualLine {
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

pub static BUFFER: RwLock<Vec<String>> = RwLock::new(Vec::new());
pub static VISUAL_LINES: Mutex<Vec<VisualLine>> = Mutex::new(Vec::new());

pub static CURSOR_X: RwLock<i32> = RwLock::new(0);
pub static CURSOR_Y: RwLock<i32> = RwLock::new(0);

pub fn generate_visual_lines(max_width: i32, d: &mut RaylibDrawHandle) {
    let cursor_y = crate::editor::buffer::CURSOR_Y.read().unwrap();
    let buffer = BUFFER.read().unwrap();
    let mut visual_lines = VISUAL_LINES.lock().unwrap();
    visual_lines.clear();

    for (line_index, line) in buffer.iter().enumerate() {
        let mut line_font_size = config::EDITOR_FONT_SIZE;

        let mut start = 0;

        if line.starts_with("# ") && *cursor_y as usize != line_index {
            start = 2;
            line_font_size = config::EDITOR_FONT_SIZE_H1;
        }

        let chars: Vec<char> = line.chars().collect();

        while start < chars.len() {
            let mut end = start;
            let mut last_space = None;
            let mut text = String::new();

            while end < chars.len() {
                text.push(chars[end]);

                if chars[end].is_whitespace() {
                    last_space = Some(end);
                }

                let width = d.measure_text(&text, line_font_size);

                if width > max_width {
                    break;
                }

                end += 1;
            }

            if end == chars.len() {
                visual_lines.push(VisualLine {
                    start,
                    end,
                    line: line_index,
                });
                break;
            }

            if let Some(space) = last_space {
                // Wrap at the last space.
                visual_lines.push(VisualLine {
                    start,
                    end: space,
                    line: line_index,
                });

                // Skip whitespace at the beginning of the next visual line.
                start = space + 1;
                while start < chars.len() && chars[start].is_whitespace() {
                    start += 1;
                }
            } else {
                // No spaces in this segment (very long word).
                if end == start {
                    end += 1;
                }

                visual_lines.push(VisualLine {
                    start,
                    end,
                    line: line_index,
                });

                start = end;
            }
        }

        if chars.is_empty() {
            visual_lines.push(VisualLine {
                start: 0,
                end: 0,
                line: line_index,
            });
        }
    }
}
