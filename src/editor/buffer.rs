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

pub static CURSOR_X: Mutex<i32> = Mutex::new(0);
pub static CURSOR_Y: Mutex<i32> = Mutex::new(0);

pub fn generate_visual_lines(max_width: i32, d: &mut RaylibDrawHandle) {
    let buffer = BUFFER.read().unwrap();
    let mut visual_lines = VISUAL_LINES.lock().unwrap();
    visual_lines.clear();

    for (line_index, line) in buffer.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;

        while start < chars.len() {
            let mut end = start;
            let mut current_text = String::new();

            while end < chars.len() {
                current_text.push(chars[end]);

                let width = d.measure_text(&current_text, config::EDITOR_FONT_SIZE);

                if width > max_width {
                    current_text.pop();
                    break;
                }

                end += 1;
            }

            // Ensure progress even if a single glyph exceeds max_width
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
}
