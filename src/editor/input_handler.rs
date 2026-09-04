use raylib::prelude::*;

fn prev_word_boundary(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    // Skip whitespace immediately before the cursor.
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    // Skip the word itself.
    while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    i
}

fn prev_word_start(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    // Skip whitespace immediately before the cursor.
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    // Skip the word itself.
    while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    i
}

fn next_word_end(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    // Skip whitespace after the cursor.
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    // Skip the word itself.
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    i
}

fn next_word_boundary(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    // Skip the current word.
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    // Skip whitespace after it.
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    i
}

pub fn handle_input(rl: &mut RaylibHandle) {
    let visual_lines = crate::editor::buffer::VISUAL_LINES.lock().unwrap();
    let mut buffer = crate::editor::buffer::BUFFER.write().unwrap();
    let mut cursor_x = crate::editor::buffer::CURSOR_X.write().unwrap();
    let mut cursor_y = crate::editor::buffer::CURSOR_Y.write().unwrap();

    if buffer.is_empty() {
        buffer.push(String::new());
    }

    let current_visual = visual_lines.iter().position(|vl| {
        vl.line == *cursor_y as usize
            && *cursor_x as usize >= vl.start
            && *cursor_x as usize <= vl.end
    });

    // ------------------------------------------------------------
    // Text input
    // ------------------------------------------------------------

    while let Some(ch) = rl.get_char_pressed() {
        let c = char::from_u32(ch as u32).unwrap();

        if !c.is_control() {
            buffer[*cursor_y as usize].insert(*cursor_x as usize, c);
            *cursor_x += 1;
        }
    }

    // ------------------------------------------------------------
    // Ctrl + Backspace = delete previous word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && (rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE))
    {
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        if x > 0 {
            let new_x = prev_word_start(&buffer[y], x);

            buffer[y].drain(new_x..x);
            *cursor_x = new_x as i32;
        } else if y > 0 {
            // At the beginning of a line:
            // join this line with the previous one.
            let current_line = buffer.remove(y);
            let prev_len = buffer[y - 1].len();

            buffer[y - 1].push_str(&current_line);

            *cursor_y -= 1;
            *cursor_x = prev_len as i32;
        }

        return;
    }

    // ------------------------------------------------------------
    // Ctrl + Delete = delete next word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && (rl.is_key_pressed(KeyboardKey::KEY_DELETE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_DELETE))
    {
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        if x < buffer[y].len() {
            let new_x = next_word_end(&buffer[y], x);
            buffer[y].drain(x..new_x);
        } else if y < buffer.len() - 1 {
            // At the end of a line:
            // join the next line onto this one.
            let next_line = buffer.remove(y + 1);
            buffer[y].push_str(&next_line);
        }

        return;
    }

    // ------------------------------------------------------------
    // Ctrl + Left = previous word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && (rl.is_key_pressed(KeyboardKey::KEY_LEFT)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_LEFT))
    {
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        if x > 0 {
            *cursor_x = prev_word_boundary(&buffer[y], x) as i32;
        } else if y > 0 {
            *cursor_y -= 1;
            *cursor_x = buffer[*cursor_y as usize].len() as i32;
        }

        return;
    }

    // ------------------------------------------------------------
    // Ctrl + Right = next word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && (rl.is_key_pressed(KeyboardKey::KEY_RIGHT)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_RIGHT))
    {
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        if x < buffer[y].len() {
            *cursor_x = next_word_boundary(&buffer[y], x) as i32;
        } else if y < buffer.len() - 1 {
            *cursor_y += 1;
            *cursor_x = 0;
        }

        return;
    }

    // ------------------------------------------------------------
    // Normal Backspace
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
    {
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

    // ------------------------------------------------------------
    // Enter
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER)
    {
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        let new_line = buffer[y][x..].to_string();

        buffer[y].truncate(x);
        buffer.insert(y + 1, new_line);

        *cursor_y += 1;
        *cursor_x = 0;
    }

    // ------------------------------------------------------------
    // Normal Right
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_RIGHT) || rl.is_key_pressed_repeat(KeyboardKey::KEY_RIGHT)
    {
        if *cursor_x < buffer[*cursor_y as usize].len() as i32 {
            *cursor_x += 1;
        } else if *cursor_y < buffer.len() as i32 - 1 {
            *cursor_y += 1;
            *cursor_x = 0;
        }
    }

    // ------------------------------------------------------------
    // Normal Left
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_LEFT) || rl.is_key_pressed_repeat(KeyboardKey::KEY_LEFT) {
        if *cursor_x > 0 {
            *cursor_x -= 1;
        } else if *cursor_y > 0 {
            *cursor_y -= 1;
            *cursor_x = buffer[*cursor_y as usize].len() as i32;
        }
    }

    // ------------------------------------------------------------
    // Up
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_UP) || rl.is_key_pressed_repeat(KeyboardKey::KEY_UP) {
        if let Some(current) = current_visual {
            if current > 0 {
                let from = &visual_lines[current];
                let to = &visual_lines[current - 1];

                let offset = *cursor_x as usize - from.start;

                *cursor_y = to.line as i32;
                *cursor_x = (to.start + offset).min(to.end) as i32;
            }
        }
    }

    // ------------------------------------------------------------
    // Down
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_DOWN) || rl.is_key_pressed_repeat(KeyboardKey::KEY_DOWN) {
        if let Some(current) = current_visual {
            if current + 1 < visual_lines.len() {
                let from = &visual_lines[current];
                let to = &visual_lines[current + 1];

                let offset = *cursor_x as usize - from.start;

                *cursor_y = to.line as i32;
                *cursor_x = (to.start + offset).min(to.end) as i32;
            }
        }
    }
}
