use std::sync::RwLock;

use crate::config;
use crate::editor::text;
use crate::filesystem;
use crate::graph::processing::*;
use crate::graph::settings;
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

pub fn draw(d: &mut RaylibDrawHandle) {
    let hover_node = HOVER_NODE.read().unwrap();
    let selected_node = SELECTED_NODE.read().unwrap();
    let delete_pending = DELETE_PENDING.read().unwrap();
    let nodes = NODES.read().unwrap();
    let edges = EDGES.read().unwrap();
    let camera = CAMERA.read().unwrap();

    let mut mode = d.begin_mode2D(*camera);

    mode.clear_background(Color::BLACK);

    for edge in edges.iter() {
        let n1 = &nodes[edge.n1];
        let n2 = &nodes[edge.n2];
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

    for (i, node) in nodes.iter().enumerate() {
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
                if let Some(path) = resolve_header_path(header) {
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

        let text_w = text::measure_f(&mode, &node.name, 5);

        text::draw_f(
            &mut mode,
            &node.name,
            node.position.x - text_w / 2.0,
            node.position.y + node.radius + 6.0,
            5,
            Color::WHITE.alpha(((camera.zoom - 2.0) / 0.5).clamp(0.0, 1.0)),
        );
    }

    drop(mode);
    drop(camera);

    // Draw delete confirmation prompt (outside camera mode, in screen space)
    if *delete_pending {
        if let Some(idx) = *selected_node {
            let name = &nodes[idx].name;
            let prompt = format!("Delete '{}'? (Y/N)", name);
            let fs = config::scaled_size(20);
            let box_h = config::scaled_size(30);
            let text_width = text::measure(d, &prompt, fs);
            let x = (config::width() - text_width) / 2;
            d.draw_rectangle(x - 10, 10, text_width + 20, box_h, Color::BLACK.alpha(0.7));
            text::draw(d, &prompt, x, 10 + (box_h - fs) / 2, fs, Color::WHITE);
        }
    }

    // Draw add-note name prompt (outside camera mode, in screen space)
    let adding_note = *crate::graph::processing::ADDING_NOTE.read().unwrap();
    if adding_note {
        let adding_name = crate::graph::processing::ADDING_NAME.read().unwrap();
        let prompt_base = "Filename: ";
        let full = format!("{}{}", prompt_base, adding_name);
        let fs = config::scaled_size(20);
        let box_h = config::scaled_size(30);
        let label_width = text::measure(d, prompt_base, fs);
        let text_width = text::measure(d, &full, fs);
        let x = (config::width() - text_width) / 2 - 10;
        let y = 10;
        d.draw_rectangle(x, y, text_width + 20, box_h, Color::BLACK.alpha(0.7));
        text::draw(d, prompt_base, x + 10, y + (box_h - fs) / 2, fs, Color::WHITE);

        // Draw the input content (in a lighter color) plus a cursor
        text::draw(d, &adding_name, x + 10 + label_width, y + (box_h - fs) / 2, fs, Color::SKYBLUE);
        let name_width = text::measure(d, &adding_name, fs);
        let cursor_x = x + 10 + label_width + name_width;
        d.draw_rectangle(cursor_x, y + (box_h - fs) / 2, 2, fs, Color::WHITE);
    }

    // Right-click context menu (screen space)
    let context_node = *crate::graph::processing::CONTEXT_NODE.read().unwrap();
    let context_empty = *crate::graph::processing::CONTEXT_EMPTY.read().unwrap();
    let renaming = *crate::graph::processing::RENAMING.read().unwrap();
    if (context_node.is_some() || context_empty) && !renaming {
        let (mx, my) = *crate::graph::processing::CONTEXT_POS.read().unwrap();
        let screen_mouse = d.get_mouse_position();
        let mw = config::scaled_size(config::CONTEXT_MENU_W);
        let mh = config::scaled_size(config::CONTEXT_MENU_ITEM_H);
        let mf = config::scaled_size(18);
        let my_ofs = (mh - mf) / 2;

        if context_empty {
            // Empty-space menu: a single "Add Node" action for now; this is
            // where more global actions can hang in the future.
            let menu_h = mh;
            d.draw_rectangle(mx, my, mw, menu_h, Color::new(20, 20, 20, 235));
            let hover = screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + mw
                && screen_mouse.y as i32 >= my
                && screen_mouse.y as i32 <= my + mh;
            d.draw_rectangle(
                mx, my, mw, mh,
                if hover { Color::new(76, 180, 120, 160) } else { Color::new(0, 0, 0, 0) },
            );
            text::draw(d, "Add Node", mx + 10, my + my_ofs, mf, Color::WHITE);
        } else {
            // Determine sub-graph availability for this node
            let node_name = context_node
            .and_then(|i| nodes.get(i))
            .map(|n| n.name.clone())
            .unwrap_or_default();
        let dir = crate::graph::processing::DIR_PATH.read().unwrap();
        let subgraph_exists = filesystem::is_dir(&dir.join(&node_name));
        drop(dir);

        let show_create = !subgraph_exists;
        let show_open = subgraph_exists;

        // Compute menu height: Rename + Delete + Set Header Image always present,
        // plus one of the sub-graph items when applicable.
        let mut item_count = 3i32;
        if show_create { item_count += 1; }
        if show_open { item_count += 1; }
        let menu_h = item_count * mh;

        // Background
        d.draw_rectangle(mx, my, mw, menu_h, Color::new(20, 20, 20, 235));

        // --- Row 0: Rename ---
        let hover_rename = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= my
            && screen_mouse.y as i32 <= my + mh;
        d.draw_rectangle(
            mx, my, mw, mh,
            if hover_rename { Color::new(76, 128, 204, 160) } else { Color::new(0, 0, 0, 0) },
        );
        text::draw(d, "Rename", mx + 10, my + my_ofs, mf, Color::WHITE);

        let mut y_off = mh;
        d.draw_line(mx, my + y_off, mx + mw, my + y_off, config::CONTEXT_MENU_SEP_COLOR);

        // --- Row 1: Delete ---
        let hover_delete = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= my + y_off
            && screen_mouse.y as i32 <= my + y_off + mh;
        d.draw_rectangle(
            mx, my + y_off, mw, mh,
            if hover_delete { Color::new(200, 60, 60, 160) } else { Color::new(0, 0, 0, 0) },
        );
        text::draw(d, "Delete", mx + 10, my + y_off + my_ofs, mf, Color::WHITE);
        y_off += mh;

        // Separator before sub-graph items (only when at least one is shown)
        if show_create || show_open {
            d.draw_line(mx, my + y_off, mx + mw, my + y_off, config::CONTEXT_MENU_SEP_COLOR);
        }

        // --- Row 2 (conditional): Create Sub-Graph ---
        if show_create {
            let hover_create = screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + mw
                && screen_mouse.y as i32 >= my + y_off
                && screen_mouse.y as i32 <= my + y_off + mh;
            d.draw_rectangle(
                mx, my + y_off, mw, mh,
                if hover_create { Color::new(76, 180, 120, 160) } else { Color::new(0, 0, 0, 0) },
            );
            text::draw(d, "Create Sub-Graph", mx + 10, my + y_off + my_ofs, mf, Color::WHITE);
            y_off += mh;
        }

        // --- Row 2/3 (conditional): Open Sub-Graph ---
        if show_open {
            let hover_open = screen_mouse.x as i32 >= mx
                && screen_mouse.x as i32 <= mx + mw
                && screen_mouse.y as i32 >= my + y_off
                && screen_mouse.y as i32 <= my + y_off + mh;
            d.draw_rectangle(
                mx, my + y_off, mw, mh,
                if hover_open { Color::new(76, 128, 204, 160) } else { Color::new(0, 0, 0, 0) },
            );
            text::draw(d, "Open Sub-Graph", mx + 10, my + y_off + my_ofs, mf, Color::WHITE);
            y_off += mh;
        }

        // --- Row (last): Set Header ---
        d.draw_line(mx, my + y_off, mx + mw, my + y_off, config::CONTEXT_MENU_SEP_COLOR);
        let hover_header = screen_mouse.x as i32 >= mx
            && screen_mouse.x as i32 <= mx + mw
            && screen_mouse.y as i32 >= my + y_off
            && screen_mouse.y as i32 <= my + y_off + mh;
        d.draw_rectangle(
            mx, my + y_off, mw, mh,
            if hover_header { Color::new(76, 128, 204, 160) } else { Color::new(0, 0, 0, 0) },
        );
        text::draw(d, "Set Header", mx + 10, my + y_off + my_ofs, mf, Color::WHITE);
        }
    }

    // Rename-note name prompt (screen space)
    if renaming {
        let rename_name = crate::graph::processing::RENAME_NAME.read().unwrap();
        let prompt_base = "Rename to: ";
        let full = format!("{}{}", prompt_base, rename_name);
        let fs = config::scaled_size(20);
        let box_h = config::scaled_size(30);
        let label_width = text::measure(d, prompt_base, fs);
        let text_width = text::measure(d, &full, fs);
        let x = (config::width() - text_width) / 2 - 10;
        let y = 10;
        d.draw_rectangle(x, y, text_width + 20, box_h, Color::BLACK.alpha(0.7));
        text::draw(d, prompt_base, x + 10, y + (box_h - fs) / 2, fs, Color::WHITE);
        text::draw(d, &rename_name, x + 10 + label_width, y + (box_h - fs) / 2, fs, Color::SKYBLUE);
        let name_width = text::measure(d, &rename_name, fs);
        let cursor_x = x + 10 + label_width + name_width;
        d.draw_rectangle(cursor_x, y + (box_h - fs) / 2, 2, fs, Color::WHITE);
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
        let text_color = Color::new(200, 200, 200, 220);
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
                let color = if hover { Color::SKYBLUE } else { text_color };
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
                text::draw(d, &label, x, config::BREADCRUMB_Y, bf, Color::WHITE);
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
        Color::new(76, 128, 204, 180)
    } else if over_btn {
        Color::new(76, 128, 204, 120)
    } else {
        Color::new(40, 40, 46, 200)
    };
    d.draw_rectangle(
        settings::settings_button_x(),
        settings::SETTINGS_BUTTON_Y,
        sb_w,
        sb_h,
        btn_bg,
    );
    let btn_label = "Settings";
    let btn_w = text::measure(d, btn_label, sb_f);
    text::draw(
        d,
        btn_label,
        settings::settings_button_x() + (sb_w - btn_w) / 2,
        settings::SETTINGS_BUTTON_Y + (sb_h - sb_f) / 2,
        sb_f,
        Color::WHITE,
    );

    // Settings dialog / font picker
    if settings_open {
        d.draw_rectangle(0, 0, config::width(), config::height(), Color::new(0, 0, 0, 120));

        let px = settings::panel_x();
        let py = settings::panel_y();
        let pw = settings::panel_w();
        let ph = settings::panel_h();
        let th = settings::title_h();
        let rh = settings::row_h();
        let tf = config::scaled_size(24);
        let rf = config::scaled_size(16);
        d.draw_rectangle(
            px,
            py,
            pw,
            ph,
            Color::new(25, 25, 30, 245),
        );
        text::draw(d, "Settings", px + 12, py + (th - tf) / 2, tf, Color::WHITE);
        d.draw_line(
            px,
            py + th,
            px + pw,
            py + th,
            Color::new(255, 255, 255, 50),
        );

        let fonts = crate::graph::settings::FONTS.read().unwrap();
        let scroll = *crate::graph::settings::SETTINGS_SCROLL.read().unwrap();
        let selected = *crate::graph::settings::SELECTED_FONT.read().unwrap();

        let list_top = py + th;
        let list_h = ph - th;

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

            if row_hovered {
                sc.draw_rectangle(
                    px,
                    row_y,
                    pw,
                    rh,
                    Color::new(76, 128, 204, 120),
                );
            }
            if row_selected {
                sc.draw_rectangle_lines(
                    px,
                    row_y,
                    pw,
                    rh,
                    Color::new(76, 128, 204, 255),
                );
            }

            let color = if row_selected {
                Color::SKYBLUE
            } else {
                Color::WHITE
            };
            text::draw(&mut sc, &fam.name, px + 12, row_y + (rh - rf) / 2, rf, color);
        }
        drop(sc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
