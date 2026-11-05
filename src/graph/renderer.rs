use std::sync::RwLock;

use crate::config;
use crate::editor::text;
use crate::graph::hover_popup::MenuItem;
use crate::graph::processing::*;
use crate::graph::settings;
use crate::ui;
use raylib::prelude::*;

pub static CAMERA: RwLock<Camera2D> = RwLock::new(Camera2D {
    target: Vector2 {
        x: config::DEFAULT_W as f32 / 2.0,
        y: config::DEFAULT_H as f32 / 2.0,
    },
    offset: Vector2 {
        x: config::DEFAULT_W as f32 / 2.0,
        y: config::DEFAULT_H as f32 / 2.0,
    },
    rotation: 0.0,
    zoom: 1.0,
});

// ---------------------------------------------------------------------------
// Vertical zoom scale (right edge, auto-hiding). The stylus-friendly zoom: it
// fades in while the pointer hovers its band and slides to rescale the camera,
// mapping the same 0.1..10 range as the mouse wheel on a log scale so the
// whole band stays useful. The renderer fades and draws it; the input handler
// reports hover and drives the drag. Both share this one band geometry so the
// hit-test and the drawn track can never disagree.
// ---------------------------------------------------------------------------

pub const ZOOM_SCALE_MIN: f32 = 0.1;
pub const ZOOM_SCALE_MAX: f32 = 10.0;

pub static ZOOM_SCALE_HOVER: RwLock<bool> = RwLock::new(false);
pub static ZOOM_SCALE_DRAGGING: RwLock<bool> = RwLock::new(false);
static ZOOM_SCALE_ALPHA: RwLock<f32> = RwLock::new(0.0);

// Breathing room between the zoom scale and the window's right edge, and the
// height of its strip (a short band rather than a full-height rail).
const ZOOM_SCALE_RIGHT_MARGIN: i32 = 16;
const ZOOM_SCALE_HEIGHT: i32 = 320;

// The pointer band (screen space): a narrow strip near the right edge, held
// clear of the edge and bounded in height. Everything derives from this rect.
pub fn zoom_scale_band() -> (i32, i32, i32, i32) {
    let w = config::scaled_size(26);
    let right = config::scaled_size(ZOOM_SCALE_RIGHT_MARGIN);
    let top = config::scaled_size(72);
    let bottom = (top + config::scaled_size(ZOOM_SCALE_HEIGHT))
        .min(config::height() - config::scaled_size(28))
        .max(top + 1);
    (config::width() - w - right, top, w, bottom - top)
}

// Camera zoom <-> band position (log-spaced both ways, so the thumb round-trips
// exactly: t_to_zoom(zoom_to_t(z)) == z up to float rounding).
pub fn zoom_to_t(z: f32) -> f32 {
    (z.ln() - ZOOM_SCALE_MIN.ln()) / (ZOOM_SCALE_MAX.ln() - ZOOM_SCALE_MIN.ln())
}
pub fn t_to_zoom(t: f32) -> f32 {
    ZOOM_SCALE_MIN * (ZOOM_SCALE_MAX / ZOOM_SCALE_MIN).powf(t.clamp(0.0, 1.0))
}

// Base on-screen size of a node label in pixels (before the global Ctrl +/- text
// zoom). Graph labels keep this constant screen size at ANY graph zoom: the
// world-unit font is label_px / camera.zoom, so the words read the same at the
// default overview (zoom 0.3, discs 3.8 units) and zoomed in.
const GRAPH_LABEL_SIZE: i32 = 10;

// How many zoom steps before the "Label Fade" slider point labels start to
// appear. The slider sets the zoom at which labels reach full opacity (default
// 0.5); the fade band is [limit - WINDOW, limit]. The wheel steps 0.1/notch,
// so at the default overview (2.2/sqrt(n), floor 0.3) a couple of notches
// cases bring labels to full opacity.
const LABEL_FADE_WINDOW: f32 = 0.25;

// How far (screen pixels) beyond each viewport edge content is preloaded.
// Large enough that panning/zooming smoothly reveal already-decoded textures,
// small enough that offscreen notes two screens away stay unloaded.
const PRELOAD_MARGIN_PX: f32 = 150.0;

// The world-space rectangle currently framed by the camera, expanded by
// PRELOAD_MARGIN_PX (in world units) so items entering the frame are loaded a
// frame before they become visible. world = (screen - offset) / zoom + target.
pub fn world_view_bounds() -> Rectangle {
    let cam = CAMERA.read().unwrap();
    let m = PRELOAD_MARGIN_PX / cam.zoom;
    let left = (-cam.offset.x) / cam.zoom + cam.target.x - m;
    let top = (-cam.offset.y) / cam.zoom + cam.target.y - m;
    let right = (config::width() as f32 - cam.offset.x) / cam.zoom + cam.target.x + m;
    let bottom = (config::height() as f32 - cam.offset.y) / cam.zoom + cam.target.y + m;
    Rectangle::new(left, top, right - left, bottom - top)
}

// Does a disc (node) overlap the rectangle? The cheap clamp-to-rect distance
// test covers both "center inside the rect" and "circle overlapping an edge".
pub fn circle_intersects_rect(center: Vector2, radius: f32, rect: Rectangle) -> bool {
    let cx = center.x.clamp(rect.x, rect.x + rect.width);
    let cy = center.y.clamp(rect.y, rect.y + rect.height);
    let dx = center.x - cx;
    let dy = center.y - cy;
    dx * dx + dy * dy <= radius * radius
}

pub fn draw(d: &mut RaylibDrawHandle, editor_open: bool) {
    let hover_node = HOVER_NODE.read().unwrap();
    let selected_node = SELECTED_NODE.read().unwrap();
    let delete_pending = DELETE_PENDING.read().unwrap();
    let nodes = NODES.read().unwrap();
    let edges = EDGES.read().unwrap();
    // Snapshot the visible world rect BEFORE taking the camera lock
    // (world_view_bounds re-locks CAMERA; RwLock is not reentrant). Everything
    // outside it is skipped: large graphs cull to what the camera actually
    // shows instead of drawing thousands of off-screen discs and labels.
    let bounds = world_view_bounds();
    let camera = CAMERA.read().unwrap();

    let mut mode = d.begin_mode2D(*camera);

    mode.clear_background(Color::BLACK);

    // Margin covers the endpoint dot plus the label reach below the disc, so
    // an edge whose endpoint sits just outside the viewport still draws. An
    // edge is only skipped when BOTH endpoints are fully outside the cull rect.
    // The label reach is the constant on-screen label size in world units at the
    // most zoomed-out camera, so the margin is generous to keep labels from
    // popping at the viewport edges.
    const NODE_CULL_MARGIN: f32 = NODE_MAX_RADIUS + 140.0;
    for edge in edges.iter() {
        let n1 = &nodes[edge.n1];
        let n2 = &nodes[edge.n2];
        if !circle_intersects_rect(n1.position, NODE_CULL_MARGIN, bounds)
            && !circle_intersects_rect(n2.position, NODE_CULL_MARGIN, bounds)
        {
            continue;
        }
        let edge_color = Color::new(110, 110, 110, 255);

        let from = n1.position;
        let to = n2.position;

        mode.draw_line_ex(from, to, 2., edge_color);

        // A small filled dot at the child end: a plain circle reads as the
        // edge's destination without the visual weight of an arrowhead. It
        // sits just outside the state discs so halos never clip it.
        let axis = to - from;
        let len = axis.length();
        if len > 0.0 {
            let dir = axis * (1.0 / len);
            let dot_center = to - dir * (n2.radius + 4.0);
            mode.draw_circle_v(dot_center, 3.0, edge_color);
        }
    }

    // Label fade comes from the panel's "Label Fade" slider (idx 9): the zoom
    // at which labels reach full opacity, fading in over LABEL_FADE_WINDOW
    // below it. Constant per frame, so hoisted out of the node loop.
    let label_fade_limit = read_slider_value(9);
    let label_alpha =
        ((camera.zoom - (label_fade_limit - LABEL_FADE_WINDOW)) / LABEL_FADE_WINDOW)
            .clamp(0.0, 1.0);
    // World font that keeps labels a constant on-screen size at any zoom (see
    // the comment inside the loop).
    let label_px = config::scaled_size(GRAPH_LABEL_SIZE);
    let label_font = 5_i32.max((label_px as f32 / camera.zoom) as i32);

    for (i, node) in nodes.iter().enumerate() {
        // Cull nodes whose disc and label are fully outside the view.
        if !circle_intersects_rect(node.position, NODE_CULL_MARGIN, bounds) {
            continue;
        }

        let is_hover = *hover_node == Some(i);
        let is_selected = *selected_node == Some(i);

        // Node size comes from its child count (set in rebuild_edges).
        let draw_radius = node.radius;

        // State discs are filled and drawn BEHIND the node body: the node
        // covers their centre, so only the margin reads as a ring around it.
        // Drawn largest-first so a smaller disc never overlaps a bigger one.

        // Sub-graph: dim gray disc, subtler than selection.
        if node.has_subgraph {
            mode.draw_circle_v(node.position, draw_radius + 2.5, Color::new(160, 160, 160, 200));
        }
        // Hover: faint near-invisible halo so it reads as "active" only.
        if is_hover {
            mode.draw_circle_v(node.position, draw_radius + 1.5, Color::new(255, 255, 255, 90));
        }

        // Selection is signaled by the node body flipping to light pink.
        let node_color = if is_selected {
            Color::LIGHTPINK
        } else {
            node.color
        };
        mode.draw_circle_v(node.position, draw_radius, node_color);

        // A header image (frontmatter `header:` pointing into assets/) is
        // drawn centred inside the node circle as a disc that nearly matches
        // the node's own radius, so it reads as the node's face rather than a
        // small icon floating in it.
        if let Some(header) = &node.header {
            if crate::editor::images::is_image_target(header) {
                if let Some(path) = resolve_header_path(&node.path, header) {
                    if crate::editor::images::has_thumb(&path) {
                        let disc = draw_radius * 1.9;
                        crate::editor::images::draw_thumb(
                            &mut mode,
                            &path,
                            node.position.x,
                            node.position.y,
                            disc,
                        );
                    }
                }
            }
        }

        // Node labels are world-anchored but never shrink below a readable
        // on-screen size: the world font is label_px / camera.zoom, so the
        // words stay label_px screen pixels at every zoom (also scaled by the
        // global Ctrl +/- text zoom). Only once the camera is close enough
        // that the base 5-unit world font already measures that large do
        // labels grow with the graph again. Opacity is the per-frame
        // label_alpha above, driven by the "Label Fade" slider.
        // A dated journal note's label reads as "19th Aug 2025"; the node's real
        // name stays the file stem so rename and save never see the pretty form.
        let label = crate::journal::pretty_name(&node.name);
        let text_w = text::measure_f(&mode, &label, label_font);

        text::draw_f(
            &mut mode,
            &label,
            node.position.x - text_w / 2.0,
            node.position.y + node.radius + 6.0,
            label_font,
            Color::WHITE.alpha(label_alpha),
        );
    }

    // Keyboard-navigation reticle: a small crosshair pinned to the centre of
    // the view, drawn in world space at a constant on-screen size so it tracks
    // the graph as W/A/S/D pans the camera underneath it.
    if *crate::graph::kb_nav::RETICLE_ACTIVE.read().unwrap() {
        let c = crate::graph::kb_nav::reticle_world(&camera);
        let col = Color::new(255, 255, 255, 220);
        let t = 1.2 / camera.zoom;
        let gap = 3.0 / camera.zoom;
        let len = 11.0 / camera.zoom;
        mode.draw_line_ex(
            Vector2::new(c.x - len, c.y),
            Vector2::new(c.x - gap, c.y),
            t,
            col,
        );
        mode.draw_line_ex(
            Vector2::new(c.x + gap, c.y),
            Vector2::new(c.x + len, c.y),
            t,
            col,
        );
        mode.draw_line_ex(
            Vector2::new(c.x, c.y - len),
            Vector2::new(c.x, c.y - gap),
            t,
            col,
        );
        mode.draw_line_ex(
            Vector2::new(c.x, c.y + gap),
            Vector2::new(c.x, c.y + len),
            t,
            col,
        );
        mode.draw_circle_v(c, 1.6 / camera.zoom, col);
    }

    drop(mode);
    drop(camera);

    // Hover action popup (screen space). It follows the hovered node and lists
    // that node's shortcuts as clickable rows. Hidden whenever another modal
    // owns the screen or the graph is covered by the editor/palette.
    let popup_modal = *crate::graph::processing::RENAMING.read().unwrap()
        || *crate::graph::processing::ADDING_NOTE.read().unwrap()
        || *delete_pending
        || *crate::graph::processing::SHOW_FORCE_PANEL.read().unwrap()
        || crate::graph::processing::CONTEXT_NODE
            .read()
            .unwrap()
            .is_some()
        || *crate::graph::processing::CONTEXT_EMPTY.read().unwrap()
        || *crate::graph::settings::SETTINGS_OPEN.read().unwrap();
    if !editor_open && !crate::global_palette::is_open() && !popup_modal {
        crate::graph::hover_popup::draw(d, &nodes);
    }

    // Draw delete confirmation prompt (outside camera mode, in screen space)
    if *delete_pending {
        if let Some(idx) = *selected_node {
            let name = &nodes[idx].name;
            let prompt = format!("Delete '{}'? (Y/N)", name);
            let fs = config::scaled_size(18);
            let box_h = config::scaled_size(38);
            let pad = config::scaled_size(16);
            let text_width = text::measure(d, &prompt, fs);
            let box_w = text_width + 2 * pad;
            let x = (config::width() - box_w) / 2;
            let y = config::scaled_size(12);
            ui::panel(
                d,
                Rectangle::new(x as f32, y as f32, box_w as f32, box_h as f32),
                0.32,
            );
            d.draw_rectangle_rounded(
                Rectangle::new((x + 3) as f32, (y + 9) as f32, 3.0, (box_h - 18) as f32),
                0.5,
                4,
                ui::DANGER,
            );
            text::draw(
                d,
                &prompt,
                x + pad,
                y + (box_h - fs) / 2,
                fs,
                ui::TEXT,
            );
        }
    }

    // Draw add-note name prompt (outside camera mode, in screen space)
    let adding_note = *crate::graph::processing::ADDING_NOTE.read().unwrap();
    if adding_note {
        let adding_name = crate::graph::processing::ADDING_NAME.read().unwrap();
        let prompt_base = "Filename: ";
        let full = format!("{}{}", prompt_base, adding_name);
        let fs = config::scaled_size(18);
        let box_h = config::scaled_size(38);
        let pad = config::scaled_size(16);
        let label_width = text::measure(d, prompt_base, fs);
        let text_width = text::measure(d, &full, fs);
        let box_w = text_width + 2 * pad;
        let x = (config::width() - box_w) / 2;
        let y = config::scaled_size(12);
        ui::panel(
            d,
            Rectangle::new(x as f32, y as f32, box_w as f32, box_h as f32),
            0.32,
        );
        d.draw_rectangle_rounded(
            Rectangle::new((x + 3) as f32, (y + 9) as f32, 3.0, (box_h - 18) as f32),
            0.5,
            4,
            ui::ACCENT,
        );
        let ty = y + (box_h - fs) / 2;
        text::draw(d, prompt_base, x + pad, ty, fs, ui::TEXT_SECONDARY);

        // Draw the input content in the primary ink plus an accent cursor.
        text::draw(d, &adding_name, x + pad + label_width, ty, fs, ui::TEXT);
        let name_width = text::measure(d, &adding_name, fs);
        let cursor_x = x + pad + label_width + name_width;
        d.draw_rectangle(cursor_x, ty, 2, fs, ui::ACCENT);
    }

    // Right-click context menu (screen space)
    let context_node = *crate::graph::processing::CONTEXT_NODE.read().unwrap();
    let context_empty = *crate::graph::processing::CONTEXT_EMPTY.read().unwrap();
    let renaming = *crate::graph::processing::RENAMING.read().unwrap();
    if (context_node.is_some() || context_empty) && !renaming {
        let (mx, my) = *crate::graph::processing::CONTEXT_POS.read().unwrap();
        let mw = config::scaled_size(config::CONTEXT_MENU_W);
        let mh = config::scaled_size(config::CONTEXT_MENU_ITEM_H);
        let pad = crate::graph::hover_popup::menu_pad();

        // The panel extends `pad` beyond the row area, so the rows inside it
        // line up exactly with the hover popup's rows while the existing
        // CONTEXT_POS hit-testing keeps working unchanged.
        let (items, menu_h) = if context_empty {
            // Empty-space menu: "Add Node" always, plus "Paste" when the
            // clipboard holds a note (it drops into the folder this graph
            // level is showing — e.g. the sub-graph you navigated into).
            let has_clip = crate::graph::processing::CLIPBOARD.read().unwrap().is_some();
            let rows = if has_clip { 2usize } else { 1usize };
            let items = (0..rows)
                .map(|i| {
                    let (label, key) = if i == 0 {
                        ("Add Node", "N")
                    } else {
                        ("Paste", "Ctrl+V")
                    };
                    MenuItem {
                        label,
                        key: Some(key),
                        hover_color: Color::new(122, 205, 158, 140),
                        separator_before: false,
                        rect: Rectangle::new(
                            mx as f32,
                            (my + i as i32 * mh) as f32,
                            mw as f32,
                            mh as f32,
                        ),
                    }
                })
                .collect::<Vec<_>>();
            (items, rows as i32 * mh)
        } else {
            // Node menu: rows come from context_menu_rows (shared with the
            // input handler) so drawing and hit-testing can't drift apart.
            let rows: Vec<ContextRow> = context_menu_rows(context_node, &nodes);
            let menu_h = rows.len() as i32 * mh;
            let items = rows
                .iter()
                .copied()
                .enumerate()
                .map(|(i, row)| MenuItem {
                    label: row.label(),
                    key: row.key(),
                    hover_color: row.hover_color(),
                    separator_before: i > 0 && row.group() != rows[i - 1].group(),
                    rect: Rectangle::new(
                        mx as f32,
                        (my + i as i32 * mh) as f32,
                        mw as f32,
                        mh as f32,
                    ),
                })
                .collect::<Vec<_>>();
            (items, menu_h)
        };

        let panel = Rectangle::new(
            (mx - pad) as f32,
            (my - pad) as f32,
            (mw + 2 * pad) as f32,
            (menu_h + 2 * pad) as f32,
        );
        let focused = *crate::graph::processing::CONTEXT_FOCUS.read().unwrap();
        crate::graph::hover_popup::draw_menu(d, panel, &items, focused);
    }

    // Rename-note name prompt (screen space)
    if renaming {
        let rename_name = crate::graph::processing::RENAME_NAME.read().unwrap();
        let prompt_base = "Rename to: ";
        let full = format!("{}{}", prompt_base, rename_name);
        let fs = config::scaled_size(18);
        let box_h = config::scaled_size(38);
        let pad = config::scaled_size(16);
        let label_width = text::measure(d, prompt_base, fs);
        let text_width = text::measure(d, &full, fs);
        let box_w = text_width + 2 * pad;
        let x = (config::width() - box_w) / 2;
        let y = config::scaled_size(12);
        ui::panel(
            d,
            Rectangle::new(x as f32, y as f32, box_w as f32, box_h as f32),
            0.32,
        );
        d.draw_rectangle_rounded(
            Rectangle::new((x + 3) as f32, (y + 9) as f32, 3.0, (box_h - 18) as f32),
            0.5,
            4,
            ui::ACCENT,
        );
        let ty = y + (box_h - fs) / 2;
        text::draw(d, prompt_base, x + pad, ty, fs, ui::TEXT_SECONDARY);
        text::draw(d, &rename_name, x + pad + label_width, ty, fs, ui::TEXT);
        let name_width = text::measure(d, &rename_name, fs);
        let cursor_x = x + pad + label_width + name_width;
        d.draw_rectangle(cursor_x, ty, 2, fs, ui::ACCENT);
    }

    // Breadcrumb trail (only when inside a sub-graph)
    let nav_stack = crate::graph::processing::NAV_STACK.read().unwrap();
    if !nav_stack.is_empty() {
        let dir = crate::graph::processing::DIR_PATH.read().unwrap();
        let screen_mouse = d.get_mouse_position();
        let settings_open = *crate::graph::settings::SETTINGS_OPEN.read().unwrap();

        // Build path components with one entry per level: the root directory
        // (level 0) uses its own folder name rather than a separate "Root"
        // label, so the root doesn't appear twice when it also carries a name.
        let mut components: Vec<String> = Vec::new();
        if let Some(root) = nav_stack.first() {
            let label = root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Root".to_string());
            components.push(label);
        } else {
            components.push("Root".to_string());
        }
        // Intermediate levels: folders the stack pushed between root and now.
        for p in nav_stack.iter().skip(1) {
            if let Some(name) = p.file_name() {
                components.push(name.to_string_lossy().to_string());
            }
        }
        // Current folder.
        if let Some(name) = dir.file_name() {
            components.push(name.to_string_lossy().to_string());
        }
        drop(dir);

        let mut x = config::BREADCRUMB_PAD;
        let text_color = ui::TEXT_MUTED;
        let total = components.len();
        let bf = config::scaled_size(16);
        let band_h = bf + 4;

        for (level, comp) in components.iter().enumerate() {
            let label = if level == 0 { comp.clone() } else { format!("/{}", comp) };
            let w = text::measure(d, &label, bf);
            let is_current = level == total - 1;

            if !is_current {
                let hover = screen_mouse.x as i32 >= x
                    && screen_mouse.x as i32 <= x + w
                    && screen_mouse.y as i32 >= config::BREADCRUMB_Y
                    && screen_mouse.y as i32 <= config::BREADCRUMB_Y + band_h;
                let color = if hover { ui::ACCENT } else { text_color };
                text::draw(d, &label, x, config::BREADCRUMB_Y, bf, color);

                // Detect click and store the level for the input handler.
                // Only when no menu/modal is active, so the value can't be
                // left stale by an input path that returns early.
                if context_node.is_none()
                    && !context_empty
                    && !renaming
                    && !adding_note
                    && !settings_open
                    && hover
                    && d.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
                {
                    *crate::graph::processing::BREADCRUMB_CLICK.write().unwrap() = Some(level);
                }
            } else {
                text::draw(d, &label, x, config::BREADCRUMB_Y, bf, ui::TEXT);
            }

            x += w;
        }
    }

    // Settings button (screen space, top-left)
    let settings_open = *crate::graph::settings::SETTINGS_OPEN.read().unwrap();
    let screen_mouse = d.get_mouse_position();
    let sb_w = config::scaled_size(settings::SETTINGS_BUTTON_W);
    let sb_h = config::scaled_size(settings::SETTINGS_BUTTON_H);
    let sb_f = config::scaled_size(18);
    let over_btn = screen_mouse.x as i32 >= settings::settings_button_x()
        && screen_mouse.x as i32 <= settings::settings_button_x() + sb_w
        && screen_mouse.y as i32 >= settings::SETTINGS_BUTTON_Y
        && screen_mouse.y as i32 <= settings::SETTINGS_BUTTON_Y + sb_h;

    let btn_bg = if settings_open {
        ui::ACCENT_STRONG
    } else if over_btn {
        ui::SURFACE_RAISED
    } else {
        ui::SURFACE
    };
    let btn_rect = Rectangle::new(
        settings::settings_button_x() as f32,
        settings::SETTINGS_BUTTON_Y as f32,
        sb_w as f32,
        sb_h as f32,
    );
    ui::surface(d, btn_rect, 0.32, btn_bg);
    ui::border(
        d,
        btn_rect,
        0.32,
        if settings_open { ui::ACCENT } else { ui::BORDER },
    );

    // Tiny sliders mark: three rails with a knob on each, so the button reads
    // as settings even before the label does.
    let icon_x = settings::settings_button_x() + config::scaled_size(12);
    let icon_cy = settings::SETTINGS_BUTTON_Y + sb_h / 2;
    let icon_w = config::scaled_size(12);
    let icon_step = config::scaled_size(5);
    for (i, offset) in [-icon_step, 0, icon_step].iter().enumerate() {
        let ly = icon_cy + *offset;
        d.draw_line(
            icon_x,
            ly,
            icon_x + icon_w,
            ly,
            if settings_open { ui::TEXT } else { ui::TEXT_SECONDARY },
        );
        let knob_x = icon_x + icon_w * (2 - i as i32) / 3;
        d.draw_circle(knob_x, ly, config::scaled_size(2) as f32, ui::ACCENT);
    }

    let btn_label = "Settings";
    let btn_w = text::measure(d, btn_label, sb_f);
    let text_left = config::scaled_size(26);
    text::draw(
        d,
        btn_label,
        settings::settings_button_x()
            + text_left
            + ((sb_w - text_left - btn_w) / 2).max(config::scaled_size(2)),
        settings::SETTINGS_BUTTON_Y + (sb_h - sb_f) / 2,
        sb_f,
        ui::TEXT,
    );

    // Settings dialog / font picker
    if settings_open {
        d.draw_rectangle(0, 0, config::width(), config::height(), Color::new(3, 4, 7, 180));

        let px = settings::panel_x();
        let py = settings::panel_y();
        let pw = settings::panel_w();
        let ph = settings::panel_h();
        let th = settings::title_h();
        let rh = settings::row_h();
        let tf = config::scaled_size(24);
        let rf = config::scaled_size(16);
        let panel = Rectangle::new(px as f32, py as f32, pw as f32, ph as f32);
        ui::panel(d, panel, 0.035);

        // Accent tick before the title, then the dialog name.
        d.draw_rectangle_rounded(
            Rectangle::new((px + 14) as f32, (py + (th - 18) / 2) as f32, 3.0, 18.0),
            0.5,
            4,
            ui::ACCENT,
        );
        text::draw(d, "Settings", px + 26, py + (th - tf) / 2, tf, ui::TEXT);
        let esc = "Esc to close";
        let esc_font = config::scaled_size(12);
        let esc_w = text::measure(d, esc, esc_font);
        text::draw(
            d,
            esc,
            px + pw - esc_w - config::scaled_size(16),
            py + (th - esc_font) / 2,
            esc_font,
            ui::TEXT_MUTED,
        );
        d.draw_line(
            px,
            py + th,
            px + pw,
            py + th,
            ui::BORDER,
        );

        // --- Editor options section (modifiable layout values) ---
        let section_font = config::scaled_size(14);
        let step_font = config::scaled_size(16);
        text::draw(
            d,
            "OPTIONS",
            px + 18,
            settings::options_top() + (settings::options_label_h() - section_font) / 2,
            section_font,
            ui::ACCENT,
        );
        for row in 0..settings::OPTION_ROWS {
            let row_y = settings::option_row_y(row);
            let (value, min, max) = settings::option_value(row);
            let row_hovered = screen_mouse.x as i32 >= px
                && screen_mouse.x as i32 <= px + pw
                && screen_mouse.y as i32 >= row_y
                && screen_mouse.y as i32 <= row_y + rh;
            if row_hovered {
                ui::row_pill(
                    d,
                    Rectangle::new(
                        (px + 8) as f32,
                        (row_y + 1) as f32,
                        (pw - 16) as f32,
                        (rh - 2) as f32,
                    ),
                    ui::SURFACE_HOVER,
                );
            }
            text::draw(
                d,
                settings::option_label(row),
                px + 18,
                row_y + (rh - section_font) / 2,
                section_font,
                ui::TEXT,
            );
            // Value, right-aligned just left of the steppers, dimmed once the
            // setting sits at a limit so a dead press reads as dead. Toggle
            // rows always show On/Off at full brightness.
            let at_limit = !settings::option_is_toggle(row)
                && ((value == min && min != max) || value == max);
            let vlabel = settings::option_value_label(row);
            let vw = text::measure(d, &vlabel, section_font);
            let (minus, plus) = settings::stepper_rects(row);
            text::draw(
                d,
                &vlabel,
                minus.x as i32 - 10 - vw,
                row_y + (rh - section_font) / 2,
                section_font,
                if at_limit {
                    ui::TEXT_MUTED
                } else {
                    ui::TEXT_SECONDARY
                },
            );
            for (rect, glyph, up) in [(minus, "-", false), (plus, "+", true)] {
                let hovered = screen_mouse.x as i32 >= rect.x as i32
                    && screen_mouse.x as i32 <= (rect.x + rect.width) as i32
                    && screen_mouse.y as i32 >= rect.y as i32
                    && screen_mouse.y as i32 <= (rect.y + rect.height) as i32;
                // A press that would not move the value stays dimmed.
                let live = settings::nudge_preview(row, up);
                d.draw_rectangle_rounded(
                    rect,
                    0.4,
                    4,
                    if hovered && live {
                        ui::ACCENT_STRONG
                    } else if live {
                        ui::ACCENT_SOFT
                    } else {
                        Color::new(255, 255, 255, 8)
                    },
                );
                if live {
                    d.draw_rectangle_rounded_lines_ex(rect, 0.4, 4, 1.0, ui::BORDER);
                }
                let gw = text::measure(d, glyph, step_font);
                text::draw(
                    d,
                    glyph,
                    rect.x as i32 + ((rect.width as i32) - gw) / 2,
                    rect.y as i32 + ((rect.height as i32) - step_font) / 2,
                    step_font,
                    if live {
                        ui::TEXT
                    } else {
                        ui::TEXT_MUTED
                    },
                );
            }
        }
        // Divider between the options and the font list below.
        let list_top = settings::font_list_top();
        d.draw_line(
            px,
            list_top - config::scaled_size(4),
            px + pw,
            list_top - config::scaled_size(4),
            ui::BORDER,
        );

        let fonts = crate::graph::settings::FONTS.read().unwrap();
        let scroll = *crate::graph::settings::SETTINGS_SCROLL.read().unwrap();
        let selected = *crate::graph::settings::SELECTED_FONT.read().unwrap();

        let list_h = py + ph - list_top;

        let mut sc = d.begin_scissor_mode(px + 1, list_top, pw - 2, list_h - 1);
        for i in 0..settings::SETTINGS_VISIBLE_ROWS {
            let idx = scroll + i;
            let Some(fam) = fonts.get(idx) else { break };
            let row_y = list_top + (i as i32) * rh;
            let row_hovered = screen_mouse.x as i32 >= px
                && screen_mouse.x as i32 <= px + pw
                && screen_mouse.y as i32 >= row_y
                && screen_mouse.y as i32 <= row_y + rh;
            let row_selected = selected == Some(idx);

            if row_hovered || row_selected {
                ui::row_pill(
                    &mut sc,
                    Rectangle::new(
                        (px + 8) as f32,
                        (row_y + 1) as f32,
                        (pw - 16) as f32,
                        (rh - 2) as f32,
                    ),
                    if row_selected {
                        ui::ACCENT_SOFT
                    } else {
                        ui::SURFACE_HOVER
                    },
                );
            }
            if row_selected {
                sc.draw_rectangle_rounded(
                    Rectangle::new((px + 11) as f32, (row_y + 6) as f32, 3.0, (rh - 12) as f32),
                    0.5,
                    4,
                    ui::ACCENT,
                );
            }

            let color = if row_selected {
                ui::TEXT
            } else {
                ui::TEXT_SECONDARY
            };
            text::draw(&mut sc, &fam.name, px + 20, row_y + (rh - rf) / 2, rf, color);
        }
        drop(sc);
    }

    // Force debug panel (temporary tuning UI): live sliders for every force
    // parameter plus a switch for the alpha cooldown. Skipped while the
    // settings dialog covers the screen.
    if !settings_open && *SHOW_FORCE_PANEL.read().unwrap() {
        let m = d.get_mouse_position();
        let alpha = *SIM_ALPHA.read().unwrap();
        let alpha_cooling = *ALPHA_COOLING_ENABLED.read().unwrap();

        let px = PANEL_X;
        let py = PANEL_Y;
        let pw = config::scaled_size(PANEL_W);
        let th = config::scaled_size(PANEL_TITLE_H);
        let rh = config::scaled_size(PANEL_ROW_H);
        let body_h = rh * (SLIDER_COUNT as i32 + 2); // toggle row + sliders + respawn
        let total_h = th + body_h;
        let tf = config::scaled_size(18);
        let rf = config::scaled_size(14);
        let vf = config::scaled_size(12);

        ui::panel(
            d,
            Rectangle::new(px as f32, py as f32, pw as f32, total_h as f32),
            0.03,
        );
        let title = format!("Force Controls   alpha={:.3}", alpha);
        text::draw(
            d,
            &title,
            px + config::scaled_size(14),
            py + (th - tf) / 2,
            tf,
            ui::TEXT,
        );
        d.draw_line(px, py + th, px + pw, py + th, ui::BORDER);

        // Alpha-cooldown toggle row (hit-test matches hit_test_alpha_toggle).
        let toggle_y = panel_body_y();
        let toggle_hover = m.x as i32 >= px
            && m.x as i32 <= px + pw
            && m.y as i32 >= toggle_y
            && m.y as i32 <= toggle_y + rh;
        if toggle_hover {
            ui::row_pill(
                d,
                Rectangle::new(
                    (px + 8) as f32,
                    (toggle_y + 1) as f32,
                    (pw - 16) as f32,
                    (rh - 2) as f32,
                ),
                ui::SURFACE_HOVER,
            );
        }
        let on_label = if alpha_cooling {
            "Alpha Cooldown: ON"
        } else {
            "Alpha Cooldown: OFF"
        };
        text::draw(
            d,
            on_label,
            px + config::scaled_size(14),
            toggle_y + (rh - rf) / 2,
            rf,
            if alpha_cooling { ui::SUCCESS } else { ui::TEXT_MUTED },
        );

        // Slider rows: track between TRACK_LEFT..TRACK_RIGHT, thumb at value.
        let track_l = px + config::scaled_size(TRACK_LEFT);
        let track_r = px + config::scaled_size(TRACK_RIGHT);
        let track_w = track_r - track_l;
        let labels = [
            "Link Strength",
            "Velocity Decay",
            "Center Pull",
            "Charge Radius",
            "Rep K",
            "Alpha Decay",
            "Radius Scale",
            "Radius Var.",
            "Collide Pad",
            "Label Fade",
        ];
        for (idx, label) in labels.iter().enumerate() {
            let row_y = slider_row_y(idx);
            let value = read_slider_value(idx);
            let (lo, hi) = SLIDER_RANGES[idx];
            let t = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
            let track_y = row_y + rh / 2 - 3;
            let thumb_x = track_l + (t * track_w as f32) as i32;

            d.draw_rectangle_rounded(
                Rectangle::new(track_l as f32, track_y as f32, track_w as f32, 6.0),
                0.5,
                4,
                Color::new(255, 255, 255, 14),
            );
            d.draw_rectangle_rounded(
                Rectangle::new((thumb_x - 4) as f32, (track_y - 3) as f32, 8.0, 12.0),
                0.5,
                4,
                ui::ACCENT,
            );
            text::draw(
                d,
                label,
                px + config::scaled_size(14),
                row_y + (rh - rf) / 2,
                rf,
                ui::TEXT_SECONDARY,
            );
            let val = if value >= 100.0 {
                format!("{value:.0}")
            } else {
                format!("{value:.3}")
            };
            text::draw(d, &val, track_r + 8, row_y + (rh - vf) / 2, vf, ui::ACCENT);
        }

        // Bottom row: Reset Tweaks (left) and Respawn (right), split at the
        // panel midpoint. Both are hit-tested from the same geometry in
        // processing (hit_test_reset_button / hit_test_respawn_button).
        let btn_y = respawn_button_y();
        let mid = px + pw / 2;
        let reset_hover = m.x as i32 >= px + 8
            && m.x as i32 <= mid - 4
            && m.y as i32 >= btn_y
            && m.y as i32 <= btn_y + rh;
        let respawn_hover = m.x as i32 >= mid + 4
            && m.x as i32 <= px + pw - 8
            && m.y as i32 >= btn_y
            && m.y as i32 <= btn_y + rh;
        let btn_color = |hover: bool| {
            if hover {
                ui::ACCENT_STRONG
            } else {
                ui::ACCENT_SOFT
            }
        };
        d.draw_rectangle_rounded(
            Rectangle::new(
                (px + 8) as f32,
                (btn_y + 3) as f32,
                (mid - px - 12) as f32,
                (rh - 6) as f32,
            ),
            0.32,
            5,
            btn_color(reset_hover),
        );
        let reset_label = "Reset Tweaks";
        let reset_w = text::measure(d, reset_label, rf);
        text::draw(
            d,
            reset_label,
            px + 8 + (mid - px - 12 - reset_w) / 2,
            btn_y + (rh - rf) / 2,
            rf,
            ui::TEXT,
        );
        d.draw_rectangle_rounded(
            Rectangle::new(
                (mid + 4) as f32,
                (btn_y + 3) as f32,
                (px + pw - 8 - mid - 4) as f32,
                (rh - 6) as f32,
            ),
            0.32,
            5,
            btn_color(respawn_hover),
        );
        let respawn_label = "Respawn";
        let respawn_w = text::measure(d, respawn_label, rf);
        text::draw(
            d,
            respawn_label,
            mid + 4 + (px + pw - 8 - mid - 4 - respawn_w) / 2,
            btn_y + (rh - rf) / 2,
            rf,
            ui::TEXT,
        );
    }

    // Vertical zoom scale: fades in while the pointer hovers its band near the
    // right edge (or drags it) and floats on top of the graph, so a stylus can
    // grab it over any node. Skipped while the settings dialog covers the
    // screen, like the force panel.
    if !settings_open {
        draw_zoom_scale(d);
    }
}

// Auto-hiding vertical zoom scale drawing. Runs last in the graph frame so it
// floats above nodes at the right edge. `alpha` eases toward 1.0 while the
// input handler reports hover/drag and back to 0.0 when it does not.
fn draw_zoom_scale(d: &mut RaylibDrawHandle) {
    let mut alpha = *ZOOM_SCALE_ALPHA.read().unwrap();
    let target = if *ZOOM_SCALE_HOVER.read().unwrap() || *ZOOM_SCALE_DRAGGING.read().unwrap() {
        1.0
    } else {
        0.0
    };
    alpha += (target - alpha) * 0.18;
    *ZOOM_SCALE_ALPHA.write().unwrap() = alpha;
    if alpha <= 0.02 {
        return;
    }

    let (bx, btop, bw, bh) = zoom_scale_band();
    let track_w = config::scaled_size(4);
    let track_x = bx + (bw - track_w) / 2;

    // Faint guide rail along the whole band.
    d.draw_rectangle_rounded(
        Rectangle::new(track_x as f32, btop as f32, track_w as f32, bh as f32),
        0.5,
        4,
        Color::new(255, 255, 255, (18.0 * alpha) as u8),
    );

    // Thumb: the current camera zoom projected onto the band (log-spaced).
    let zoom = CAMERA.read().unwrap().zoom;
    let t = zoom_to_t(zoom).clamp(0.0, 1.0);
    let thumb_h = config::scaled_size(30) as f32;
    let thumb_w = config::scaled_size(10);
    let ty = btop as f32 + (1.0 - t) * (bh as f32 - thumb_h);
    let tx = bx + (bw - thumb_w) / 2;
    d.draw_rectangle_rounded(
        Rectangle::new(tx as f32, ty, thumb_w as f32, thumb_h),
        0.5,
        4,
        Color::new(104, 156, 242, (220.0 * alpha) as u8),
    );

    // The current zoom percentage floats beside the thumb while readable.
    if alpha >= 0.4 {
        let label = format!("{:.0}%", zoom * 100.0);
        let fs = config::scaled_size(12);
        let lw = text::measure(d, &label, fs) as i32;
        let lx = tx - lw - config::scaled_size(10);
        let ly = (ty + thumb_h * 0.5 - fs as f32 / 2.0) as i32;
        d.draw_rectangle_rounded(
            Rectangle::new(
                (lx - 6) as f32,
                (ly - 4) as f32,
                (lw + 12) as f32,
                (fs + 8) as f32,
            ),
            0.4,
            5,
            Color::new(14, 16, 21, (210.0 * alpha) as u8),
        );
        text::draw(
            d,
            &label,
            lx,
            ly,
            fs,
            Color::new(237, 240, 246, (235.0 * alpha) as u8),
        );
    }
}

// Current value of the slider at `idx` (0=Link Strength ... 9=Label Fade),
// read from the live PARAM_* statics the debug panel writes.
fn read_slider_value(idx: usize) -> f32 {
    match idx {
        0 => *PARAM_SPRING_K.read().unwrap(),
        1 => *PARAM_DAMPING.read().unwrap(),
        2 => *PARAM_GRAVITY_K.read().unwrap(),
        3 => *PARAM_REPULSION_RADIUS.read().unwrap(),
        4 => *PARAM_REPULSION_K.read().unwrap(),
        5 => *PARAM_ALPHA_DECAY.read().unwrap(),
        6 => *PARAM_RADIUS_SCALE.read().unwrap(),
        7 => *PARAM_RADIUS_VARIATION.read().unwrap(),
        8 => *PARAM_COLLIDE_PAD.read().unwrap(),
        9 => *PARAM_LABEL_FADE.read().unwrap(),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_scale_mapping_round_trips_and_clamps() {
        // Band extremes map to the camera zoom limits.
        assert!((t_to_zoom(0.0) - ZOOM_SCALE_MIN).abs() < 1e-6);
        assert!((t_to_zoom(1.0) - ZOOM_SCALE_MAX).abs() < 1e-6);
        // Out-of-range t clamps instead of extending past the limits.
        assert!((t_to_zoom(-0.5) - ZOOM_SCALE_MIN).abs() < 1e-6);
        assert!((t_to_zoom(2.0) - ZOOM_SCALE_MAX).abs() < 1e-6);
        // Mid-band is log-midway, and zoom <-> t round-trips.
        let mid = t_to_zoom(0.5);
        assert!((mid - (ZOOM_SCALE_MIN * ZOOM_SCALE_MAX).sqrt()).abs() < 1e-4);
        for z in [0.1, 0.3, 1.0, 3.0, 10.0] {
            let t = zoom_to_t(z).clamp(0.0, 1.0);
            assert!((t_to_zoom(t) - z).abs() < 1e-4);
        }
        // The band stays inside the window with a sane vertical extent.
        let (bx, btop, bw, bh) = zoom_scale_band();
        assert!(bx > 0 && bx + bw <= crate::config::width());
        assert!(bh > 0 && btop + bh <= crate::config::height());
        // The bar is held clear of the right edge rather than flush against it.
        assert!(bx + bw < crate::config::width());
    }

    #[test]
    fn circle_rect_intersection_cases() {
        let rect = Rectangle::new(10.0, 20.0, 100.0, 50.0);
        // Center inside.
        assert!(circle_intersects_rect(Vector2::new(60.0, 45.0), 5.0, rect));
        // Overlapping an edge (center just outside).
        assert!(circle_intersects_rect(Vector2::new(110.0, 45.0), 5.0, rect));
        // Touching exactly.
        assert!(circle_intersects_rect(Vector2::new(115.0, 45.0), 5.0, rect));
        // Clear of the rect.
        assert!(!circle_intersects_rect(Vector2::new(200.0, 45.0), 5.0, rect));
        // Far corner miss.
        assert!(!circle_intersects_rect(Vector2::new(-30.0, -30.0), 5.0, rect));
        // A huge radius covers a distant rect.
        assert!(circle_intersects_rect(Vector2::new(-40.0, -40.0), 500.0, rect));
    }

    #[test]
    fn world_bounds_follow_camera() {
        let w = crate::config::width() as f32;
        let h = crate::config::height() as f32;

        // Zoom 1, centered: the whole window centered on the origin, expanded
        // by the preload margin on every side.
        *CAMERA.write().unwrap() = Camera2D {
            target: Vector2::new(0.0, 0.0),
            offset: Vector2::new(w / 2.0, h / 2.0),
            rotation: 0.0,
            zoom: 1.0,
        };
        let m = PRELOAD_MARGIN_PX;
        let b = world_view_bounds();
        assert!((b.x - (-(w / 2.0 + m))).abs() < 0.001);
        assert!((b.y - (-(h / 2.0 + m))).abs() < 0.001);
        assert!((b.width - (w + 2.0 * m)).abs() < 0.001);
        assert!((b.height - (h + 2.0 * m)).abs() < 0.001);

        // Zoom 2: the pen sees half the world, and the margin shrinks too.
        *CAMERA.write().unwrap() = Camera2D {
            target: Vector2::new(0.0, 0.0),
            offset: Vector2::new(w / 2.0, h / 2.0),
            rotation: 0.0,
            zoom: 2.0,
        };
        let m = PRELOAD_MARGIN_PX / 2.0;
        let b = world_view_bounds();
        assert!((b.x - (-(w / 4.0 + m))).abs() < 0.001);
        assert!((b.y - (-(h / 4.0 + m))).abs() < 0.001);
        assert!((b.width - (w / 2.0 + 2.0 * m)).abs() < 0.001);
        assert!((b.height - (h / 2.0 + 2.0 * m)).abs() < 0.001);
    }
}
