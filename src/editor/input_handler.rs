use raylib::prelude::*;

use crate::config;
use crate::editor::autocomplete;
use crate::editor::buffer;

fn prev_word_boundary(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    i
}

fn prev_word_start(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }

    i
}

fn next_word_end(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    i
}

fn next_word_boundary(line: &str, x: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = x.min(bytes.len());

    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    i
}

pub fn handle_input(rl: &mut RaylibHandle) {
    // Keep a trailing empty line reachable before any navigation this frame.
    buffer::ensure_trailing_newline();

    let visual_lines = buffer::VISUAL_LINES.lock().unwrap();
    let mut buffer = buffer::BUFFER.write().unwrap();
    let mut cursor_x = buffer::CURSOR_X.write().unwrap();
    let mut cursor_y = buffer::CURSOR_Y.write().unwrap();
    let mut anchor_x = buffer::ANCHOR_X.write().unwrap();
    let mut anchor_y = buffer::ANCHOR_Y.write().unwrap();

    if buffer.is_empty() {
        buffer.push(String::new());
    }

    let current_visual = visual_lines.iter().position(|vl| {
        vl.line == *cursor_y as usize
            && *cursor_x as usize >= vl.start
            && *cursor_x as usize <= vl.end
    });

    // Compute selection state once
    let sel = buffer::selection_range(*anchor_x, *anchor_y, *cursor_x, *cursor_y);

    // Mouse wheel scrolls the content viewport. The renderer clamps the
    // offset to the real content height each frame, so we just nudge it.
    let wheel = rl.get_mouse_wheel_move();
    if wheel != 0.0 {
        let step = config::EDITOR_FONT_SIZE as f32 * 3.0;
        let mut scroll = buffer::SCROLL_Y.write().unwrap();
        let delta = (wheel * step) as i32;
        if delta != 0 {
            *scroll -= delta;
        }
    }

    // ------------------------------------------------------------
    // Ctrl + C = copy
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) && rl.is_key_pressed(KeyboardKey::KEY_C) {
        if let Some((sy, sx, ey, ex)) = sel {
            let text = if sy == ey {
                buffer[sy][sx..ex].to_string()
            } else {
                let mut s = buffer[sy][sx..].to_string();
                for line_i in sy + 1..ey {
                    s.push('\n');
                    s.push_str(&buffer[line_i]);
                }
                s.push('\n');
                s.push_str(&buffer[ey][..ex]);
                s
            };
            drop(buffer);
            drop(cursor_x);
            drop(cursor_y);
            drop(anchor_x);
            drop(anchor_y);
            let _ = rl.set_clipboard_text(&text);
        }
        return;
    }

    // ------------------------------------------------------------
    // Ctrl + X = cut
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) && rl.is_key_pressed(KeyboardKey::KEY_X) {
        if let Some((sy, sx, ey, ex)) = sel {
            let text = if sy == ey {
                buffer[sy][sx..ex].to_string()
            } else {
                let mut s = buffer[sy][sx..].to_string();
                for line_i in sy + 1..ey {
                    s.push('\n');
                    s.push_str(&buffer[line_i]);
                }
                s.push('\n');
                s.push_str(&buffer[ey][..ex]);
                s
            };
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
            *anchor_x = nx;
            *anchor_y = ny;
            drop(buffer);
            drop(cursor_x);
            drop(cursor_y);
            drop(anchor_x);
            drop(anchor_y);
            let _ = rl.set_clipboard_text(&text);
        }
        return;
    }

    // ------------------------------------------------------------
    // Ctrl + V = paste
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) && rl.is_key_pressed(KeyboardKey::KEY_V) {
        // Delete selection first if active
        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
        }

        // Drop locks to read clipboard
        drop(buffer);
        drop(cursor_x);
        drop(cursor_y);
        drop(anchor_x);
        drop(anchor_y);
        let clip = rl.get_clipboard_text().unwrap_or_default();
        let mut buffer = buffer::BUFFER.write().unwrap();
        let mut cursor_x = buffer::CURSOR_X.write().unwrap();
        let mut cursor_y = buffer::CURSOR_Y.write().unwrap();
        let mut anchor_x = buffer::ANCHOR_X.write().unwrap();
        let mut anchor_y = buffer::ANCHOR_Y.write().unwrap();

        if !clip.is_empty() {
            let y = *cursor_y as usize;
            let x = *cursor_x as usize;
            let lines: Vec<&str> = clip.split('\n').collect();

            if lines.len() == 1 {
                buffer[y].insert_str(x, &lines[0]);
                *cursor_x += lines[0].len() as i32;
            } else {
                let tail = buffer[y][x..].to_string();
                buffer[y].truncate(x);
                buffer[y].push_str(lines[0]);

                for (i, line) in lines[1..lines.len() - 1].iter().enumerate() {
                    buffer.insert(y + 1 + i, line.to_string());
                }

                let last = lines.last().unwrap();
                let new_line_idx = y + lines.len() - 1;
                let mut new_line = last.to_string();
                new_line.push_str(&tail);
                buffer.insert(new_line_idx, new_line);

                *cursor_y = new_line_idx as i32;
                *cursor_x = last.len() as i32;
            }

            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
            buffer::mark_modified();
        }
        return;
    }

    // ------------------------------------------------------------
    // Ctrl + A = select all
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) && rl.is_key_pressed(KeyboardKey::KEY_A) {
        *anchor_x = 0;
        *anchor_y = 0;
        let last = buffer.len() - 1;
        *cursor_y = last as i32;
        *cursor_x = buffer[last].len() as i32;
        return;
    }

    // ------------------------------------------------------------
    // Autocomplete: detect [[ ... and handle its keys
    // ------------------------------------------------------------

    autocomplete::refresh(&buffer, *cursor_y as usize, *cursor_x as usize);

    {
        let mut state = autocomplete::AUTOCOMPLETE.write().unwrap();
        if state.active {
            let up = rl.is_key_pressed(KeyboardKey::KEY_UP)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_UP);
            let down = rl.is_key_pressed(KeyboardKey::KEY_DOWN)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_DOWN);
            let tab = rl.is_key_pressed(KeyboardKey::KEY_TAB)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_TAB);
            let enter = rl.is_key_pressed(KeyboardKey::KEY_ENTER)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER);
            let esc = rl.is_key_pressed(KeyboardKey::KEY_ESCAPE);

            if (up || down || tab) && !state.matches.is_empty() {
                let n = state.matches.len();
                state.selected = if down || tab {
                    if state.selected + 1 >= n {
                        0
                    } else {
                        state.selected + 1
                    }
                } else if state.selected == 0 {
                    n - 1
                } else {
                    state.selected - 1
                };
                return;
            }

            if enter && !state.matches.is_empty() {
                let candidate = state.matches[state.selected].clone();
                let filter_len = state.filter.len();
                let y = *cursor_y as usize;
                let x = *cursor_x as usize;
                if x >= filter_len {
                    // Links are written extension-less and resolve to the
                    // .md file by name, so just close the bracket.
                    let replacement = format!("{}]]", candidate);
                    buffer[y].replace_range(x - filter_len..x, &replacement);
                    *cursor_x = (x - filter_len + replacement.len()) as i32;
                    *anchor_x = *cursor_x;
                    *anchor_y = *cursor_y;
                    buffer::mark_modified();
                }
                state.active = false;
                return;
            }

            if esc {
                state.active = false;
                state.suppress = true;
                state.suppress_filter = state.filter.clone();
                state.esc_consumed = true;
                return;
            }
        }
    }

    // ------------------------------------------------------------
    // Tab / Shift+Tab = indent / unindent with a real tab character
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
        let shift = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);

        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
        }

        let y = *cursor_y as usize;
        let x = *cursor_x as usize;

        if shift {
            // Remove one tab immediately before the cursor.
            if x > 0 && buffer[y].as_bytes().get(x - 1) == Some(&b'\t') {
                buffer[y].remove(x - 1);
                *cursor_x -= 1;
            }
        } else {
            buffer[y].insert(x, '\t');
            *cursor_x += 1;
        }

        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
        buffer::mark_modified();
        return;
    }

    // ------------------------------------------------------------
    // Text input
    // ------------------------------------------------------------

    while let Some(ch) = rl.get_char_pressed() {
        let c = char::from_u32(ch as u32).unwrap();

        if !c.is_control() {
            if let Some((sy, sx, ey, ex)) = sel {
                let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
                *cursor_x = nx;
                *cursor_y = ny;
            }
            let y = *cursor_y as usize;
            let x = *cursor_x as usize;
            buffer[y].insert(x, c);
            *cursor_x += 1;
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
            buffer::mark_modified();
        }
    }

    // ------------------------------------------------------------
    // Ctrl + Backspace = delete previous word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && (rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE))
    {
        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
            *anchor_x = nx;
            *anchor_y = ny;
        } else {
            let y = *cursor_y as usize;
            let x = *cursor_x as usize;

            if x > 0 {
                let new_x = prev_word_start(&buffer[y], x);
                buffer[y].drain(new_x..x);
                *cursor_x = new_x as i32;
            } else if y > 0 {
                let current_line = buffer.remove(y);
                let prev_len = buffer[y - 1].len();
                buffer[y - 1].push_str(&current_line);
                *cursor_y -= 1;
                *cursor_x = prev_len as i32;
            }
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        buffer::mark_modified();
        return;
    }

    // ------------------------------------------------------------
    // Ctrl + Delete = delete next word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && (rl.is_key_pressed(KeyboardKey::KEY_DELETE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_DELETE))
    {
        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
            *anchor_x = nx;
            *anchor_y = ny;
        } else {
            let y = *cursor_y as usize;
            let x = *cursor_x as usize;

            if x < buffer[y].len() {
                let new_x = next_word_end(&buffer[y], x);
                buffer[y].drain(x..new_x);
            } else if y < buffer.len() - 1 {
                let next_line = buffer.remove(y + 1);
                buffer[y].push_str(&next_line);
            }
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        buffer::mark_modified();
        return;
    }

    // ------------------------------------------------------------
    // Ctrl + Shift + Left = select previous word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
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
    // Ctrl + Shift + Right = select next word
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        && rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
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

        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
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

        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
        return;
    }

    // ------------------------------------------------------------
    // Shift + Left = select left
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        && (rl.is_key_pressed(KeyboardKey::KEY_LEFT)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_LEFT))
    {
        if sel.is_none() {
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        if *cursor_x > 0 {
            *cursor_x -= 1;
        } else if *cursor_y > 0 {
            *cursor_y -= 1;
            *cursor_x = buffer[*cursor_y as usize].len() as i32;
        }
        return;
    }

    // ------------------------------------------------------------
    // Shift + Right = select right
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        && (rl.is_key_pressed(KeyboardKey::KEY_RIGHT)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_RIGHT))
    {
        if sel.is_none() {
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        if (*cursor_x as usize) < buffer[*cursor_y as usize].len() {
            *cursor_x += 1;
        } else if *cursor_y < buffer.len() as i32 - 1 {
            *cursor_y += 1;
            *cursor_x = 0;
        }
        return;
    }

    // ------------------------------------------------------------
    // Shift + Up = select up
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        && (rl.is_key_pressed(KeyboardKey::KEY_UP)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_UP))
    {
        if sel.is_none() {
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        if let Some(current) = current_visual {
            if current > 0 {
                let from = &visual_lines[current];
                let to = &visual_lines[current - 1];
                let offset = *cursor_x as usize - from.start;
                *cursor_y = to.line as i32;
                *cursor_x = (to.start + offset).min(to.end) as i32;
            }
        }
        return;
    }

    // ------------------------------------------------------------
    // Shift + Down = select down
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        && (rl.is_key_pressed(KeyboardKey::KEY_DOWN)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_DOWN))
    {
        if sel.is_none() {
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        if let Some(current) = current_visual {
            if current + 1 < visual_lines.len() {
                let from = &visual_lines[current];
                let to = &visual_lines[current + 1];
                let offset = *cursor_x as usize - from.start;
                *cursor_y = to.line as i32;
                *cursor_x = (to.start + offset).min(to.end) as i32;
            }
        }
        return;
    }

    // ------------------------------------------------------------
    // Shift + Home = select to line start
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        && (rl.is_key_pressed(KeyboardKey::KEY_HOME)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_HOME))
    {
        if sel.is_none() {
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        *cursor_x = 0;
        return;
    }

    // ------------------------------------------------------------
    // Shift + End = select to line end
    // ------------------------------------------------------------

    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        && (rl.is_key_pressed(KeyboardKey::KEY_END)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_END))
    {
        if sel.is_none() {
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        *cursor_x = buffer[*cursor_y as usize].len() as i32;
        return;
    }

    // ------------------------------------------------------------
    // Normal Backspace
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
    {
        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
            *anchor_x = nx;
            *anchor_y = ny;
        } else if *cursor_x > 0 {
            buffer[*cursor_y as usize].remove(*cursor_x as usize - 1);
            *cursor_x -= 1;
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        } else if *cursor_y > 0 {
            let prev_line_len = buffer[*cursor_y as usize - 1].len() as i32;
            let buffered_line = buffer.remove(*cursor_y as usize);
            buffer[*cursor_y as usize - 1].push_str(&buffered_line);
            *cursor_y -= 1;
            *cursor_x = prev_line_len;
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        buffer::mark_modified();
    }

    // ------------------------------------------------------------
    // Delete key
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_DELETE)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_DELETE)
    {
        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
            *anchor_x = nx;
            *anchor_y = ny;
        } else {
            let y = *cursor_y as usize;
            let x = *cursor_x as usize;
            if (x as usize) < buffer[y].len() {
                buffer[y].remove(x);
            } else if y < buffer.len() - 1 {
                let next_line = buffer.remove(y + 1);
                buffer[y].push_str(&next_line);
            }
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        buffer::mark_modified();
    }

    // ------------------------------------------------------------
    // Home = go to line start
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_HOME)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_HOME)
    {
        *cursor_x = 0;
        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
    }

    // ------------------------------------------------------------
    // End = go to line end
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_END)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_END)
    {
        *cursor_x = buffer[*cursor_y as usize].len() as i32;
        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
    }

    // ------------------------------------------------------------
    // Enter
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER) {
        if let Some((sy, sx, ey, ex)) = sel {
            let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
            *cursor_x = nx;
            *cursor_y = ny;
        }
        let y = *cursor_y as usize;
        let x = *cursor_x as usize;
        let new_line = buffer[y][x..].to_string();
        buffer[y].truncate(x);
        buffer.insert(y + 1, new_line);
        *cursor_y += 1;
        *cursor_x = 0;
        *anchor_x = 0;
        *anchor_y = *cursor_y;
        buffer::mark_modified();
    }

    // ------------------------------------------------------------
    // Normal Right
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_RIGHT) || rl.is_key_pressed_repeat(KeyboardKey::KEY_RIGHT) {
        if *cursor_x < buffer[*cursor_y as usize].len() as i32 {
            *cursor_x += 1;
        } else if *cursor_y < buffer.len() as i32 - 1 {
            *cursor_y += 1;
            *cursor_x = 0;
        }
        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
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
        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
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
        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
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
        *anchor_x = *cursor_x;
        *anchor_y = *cursor_y;
    }
}
