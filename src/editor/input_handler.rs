use raylib::prelude::*;

pub fn handle_input(rl: &mut RaylibHandle) {
    let mut buffer = crate::editor::buffer::BUFFER.lock().unwrap();
    let mut cursorI = crate::editor::buffer::CURSOR_INDEX.lock().unwrap();
    let mut cursorLine = crate::editor::buffer::CURSOR_LINE.lock().unwrap();

    while let Some(ch) = rl.get_char_pressed() {
        let c = char::from_u32(ch as u32).unwrap();

        match c {
            // Enter key
            '\n' | '\r' => {
                buffer.push('\n');
            }

            // Backspace (handled separately below usually, but included here if mapped)
            '\u{8}' | '\u{7f}' => {
                buffer.pop();
            }

            // Normal printable characters
            _ => {
                if !c.is_control() {
                    buffer.push(c);
                }
            }
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE) {
        buffer.pop();
    }

    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
        buffer.push('\n');
    }

    // TODO: complete this shit
    if rl.is_key_pressed(KeyboardKey::KEY_UP) {
        *cursorLine = if *cursorLine >= 1 { *cursorLine - 1 } else { 0 };
    }

    if rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
        *cursorLine += 1;
    }

    if rl.is_key_pressed(KeyboardKey::KEY_RIGHT) {
        *cursorLine = if *cursorLine >= 1 { *cursorLine - 1 } else { 0 };
    }

    if rl.is_key_pressed(KeyboardKey::KEY_LEFT) {
        *cursorLine = if *cursorLine >= 1 { *cursorLine - 1 } else { 0 };
    }
}
