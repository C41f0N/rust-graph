use raylib::prelude::*;

pub fn handle_input(rl: &mut RaylibHandle) {
    let mut buffer = crate::editor::buffer::BUFFER.write().unwrap();
    let mut cursor_x = crate::editor::buffer::CURSOR_X.lock().unwrap();
    let mut cursor_y = crate::editor::buffer::CURSOR_Y.lock().unwrap();

    if buffer.is_empty() {
        buffer.push(String::new());
    }

    while let Some(ch) = rl.get_char_pressed() {
        let c = char::from_u32(ch as u32).unwrap();

        if !c.is_control() {
            buffer[*cursor_y as usize].insert(*cursor_x as usize, c);
            *cursor_x += 1;
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE) {
        if *cursor_x > 0 {
            buffer[*cursor_y as usize].remove(*cursor_x as usize - 1);
            *cursor_x -= 1;
        } else if *cursor_y > 0 {
            let prev_line_len = buffer[*cursor_y as usize - 1].len() as i32;
            let buffered_line = buffer.remove(*cursor_y as usize);
            buffer[*cursor_y as usize - 1].push_str(&buffered_line);
            *cursor_y -= 1;
            *cursor_x = prev_line_len;
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        let new_line = buffer[y][x..].to_string();
        buffer[y].truncate(x);
        buffer.insert(y + 1, new_line);

        *cursor_y += 1;
        *cursor_x = 0;
    }

    if rl.is_key_pressed(KeyboardKey::KEY_RIGHT) {
        if *cursor_x < buffer[*cursor_y as usize].len() as i32 {
            *cursor_x += 1;
        } else if *cursor_y < buffer.len() as i32 - 1 {
            *cursor_y += 1;
            *cursor_x = 0;
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_LEFT) {
        if *cursor_x > 0 {
            *cursor_x -= 1;
        } else if *cursor_y > 0 {
            *cursor_y -= 1;
            *cursor_x = buffer[*cursor_y as usize].len() as i32;
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_UP) {
        if *cursor_y > 0 {
            *cursor_y -= 1;
            *cursor_x = (*cursor_x).min(buffer[*cursor_y as usize].len() as i32);
        }
    }

    if rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
        if *cursor_y < buffer.len() as i32 - 1 {
            *cursor_y += 1;
            *cursor_x = (*cursor_x).min(buffer[*cursor_y as usize].len() as i32);
        }
    }
}
