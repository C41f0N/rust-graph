use raylib::prelude::*;

use crate::config;
use crate::editor;
use crate::editor::blocks;
use crate::editor::buffer;
use crate::editor::markdown;
use crate::editor::text;
use crate::frontmatter;

use std::path::PathBuf;

// Layout for one visual line: the font size used for its glyphs, its advance
// height (image height for whole-line image links, otherwise the line font
// size; line spacing is added by the caller), and the image to draw (if any).
// The draw loop, the scrollbar and cursor-follow all derive from the same
// layout so they can never disagree about how tall a line is.
fn line_layout(
    line: &buffer::VisualLine,
    line_text: &str,
    kind: blocks::LineKind,
    editing_line: bool,
    editor_x: i32,
    padding: i32,
    max_width: i32,
) -> (i32, i32, Option<(PathBuf, i32, i32)>) {
    let mut line_font_size = config::EDITOR_FONT_SIZE;
    if !editing_line {
        if let Some((level, _)) = markdown::heading_info(line_text) {
            line_font_size = config::EDITOR_HEADING_SIZE[(level - 1) as usize];
        }
    }

    let mut advance = line_font_size;
    let mut image = None;

    // A frontmatter block the cursor is NOT editing collapses to one bar for
    // its whole extent: the opening delimiter line carries the bar height,
    // every remaining line contributes nothing.
    if kind == blocks::LineKind::Frontmatter && !editing_line {
        if line.line == 0 && line.start == 0 {
            advance = config::EDITOR_FRONTMATTER_HEIGHT;
        } else {
            advance = 0;
        }
        return (line_font_size, advance, image);
    }

    if !editing_line && line.start == 0 {
        if let Some(target) = markdown::image_link_target(line_text) {
            if let Some(path) = crate::editor::images::resolve_path(&target) {
                if crate::editor::images::has_texture(&path) {
                    let content_x = editor_x + padding + line.indent;
                    let avail_w = (editor_x + padding + max_width - content_x).max(1);
                    if let Some((w, h)) = crate::editor::images::fit(
                        &path,
                        avail_w,
                        config::EDITOR_IMAGE_MAX_HEIGHT,
                    ) {
                        image = Some((path, w, h));
                        advance = h;
                    }
                }
            }
        }
    }

    (line_font_size, advance, image)
}

// Whether a visual line renders as editable raw source. Normally that is
// "the cursor is on this line"; a frontmatter line is also raw whenever the
// cursor sits anywhere inside the block, so the header edits as one unit.
fn line_editing(
    line: &buffer::VisualLine,
    fm_range: Option<(usize, usize)>,
    cursor_y: i32,
) -> bool {
    if let Some((start, end)) = fm_range {
        let cy = cursor_y as usize;
        if cy >= start && cy <= end && line.line >= start && line.line <= end {
            return true;
        }
    }
    cursor_y as usize == line.line
}

pub fn draw(mut d: RaylibDrawHandle, editor_open: bool, editor_dimentions: Vector2) {
    let buf = buffer::BUFFER.read().unwrap();
    let cursor_x = buffer::CURSOR_X.read().unwrap();
    let cursor_y = buffer::CURSOR_Y.read().unwrap();
    let anchor_x = buffer::ANCHOR_X.read().unwrap();
    let anchor_y = buffer::ANCHOR_Y.read().unwrap();
    let font_color = config::EDITOR_FONT_COLOR;
    let padding = config::EDITOR_PADDING;

    let editor_height = (editor_dimentions.y * config::HEIGHT as f32) as i32;
    let editor_width = (editor_dimentions.x * config::WIDTH as f32) as i32;

    if editor_open {
        // The background
        d.draw_rectangle_rounded(
            Rectangle::new(
                config::WIDTH as f32 * (1. - editor_dimentions.x) * 0.5,
                config::HEIGHT as f32 * (1. - editor_dimentions.y) * 0.5,
                config::WIDTH as f32 * editor_dimentions.x,
                config::HEIGHT as f32 * editor_dimentions.y,
            ),
            0.05,
            0,
            Color::BLACK.alpha(0.5),
        );

        let editor_x = ((1.0 - editor_dimentions.x) * 0.5 * config::WIDTH as f32) as i32;
        let editor_y = ((1.0 - editor_dimentions.y) * 0.5 * config::HEIGHT as f32) as i32;

        let max_width = editor_width - 2 * padding;
        let editor_bottom = editor_y + editor_height;

        // --- Heading bar ---
        let header_h = config::EDITOR_HEADER_HEIGHT;

        // Header background (slightly lighter than the editor bg)
        d.draw_rectangle(
            editor_x,
            editor_y,
            editor_width,
            header_h,
            Color::new(30, 30, 35, 200),
        );

        // Note name on the left
        let note_name = {
            let editing = crate::graph::processing::EDITING_NODE.read().unwrap();
            let nodes = crate::graph::processing::NODES.read().unwrap();
            editing
                .and_then(|i| nodes.get(i))
                .map(|n| n.name.clone())
                .unwrap_or_default()
        };
        let name_y = editor_y + (header_h - config::EDITOR_FONT_SIZE) / 2;
        text::draw(&mut d, 
            &note_name,
            editor_x + padding,
            name_y,
            config::EDITOR_FONT_SIZE,
            config::EDITOR_FONT_COLOR,
        );

        // Close X button on the right
        let x_label = "X";
        let x_w = text::measure(&d, x_label, config::EDITOR_FONT_SIZE);
        let x_x = editor_x + editor_width - padding - x_w;
        let x_y = name_y;

        let mouse = d.get_mouse_position();
        let over_x = mouse.x as i32 >= x_x
            && mouse.x as i32 <= x_x + x_w
            && mouse.y as i32 >= x_y
            && mouse.y as i32 <= x_y + config::EDITOR_FONT_SIZE;

        let x_color = if over_x {
            Color::RED
        } else {
            Color::new(180, 180, 180, 255)
        };
        text::draw(&mut d, x_label, x_x, x_y, config::EDITOR_FONT_SIZE, x_color);

        if over_x && d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            editor::CLOSE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Separator line below the header
        d.draw_line(
            editor_x,
            editor_y + header_h,
            editor_x + editor_width,
            editor_y + header_h,
            Color::new(255, 255, 255, 40),
        );

        // --- Content area (shifted down by header) ---
        let content_y = editor_y + header_h;

        let kinds = blocks::classify(&buf);
        let fm_range = frontmatter::line_range(&buf);

        crate::editor::buffer::generate_visual_lines(max_width, &mut d);

        // Layout every visual line up front. This single layout is the source
        // of truth for the draw loop, the content height (which bounds the
        // scroll offset) and cursor-follow.
        let vlines = buffer::VISUAL_LINES.lock().unwrap();
        let mut layouts: Vec<(i32, i32, Option<(PathBuf, i32, i32)>)> =
            Vec::with_capacity(vlines.len());
        let mut total_h: i32 = 0;
        for vl in vlines.iter() {
            let editing = line_editing(vl, fm_range, *cursor_y);
            let (fsz, adv, img) =
                line_layout(vl, &buf[vl.line], kinds.get(vl.line).copied().unwrap_or(blocks::LineKind::Paragraph), editing, editor_x, padding, max_width);
            total_h += adv + config::EDITOR_LINE_SPACING;
            layouts.push((fsz, adv, img));
        }

        // Clamp the scroll offset to the real content height.
        let viewport_h = (editor_height - header_h - padding).max(1);
        let max_scroll = (total_h - viewport_h).max(0);
        let mut scroll = (*buffer::SCROLL_Y.read().unwrap()).clamp(0, max_scroll);

        // --- Scrollbar (right edge, draggable) ---
        let bar_w = 6;
        let bar_x = editor_x + editor_width - bar_w - padding;
        let mut dragging_bar = false;
        if max_scroll > 0 {
            let bar_h = viewport_h;
            let m = d.get_mouse_position();
            let over = m.x as i32 >= bar_x
                && m.x as i32 <= bar_x + bar_w
                && m.y as i32 >= content_y
                && m.y as i32 <= content_y + bar_h;
            if over && d.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                let frac = ((m.y as i32 - content_y) as f32 / bar_h as f32).clamp(0.0, 1.0);
                scroll = (frac * max_scroll as f32) as i32;
                dragging_bar = true;
            }

            d.draw_rectangle(bar_x, content_y, bar_w, bar_h, Color::new(255, 255, 255, 18));

            let thumb_h = ((bar_h as f32 * (viewport_h as f32 / total_h as f32)).max(24.0)) as i32;
            let thumb_y = content_y
                + ((scroll as f32 * (bar_h - thumb_h) as f32 / max_scroll as f32) as i32);
            let thumb_color = if over {
                Color::new(200, 200, 200, 140)
            } else {
                Color::new(160, 160, 160, 80)
            };
            d.draw_rectangle(bar_x, thumb_y, bar_w, thumb_h, thumb_color);
        }

        let sel = buffer::selection_range(*anchor_x, *anchor_y, *cursor_x, *cursor_y);

        // Cursor line range (content-relative) discovered during the draw
        // loop, used afterwards to keep the cursor visible.
        let mut cursor_line_top: Option<i32> = None;
        let mut cursor_line_bottom: Option<i32> = None;

        // Clip all content drawing to the area below the heading bar and
        // translate it up by the scroll offset.
        d.draw_scissor_mode(editor_x, content_y, editor_width, editor_height - header_h, |mut s| {
            let mut line_y = 0;
            let mut quote_rail: Option<(i32, i32, i32)> = None;

            for (vi, line) in vlines.iter().enumerate() {
                let (line_font_size, advance, ref image) = layouts[vi];
                let editing_line = line_editing(line, fm_range, *cursor_y);
                let kind = kinds
                    .get(line.line)
                    .copied()
                    .unwrap_or(blocks::LineKind::Paragraph);
                let is_fence =
                    matches!(kind, blocks::LineKind::FencedCode | blocks::LineKind::FenceDelimiter);
                let format = !editing_line && !is_fence;

                // Hidden frontmatter bar: collapsed when the cursor is outside
                // the block. Shown on the very first visual line (line 0,
                // start 0); all remaining block lines contribute zero height
                // from the layout pass and fall through here.
                if kind == blocks::LineKind::Frontmatter && !editing_line {
                    if line.line == 0 && line.start == 0 {
                        let draw_top = content_y + line_y - scroll;
                        s.draw_rectangle(
                            editor_x + padding,
                            draw_top,
                            max_width,
                            config::EDITOR_FRONTMATTER_HEIGHT,
                            config::EDITOR_FRONTMATTER_BG,
                        );
                        text::draw(
                            &mut s,
                            "--- frontmatter ---",
                            editor_x + padding + 6,
                            draw_top + 4,
                            config::EDITOR_FONT_SIZE - 2,
                            config::EDITOR_FRONTMATTER_COLOR,
                        );
                    }
                    line_y += advance + config::EDITOR_LINE_SPACING;
                    continue;
                }

                // Record where the cursor's visual line sits so the view can
                // follow it after this frame.
                if editing_line
                    && *cursor_x as usize >= line.start
                    && *cursor_x as usize <= line.end
                {
                    cursor_line_top = Some(line_y);
                    cursor_line_bottom = Some(line_y + advance);
                }

                let content_x = editor_x + padding + line.indent;
                let draw_top = content_y + line_y - scroll;

                // Horizontal rule: draw a line instead of text.
                if !editing_line && matches!(kind, blocks::LineKind::HorizontalRule) {
                    let y_mid = draw_top + line_font_size / 2;
                    s.draw_line(
                        editor_x + padding,
                        y_mid,
                        editor_x + padding + max_width,
                        y_mid,
                        config::EDITOR_HR_COLOR,
                    );
                    line_y += advance + config::EDITOR_LINE_SPACING;
                    continue;
                }

                // Fenced code block: background for the full container width.
                if is_fence && !editing_line {
                    s.draw_rectangle(
                        editor_x + padding,
                        draw_top,
                        max_width,
                        line_font_size,
                        config::EDITOR_CODE_BG,
                    );
                }

                // Draw selection highlight for this visual line
                if let Some((sy, sx, ey, ex)) = sel {
                    if let Some((_, iw, ih)) = &image {
                        if line.line >= sy && line.line <= ey {
                            s.draw_rectangle(
                                content_x,
                                draw_top,
                                *iw,
                                *ih,
                                config::EDITOR_SELECTION_COLOR,
                            );
                        }
                    } else {
                        if line.line >= sy && line.line <= ey {
                            // Determine the selection range within this visual line
                            let vis_sel_start = if line.line == sy {
                                sx.max(line.start)
                            } else {
                                line.start
                            };
                            let vis_sel_end = if line.line == ey {
                                ex.min(line.end)
                            } else {
                                line.end
                            };

                            if vis_sel_start < vis_sel_end
                                || (vis_sel_start == vis_sel_end
                                    && line.start == line.end
                                    && vis_sel_start == line.start)
                            {
                                let x_start = content_x
                                    + text::measure(&*s, 
                                        &markdown::measure_line(
                                            &buf[line.line][line.start..vis_sel_start],
                                            format,
                                        ),
                                        line_font_size,
                                    );
                                let x_end = content_x
                                    + text::measure(&*s, 
                                        &markdown::measure_line(
                                            &buf[line.line][line.start..vis_sel_end],
                                            format,
                                        ),
                                        line_font_size,
                                    );

                                s.draw_rectangle(
                                    x_start,
                                    draw_top,
                                    x_end - x_start,
                                    line_font_size,
                                    config::EDITOR_SELECTION_COLOR,
                                );
                            }
                        }
                    }
                }

                // Draw the cursor if it's on this line
                if *cursor_y == line.line as i32
                    && *cursor_x >= line.start as i32
                    && *cursor_x <= line.end as i32
                {
                    let cursor_x_abs = content_x
                        + text::measure(&*s, 
                            &markdown::measure_line(
                                &buf[line.line][line.start..*cursor_x as usize],
                                format,
                            ),
                            line_font_size,
                        );
                    let cursor_y_abs = draw_top + padding / 2;

                    let blink = ((s.get_time() * 2.0) as i32) % 2 == 0;

                    if blink {
                        s.draw_rectangle(
                            cursor_x_abs,
                            cursor_y_abs
                                + (line_font_size as f32 * (1. - config::EDITOR_CURSOR_HEIGHT_RATIO))
                                    as i32,
                            2,
                            (line_font_size as f32 * config::EDITOR_CURSOR_HEIGHT_RATIO) as i32,
                            font_color,
                        );
                    }

                    // Autocomplete popup (drawn below the cursor line)
                    let autocomplete = crate::editor::autocomplete::AUTOCOMPLETE.read().unwrap();
                    if autocomplete.active && !autocomplete.matches.is_empty() {
                        let item_h = config::AUTOCOMPLETE_ITEM_HEIGHT;
                        let mut box_w = 140;
                        for name in autocomplete
                            .matches
                            .iter()
                            .take(config::AUTOCOMPLETE_MAX_VISIBLE)
                        {
                            let w = text::measure(&*s, name, line_font_size) + 24;
                            if w > box_w {
                                box_w = w;
                            }
                        }
                        let count = autocomplete.matches.len().min(config::AUTOCOMPLETE_MAX_VISIBLE);
                        let box_h = (count as i32) * item_h;

                        let mut pop_x = cursor_x_abs.min(editor_x + editor_width - box_w - padding);
                        let mut pop_y = cursor_y_abs + line_font_size + config::EDITOR_PADDING;
                        if pop_y + box_h > editor_bottom - padding {
                            pop_y = pop_y - box_h - line_font_size - config::EDITOR_PADDING;
                        }
                        pop_x = pop_x.max(editor_x + padding);
                        // Keep the popup inside the clipped content area.
                        pop_y = pop_y.max(content_y);
                        if pop_y + box_h > editor_bottom {
                            pop_y = (editor_bottom - box_h).max(content_y);
                        }

                        s.draw_rectangle(pop_x, pop_y, box_w, box_h, config::AUTOCOMPLETE_BG);

                        for (i, name) in autocomplete
                            .matches
                            .iter()
                            .enumerate()
                            .take(config::AUTOCOMPLETE_MAX_VISIBLE)
                        {
                            if i == autocomplete.selected {
                                s.draw_rectangle(
                                    pop_x,
                                    pop_y + (i as i32) * item_h,
                                    box_w,
                                    item_h,
                                    config::AUTOCOMPLETE_SELECTED_BG,
                                );
                            }
                            text::draw(&mut *s, 
                                name,
                                pop_x + 10,
                                pop_y + (i as i32) * item_h + 2,
                                line_font_size,
                                Color::WHITE,
                            );
                        }
                    }
                    drop(autocomplete);
                }

                let is_quote = matches!(kind, blocks::LineKind::Blockquote);

                // Accumulate the quote rail across contiguous quote visual lines
                // so a block reads as one continuous vertical bar. The rail is
                // hidden where the cursor is editing (raw markers are shown).
                if is_quote && !editing_line {
                    let (_, _, h) =
                        quote_rail.get_or_insert((editor_x + padding, draw_top, 0));
                    *h += advance + config::EDITOR_LINE_SPACING;
                } else {
                    if let Some((x, top, h)) = quote_rail.take() {
                        s.draw_rectangle(x, top, 3, h, config::EDITOR_BLOCKQUOTE_BAR);
                    }
                }

                let list_marker = if !editing_line && line.start == 0 {
                    markdown::list_marker_display(&buf[line.line])
                } else {
                    None
                };

                if let Some((path, iw, ih)) = &image {
                    // A whole-line image link is drawn as its image. Any leading
                    // list marker is drawn first, then the image is scaled down
                    // to fit the available width and max image height.
                    let mut img_x = content_x;
                    if let Some((disp, _)) = &list_marker {
                        text::draw(
                            &mut *s,
                            disp,
                            img_x,
                            draw_top + padding / 2,
                            line_font_size,
                            config::EDITOR_LIST_MARKER_COLOR,
                        );
                        img_x += text::measure(&*s, disp, line_font_size);
                    }
                    crate::editor::images::draw(
                        &mut *s,
                        path,
                        img_x,
                        draw_top + padding / 2,
                        (editor_x + padding + max_width - img_x).max(1),
                        config::EDITOR_IMAGE_MAX_HEIGHT,
                    );
                    let _ = (*iw, *ih);
                } else {
                    let slice_text = buf[line.line][line.start..line.end].to_string();

                    let fence_color = if is_fence {
                        if matches!(kind, blocks::LineKind::FenceDelimiter) {
                            Some(config::EDITOR_FENCE_COLOR)
                        } else {
                            Some(config::EDITOR_CODE_COLOR)
                        }
                    } else {
                        None
                    };

                    let segments = markdown::render_line(&slice_text, format);
                    let mut seg_x = content_x;

                    for (si, seg) in segments.iter().enumerate() {
                        let mut text = seg.text.as_str();
                        if si == 0 {
                            // Quotes: the whole marker prefix (`>`, optionally
                            // preceded by spaces) is display-only in view mode; the
                            // rail replaces it and the text keeps its indent.
                            if is_quote && !editing_line && line.start == 0 {
                                if let Some(skip) = markdown::blockquote_marker_len(&buf[line.line])
                                {
                                    if skip <= text.as_bytes().len()
                                        && text.as_bytes()[..skip]
                                            == buf[line.line].as_bytes()[..skip]
                                    {
                                        text = &text[skip..];
                                    }
                                }
                            }
                            if let Some((disp, raw_len)) = &list_marker {
                                if text.as_bytes().len() >= *raw_len
                                    && text.as_bytes()[..*raw_len]
                                        == buf[line.line].as_bytes()[..*raw_len]
                                {
                                    text::draw(&mut *s, 
                                        disp,
                                        seg_x,
                                        draw_top + padding / 2,
                                        line_font_size,
                                        config::EDITOR_LIST_MARKER_COLOR,
                                    );
                                    seg_x += text::measure(&*s, disp, line_font_size);
                                    text = &text[*raw_len..];
                                }
                            }
                        }
                        let color = match seg.style {
                            markdown::SegmentStyle::Plain
                            | markdown::SegmentStyle::Bold
                            | markdown::SegmentStyle::Italic => {
                                if let Some(fc) = fence_color {
                                    fc
                                } else if is_quote {
                                    config::EDITOR_BLOCKQUOTE_COLOR
                                } else {
                                    font_color
                                }
                            }
                            markdown::SegmentStyle::Link => config::EDITOR_LINK_COLOR,
                            markdown::SegmentStyle::Code => config::EDITOR_CODE_COLOR,
                        };
                        let seg_w = text::measure(&*s, text, line_font_size);

                        if seg.style == markdown::SegmentStyle::Code {
                            s.draw_rectangle(
                                seg_x,
                                draw_top + padding / 2,
                                seg_w,
                                line_font_size,
                                config::EDITOR_CODE_BG,
                            );
                        }

                        text::draw(&mut *s, 
                            text,
                            seg_x,
                            draw_top + padding / 2,
                            line_font_size,
                            color,
                        );
                        seg_x += seg_w;
                    }
                }

                line_y += advance + config::EDITOR_LINE_SPACING;
            }

            if let Some((x, top, h)) = quote_rail.take() {
                s.draw_rectangle(x, top, 3, h, config::EDITOR_BLOCKQUOTE_BAR);
            }
        });

        // Follow the cursor only when it has moved since the last frame (arrow key,
// typing, opening a file...). A wheel scroll leaves the cursor put, so the
// view stays where the user scrolled instead of snapping back. Persist the
// cursor marker and the (clamped) scroll offset.
        let cursor_moved = {
            let mut last = buffer::LAST_CURSOR.write().unwrap();
            let cur = (*cursor_x, *cursor_y);
            let moved = *last != cur;
            *last = cur;
            moved
        };
        if !dragging_bar && cursor_moved {
            if let (Some(top), Some(bottom)) = (cursor_line_top, cursor_line_bottom) {
                if top < scroll {
                    scroll = top;
                } else if bottom > scroll + viewport_h {
                    scroll = bottom - viewport_h;
                }
            }
        }
        scroll = scroll.clamp(0, max_scroll);
        *buffer::SCROLL_Y.write().unwrap() = scroll;
    }
}