use raylib::prelude::*;

use crate::config;
use crate::editor;
use crate::editor::blocks;
use crate::editor::buffer;
use crate::editor::markdown;
use crate::editor::text;

pub fn draw(mut d: RaylibDrawHandle, editor_open: bool, editor_dimentions: Vector2) {
    let buf = buffer::BUFFER.read().unwrap();
    let cursor_x = buffer::CURSOR_X.read().unwrap();
    let cursor_y = buffer::CURSOR_Y.read().unwrap();
    let anchor_x = buffer::ANCHOR_X.read().unwrap();
    let anchor_y = buffer::ANCHOR_Y.read().unwrap();
    let font_color = config::EDITOR_FONT_COLOR;
    let padding = config::EDITOR_PADDING;

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

        let max_width = (editor_dimentions.x * config::WIDTH as f32) as i32 - 2 * padding;
        let editor_width = max_width + 2 * padding;

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

        crate::editor::buffer::generate_visual_lines(max_width, &mut d);
        let mut line_y = 0;
        let mut quote_rail: Option<(i32, i32, i32)> = None;

        let sel = buffer::selection_range(*anchor_x, *anchor_y, *cursor_x, *cursor_y);

        for line in buffer::VISUAL_LINES.lock().unwrap().iter() {
            let mut line_font_size = config::EDITOR_FONT_SIZE;
            let editing_line = *cursor_y as usize == line.line;
            let kind = kinds.get(line.line).copied().unwrap_or(blocks::LineKind::Paragraph);
            let is_fence = matches!(kind, blocks::LineKind::FencedCode | blocks::LineKind::FenceDelimiter);
            let format = !editing_line && !is_fence;

            if let Some((level, _)) = markdown::heading_info(&buf[line.line]) {
                if !editing_line {
                    line_font_size = config::EDITOR_HEADING_SIZE[(level - 1) as usize];
                }
            }

            let content_x = editor_x + padding + line.indent;

            // Horizontal rule: draw a line instead of text.
            if !editing_line && matches!(kind, blocks::LineKind::HorizontalRule) {
                let y_mid = content_y + line_y + line_font_size / 2;
                d.draw_line(
                    editor_x + padding,
                    y_mid,
                    editor_x + padding + max_width,
                    y_mid,
                    config::EDITOR_HR_COLOR,
                );
                line_y += line_font_size + config::EDITOR_LINE_SPACING;
                continue;
            }

            // Fenced code block: background for the full container width.
            if is_fence && !editing_line {
                d.draw_rectangle(
                    editor_x + padding,
                    content_y + line_y,
                    max_width,
                    line_font_size,
                    config::EDITOR_CODE_BG,
                );
            }

            // Draw selection highlight for this visual line
            if let Some((sy, sx, ey, ex)) = sel {
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

                    if vis_sel_start < vis_sel_end || (vis_sel_start == vis_sel_end && line.start == line.end && vis_sel_start == line.start) {
                        let x_start = content_x
                            + text::measure(&d, 
                                &markdown::measure_line(
                                    &buf[line.line][line.start..vis_sel_start],
                                    format,
                                ),
                                line_font_size,
                            );
                        let x_end = content_x
                            + text::measure(&d, 
                                &markdown::measure_line(
                                    &buf[line.line][line.start..vis_sel_end],
                                    format,
                                ),
                                line_font_size,
                            );

                        d.draw_rectangle(
                            x_start,
                            content_y + line_y,
                            x_end - x_start,
                            line_font_size,
                            config::EDITOR_SELECTION_COLOR,
                        );
                    }
                }
            }

            // Draw the cursor if it's on this line
            if *cursor_y == line.line as i32
                && *cursor_x >= line.start as i32
                && *cursor_x <= line.end as i32
            {
                let cursor_x_abs = content_x
                    + text::measure(&d, 
                        &markdown::measure_line(
                            &buf[line.line][line.start..*cursor_x as usize],
                            format,
                        ),
                        line_font_size,
                    );
                let cursor_y_abs = content_y + line_y + padding / 2;

                let blink = ((d.get_time() * 2.0) as i32) % 2 == 0;

                if blink {
                    d.draw_rectangle(
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
                        let w = text::measure(&d, name, line_font_size) + 24;
                        if w > box_w {
                            box_w = w;
                        }
                    }
                    let count = autocomplete
                        .matches
                        .len()
                        .min(config::AUTOCOMPLETE_MAX_VISIBLE);
                    let box_h = (count as i32) * item_h;

                    let editor_right =
                        editor_x + (editor_dimentions.x * config::WIDTH as f32) as i32;
                    let editor_bottom =
                        editor_y + (editor_dimentions.y * config::HEIGHT as f32) as i32;

                    let mut pop_x = cursor_x_abs.min(editor_right - box_w - padding);
                    let mut pop_y = cursor_y_abs + line_font_size + config::EDITOR_PADDING;
                    if pop_y + box_h > editor_bottom - padding {
                        pop_y = pop_y - box_h - line_font_size - config::EDITOR_PADDING;
                    }
                    pop_x = pop_x.max(editor_x + padding);

                    d.draw_rectangle(pop_x, pop_y, box_w, box_h, config::AUTOCOMPLETE_BG);

                    for (i, name) in autocomplete
                        .matches
                        .iter()
                        .enumerate()
                        .take(config::AUTOCOMPLETE_MAX_VISIBLE)
                    {
                        if i == autocomplete.selected {
                            d.draw_rectangle(
                                pop_x,
                                pop_y + (i as i32) * item_h,
                                box_w,
                                item_h,
                                config::AUTOCOMPLETE_SELECTED_BG,
                            );
                        }
                        text::draw(&mut d, 
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

            let slice_text = buf[line.line][line.start..line.end].to_string();

            let is_quote = matches!(kind, blocks::LineKind::Blockquote);

            // Accumulate the quote rail across contiguous quote visual lines
            // so a block reads as one continuous vertical bar. The rail is
            // hidden where the cursor is editing (raw markers are shown).
            if is_quote && !editing_line {
                let (_, _, h) = quote_rail.get_or_insert((editor_x + padding, content_y + line_y, 0));
                *h += line_font_size + config::EDITOR_LINE_SPACING;
            } else {
                if let Some((x, top, h)) = quote_rail.take() {
                    d.draw_rectangle(x, top, 3, h, config::EDITOR_BLOCKQUOTE_BAR);
                }
            }

            let list_marker = if !editing_line && line.start == 0 {
                markdown::list_marker_display(&buf[line.line])
            } else {
                None
            };

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
                        if let Some(skip) = markdown::blockquote_marker_len(&buf[line.line]) {
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
                            text::draw(&mut d, 
                                disp,
                                seg_x,
                                content_y + line_y + padding / 2,
                                line_font_size,
                                config::EDITOR_LIST_MARKER_COLOR,
                            );
                            seg_x += text::measure(&d, disp, line_font_size);
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
                let seg_w = text::measure(&d, text, line_font_size);

                if seg.style == markdown::SegmentStyle::Code {
                    d.draw_rectangle(
                        seg_x,
                        content_y + line_y + padding / 2,
                        seg_w,
                        line_font_size,
                        config::EDITOR_CODE_BG,
                    );
                }

                text::draw(&mut d, 
                    text,
                    seg_x,
                    content_y + line_y + padding / 2,
                    line_font_size,
                    color,
                );
                seg_x += seg_w;
            }

            line_y += line_font_size + config::EDITOR_LINE_SPACING;
        }

        if let Some((x, top, h)) = quote_rail.take() {
            d.draw_rectangle(x, top, 3, h, config::EDITOR_BLOCKQUOTE_BAR);
        }
    }
}
