use crate::config;
use crate::editor::markdown;
use crate::editor::text;
use crate::filesystem;
use raylib::prelude::*;
use std::path::Path;
use std::sync::Mutex;
use std::sync::RwLock;

pub struct VisualLine {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub indent: i32,
}

pub static BUFFER: RwLock<Vec<String>> = RwLock::new(Vec::new());
pub static VISUAL_LINES: Mutex<Vec<VisualLine>> = Mutex::new(Vec::new());

pub static CURSOR_X: RwLock<i32> = RwLock::new(0);
pub static CURSOR_Y: RwLock<i32> = RwLock::new(0);

pub static ANCHOR_X: RwLock<i32> = RwLock::new(0);
pub static ANCHOR_Y: RwLock<i32> = RwLock::new(0);

// Vertical scroll offset (pixels) of the editor content viewport. Clamped by
// the renderer each frame to the actual content height, so the input handler
// can nudge it freely (e.g. mouse wheel).
pub static SCROLL_Y: RwLock<i32> = RwLock::new(0);

// Position of the cursor as of the last rendered frame. The renderer only
// auto-follows the cursor when it has moved since the previous frame, so a
// wheel scroll (which leaves the cursor put) does not yank the view back.
pub static LAST_CURSOR: RwLock<(i32, i32)> = RwLock::new((0, 0));

pub fn selection_range(ax: i32, ay: i32, cx: i32, cy: i32) -> Option<(usize, usize, usize, usize)> {
    if ax == cx && ay == cy {
        return None;
    }
    if (ay, ax) <= (cy, cx) {
        Some((ay as usize, ax as usize, cy as usize, cx as usize))
    } else {
        Some((cy as usize, cx as usize, ay as usize, ax as usize))
    }
}

// Record that the buffer was just edited. Flags it dirty and stamps the
// current time (raylib GetTime scaled to milliseconds) so the main loop can
// autosave once typing has paused.
pub fn mark_modified() {
    crate::editor::DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
    crate::editor::LAST_EDIT_MILLIS.store(
        (rl_get_time() * 1000.0) as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn rl_get_time() -> f64 {
    unsafe { raylib::ffi::GetTime() }
}

// The editor always has a trailing empty line to move down into, like other
// block editors. Callers must not hold a BUFFER lock; reuse
// ensure_trailing_newline_locked() when a guard is already live.
pub fn ensure_trailing_newline() {
    let mut buffer = BUFFER.write().unwrap();
    ensure_trailing_newline_locked(&mut buffer);
}

fn ensure_trailing_newline_locked(buffer: &mut Vec<String>) {
    if buffer.is_empty() || !buffer.last().unwrap().is_empty() {
        buffer.push(String::new());
    }
}

pub fn delete_selection(
    buffer: &mut Vec<String>,
    start_y: usize,
    start_x: usize,
    end_y: usize,
    end_x: usize,
) -> (i32, i32) {
    if start_y == end_y {
        buffer[start_y].drain(start_x..end_x);
    } else {
        let tail = buffer[end_y][end_x..].to_string();
        buffer[start_y].truncate(start_x);
        buffer[start_y].push_str(&tail);
        for _ in start_y + 1..=end_y {
            buffer.remove(start_y + 1);
        }
    }
    (start_x as i32, start_y as i32)
}

pub fn generate_visual_lines(max_width: i32, d: &mut RaylibDrawHandle) {
    let cursor_y = crate::editor::buffer::CURSOR_Y.read().unwrap();
    let buffer = BUFFER.read().unwrap();
    let mut visual_lines = VISUAL_LINES.lock().unwrap();
    visual_lines.clear();

    let kinds = crate::editor::blocks::classify(&buffer);

    for (line_index, line) in buffer.iter().enumerate() {
        let editing_line = *cursor_y as usize == line_index;
        let kind = kinds.get(line_index).copied().unwrap_or(crate::editor::blocks::LineKind::Paragraph);

        // A nested list item sits indented by its depth; wrapped
        // continuations also hang by four spaces. A quote line is nudged
        // right by its marker width so wrapped continuations align under the
        // quote text instead of the `>` marker. All are measured like real
        // spaces so the wrap and the render never disagree.
        let mut base_indent = 0;
        let mut hang_indent = 0;
        if !editing_line {
            let font_size = config::EDITOR_FONT_SIZE;
            match kind {
                crate::editor::blocks::LineKind::List { depth } => {
                    base_indent = text::measure(d, "  ", font_size) * depth as i32;
                    hang_indent = text::measure(d, "    ", font_size);
                }
                crate::editor::blocks::LineKind::Blockquote => {
                    base_indent = markdown::blockquote_marker_len(line)
                        .map(|len| text::measure(d, &line[..len], font_size))
                        .unwrap_or(0);
                    hang_indent = 0;
                }
                _ => {}
            }
        }

        // Fence lines are drawn as raw source, so they must be measured raw.
        let format = !editing_line
            && !matches!(
                kind,
                crate::editor::blocks::LineKind::FencedCode
                    | crate::editor::blocks::LineKind::FenceDelimiter
            );

        visual_lines.extend(wrap_line(
            line,
            max_width,
            line_index,
            base_indent,
            hang_indent,
            format,
            |t, size| text::measure(d, t, size),
        ));
    }
}

// Wrap one buffer line into visual lines. `measure` is injected so the
// logic is unit-testable; on-screen it is the raylib measure_text.
// `base_indent` shifts the whole item (nested list depth), `hang_indent` is
// added to every line after the first in a list item, and `format` selects
// formatted vs raw measurement (fence lines stay raw).
fn wrap_line(
    line: &str,
    max_width: i32,
    buf_index: usize,
    base_indent: i32,
    hang_indent: i32,
    format: bool,
    measure: impl Fn(&str, i32) -> i32,
) -> Vec<VisualLine> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut line_font_size = config::EDITOR_FONT_SIZE;

    if let Some((level, skip)) = markdown::heading_info(line) {
        if format {
            start = skip;
            line_font_size = config::EDITOR_HEADING_SIZE[(level - 1) as usize];
        }
    }

    let chars: Vec<char> = line.chars().collect();

    if chars.is_empty() {
        out.push(VisualLine {
            start: 0,
            end: 0,
            line: buf_index,
            indent: 0,
        });
        return out;
    }

    // Continuation lines of a list item hang indented by four spaces.
    let list_indent = hang_indent;
    let mut emitted = false;

    while start < chars.len() {
        // The first visual line of a list item carries the marker inline
        // (indent 0); every following visual line gets the hanging indent,
        // and a nested item's whole block sits at `base_indent`.
        let cur_indent = base_indent + if emitted { list_indent } else { 0 };
        let mut end = start;
        let mut last_space = None;
        let mut text = String::new();

        while end < chars.len() {
            text.push(chars[end]);

            if chars[end].is_whitespace() {
                last_space = Some(end);
            }

            let width = cur_indent
                + measure(&markdown::measure_line(&text, format), line_font_size);

            if width > max_width {
                break;
            }

            end += 1;
        }

        if end == chars.len() {
            out.push(VisualLine {
                start,
                end,
                line: buf_index,
                indent: cur_indent,
            });
            break;
        }

        if let Some(space) = last_space {
            // Wrap at the last space.
            out.push(VisualLine {
                start,
                end: space,
                line: buf_index,
                indent: cur_indent,
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

            out.push(VisualLine {
                start,
                end,
                line: buf_index,
                indent: cur_indent,
            });

            start = end;
        }
        emitted = true;
    }

    out
}

pub fn load_from_file(path: &Path) {
    let content = filesystem::read_file(path);
    let mut buffer = BUFFER.write().unwrap();
    let mut cursor_x = CURSOR_X.write().unwrap();
    let mut cursor_y = CURSOR_Y.write().unwrap();
    let mut anchor_x = ANCHOR_X.write().unwrap();
    let mut anchor_y = ANCHOR_Y.write().unwrap();

    buffer.clear();
    // Force a follow on the first rendered frame after opening: the cursor is
    // about to move to the end of the (possibly new) file.
    *LAST_CURSOR.write().unwrap() = (i32::MIN, i32::MIN);
    if content.is_empty() {
        buffer.push(String::new());
    } else {
        for line in content.lines() {
            buffer.push(line.to_string());
        }
    }
    // Place the cursor at the end of the file
    *cursor_y = (buffer.len() - 1) as i32;
    *cursor_x = buffer.last().map_or(0, |l| l.len() as i32);
    *anchor_x = *cursor_x;
    *anchor_y = *cursor_y;
    ensure_trailing_newline_locked(&mut buffer);
    crate::editor::DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
    crate::editor::LAST_EDIT_MILLIS.store(0, std::sync::atomic::Ordering::Relaxed);
}

pub fn save_to_file(path: &Path) {
    let buffer = BUFFER.read().unwrap();
    let content = buffer.join("\n");
    drop(buffer);
    filesystem::write_file(path, &content);
    crate::editor::DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fake measure: 10px per char, independent of font size.
    fn pix(_s: &str, _n: i32) -> i32 {
        10 * _s.chars().count() as i32
    }

    #[test]
    fn plain_line_wraps_without_indent() {
        let vls = wrap_line(
            "one two three four five six seven eight",
            100,
            0,
            0,
            0,
            true,
            pix,
        );
        assert!(vls.len() >= 2);
        for vl in &vls {
            assert_eq!(vl.indent, 0);
        }
    }

    #[test]
    fn load_keeps_trailing_empty_line() {
        *BUFFER.write().unwrap() = vec!["a".into(), "b".into()];
        ensure_trailing_newline();
        let b = BUFFER.read().unwrap();
        assert_eq!(&b[..], &["a".to_string(), "b".to_string(), String::new()]);
    }

    #[test]
    fn trailing_empty_line_is_not_duplicated() {
        *BUFFER.write().unwrap() = vec!["a".into(), String::new()];
        ensure_trailing_newline();
        let b = BUFFER.read().unwrap();
        assert_eq!(&b[..], &["a".to_string(), String::new()]);
    }

    #[test]
    fn quote_continuations_hang_by_marker_width() {
        // View mode: the caller sets base = marker width, hang = 0, so every
        // visual line sits right of the rail. "> " = 2 chars = 20px.
        let line = "> one two three four five six seven eight nine ten";
        let vls = wrap_line(line, 100, 0, 20, 0, true, pix);
        assert!(vls.len() >= 2, "expected the quote to wrap");
        assert_eq!(vls[0].indent, 20, "content starts after the marker");
        for vl in &vls[1..] {
            assert_eq!(vl.indent, 20, "continuations align under the content");
        }
    }

    #[test]
    fn list_continuation_lines_hang_indented() {
        // 10 chars/line at max_width 100; "    " = 40px indent.
        let line = "- one two three four five six seven eight nine ten";
        let vls = wrap_line(line, 100, 0, 0, 40, true, pix);
        assert!(vls.len() >= 2, "expected the item to wrap");
        assert_eq!(vls[0].indent, 0, "first visual line has no indent");
        for vl in &vls[1..] {
            assert_eq!(vl.indent, 40, "continuation hangs by four spaces");
        }
    }

    #[test]
    fn nested_list_indents_by_depth() {
        // depth 1 line: base 20px (two spaces), hang 40px.
        let line = "  - one two three four five six seven eight nine ten";
        let vls = wrap_line(line, 100, 0, 20, 40, true, pix);
        assert!(vls.len() >= 2);
        assert_eq!(vls[0].indent, 20);
        for vl in &vls[1..] {
            assert_eq!(vl.indent, 60);
        }
    }

    #[test]
    fn editing_line_shows_no_indent() {
        // In real use generate_visual_lines zeroes the indents for the
        // editing line; wrap_line is a mechanical function, so the caller
        // passes 0.
        let line = "- one two three four five six seven eight nine ten";
        let vls = wrap_line(line, 100, 0, 0, 0, false, pix);
        for vl in &vls {
            assert_eq!(vl.indent, 0);
        }
    }

    #[test]
    fn empty_line_is_single_visual_line() {
        let vls = wrap_line("", 100, 3, 0, 0, true, pix);
        assert_eq!(vls.len(), 1);
        assert_eq!(vls[0].line, 3);
        assert_eq!(vls[0].indent, 0);
    }
}
