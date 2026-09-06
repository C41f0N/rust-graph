use raylib::prelude::*;

use crate::config;
use crate::editor::buffer;
use crate::editor::markdown;

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

        crate::editor::buffer::generate_visual_lines(max_width, &mut d);
        let mut line_y = 0;

        let sel = buffer::selection_range(*anchor_x, *anchor_y, *cursor_x, *cursor_y);

        for line in buffer::VISUAL_LINES.lock().unwrap().iter() {
            let mut line_font_size = config::EDITOR_FONT_SIZE;

            if buf[line.line].starts_with("# ") && *cursor_y as usize != line.line {
                line_font_size = config::EDITOR_FONT_SIZE_H1;
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
                        let x_start = editor_x
                            + padding
                            + d.measure_text(
                                buf[line.line][line.start..vis_sel_start]
                                    .to_string()
                                    .as_str(),
                                line_font_size,
                            );
                        let x_end = editor_x
                            + padding
                            + d.measure_text(
                                buf[line.line][line.start..vis_sel_end]
                                    .to_string()
                                    .as_str(),
                                line_font_size,
                            );

                        d.draw_rectangle(
                            x_start,
                            editor_y + line_y,
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
                let cursor_x_abs = editor_x
                    + d.measure_text(
                        buf[line.line].as_str()[line.start..*cursor_x as usize]
                            .to_string()
                            .as_str(),
                        line_font_size,
                    )
                    + padding;
                let cursor_y_abs = editor_y + line_y + padding / 2;

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
                        let w = d.measure_text(name, line_font_size) + 24;
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
                        d.draw_text(
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

            let segments = markdown::line_segments(&slice_text);
            let mut seg_x = editor_x + padding;

            for seg in segments {
                let color = match seg.style {
                    markdown::SegmentStyle::Plain => font_color,
                    markdown::SegmentStyle::Link => config::EDITOR_LINK_COLOR,
                    markdown::SegmentStyle::Code => config::EDITOR_CODE_COLOR,
                };
                let seg_w = d.measure_text(&seg.text, line_font_size);

                if seg.style == markdown::SegmentStyle::Code {
                    d.draw_rectangle(
                        seg_x,
                        editor_y + line_y + padding / 2,
                        seg_w,
                        line_font_size,
                        config::EDITOR_CODE_BG,
                    );
                }

                d.draw_text(
                    &seg.text,
                    seg_x,
                    editor_y + line_y + padding / 2,
                    line_font_size,
                    color,
                );
                seg_x += seg_w;
            }

            line_y += line_font_size + config::EDITOR_LINE_SPACING;
        }
    }
}
