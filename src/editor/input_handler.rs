use raylib::prelude::*;

use crate::config;
use crate::editor::autocomplete;
use crate::editor::buffer;
use crate::editor::history;
use crate::editor::hit_test;
use crate::editor::text;
use crate::frontmatter;

// Execute a command chosen from the slash palette. Runs inline on the input
// thread; asset import defers the (blocking) native dialog to main.rs and
// returns immediately.
fn run_command(cmd: crate::editor::command::Command) {
    match cmd {
        crate::editor::command::Command::AddAsset => {
            crate::editor::command::request_asset_pick();
        }
    }
}

// Remove the "/command" text (from the word-initial slash to `x`) on line `y`.
// Returns the new cursor X position.  If the slash is not found (e.g. the
// region was already erased), `x` is returned unchanged.
fn remove_command_text(buffer: &mut Vec<String>, y: usize, x: usize) -> i32 {
    if let Some(flen) = crate::editor::command::detect(&buffer[y], x) {
        let span = flen.len() + 1; // filter + the leading '/'
        if x >= span {
            buffer[y].drain(x - span..x);
            return (x - span) as i32;
        }
    }
    x as i32
}

// Remove one indentation level from the start of `line`: a single leading tab,
// or up to `tab_width` leading spaces (whatever is present). Returns the number
// of bytes removed.
fn remove_indent(line: &mut String, tab_width: usize) -> usize {
    if line.starts_with('\t') {
        line.drain(..1);
        1
    } else {
        let removed = line
            .as_bytes()
            .iter()
            .take(tab_width)
            .take_while(|&&b| b == b' ')
            .count();
        if removed > 0 {
            line.drain(..removed);
        }
        removed
    }
}

// Word motion classes. Punctuation is a separator, not a word: Ctrl+Right on
// "foo-bar" stops after "foo" and after "bar", never on the "-". Underscore
// stays a word character so identifiers move as one token.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Punct,
}

fn class_of(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

// Cursor offsets come from the buffer and are always char boundaries, but a
// click or a stale index must never slice a multi-byte glyph in half.
fn clamp_boundary(line: &str, x: usize) -> usize {
    let mut i = x.min(line.len());
    while i > 0 && !line.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn char_at(line: &str, i: usize) -> Option<char> {
    line.get(i..)?.chars().next()
}

fn char_before(line: &str, i: usize) -> Option<char> {
    line.get(..i)?.chars().next_back()
}

fn skip_word_forward(line: &str, mut i: usize) -> usize {
    while let Some(c) = char_at(line, i) {
        if class_of(c) != CharClass::Word {
            break;
        }
        i += c.len_utf8();
    }
    i
}

fn skip_word_backward(line: &str, mut i: usize) -> usize {
    while let Some(c) = char_before(line, i) {
        if class_of(c) != CharClass::Word {
            break;
        }
        i -= c.len_utf8();
    }
    i
}

// Spaces and punctuation are both separators for word motion.
fn skip_separators_forward(line: &str, mut i: usize) -> usize {
    while let Some(c) = char_at(line, i) {
        if class_of(c) == CharClass::Word {
            break;
        }
        i += c.len_utf8();
    }
    i
}

fn skip_separators_backward(line: &str, mut i: usize) -> usize {
    while let Some(c) = char_before(line, i) {
        if class_of(c) == CharClass::Word {
            break;
        }
        i -= c.len_utf8();
    }
    i
}

// Ctrl+Right: finish the current word, or skip separators and finish the next
// one. Punctuation runs are crossed in the same move as the following word.
fn next_word_boundary(line: &str, x: usize) -> usize {
    let i = clamp_boundary(line, x);
    if char_at(line, i).is_some_and(|c| class_of(c) == CharClass::Word) {
        return skip_word_forward(line, i);
    }
    let i = skip_separators_forward(line, i);
    skip_word_forward(line, i)
}

// Ctrl+Left: mirror of the above. Starting at a word's first character skips
// the separators behind it and lands at the start of the previous word.
fn prev_word_boundary(line: &str, x: usize) -> usize {
    let i = clamp_boundary(line, x);
    if i > 0 && char_before(line, i).is_some_and(|c| class_of(c) == CharClass::Word) {
        return skip_word_backward(line, i);
    }
    let i = skip_separators_backward(line, i);
    skip_word_backward(line, i)
}

// Double-click range: expand over the word or punctuation run under the
// caret. On whitespace or end-of-line, select the word before it when one
// exists, otherwise the next word.
fn word_range_at(line: &str, x: usize) -> (usize, usize) {
    let i = clamp_boundary(line, x);
    if let Some(c) = char_at(line, i) {
        let cls = class_of(c);
        if cls != CharClass::Space {
            let mut start = i;
            while let Some(prev) = char_before(line, start) {
                if class_of(prev) != cls {
                    break;
                }
                start -= prev.len_utf8();
            }
            let mut end = i;
            while let Some(next) = char_at(line, end) {
                if class_of(next) != cls {
                    break;
                }
                end += next.len_utf8();
            }
            return (start, end);
        }
    }

    let before = skip_separators_backward(line, i);
    if before > 0 {
        let start = skip_word_backward(line, before);
        if start < before {
            return (start, before);
        }
    }

    let start = skip_separators_forward(line, i);
    let end = skip_word_forward(line, start);
    if end > start {
        (start, end)
    } else {
        (i, i)
    }
}

// Either Control key is Ctrl: shortcuts must not care which side of the
// keyboard the user reaches for (same for Shift in the Ctrl+Shift combos).
fn ctrl_down(rl: &RaylibHandle) -> bool {
    rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL)
}

fn shift_down(rl: &RaylibHandle) -> bool {
    rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
        || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT)
}

pub fn handle_input(rl: &mut RaylibHandle) {
    // Keep a trailing empty line reachable before any navigation this frame.
    buffer::ensure_trailing_newline();

    // ------------------------------------------------------------
    // No node open: there is no file behind the buffer, so nothing may be
    // edited or saved. The editor shows a placeholder instead; it only accepts
    // the "Create New Node" click and, once active, the filename prompt.
    // ------------------------------------------------------------
    let has_node = crate::editor::tabs::has_any();
    if !has_node {
        if crate::editor::CREATING_NODE.load(std::sync::atomic::Ordering::Relaxed) {
            // Filename prompt: type the name, Enter creates, Escape cancels.
            while let Some(ch) = rl.get_char_pressed() {
                let c = char::from_u32(ch as u32).unwrap();
                if !c.is_control() {
                    crate::editor::NEW_NODE_NAME.write().unwrap().push(c);
                }
            }
            if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
            {
                crate::editor::NEW_NODE_NAME.write().unwrap().pop();
            }
            if rl.is_key_pressed(KeyboardKey::KEY_ENTER)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER)
            {
                let stem = crate::editor::NEW_NODE_NAME.read().unwrap().clone();
                let stem = if stem.trim().is_empty() {
                    "untitled".to_string()
                } else {
                    stem.trim().to_string()
                };
                // Clear the prompt state before any further work (the read
                // guard above is a temporary and is already dropped).
                *crate::editor::NEW_NODE_NAME.write().unwrap() = String::new();
                crate::editor::CREATING_NODE.store(false, std::sync::atomic::Ordering::Relaxed);

                let dir = crate::graph::processing::DIR_PATH.read().unwrap().clone();
                crate::editor::tabs::create_and_open(&dir, &stem);
            }
            if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                *crate::editor::NEW_NODE_NAME.write().unwrap() = String::new();
                crate::editor::CREATING_NODE.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }

        // Placeholder: the only mouse affordance is the create button.
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            let m = rl.get_mouse_position();
            let rect = crate::editor::NEW_NODE_BUTTON.read().unwrap().clone();
            if let Some((bx, by, bw, bh)) = rect {
                if m.x as i32 >= bx
                    && m.x as i32 <= bx + bw
                    && m.y as i32 >= by
                    && m.y as i32 <= by + bh
                {
                    *crate::editor::NEW_NODE_NAME.write().unwrap() = String::new();
                    crate::editor::CREATING_NODE.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        return;
    }

    // ------------------------------------------------------------
    // Inline title rename: while active, every keystroke edits the draft
    // instead of the note. Enter commits, Escape cancels. Runs before the
    // buffer/caret handlers so typing never leaks into the document.
    // ------------------------------------------------------------
    if crate::editor::TITLE_RENAME_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        while let Some(ch) = rl.get_char_pressed() {
            let c = char::from_u32(ch as u32).unwrap();
            if !c.is_control() {
                crate::editor::TITLE_RENAME_DRAFT.write().unwrap().push(c);
            }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
        {
            crate::editor::TITLE_RENAME_DRAFT.write().unwrap().pop();
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ENTER)
            || rl.is_key_pressed(KeyboardKey::KEY_KP_ENTER)
        {
            let name = crate::editor::TITLE_RENAME_DRAFT.read().unwrap().clone();
            crate::editor::TITLE_RENAME_DRAFT.write().unwrap().clear();
            crate::editor::TITLE_RENAME_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
            crate::editor::tabs::rename_active(&name);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            crate::editor::TITLE_RENAME_DRAFT.write().unwrap().clear();
            crate::editor::TITLE_RENAME_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        return;
    }

    let visual_lines = buffer::VISUAL_LINES.lock().unwrap();

    // ------------------------------------------------------------
    // Tab bar: wheel over it scrolls it horizontally; a press selects or
    // closes a tab. Both are handled before any content/caret logic so the
    // two never fight over the same input.
    // ------------------------------------------------------------
    {
        let (bbx, bby, bbw, _) = crate::editor::panel_bounds();
        let tab_y = bby + config::EDITOR_HEADER_HEIGHT;
        if crate::editor::tabs::handle_bar_wheel(rl, bbx, tab_y, bbw) {
            return;
        }
        if crate::editor::tabs::handle_bar_click(rl, bbx, tab_y, bbw) {
            return;
        }
    }

    let mut buffer = buffer::BUFFER.write().unwrap();
    let mut cursor_x = buffer::CURSOR_X.write().unwrap();
    let mut cursor_y = buffer::CURSOR_Y.write().unwrap();
    let mut anchor_x = buffer::ANCHOR_X.write().unwrap();
    let mut anchor_y = buffer::ANCHOR_Y.write().unwrap();

    if buffer.is_empty() {
        buffer.push(String::new());
    }

    // Printable characters typed this frame, collected ONCE up front: the
    // autocomplete/slash gates need to know "did the user type just now?",
    // and a navigation-mode shortcut key (e/g/q) must never print itself into
    // the buffer. The actual insertion happens later in the text-input path.
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
    let typing_this_frame = !typed.is_empty()
        || rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
        || rl.is_key_pressed(KeyboardKey::KEY_DELETE)
        || rl.is_key_pressed_repeat(KeyboardKey::KEY_DELETE);

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
        let step = config::editor_scaled_size(config::EDITOR_FONT_SIZE) as f32 * 3.0;
        let mut scroll = buffer::SCROLL_Y.write().unwrap();
        let delta = (wheel * step) as i32;
        if delta != 0 {
            *scroll -= delta;
        }
    }

    // ------------------------------------------------------------
    // Navigation mode: the caret is detached from the text (every line
    // renders as formatted view-source) and the cursor line is only
    // *selected*, highlighted by a bar. Arrow keys move the selection across
    // source lines; Enter/e re-enter edit mode on the selected line; g closes
    // the editor into the graph; q toggles editor maximization; Esc closes
    // the editor as well. J opens today's journal note and, when the open
    // note is a dated journal entry, [ / ] step a day backward/forward. A
    // left-click inside the content selects the clicked line and drops
    // straight back into editing (the normal mouse block below places the
    // caret). Everything else is ignored here; the typed queue was already
    // drained above so a shortcut can never type itself.
    // ------------------------------------------------------------
    if crate::editor::NAV_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        if rl.is_key_pressed(KeyboardKey::KEY_ENTER) || rl.is_key_pressed(KeyboardKey::KEY_E) {
            crate::editor::NAV_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_G) || rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            crate::editor::CLOSE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_Q) {
            let fs = crate::editor::FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed);
            crate::editor::FULLSCREEN.store(!fs, std::sync::atomic::Ordering::Relaxed);
            return;
        }
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
            return;
        }
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
            return;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_HOME) || rl.is_key_pressed_repeat(KeyboardKey::KEY_HOME) {
            *cursor_y = 0;
            *cursor_x = 0;
            *anchor_x = 0;
            *anchor_y = 0;
            return;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_END) || rl.is_key_pressed_repeat(KeyboardKey::KEY_END) {
            let last = buffer.len() - 1;
            *cursor_y = last as i32;
            *cursor_x = buffer[last].len() as i32;
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
            return;
        }
        // Journal shortcuts, only meaningful in nav mode where every other
        // key is inert: J opens today's note, [ / ] step a day back/forward
        // from whichever dated journal note is open. The tab switch is
        // deferred through OPEN_NOTE_REQUESTED because BUFFER's write lock
        // is held for this whole run; only the file/node ensure happens here.
        if rl.is_key_pressed(KeyboardKey::KEY_J) {
            crate::journal::open_today_requested();
            return;
        }
        let prev = rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_LEFT_BRACKET);
        let next = rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_RIGHT_BRACKET);
        if prev || next {
            // A non-journal tab makes the step a no-op; either way the key
            // is consumed so it never reaches the caret below.
            crate::journal::open_adjacent_requested(if prev { -1 } else { 1 });
            return;
        }
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            let m = rl.get_mouse_position();
let (_ex, ey, _ew, eh) = crate::editor::content_bounds();
            let content_top = ey + config::EDITOR_HEADER_HEIGHT + crate::editor::tabs::TABS_H;
            let content_h = eh - config::EDITOR_HEADER_HEIGHT - crate::editor::tabs::TABS_H;
            if (m.y as i32) >= content_top && (m.y as i32) < content_top + content_h {
                // Select the clicked line and re-enter edit mode; the normal
                // mouse block below places the caret this same frame.
                crate::editor::NAV_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
                hit_test::reset_click_state();
                // Fall through to caret placement.
            } else {
                return;
            }
        } else {
            return;
        }
    }

    // ------------------------------------------------------------
    // Mouse: click to place the caret, drag to select, double/triple-click
    // for word/line selection, click to accept an autocomplete row, and
    // click on the collapsed frontmatter bar to expand it.
    // ------------------------------------------------------------

    {
        let (ex, ey, ew, eh) = crate::editor::content_bounds();
        let header_h = config::EDITOR_HEADER_HEIGHT;
        let content_top = ey + header_h + crate::editor::tabs::TABS_H;
        let content_h = eh - header_h - crate::editor::tabs::TABS_H;
        let bar_w = 6;
        let bar_x = ex + ew - bar_w - config::scaled_size(config::EDITOR_SCROLLBAR_GAP);

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
                        let row = (idx / config::editor_scaled_size(config::AUTOCOMPLETE_ITEM_HEIGHT))
                            .clamp(0, ac.matches.len() as i32 - 1) as usize;
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
                // A click on a fold caret toggles that fold; it wins over
                // caret placement and never starts a drag.
                {
                    let markers = hit_test::FOLD_MARKERS.lock().unwrap();
                    let clicked = markers.iter().find(|(fx, fy, fw, fh, _)| {
                        m.x as i32 >= *fx
                            && m.x as i32 <= *fx + *fw
                            && m.y as i32 >= *fy
                            && m.y as i32 <= *fy + *fh
                    });
                    if let Some(&(_, _, _, _, head)) = clicked {
                        drop(markers);
                        buffer::toggle_fold(head);
                        if std::env::var("FOLD_DEBUG").is_ok() {
                            eprintln!(
                                "[fold] click hit head={} folded_after={:?}",
                                head,
                                *crate::editor::FOLDED_LINES.read().unwrap()
                            );
                        }
                        hit_test::MOUSE_DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
                        hit_test::reset_click_state();
                        return;
                    }
                }

                // A click on a rendered [[wikilink]] opens the note it names.
                // Only the target is recorded here (via OPEN_NOTE_REQUESTED):
                // the input handler holds BUFFER's write lock for its whole
                // run, so main opens the tab after it returns. This wins over
                // caret placement on formatted lines and never starts a drag.
                {
                    let hits = hit_test::LINK_HITS.lock().unwrap();
                    let clicked = hits.iter().find(|hit| {
                        m.x as i32 >= hit.x
                            && m.x as i32 <= hit.x + hit.w
                            && m.y as i32 >= hit.y
                            && m.y as i32 <= hit.y + hit.h
                    });
                    if let Some(hit) = clicked {
                        let target = hit.target.clone();
                        drop(hits);
                        if let Some((path, name)) =
                            crate::graph::processing::resolve_wikilink(&target)
                        {
                            *crate::editor::OPEN_NOTE_REQUESTED.write().unwrap() =
                                Some((path, name));
                            hit_test::MOUSE_DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
                            hit_test::reset_click_state();
                            return;
                        }
                    }
                }

                let scroll = *buffer::SCROLL_Y.read().unwrap();
                if std::env::var("FOLD_DEBUG").is_ok() {
                    eprintln!(
                        "[fold] click NOT on marker m=({}, {}) folded={:?}",
                        m.x as i32, m.y as i32,
                        *crate::editor::FOLDED_LINES.read().unwrap()
                    );
                }
                let rel_y = m.y as i32 - content_top + scroll;
                let hits = hit_test::VISUAL_HIT.lock().unwrap();

                if let Some(row) = hit_test::row_at_y(&hits, rel_y) {
                    if row.fm_bar {
                        // The fm_bar row spans the whole content width as a
                        // collapsed area, but only clicks on the pill itself
                        // open the block. Clicks on the dead band around it
                        // place no caret.
                        let pill = hit_test::FRONTMATTER_PILL.lock().unwrap();
                        let in_pill = pill.is_some_and(|(px, py, pw, ph)| {
                            m.x as i32 >= px
                                && m.x as i32 <= px + pw
                                && m.y as i32 >= py
                                && m.y as i32 <= py + ph
                        });
                        drop((hits, pill));
                        if !in_pill {
                            hit_test::MOUSE_DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
                            hit_test::reset_click_state();
                            return;
                        }
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
                        hit_test::MOUSE_DRAGGING.store(true, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }

                    if !row.image {
                        let px = m.x as i32 - (ex + config::scaled_size(config::editor_padding()));
                        let off = hit_test::offset_at_px(&buffer[row.line], row, px, |t, s, st| text::measure_styled(&rl, t, s, st));
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
                                let (start, end) = word_range_at(line, byte);
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
                            let px = m.x as i32 - (ex + config::scaled_size(config::editor_padding()));
                            let off = hit_test::offset_at_px(&buffer[row.line], row, px, |t, s, st| {
                                text::measure_styled(&rl, t, s, st)
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

    if ctrl_down(rl) && rl.is_key_pressed(KeyboardKey::KEY_C) {
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

    if ctrl_down(rl) && rl.is_key_pressed(KeyboardKey::KEY_X) {
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

    if ctrl_down(rl) && rl.is_key_pressed(KeyboardKey::KEY_V) {
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

    if ctrl_down(rl) && rl.is_key_pressed(KeyboardKey::KEY_A) {
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
        let ctrl = ctrl_down(rl);
        let shift = shift_down(rl);
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
    // Slash-command picker. The "/command" text itself stays in the buffer as
    // ordinary characters; the popup is only *shown* while the caret sits in an
    // unclosed word-initial "/..." token. Enter applies the selected command:
    // the "/..."" text is removed and the command runs. Up/Down/Tab navigate,
    // Esc dismisses. All other keys fall through to normal editing -- nothing
    // is ever swallowed. Like the [[ autocomplete, the popup only (re)opens
    // after the user types; moving the caret into a "/..." region never does.
    // ------------------------------------------------------------
    let palette_was_active = crate::editor::command::COMMAND_PALETTE
        .read()
        .unwrap()
        .active;
    if typing_this_frame || palette_was_active {
        {
            let mut cmd = crate::editor::command::COMMAND_PALETTE.write().unwrap();
            let filter =
                crate::editor::command::detect(&buffer[*cursor_y as usize], *cursor_x as usize);
            if filter.is_some() {
                // A slash-command region is authoritative over link autocomplete;
                // stop the [[ popup from also showing this frame.
                autocomplete::AUTOCOMPLETE.write().unwrap().active = false;
            }
            // Mirror the [[ autocomplete refresh logic (esc_consumed reset, match
            // filtering, sticky suppression until the region content changes).
            cmd.esc_consumed = false;
        if cmd.suppress && filter.as_deref() == Some(cmd.suppress_filter.as_str()) {
            cmd.active = false;
            cmd.matches.clear();
            drop(cmd);
        } else {
            cmd.suppress = false;
            cmd.active = false;
            cmd.matches.clear();
            cmd.selected = 0;
            if let Some(f) = filter {
                cmd.filter = f.clone();
                cmd.matches = crate::editor::command::refresh(&f);
                // Active while the region exists so Enter can still clean the
                // text up even when nothing matches. The renderer only draws
                // the popup when matches are non-empty.
                cmd.active = true;
            }
            drop(cmd);
        }
    }
    {
        let mut cmd = crate::editor::command::COMMAND_PALETTE.write().unwrap();
        if !cmd.active {
            drop(cmd);
        } else {
            // Clicking a command row runs it, like the [[ popup.
            let m = rl.get_mouse_position();
            if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                let rect = hit_test::COMMAND_RECT.lock().unwrap().clone();
                if let Some((px, py, pw, ph)) = rect {
                    let inside = m.x as i32 >= px
                        && m.x as i32 <= px + pw
                        && m.y as i32 >= py
                        && m.y as i32 <= py + ph;
                    if inside && !cmd.matches.is_empty() {
                        let row = ((m.y as i32 - py) / config::editor_scaled_size(config::AUTOCOMPLETE_ITEM_HEIGHT))
                            .clamp(0, cmd.matches.len() as i32 - 1) as usize;
                        cmd.selected = row;
                        let chosen = cmd.matches[row].1;
                        cmd.active = false;
                        cmd.matches.clear();
                        drop(cmd);
                        let y = *cursor_y as usize;
                        let x = *cursor_x as usize;
                        let newx = remove_command_text(&mut buffer, y, x);
                        *cursor_x = newx;
                        *anchor_x = newx;
                        buffer::mark_modified();
                        run_command(chosen);
                        return;
                    }
                }
                cmd.active = false;
                cmd.matches.clear();
                drop(cmd);
                return;
            }

            let up = rl.is_key_pressed(KeyboardKey::KEY_UP)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_UP);
            let down = rl.is_key_pressed(KeyboardKey::KEY_DOWN)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_DOWN);
            let tab = rl.is_key_pressed(KeyboardKey::KEY_TAB)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_TAB);
            let enter = rl.is_key_pressed(KeyboardKey::KEY_ENTER)
                || rl.is_key_pressed_repeat(KeyboardKey::KEY_ENTER);
            let esc = rl.is_key_pressed(KeyboardKey::KEY_ESCAPE);

            if (up || down || tab) && !cmd.matches.is_empty() {
                let n = cmd.matches.len();
                cmd.selected = if down || tab {
                    if cmd.selected + 1 >= n {
                        0
                    } else {
                        cmd.selected + 1
                    }
                } else if cmd.selected == 0 {
                    n - 1
                } else {
                    cmd.selected - 1
                };
                drop(cmd);
                return;
            }
            if enter {
                // Remove the "/command" text (everything from the word-initial
                // slash up to the caret), then run the selected command (if
                // any). Snapshot first so the deletion in undoable.
                let y = *cursor_y as usize;
                let x = *cursor_x as usize;
                let chosen = cmd.matches.get(cmd.selected).map(|&(_, c)| c);
                cmd.active = false;
                cmd.matches.clear();
                drop(cmd);
                history::snapshot(
                    &buffer,
                    (*cursor_y, *cursor_x),
                    history::EditKind::Other,
                    edit_now_ms,
                );
                let newx = remove_command_text(&mut buffer, y, x);
                *cursor_x = newx;
                *anchor_x = newx;
                *anchor_y = *cursor_y;
                buffer::mark_modified();
                if let Some(c) = chosen {
                    run_command(c);
                }
                return;
            }
            if esc {
                cmd.active = false;
                cmd.matches.clear();
                cmd.suppress = true;
                cmd.suppress_filter = cmd.filter.clone();
                cmd.esc_consumed = true;
                drop(cmd);
                return;
            }
            drop(cmd);
        }
    }
    } // gate: typing_this_frame || palette_was_active

    // A finished asset import (copied by main.rs into assets/) lands here:
    // insert the link on its own line below the caret, then carry on as an
    // ordinary edit so undo/autosave behave like any typed change.
    if let Some(target) = crate::editor::command::ASSET_INSERT.write().unwrap().take() {
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);
        let y = *cursor_y as usize;
        let (ny, nx) = crate::editor::command::insert_asset_link(&mut buffer, y, &target);
        *cursor_y = ny;
        *cursor_x = nx;
        *anchor_y = ny;
        *anchor_x = nx;
        // Force the renderer to follow the moved caret next frame.
        *buffer::LAST_CURSOR.write().unwrap() = (i32::MIN, i32::MIN);
        buffer::mark_modified();
    }

    // ------------------------------------------------------------
    // Autocomplete: detect [[ ... and handle its keys. The popup only (re)opens
    // after the user types inside the region (or while one is already up and
    // being filtered); merely arrowing or clicking the caret onto a [[ line
    // never summons it.
    // ------------------------------------------------------------

    let ac_was_active = autocomplete::AUTOCOMPLETE.read().unwrap().active;
    if typing_this_frame || ac_was_active {
        autocomplete::refresh(&buffer, *cursor_y as usize, *cursor_x as usize);
    }

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
    // Tab / Shift+Tab = indent / unindent. Tab inserts a real tab character
    // (TAB_WIDTH columns wide when rendered); Shift+Tab removes one leading
    // tab, falling back to up to a tab width of leading spaces.
    // ------------------------------------------------------------

    if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
        let shift = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
        let tab_width = crate::editor::markdown::TAB_WIDTH as usize;

        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::Other, edit_now_ms);

        if let Some((sy, _sx, ey, _ex)) = sel {
            // Multi-line: indent / unindent every line in the selection.
            for y in sy..=ey {
                if shift {
                    remove_indent(&mut buffer[y], tab_width);
                } else {
                    buffer[y].insert_str(0, "\t");
                }
            }
            *cursor_x = buffer[ey].len() as i32;
            *cursor_y = ey as i32;
            *anchor_x = 0;
            *anchor_y = sy as i32;
        } else {
            let y = *cursor_y as usize;
            let x = *cursor_x as usize;
            if shift {
                let removed = remove_indent(&mut buffer[y], tab_width);
                if removed > 0 {
                    *cursor_x = if (x as i32) < removed as i32 {
                        0
                    } else {
                        *cursor_x - removed as i32
                    };
                }
            } else {
                buffer[y].insert_str(x, "\t");
                *cursor_x += 1;
            }
            *anchor_x = *cursor_x;
            *anchor_y = *cursor_y;
        }
        buffer::mark_modified();
        return;
    }

    // ------------------------------------------------------------
    // Text input. `typed` was collected up front (above the mouse handling)
    // so the autocomplete/slash gates could key off it and so a compacted
    // shortcut key (e/g/q in nav mode) never reaches the buffer.
    // ------------------------------------------------------------

    if !typed.is_empty() {
        history::snapshot(&buffer, (*cursor_y, *cursor_x), history::EditKind::CharInsert, edit_now_ms);
        for c in typed {
            if let Some((sy, sx, ey, ex)) = sel {
                // Wrap selection with the typed character if it's a pair opener
                let ch_char = c;
                let wrap_pair = match ch_char {
                    '(' => Some(')'),
                    '[' => Some(']'),
                    '{' => Some('}'),
                    '<' => Some('>'),
                    '"' => Some('"'),
                    '\'' => Some('\''),
                    '`' => Some('`'),
                    _ => None,
                };
                if let Some(close) = wrap_pair {
                    // Insert around selection (single or multi-line)
                    if sy == ey {
                        let line = buffer[sy].clone();
                        let left = line[..sx].to_string();
                        let mid = line[sx..ex].to_string();
                        let right = line[ex..].to_string();
                        buffer[sy] = format!("{}{}{}{}{}", left, ch_char, mid, close, right);
                        *anchor_x = sx as i32;
                        *anchor_y = sy as i32;
                        *cursor_x = (sx + mid.len() + 2) as i32;
                        *cursor_y = sy as i32;
                    } else {
                        // Multi-line: wrap first and last
                        let first = buffer[sy].clone();
                        let last = buffer[ey].clone();
                        buffer[sy] = format!("{}{}{}", first[..sx].to_string(), ch_char, first[sx..].to_string());
                        buffer[ey] = format!("{}{}{}", last[..ex].to_string(), close, last[ex..].to_string());
                        *anchor_y = sy as i32;
                        *anchor_x = sx as i32;
                        *cursor_y = ey as i32;
                        *cursor_x = (ex + 1) as i32; // position after inserted close on last line
                    }
                } else {
                    let (nx, ny) = buffer::delete_selection(&mut buffer, sy, sx, ey, ex);
                    *cursor_x = nx;
                    *cursor_y = ny;
                    let y2 = *cursor_y as usize;
                    let x2 = *cursor_x as usize;
                    buffer[y2].insert(x2, ch_char);
                    *cursor_x += 1;
                    *anchor_x = *cursor_x;
                    *anchor_y = *cursor_y;
                }
            } else {
                let y = *cursor_y as usize;
                let x = *cursor_x as usize;
                buffer[y].insert(x, c);
                *cursor_x += 1;
                *anchor_x = *cursor_x;
                *anchor_y = *cursor_y;
            }
            buffer::mark_modified();
        }
    }

    // ------------------------------------------------------------
    // Ctrl + Backspace = delete previous word
    // ------------------------------------------------------------

    if ctrl_down(rl)
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
                let new_x = prev_word_boundary(&buffer[y], x);
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

    if ctrl_down(rl)
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
                let new_x = next_word_boundary(&buffer[y], x);
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

    if ctrl_down(rl)
        && shift_down(rl)
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

    if ctrl_down(rl)
        && shift_down(rl)
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

    if ctrl_down(rl)
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

    if ctrl_down(rl)
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

    // ------------------------------------------------------------
    // Escape: detach the caret into navigation mode. (If still navigating, Esc
    // closes the editor -- handled in the nav block above. Escape presses that
    // belonged to the autocomplete or slash popups returned earlier.)
    // ------------------------------------------------------------
    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
        crate::editor::NAV_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod word_motion_tests {
    use super::{next_word_boundary, prev_word_boundary, word_range_at};

    #[test]
    fn punctuation_is_a_separator() {
        let line = "foo-bar baz";
        assert_eq!(next_word_boundary(line, 0), 3); // end of "foo"
        assert_eq!(next_word_boundary(line, 3), 7); // skip "-", end of "bar"
        assert_eq!(next_word_boundary(line, 4), 7);
        assert_eq!(next_word_boundary(line, 7), 11); // skip space, end of "baz"

        assert_eq!(prev_word_boundary(line, 11), 8); // start of "baz"
        assert_eq!(prev_word_boundary(line, 8), 4); // skip space, start of "bar"
        assert_eq!(prev_word_boundary(line, 4), 0); // skip "-", start of "foo"
        assert_eq!(prev_word_boundary(line, 3), 0);
    }

    #[test]
    fn mixed_punctuation_and_spaces() {
        let line = "a.b,c";
        assert_eq!(next_word_boundary(line, 0), 1);
        assert_eq!(next_word_boundary(line, 1), 3); // skip ".", end of "b"
        assert_eq!(next_word_boundary(line, 3), 5); // skip ",", end of "c"
        assert_eq!(prev_word_boundary(line, 5), 4);
        assert_eq!(prev_word_boundary(line, 4), 2);
        assert_eq!(prev_word_boundary(line, 2), 0);
    }

    #[test]
    fn underscore_stays_in_words() {
        let line = "snake_case";
        assert_eq!(next_word_boundary(line, 0), 10);
        assert_eq!(prev_word_boundary(line, 10), 0);
        assert_eq!(word_range_at(line, 4), (0, 10));
    }

    #[test]
    fn leading_and_trailing_whitespace() {
        let line = "  foo  ";
        assert_eq!(next_word_boundary(line, 0), 5);
        assert_eq!(prev_word_boundary(line, 7), 2);
        assert_eq!(word_range_at(line, 0), (2, 5));
        assert_eq!(word_range_at(line, 7), (2, 5));
    }

    #[test]
    fn double_click_selects_word_or_punctuation_run() {
        let line = "foo-bar";
        assert_eq!(word_range_at(line, 0), (0, 3));
        assert_eq!(word_range_at(line, 2), (0, 3));
        assert_eq!(word_range_at(line, 3), (3, 4));
        assert_eq!(word_range_at(line, 4), (4, 7));
        assert_eq!(word_range_at(line, 7), (4, 7));
    }

    #[test]
    fn multibyte_text_never_splits_a_char() {
        let line = "héllo-wörld";
        let n = line.len();
        for x in 0..=n {
            for i in [
                next_word_boundary(line, x),
                prev_word_boundary(line, x),
                word_range_at(line, x).0,
                word_range_at(line, x).1,
            ] {
                assert!(line.is_char_boundary(i), "offset {i} split {line:?}");
            }
        }
        assert_eq!(next_word_boundary(line, 0), 6); // end of "héllo"
        assert_eq!(next_word_boundary(line, 6), n); // skip "-", end of "wörld"
        assert_eq!(prev_word_boundary(line, n), 7); // start of "wörld"
        assert_eq!(prev_word_boundary(line, 7), 0);
    }
}
