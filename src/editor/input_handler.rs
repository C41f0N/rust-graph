use raylib::prelude::*;

use crate::config;
use crate::editor::autocomplete;
use crate::editor::buffer;
use crate::editor::history;
use crate::editor::hit_test;
use crate::editor::text;
use crate::frontmatter;

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

    // Timestamp shared by all history snapshots this frame.
    let edit_now_ms = (rl.get_time() * 1000.0) as u64;

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
    // Mouse: click to place the caret, drag to select, double/triple-click
    // for word/line selection, click to accept an autocomplete row, and
    // click on the collapsed frontmatter bar to expand it.
    // ------------------------------------------------------------

    {
        let (ex, ey, ew, eh) = config::editor_panel_bounds();
        let header_h = config::EDITOR_HEADER_HEIGHT;
        let content_top = ey + header_h;
        let content_h = eh - header_h;
        let bar_w = 6;
        let bar_x = ex + ew - bar_w - config::EDITOR_PADDING;

        // --- Left press ---
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            let m = rl.get_mouse_position();

            // Click inside the autocomplete popup accepts that row.
            let ac_rect = hit_test::AUTOCOMPLETE_RECT.lock().unwrap();
            if let Some((rx, ry, rw, rh)) = *ac_rect {
                if m.x as i32 >= rx
                    && m.x as i32 <= rx + rw
                    && m.y as i32 >= ry
                    && m.y as i32 <= ry + rh
                {
                    let mut ac = autocomplete::AUTOCOMPLETE.write().unwrap();
                    if ac.active && !ac.matches.is_empty() {
                        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
                        let idx = m.y as i32 - ry;
                        let row = (idx.max(0) as usize).min(ac.matches.len().saturating_sub(1));
                        ac.selected = row;
                        let y = *cursor_y as usize;
                        let x = *cursor_x as usize;
                        let nx = autocomplete::apply_selection(&mut ac, &mut buffer, y, x);
                        *cursor_x = nx;
                        *anchor_x = *cursor_x;
                        *anchor_y = *cursor_y;
                        if nx != x as i32 {
                            buffer::mark_modified();
                        }
                    }
                    return;
                }
            }
            drop(ac_rect);

            let inside_content =
                m.y as i32 >= content_top && (m.y as i32) < content_top + content_h;

            // A click close to the scrollbar is the scrollbar's: the renderer
            // drags it on the following frames, so just don't place a caret.
            let over_bar =
                m.x as i32 >= bar_x - 4 && m.x as i32 <= bar_x + bar_w + 4;

            if inside_content && !over_bar {
                let scroll = *buffer::SCROLL_Y.read().unwrap();
                let rel_y = m.y as i32 - content_top + scroll;
                let hits = hit_test::VISUAL_HIT.lock().unwrap();

                if let Some(row) = hit_test::row_at_y(&hits, rel_y) {
                    if row.fm_bar {
                        // Place the caret at the end of the frontmatter block
                        // so opening the note's collapsed metadata expands it.
                        if let Some((_, fm_end)) = frontmatter::line_range(&buffer) {
                            let y = fm_end;
                            let x = buffer[y].len();
                            *cursor_y = y as i32;
                            *cursor_x = x as i32;
                            *anchor_x = x as i32;
                            *anchor_y = y as i32;
                        }
                        drop(hits);
                        hit_test::MOUSE_DRAGGING.store(true, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }

                    if !row.image {
                        let px = m.x as i32 - (ex + config::EDITOR_PADDING);
                        let off = hit_test::offset_at_px(&buffer[row.line], row, px, |t, s| {
                            text::measure(&rl, t, s)
                        });
                        let off = off as i32;

                        let now_ms = (rl.get_time() * 1000.0) as u64;
                        let prev_time = hit_test::LAST_CLICK_TIME_MS.load(std::sync::atomic::Ordering::Relaxed);
                        let prev_line = hit_test::LAST_CLICK_LINE.load(std::sync::atomic::Ordering::Relaxed);
                        let prev_off = hit_test::LAST_CLICK_OFFSET.load(std::sync::atomic::Ordering::Relaxed);
                        let prev_count = hit_test::CLICK_COUNT.load(std::sync::atomic::Ordering::Relaxed);
                        let same_pos = prev_line == row.line as i32 && prev_off == off;
                        let count = hit_test::classify_click(now_ms, prev_time, same_pos, prev_count);

                        let shift = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
                            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);

                        match count {
                            2 => {
                                // Double-click: select the word at the caret.
                                let line = &buffer[row.line];
                                let byte = off as usize;
                                let start = prev_word_start(line, byte);
                                let end = next_word_end(line, byte);
                                *anchor_x = start as i32;
                                *anchor_y = row.line as i32;
                                *cursor_x = end as i32;
                                *cursor_y = row.line as i32;
                            }
                            3 => {
                                // Triple-click: select the whole source line.
                                *anchor_x = 0;
                                *anchor_y = row.line as i32;
                                *cursor_x = buffer[row.line].len() as i32;
                                *cursor_y = row.line as i32;
                            }
                            _ => {
                                // Single click: place the caret (Shift+click
                                // keeps the anchor so it extends the selection).
                                *cursor_x = off;
                                *cursor_y = row.line as i32;
                                if !shift {
                                    *anchor_x = off;
                                    *anchor_y = row.line as i32;
                                }
                            }
                        }

                        hit_test::LAST_CLICK_TIME_MS.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                        hit_test::LAST_CLICK_LINE.store(row.line as i32, std::sync::atomic::Ordering::Relaxed);
                        hit_test::LAST_CLICK_OFFSET.store(off, std::sync::atomic::Ordering::Relaxed);
                        hit_test::CLICK_COUNT.store(count, std::sync::atomic::Ordering::Relaxed);
                    }
                    drop(hits);
                    hit_test::MOUSE_DRAGGING.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            }

            // Click on the scrollbar / outside the content area: no caret.
            hit_test::MOUSE_DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
            hit_test::reset_click_state();
            return;
        }

        // --- Left held after a press: drag to select ---
        if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT)
            && hit_test::MOUSE_DRAGGING.load(std::sync::atomic::Ordering::Relaxed)
        {
            let m = rl.get_mouse_position();
            if m.y as i32 >= content_top && (m.y as i32) < content_top + content_h {
                {
                    let scroll = *buffer::SCROLL_Y.read().unwrap();
                    let rel_y = m.y as i32 - content_top + scroll;
                    let hits = hit_test::VISUAL_HIT.lock().unwrap();

                    if let Some(row) = hit_test::row_at_y(&hits, rel_y) {
                        if !row.image && !row.fm_bar {
                            let px = m.x as i32 - (ex + config::EDITOR_PADDING);
                            let off = hit_test::offset_at_px(&buffer[row.line], row, px, |t, s| {
                                text::measure(&rl, t, s)
                            });
                            *cursor_x = off as i32;
                            *cursor_y = row.line as i32;
                        }
                    }
                }

                // Autoscroll: dragging past the edge feeds the view through
                // SCROLL_Y (the renderer clamps it to the content height).
                let margin = 24;
                let step = 16;
                let mut scroll = buffer::SCROLL_Y.write().unwrap();
                if (m.y as i32) < content_top + margin && *scroll > 0 {
                    *scroll = (*scroll - step).max(0);
                } else if m.y as i32 > content_top + content_h - margin {
                    *scroll += step;
                }
            }
            return;
        }

        // --- Release ends a drag ---
        if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
            if hit_test::MOUSE_DRAGGING.load(std::sync::atomic::Ordering::Relaxed) {
                hit_test::MOUSE_DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
            }
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
            history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
    // Ctrl + Z = undo, Ctrl + Y / Ctrl + Shift + Z = redo
    // ------------------------------------------------------------

    {
        let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
        let shift = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
        let z = rl.is_key_pressed(KeyboardKey::KEY_Z);
        let y = rl.is_key_pressed(KeyboardKey::KEY_Y);

        if ctrl && (z || y) {
            let caret = (*cursor_y, *cursor_x);
            let hist = if y || (z && shift) {
                history::redo(&buffer, caret)
            } else {
                history::undo(&buffer, caret)
            };

            if let Some(hist) = hist {
                if !hist.lines.is_empty() {
                    *buffer = hist.lines;
                    *cursor_y = hist.cursor.0.min(buffer.len() as i32 - 1);
                    *cursor_x = hist
                        .cursor
                        .1
                        .min(buffer[*cursor_y as usize].len() as i32);
                    *anchor_x = *cursor_x;
                    *anchor_y = *cursor_y;
                    // Force the renderer to follow the restored caret.
                    *buffer::LAST_CURSOR.write().unwrap() = (i32::MIN, i32::MIN);
                    buffer::mark_modified();
                }
            }
            return;
        }
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
                history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
                let y = *cursor_y as usize;
                let x = *cursor_x as usize;
                let nx = autocomplete::apply_selection(&mut state, &mut buffer, y, x);
                *cursor_x = nx;
                *anchor_x = *cursor_x;
                *anchor_y = *cursor_y;
                if nx != x as i32 {
                    buffer::mark_modified();
                }
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

        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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

    let typed: Vec<char> = {
        let mut out = Vec::new();
        while let Some(ch) = rl.get_char_pressed() {
            if let Some(c) = char::from_u32(ch as u32) {
                if !c.is_control() {
                    out.push(c);
                }
            }
        }
        out
    };

    if !typed.is_empty() {
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::CharInsert, edit_now_ms);
        for c in typed {
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
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
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
