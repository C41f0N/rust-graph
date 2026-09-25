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

// Inclusive range of source-line indices that were on screen in the last
// rendered frame, set by the editor renderer. The main loop preloads only the
// images within this range, so scrolling through a long note does not decode
// every image it references.
pub static DRAW_LINE_RANGE: RwLock<(usize, usize)> = RwLock::new((0, 0));

// Position of the cursor as of the last rendered frame. The renderer only
// auto-follows the cursor when it has moved since the previous frame, so a
// wheel scroll (which leaves the cursor put) does not yank the view back.
pub static LAST_CURSOR: RwLock<(i32, i32)> = RwLock::new((0, 0));

// Make the renderer re-follow the caret next frame even though the cursor did
// not move. Used after the text scale changes, when the new re-wrapped layout
// may have pushed the caret off-screen.
pub fn force_follow_caret() {
    *LAST_CURSOR.write().unwrap() = (i32::MIN, i32::MIN);
}

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

// Clamp an inclusive (start, end) source-line range to a buffer of `len`
// lines. Returns None when there is nothing clamped to (empty buffer or
// start after end).
pub fn clamp_line_range(start: usize, end: usize, len: usize) -> Option<(usize, usize)> {
    if len == 0 {
        return None;
    }
    let start = start.min(len - 1);
    let end = end.min(len - 1);
    if start > end {
        return None;
    }
    Some((start, end))
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

// Indent-folding tree for the buffer: one (head, hidden_until) pair per line
// whose following line is strictly deeper-indented. `hidden_until` is the
// first line NOT more indented than `head`, so the foldable block is
// head+1..hidden_until-1. A heading opens an indentation block: neither the
// heading itself nor anything holding its indentation folds, because
// "content belongs to a heading" is expressed by indent, not by collapsing.
// Otherwise purely whitespace-based (tabs count as a tab width), the same way
// a code editor outlines a block.
pub fn fold_ranges(buf: &[String]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut heading_indent: Option<u32> = None;
    for i in 0..buf.len().saturating_sub(1) {
        let (_, cols) = markdown::content_indent(&buf[i]);

        // Leave the heading block once a line dedents back to the heading's
        // own indent or shallower.
        if let Some(h) = heading_indent {
            if cols <= h {
                heading_indent = None;
            }
        }
        if markdown::heading_after_markers(&buf[i]).is_some() {
            heading_indent = Some(cols);
            continue;
        }
        if heading_indent.is_some() {
            continue;
        }

        if markdown::content_indent(&buf[i + 1]).1 > cols {
            let mut j = i + 1;
            while j < buf.len() && markdown::content_indent(&buf[j]).1 > cols {
                j += 1;
            }
            out.push((i, j));
        }
    }
    out
}

// Snap a source-line index out of any fold currently hiding it. When the
// caret (or selection anchor) lands inside a folded block returns the first
// line after that block; otherwise returns `idx` untouched.
pub fn snap_out_of_fold(idx: usize) -> usize {
    let buffer = BUFFER.read().unwrap();
    let ranges = fold_ranges(&buffer);
    let folded = crate::editor::FOLDED_LINES.read().unwrap();
    for &(head, end) in &ranges {
        if idx > head && idx < end && folded.contains(&head) {
            return end.min(buffer.len().saturating_sub(1));
        }
    }
    idx
}

// Snap the caret (and selection anchor) out of any fold that is currently
// folded, so a cursor can never sit inside an invisible region. The renderer
// calls this before taking its cursor guards (generate_visual_lines runs
// later, while those guards may be held).
pub fn snap_cursors_out_of_folds() {
    let cy = snap_out_of_fold(*CURSOR_Y.read().unwrap() as usize) as i32;
    let ay = snap_out_of_fold(*ANCHOR_Y.read().unwrap() as usize) as i32;
    let cur_cy = *CURSOR_Y.read().unwrap();
    let cur_ay = *ANCHOR_Y.read().unwrap();
    if cy != cur_cy || ay != cur_ay {
        *CURSOR_Y.write().unwrap() = cy;
        *ANCHOR_Y.write().unwrap() = ay;
    }
}

// Toggle the folded state of a fold-head line. The resulting set is pruned of
// stale entries each visual-lines pass.
pub fn toggle_fold(head: usize) {
    let mut folded = crate::editor::FOLDED_LINES.write().unwrap();
    if let Some(pos) = folded.iter().position(|h| *h == head) {
        folded.remove(pos);
    } else {
        folded.push(head);
    }
}

pub fn generate_visual_lines(max_width: i32, d: &mut RaylibDrawHandle) {
    let buffer = BUFFER.read().unwrap();
    let mut visual_lines = VISUAL_LINES.lock().unwrap();
    visual_lines.clear();

    let kinds = crate::editor::blocks::classify(&buffer);

    // Fold bookkeeping: prune stale folded heads (a dead toggle click must not
    // fold a block that no longer exists), mark every line an active fold
    // hides, and snap the caret/anchor out of anything that just got hidden so
    // a cursor can never end up inside an invisible region.
    let ranges = fold_ranges(&buffer);
    {
        let mut folded = crate::editor::FOLDED_LINES.write().unwrap();
        folded.retain(|h| ranges.iter().any(|&(head, _)| head == *h));
    }
    let folded = crate::editor::FOLDED_LINES.read().unwrap();
    let mut hidden = std::collections::HashSet::new();
    for &(head, end) in &ranges {
        if folded.contains(&head) {
            hidden.extend(head + 1..end);
        }
    }

    let cursor_y = CURSOR_Y.read().unwrap();

    for (line_index, line) in buffer.iter().enumerate() {
        if hidden.contains(&line_index) {
            continue;
        }
        // While a mouse selection is in progress every line is measured in
        // view mode (matching renderer::line_editing), so wrapping can't shift
        // under the pointer mid-drag.
        let editing_line = *cursor_y as usize == line_index
            && !crate::editor::hit_test::MOUSE_DRAGGING.load(std::sync::atomic::Ordering::Relaxed);
        let kind = kinds.get(line_index).copied().unwrap_or(crate::editor::blocks::LineKind::Paragraph);

        // A nested list item sits indented by one tab width (four spaces) per
        // depth; wrapped continuations hang by two spaces so they align under
        // the item text after the marker. A quote line is nudged right by its
        // marker width so wrapped continuations align under the quote text
        // instead of the `>` marker. A prose/heading line with leading source
        // indentation is shifted right by that indentation, so its wrapped
        // continuations hang at the same left edge as the first line instead
        // of snapping back to the margin. All are measured like real spaces
        // so the wrap and the render never disagree.
        let mut base_indent = 0;
        let mut hang_indent = 0;
        if !editing_line {
            let font_size = config::scaled_size(config::EDITOR_FONT_SIZE);
            match kind {
                crate::editor::blocks::LineKind::List { depth } => {
                    base_indent = text::measure(d, "    ", font_size) * depth as i32;
                    hang_indent = text::measure(d, "  ", font_size);
                }
                crate::editor::blocks::LineKind::Blockquote => {
                    base_indent = markdown::blockquote_marker_len(line)
                        .map(|len| text::measure(d, &line[..len], font_size))
                        .unwrap_or(0);
                    hang_indent = 0;
                }
                crate::editor::blocks::LineKind::Paragraph
                | crate::editor::blocks::LineKind::Heading { .. } => {
                    // Whole-line image links draw their image (see line_layout),
                    // which already honors line.indent, so source indentation
                    // must not re-shift them through the text path.
                    if markdown::image_link_target(line).is_none() {
                        let (bytes, cols) = markdown::content_indent(line);
                        if bytes > 0 {
                            base_indent = text::measure(d, &" ".repeat(cols as usize), font_size);
                            hang_indent = 0;
                        }
                    }
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

        // Prose with base_indent gives its leading whitespace to the wrap pass,
        // which starts measuring past it; the renderer then slices from that
        // same offset, so the whitespace is never drawn twice. Lists/quotes
        // own their leading region through the marker/rail draw, so only prose
        // lines take the skip.
        let skip = if base_indent > 0
            && matches!(
                kind,
                crate::editor::blocks::LineKind::Paragraph
                    | crate::editor::blocks::LineKind::Heading { .. }
            ) {
            markdown::content_indent(line).0
        } else {
            0
        };

        visual_lines.extend(wrap_line(
            line,
            max_width,
            line_index,
            base_indent,
            hang_indent,
            kind,
            format,
            skip,
            |t, size, style| text::measure_styled(d, t, size, style),
        ));
    }
}

// Wrap one buffer line into visual lines. `measure` is injected so the
// logic is unit-testable; on-screen it is the raylib measure_text.
// `base_indent` shifts the whole item (nested list depth, quote rail, prose
// indentation), `hang_indent` is added to every line after the first in a
// list item, `kind` decides whether the line's source indent is drawn by the
// marker (lists) or by the renderer (everything else), and `format` selects
// formatted vs raw measurement (fence lines stay raw). For formatted prose
// `skip` is the leading-indentation width already accounted for by
// base_indent: measurement starts past it and the first visual line reports
// that offset, so the renderer slices straight to the content.
fn wrap_line(
    line: &str,
    max_width: i32,
    buf_index: usize,
    base_indent: i32,
    hang_indent: i32,
    kind: crate::editor::blocks::LineKind,
    format: bool,
    skip: usize,
    measure: impl Fn(&str, i32, crate::editor::markdown::SegmentStyle) -> i32,
) -> Vec<VisualLine> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut line_font_size = config::scaled_size(config::EDITOR_FONT_SIZE);

    let prefix = markdown::parse_prefix(line);
    if let Some(level) = prefix.heading_level() {
        if format {
            // Skip the heading markers into the content when the heading starts
            // the raw line (`## hi`, `\t## hi`, `  ## hi`). When it sits inside
            // a list/quote item (e.g. `- ## hi`) the leading markers stay in the
            // text and the draw pass strips them, so only the font size changes
            // here.
            if markdown::heading_info(line).is_some() {
                start = prefix.content();
            }
            line_font_size =
                config::scaled_size(config::EDITOR_HEADING_SIZE[(level - 1) as usize]);
        }
    } else if format && skip > 0 {
        // Prose indentation: base_indent already shifts the line; `skip` moves
        // measurement (and the first visual line's slice) past the whitespace
        // so it is never drawn a second time.
        start = skip;
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

    // Continuation lines of a list item hang under the item text.
    let list_indent = hang_indent;
    let mut emitted = false;

    while start < chars.len() {
        // A list's first visual line starts at the left edge: the indent it
        // carries in the source is part of the displayed marker (e.g. "    *
        // "), so the renderer must not paint base_indent again or the item is
        // double-shifted right. Continuations add the hanging indent after
        // base_indent, lining up under the text following the bullet. Every
        // other kind is drawn with its first line shifted by base_indent.
        let cur_indent = if emitted {
            base_indent + list_indent
        } else if matches!(
            kind,
            crate::editor::blocks::LineKind::List { .. }
        ) {
            0
        } else {
            base_indent
        };
        let mut end = start;
        let mut last_space = None;
        let mut text = String::new();

        while end < chars.len() {
            text.push(chars[end]);

            if chars[end].is_whitespace() {
                last_space = Some(end);
            }

            let width = cur_indent
                + markdown::segments_width(
                    &markdown::render_line(&text, format),
                    line_font_size,
                    &measure,
                );

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

    // A drag or double-click chain started before the editor closed must not
    // leak into the freshly loaded buffer.
    crate::editor::hit_test::reset_click_state();
    // A newly loaded note starts with a clean undo/redo history.
    crate::editor::history::clear();
    // Folds belong to the note that produced them; a fresh note starts open.
    crate::editor::FOLDED_LINES.write().unwrap().clear();

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
    use crate::editor::blocks::LineKind;

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
            LineKind::Paragraph,
            true,
            0,
            |t, s, _st| pix(t, s),
        );
        assert!(vls.len() >= 2);
        for vl in &vls {
            assert_eq!(vl.indent, 0);
        }
    }

    #[test]
    fn indented_paragraph_hangs_at_written_indent() {
        // 10px/char at max_width 100. "    " is 4 chars = 40px of leading ws.
        let line = "    alpha beta gamma delta epsilon zeta";
        let vls = wrap_line(
            line,
            100,
            0,
            40,
            0,
            LineKind::Paragraph,
            true,
            4,
            |t, s, _st| pix(t, s),
        );
        assert!(vls.len() >= 2, "expected the indented paragraph to wrap");
        assert_eq!(vls[0].indent, 40, "first line sits at the written indent");
        assert_eq!(vls[0].start, 4, "measurement starts past the whitespace");
        for vl in &vls[1..] {
            assert_eq!(vl.indent, 40, "continuations hang at the written indent");
            assert!(vl.start >= 4, "no continuation re-draws the whitespace");
        }
    }

    #[test]
    fn editing_line_prose_keeps_zero_skip() {
        // The caret line renders raw: no indent, no skip.
        let line = "    alpha beta gamma delta epsilon zeta";
        let vls = wrap_line(
            line,
            100,
            0,
            0,
            0,
            LineKind::Paragraph,
            false,
            0,
            |t, s, _st| pix(t, s),
        );
        for vl in &vls {
            assert_eq!(vl.indent, 0);
        }
        assert_eq!(vls[0].start, 0, "raw editing keeps the leading whitespace");
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
        let vls = wrap_line(line, 100, 0, 20, 0, LineKind::Blockquote, true, 0, |t, s, _st| pix(t, s));
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
        let vls = wrap_line(line, 100, 0, 0, 40, LineKind::List { depth: 0 }, true, 0, |t, s, _st| pix(t, s));
        assert!(vls.len() >= 2, "expected the item to wrap");
        assert_eq!(vls[0].indent, 0, "first visual line has no indent");
        for vl in &vls[1..] {
            assert_eq!(vl.indent, 40, "continuation hangs by four spaces");
        }
    }

    #[test]
    fn nested_list_indents_by_depth() {
        // depth 1 line: the marker draws the source indent, so the first
        // visual line is flat; continuations hang at base 20px + hang 40px.
        let line = "  - one two three four five six seven eight nine ten";
        let vls = wrap_line(line, 100, 0, 20, 40, LineKind::List { depth: 1 }, true, 0, |t, s, _st| pix(t, s));
        assert!(vls.len() >= 2);
        assert_eq!(vls[0].indent, 0);
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
        let vls = wrap_line(line, 100, 0, 0, 0, LineKind::List { depth: 0 }, false, 0, |t, s, _st| pix(t, s));
        for vl in &vls {
            assert_eq!(vl.indent, 0);
        }
    }

    #[test]
    fn empty_line_is_single_visual_line() {
        let vls = wrap_line("", 100, 3, 0, 0, LineKind::Paragraph, true, 0, |t, s, _st| pix(t, s));
        assert_eq!(vls.len(), 1);
        assert_eq!(vls[0].line, 3);
        assert_eq!(vls[0].indent, 0);
    }

    #[test]
    fn fold_ranges_outline_deeper_indented_block() {
        // A line with tabs/spaces under a shallower line is foldable; a
        // flattened sibling ends the block. content_indent counts a tab as 4
        // columns, so "\t\t" = 8.
        let buf = vec![
            "- item".to_string(),
            "\t- sub".to_string(),
            "\t\t- deep".to_string(),
            "plain".to_string(),
        ];
        let ranges = fold_ranges(&buf);
        assert_eq!(ranges, vec![(0, 3), (1, 3)]);
    }

    #[test]
    fn fold_ranges_skip_flat_sequence() {
        let buf = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert!(fold_ranges(&buf).is_empty());
    }

    #[test]
    fn fold_ranges_stop_at_blank_and_less_indented_lines() {
        // blank (0 cols) and the dedented sibling close the deeper run.
        let buf = vec![
            "head".to_string(),
            "    a".to_string(),
            "".to_string(),
            "tail".to_string(),
        ];
        assert_eq!(fold_ranges(&buf), vec![(0, 2)]);
    }

    #[test]
    fn fold_ranges_skip_heading_heads() {
        // Content "belongs to" a heading by indentation, never by folding, so
        // a heading line is not a fold head even when deeper lines follow.
        let buf = vec![
            "## Title".to_string(),
            "    detail".to_string(),
            "        more".to_string(),
        ];
        assert!(fold_ranges(&buf).is_empty());
    }

    #[test]
    fn fold_ranges_heading_inside_list_still_not_a_head() {
        // "- ## hi" renders as a heading too; its continuation stays visible.
        let buf = vec!["- ## hi".to_string(), "      body".to_string()];
        assert!(fold_ranges(&buf).is_empty());
    }

    #[test]
    fn clamp_line_range_bounds_and_none() {
        // In-bounds range is kept as-is.
        assert_eq!(clamp_line_range(3, 7, 10), Some((3, 7)));
        // The range is clamped to the buffer length.
        assert_eq!(clamp_line_range(8, 90, 10), Some((8, 9)));
        // Empty buffer and inverted ranges have nothing to load.
        assert_eq!(clamp_line_range(0, 0, 0), None);
        assert_eq!(clamp_line_range(5, 2, 10), None);
    }
}
