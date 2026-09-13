use raylib::prelude::*;

use crate::config;
use crate::editor;
use crate::editor::blocks;
use crate::editor::buffer;
use crate::editor::hit_test;
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
    let mut line_font_size = config::scaled_size(config::EDITOR_FONT_SIZE);
    if !editing_line {
        if let Some((level, _)) = markdown::heading_info(line_text) {
            line_font_size = config::scaled_size(config::EDITOR_HEADING_SIZE[(level - 1) as usize]);
        }
    }

    let mut advance = line_font_size;
    let mut image = None;

    // A frontmatter block the cursor is NOT editing collapses to one bar for
    // its whole extent: the opening delimiter line carries the bar height,
    // every remaining line contributes nothing.
    if kind == blocks::LineKind::Frontmatter && !editing_line {
        if line.line == 0 && line.start == 0 {
            advance = config::scaled_size(config::EDITOR_FRONTMATTER_HEIGHT);
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
    // While a mouse selection is in progress the layout freezes in view mode:
    // the caret line must not shrink when it is a heading, or the row slides
    // out from under the pointer and the hit-test snaps the caret back ("in
    // a loop") on every frame. The frontmatter block stays raw only if the
    // caret is already editing inside it.
    if let Some((start, end)) = fm_range {
        let cy = cursor_y as usize;
        if cy >= start && cy <= end && line.line >= start && line.line <= end {
            return true;
        }
    }
    if hit_test::MOUSE_DRAGGING.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    cursor_y as usize == line.line
}

pub fn draw(d: &mut RaylibDrawHandle, editor_open: bool, editor_dimentions: Vector2) {
    let buf = buffer::BUFFER.read().unwrap();
    let cursor_x = buffer::CURSOR_X.read().unwrap();
    let cursor_y = buffer::CURSOR_Y.read().unwrap();
    let anchor_x = buffer::ANCHOR_X.read().unwrap();
    let anchor_y = buffer::ANCHOR_Y.read().unwrap();
    let font_color = config::EDITOR_FONT_COLOR;
    let padding = config::scaled_size(config::EDITOR_PADDING);

    let editor_height = (editor_dimentions.y * config::height() as f32) as i32;
    let editor_width = (editor_dimentions.x * config::width() as f32) as i32;

    if editor_open {
        let fullscreen = editor::FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed);

        // The background: a rounded translucent panel normally, fully opaque
        // when the editor is fullscreen so nothing bleeds through behind it.
        if fullscreen {
            d.draw_rectangle(0, 0, config::width(), config::height(), Color::new(0, 0, 0, 255));
        } else {
            d.draw_rectangle_rounded(
                Rectangle::new(
                    config::width() as f32 * (1. - editor_dimentions.x) * 0.5,
                    config::height() as f32 * (1. - editor_dimentions.y) * 0.5,
                    config::width() as f32 * editor_dimentions.x,
                    config::height() as f32 * editor_dimentions.y,
                ),
                0.05,
                0,
                Color::BLACK.alpha(0.5),
            );
        }

        let editor_x = ((1.0 - editor_dimentions.x) * 0.5 * config::width() as f32) as i32;
        let editor_y = ((1.0 - editor_dimentions.y) * 0.5 * config::height() as f32) as i32;

        // In fullscreen the text body keeps a horizontal margin from the screen
        // edges while the heading bar above still spans the full window. All
        // body geometry (origin, wrap width, scrollbar, scissor) uses these.
        let (content_x0, content_w) = {
            let (x, _, w, _) = crate::editor::content_bounds();
            (x, w)
        };

        let max_width = content_w - 2 * padding;
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
        let note_name = crate::editor::tabs::active_name().unwrap_or_default();
        let name_y = editor_y + (header_h - config::EDITOR_FONT_SIZE) / 2;
        text::draw(d, 
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

        // Fullscreen toggle just left of the X: an outlined box, with a nested
        // box while active. Drawing it means it does not depend on the font
        // atlas having a glyph.
        let fs_side = config::EDITOR_FONT_SIZE + 8;
        let fs_x = x_x - fs_side - 8;
        let fs_y = editor_y + (header_h - fs_side) / 2;
        let inset = fs_side / 4;
        let over_fs = {
            let m = d.get_mouse_position();
            m.x as i32 >= fs_x
                && m.x as i32 <= fs_x + fs_side
                && m.y as i32 >= fs_y
                && m.y as i32 <= fs_y + fs_side
        };
        let fs_icon_color = if over_fs {
            Color::WHITE
        } else {
            Color::new(180, 180, 180, 255)
        };
        let fs_bw = fs_side - 2 * inset;
        let fs_bx = fs_x + inset;
        let fs_by = fs_y + inset;
        if fullscreen {
            // Restore glyph: two overlapping boxes offset diagonally.
            d.draw_rectangle_lines(fs_bx + 4, fs_by, fs_bw - 4, fs_bw - 4, fs_icon_color);
            d.draw_rectangle_lines(fs_bx, fs_by + 4, fs_bw - 4, fs_bw - 4, fs_icon_color);
        } else {
            // Expand glyph: a single outline box.
            d.draw_rectangle_lines(fs_bx, fs_by, fs_bw, fs_bw, fs_icon_color);
        }

        // "Open sub-graph" button (leftmost), only when the active tab's note has a
        // companion sub-graph folder. Checked via the filesystem (the node
        // index may be stale after a graph regen; the path never lies).
        let has_subgraph = {
            let dir = crate::graph::processing::DIR_PATH.read().unwrap();
            crate::editor::tabs::active_name()
                .map(|n| crate::filesystem::is_dir(&dir.join(&n)))
                .unwrap_or(false)
        };
        if has_subgraph {
            let sg_label = ">>";
            let sg_w = text::measure(&d, sg_label, config::EDITOR_FONT_SIZE);
            let sg_x = fs_x - sg_w - 10;
            let sg_y = name_y;
            let over_sg = {
                let m = d.get_mouse_position();
                m.x as i32 >= sg_x
                    && m.x as i32 <= sg_x + sg_w
                    && m.y as i32 >= sg_y
                    && m.y as i32 <= sg_y + config::EDITOR_FONT_SIZE
            };
            let sg_color = if over_sg {
                Color::WHITE
            } else {
                Color::new(180, 180, 180, 255)
            };
            d.draw_rectangle(sg_x - 4, sg_y, sg_w + 8, config::EDITOR_FONT_SIZE, Color::new(40, 40, 46, 200));
            text::draw(d, sg_label, sg_x, sg_y, config::EDITOR_FONT_SIZE, sg_color);
            if over_sg && d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                editor::OPEN_SUBGRAPH_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }

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
        text::draw(d, x_label, x_x, x_y, config::EDITOR_FONT_SIZE, x_color);

        if over_x && d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            editor::CLOSE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if over_fs && d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            editor::FULLSCREEN.store(
                !fullscreen,
                std::sync::atomic::Ordering::Relaxed,
            );
        }

        // Separator line below the header
        d.draw_line(
            editor_x,
            editor_y + header_h,
            editor_x + editor_width,
            editor_y + header_h,
            Color::new(255, 255, 255, 40),
        );

        // --- Tab bar: horizontally scrollable strip of open documents, drawn
        // under the header on top of the body background.
        crate::editor::tabs::draw_tab_bar(d, editor_x, editor_y + header_h, editor_width);

        // --- No node open: placeholder instead of a buffer body. The editor
        // is open but has nothing to save, so show guidance and a way to
        // create a note instead of an editable document.
        let has_node = crate::editor::tabs::has_any();
        if !has_node {
            let (cx0, cy0, cw, ch) = crate::editor::content_bounds();
            let body_top = cy0 + header_h;
            let body_h = (ch - header_h).max(1);
            let cx = cx0 + cw / 2;
            let cy = body_top + body_h / 2;

            if crate::editor::CREATING_NODE.load(std::sync::atomic::Ordering::Relaxed) {
                // Filename prompt (black/white, matches the placeholder).
                let prompt = "Filename:";
                let name = crate::editor::NEW_NODE_NAME.read().unwrap().clone();
                let full = format!("{} {}", prompt, name);
                let bw = text::measure(d, &full, 20) + 28;
                let bh = 34;
                let bx = cx - bw / 2;
                let by = cy - bh / 2;
                d.draw_rectangle(bx, by, bw, bh, Color::new(18, 18, 18, 235));
                d.draw_rectangle_lines_ex(
                    Rectangle::new(bx as f32, by as f32, bw as f32, bh as f32),
                    1.0,
                    Color::new(255, 255, 255, 90),
                );
                let pw = text::measure(d, &format!("{} ", prompt), 20);
                text::draw(d, &full, bx + 14, by + (bh - 20) / 2, 20, Color::WHITE);
                let caret_x = bx + 14 + pw + text::measure(d, &name, 20);
                d.draw_rectangle(caret_x, by + (bh - 20) / 2, 2, 20, Color::WHITE);
            } else {
                *crate::editor::NEW_NODE_BUTTON.write().unwrap() = None;

                let msg = "No node open";
                let msg_w = text::measure(d, msg, 24);
                text::draw(d, msg, cx - msg_w / 2, cy - 64, 24, Color::new(255, 255, 255, 190));

                let hint = "Select a node on the graph, or create a new one";
                let hint_w = text::measure(d, hint, 16);
                text::draw(d, hint, cx - hint_w / 2, cy - 30, 16, Color::new(255, 255, 255, 110));

                let blabel = "Create New Node";
                let bfont = 18;
                let bw = text::measure(d, blabel, bfont) + 32;
                let bh = 36;
                let bx = cx - bw / 2;
                let by = cy + 6;
                let m = d.get_mouse_position();
                let over = m.x as i32 >= bx
                    && m.x as i32 <= bx + bw
                    && m.y as i32 >= by
                    && m.y as i32 <= by + bh;
                d.draw_rectangle_lines_ex(
                    Rectangle::new(bx as f32, by as f32, bw as f32, bh as f32),
                    1.0,
                    if over {
                        Color::WHITE
                    } else {
                        Color::new(255, 255, 255, 130)
                    },
                );
                if over {
                    d.draw_rectangle(bx + 1, by + 1, bw - 2, bh - 2, Color::new(255, 255, 255, 14));
                }
                let lw = text::measure(d, blabel, bfont);
                let lc = if over {
                    Color::WHITE
                } else {
                    Color::new(255, 255, 255, 170)
                };
                text::draw(d, blabel, bx + (bw - lw) / 2, by + (bh - bfont) / 2, bfont, lc);
                *crate::editor::NEW_NODE_BUTTON.write().unwrap() = Some((bx, by, bw, bh));
            }
            return;
        }

        // --- Content area (shifted down by header + tab bar) ---
        let content_y = editor_y + header_h + crate::editor::tabs::TABS_H;

        let kinds = blocks::classify(&buf);
        let fm_range = frontmatter::line_range(&buf);

        crate::editor::buffer::generate_visual_lines(max_width, d);

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
                line_layout(vl, &buf[vl.line], kinds.get(vl.line).copied().unwrap_or(blocks::LineKind::Paragraph), editing, content_x0, padding, max_width);
            total_h += adv + config::EDITOR_LINE_SPACING;
            layouts.push((fsz, adv, img));
        }

        // Hit-test table for the input handler (consumed on the next frame,
        // so mouse clicks land on exactly what this frame drew). `top` is in
        // content-relative pixels, matching the draw loop's `line_y`.
        {
            let mut hits = Vec::with_capacity(vlines.len());
            let mut line_y: i32 = 0;
            for (vi, vl) in vlines.iter().enumerate() {
                let (fsz, adv, ref img) = layouts[vi];
                if adv > 0 {
                    let editing = line_editing(vl, fm_range, *cursor_y);
                    let kind = kinds
                        .get(vl.line)
                        .copied()
                        .unwrap_or(blocks::LineKind::Paragraph);
                    let is_fence = matches!(
                        kind,
                        blocks::LineKind::FencedCode | blocks::LineKind::FenceDelimiter
                    );
                    let view_mode = !editing && !is_fence;
                    let quote_skip = if view_mode
                        && matches!(kind, blocks::LineKind::Blockquote)
                        && vl.start == 0
                    {
                        markdown::blockquote_marker_len(&buf[vl.line]).unwrap_or(0)
                    } else {
                        0
                    };
                    hits.push(hit_test::HitRow {
                        line: vl.line,
                        start: vl.start,
                        end: vl.end,
                        indent_px: vl.indent,
                        font_size: fsz,
                        top: line_y,
                        advance: adv,
                        view_mode,
                        quote_skip,
                        image: img.is_some(),
                        fm_bar: kind == blocks::LineKind::Frontmatter && !editing,
                    });
                }
                line_y += adv + config::EDITOR_LINE_SPACING;
            }
            *hit_test::VISUAL_HIT.lock().unwrap() = hits;
        }

        // Only the current frame's popup is clickable; cleared before the
        // draw loop so a stale rect from a scrolled-away popup never lingers.
        *hit_test::AUTOCOMPLETE_RECT.lock().unwrap() = None;
        *hit_test::COMMAND_RECT.lock().unwrap() = None;

        // Clamp the scroll offset to the real content height.
        let viewport_h = (editor_height - header_h - crate::editor::tabs::TABS_H - padding).max(1);
        let max_scroll = (total_h - viewport_h).max(0);
        let mut scroll = (*buffer::SCROLL_Y.read().unwrap()).clamp(0, max_scroll);

        // Record which source lines fall inside the scrolled viewport so the
        // main loop's image preload only decodes what the user can see. The
        // same layout data as the draw loop: a visual line is visible when its
        // [line_y, line_y + advance) band overlaps [scroll, scroll + viewport_h).
        {
            let mut min_line = usize::MAX;
            let mut max_line = 0usize;
            let mut line_y: i32 = 0;
            for (vi, vl) in vlines.iter().enumerate() {
                let advance = layouts[vi].1;
                if line_y + advance >= scroll && line_y <= scroll + viewport_h {
                    min_line = min_line.min(vl.line);
                    max_line = max_line.max(vl.line);
                }
                line_y += advance + config::EDITOR_LINE_SPACING;
            }
            *buffer::DRAW_LINE_RANGE.write().unwrap() =
                if min_line == usize::MAX { (0, 0) } else { (min_line, max_line) };
        }

        // --- Scrollbar (right edge, draggable) ---
        let bar_w = 6;
        let bar_x = content_x0 + content_w - bar_w - padding;
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
        d.draw_scissor_mode(content_x0, content_y, content_w, editor_height - header_h - crate::editor::tabs::TABS_H, |mut s| {
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
                        let fm_h = config::scaled_size(config::EDITOR_FRONTMATTER_HEIGHT);
                        s.draw_rectangle(
                            content_x0 + padding,
                            draw_top,
                            max_width,
                            fm_h,
                            config::EDITOR_FRONTMATTER_BG,
                        );
                        text::draw(
                            &mut s,
                            "--- frontmatter ---",
                            content_x0 + padding + 6,
                            draw_top + (fm_h - config::scaled_size(config::EDITOR_FONT_SIZE - 2)) / 2,
                            config::scaled_size(config::EDITOR_FONT_SIZE - 2),
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

                let content_x = content_x0 + padding + line.indent;
                let draw_top = content_y + line_y - scroll;

                // Horizontal rule: draw a line instead of text.
                if !editing_line && matches!(kind, blocks::LineKind::HorizontalRule) {
                    let y_mid = draw_top + line_font_size / 2;
                    s.draw_line(
                        content_x0 + padding,
                        y_mid,
                        content_x0 + padding + max_width,
                        y_mid,
                        config::EDITOR_HR_COLOR,
                    );
                    line_y += advance + config::EDITOR_LINE_SPACING;
                    continue;
                }

                // Fenced code block: background for the full container width.
                if is_fence && !editing_line {
                    s.draw_rectangle(
                        content_x0 + padding,
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
                        let item_h = config::scaled_size(config::AUTOCOMPLETE_ITEM_HEIGHT);
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

                        let mut pop_x = cursor_x_abs.min(content_x0 + content_w - box_w - padding);
                        let mut pop_y = cursor_y_abs + line_font_size + config::EDITOR_PADDING;
                        if pop_y + box_h > editor_bottom - padding {
                            pop_y = pop_y - box_h - line_font_size - config::EDITOR_PADDING;
                        }
                        pop_x = pop_x.max(content_x0 + padding);
                        // Keep the popup inside the clipped content area.
                        pop_y = pop_y.max(content_y);
                        if pop_y + box_h > editor_bottom {
                            pop_y = (editor_bottom - box_h).max(content_y);
                        }

                        *hit_test::AUTOCOMPLETE_RECT.lock().unwrap() = Some((pop_x, pop_y, box_w, box_h));

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

                    // Slash-command palette popup, styled like the autocomplete
                    // popup since it solves the same "pick one of several"
                    // problem, just for commands instead of note names.
                    let palette = crate::editor::command::COMMAND_PALETTE.read().unwrap();
                    if palette.active && !palette.matches.is_empty() {
                        let item_h = config::scaled_size(config::AUTOCOMPLETE_ITEM_HEIGHT);
                        let mut box_w = 140;
                        for (name, _) in palette
                            .matches
                            .iter()
                            .take(config::AUTOCOMPLETE_MAX_VISIBLE)
                        {
                            let w = text::measure(&*s, name, line_font_size) + 24;
                            if w > box_w {
                                box_w = w;
                            }
                        }
                        let count = palette.matches.len().min(config::AUTOCOMPLETE_MAX_VISIBLE);
                        let box_h = (count as i32) * item_h;

                        let mut pop_x = cursor_x_abs.min(content_x0 + content_w - box_w - padding);
                        let mut pop_y = cursor_y_abs + line_font_size + config::EDITOR_PADDING;
                        if pop_y + box_h > editor_bottom - padding {
                            pop_y = pop_y - box_h - line_font_size - config::EDITOR_PADDING;
                        }
                        pop_x = pop_x.max(content_x0 + padding);
                        pop_y = pop_y.max(content_y);
                        if pop_y + box_h > editor_bottom {
                            pop_y = (editor_bottom - box_h).max(content_y);
                        }

                        *hit_test::COMMAND_RECT.lock().unwrap() = Some((pop_x, pop_y, box_w, box_h));

                        s.draw_rectangle(pop_x, pop_y, box_w, box_h, config::AUTOCOMPLETE_BG);

                        for (i, (name, _)) in palette
                            .matches
                            .iter()
                            .enumerate()
                            .take(config::AUTOCOMPLETE_MAX_VISIBLE)
                        {
                            if i == palette.selected {
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
                    drop(palette);
                }

                let is_quote = matches!(kind, blocks::LineKind::Blockquote);

                // Accumulate the quote rail across contiguous quote visual lines
                // so a block reads as one continuous vertical bar. The rail is
                // hidden where the cursor is editing (raw markers are shown).
                if is_quote && !editing_line {
                    let (_, _, h) =
                        quote_rail.get_or_insert((content_x0 + padding, draw_top, 0));
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
                        (content_x0 + padding + max_width - img_x).max(1),
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