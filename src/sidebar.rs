use crate::config;
use raylib::prelude::*;

// Per-view navigation bar: a small floating strip along the left edge,
// vertically centered, that stays on top of every page so the user can switch
// between the graph view and the editor view from anywhere. Minimal black and
// white: no labels, just a line-graph icon and a document icon.

pub const BAR_W: i32 = 56;
pub const BAR_MARGIN_X: i32 = 12;
pub const BUTTON_H: i32 = 40;
pub const BUTTON_GAP: i32 = 8;
pub const BUTTON_INSET: i32 = 6;
pub const BAR_PAD_V: i32 = 8;

pub const GRAPH_BUTTON: usize = 0;
pub const EDITOR_BUTTON: usize = 1;
pub const BUTTON_COUNT: usize = 2;

const BAR_MARGIN_Y: i32 = BAR_PAD_V;
const BAR_H: i32 = 2 * BUTTON_H + BUTTON_GAP + 2 * BAR_PAD_V;

// Monochrome palette: the strip is black, everything else is white alpha.
const BAR_BG: Color = Color::new(18, 18, 18, 235);
const BAR_EDGE: Color = Color::new(255, 255, 255, 36);
const HOVER_BG: Color = Color::new(255, 255, 255, 20);
const ACTIVE_BG: Color = Color::new(255, 255, 255, 14);
const ICON_IDLE: Color = Color::new(255, 255, 255, 110);
const ICON_ACTIVE: Color = Color::new(255, 255, 255, 255);
const ACCENT: Color = Color::new(255, 255, 255, 235);

// The whole strip rectangle, vertically centered on the left edge.
pub fn bar_bounds() -> (i32, i32, i32, i32) {
    let y = ((config::HEIGHT - BAR_H) / 2).max(0);
    (BAR_MARGIN_X, y, BAR_W, BAR_H)
}

// The rectangle of a single stacked button inside the strip.
pub fn button_bounds(id: usize) -> (i32, i32, i32, i32) {
    let (x, y, w, _) = bar_bounds();
    let bx = x + BUTTON_INSET;
    let bw = w - 2 * BUTTON_INSET;
    let by = y + BAR_MARGIN_Y + (id as i32) * (BUTTON_H + BUTTON_GAP);
    (bx, by, bw, BUTTON_H)
}

// Which button a screen point hits, if any.
fn button_at(m: Vector2) -> Option<usize> {
    for id in 0..BUTTON_COUNT {
        let (bx, by, bw, bh) = button_bounds(id);
        if m.x as i32 >= bx
            && m.x as i32 <= bx + bw
            && m.y as i32 >= by
            && m.y as i32 <= by + bh
        {
            return Some(id);
        }
    }
    None
}

fn hit_bar(m: Vector2) -> bool {
    let (bx, by, bw, bh) = bar_bounds();
    m.x as i32 >= bx
        && m.x as i32 <= bx + bw
        && m.y as i32 >= by
        && m.y as i32 <= by + bh
}

// Handles clicks on the strip. Returns true when the press landed on the bar
// (consumed: nothing behind it gets the click this frame). Switching views only
// toggles editor_open; main.rs's existing open/close transitions load and save
// the buffer and (re)build edges.
pub fn handle_input(rl: &mut RaylibHandle, editor_open: &mut bool) -> bool {
    if !rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        return false;
    }
    let m = rl.get_mouse_position();
    if !hit_bar(m) {
        return false;
    }
    if let Some(id) = button_at(m) {
        if id == GRAPH_BUTTON {
            *editor_open = false;
        } else {
            // Editor button: open the editor view. Open tabs persist across
            // view switches, so just reopening is enough. With no tabs yet,
            // fall back to the currently selected graph node so the button
            // always has something to show.
            if !*editor_open {
                if !crate::editor::tabs::has_any() {
                    let selected = *crate::graph::processing::SELECTED_NODE.read().unwrap();
                    if let Some(i) = selected {
                        let (path, name) = {
                            let nodes = crate::graph::processing::NODES.read().unwrap();
                            let n = &nodes[i];
                            (n.path.clone(), n.name.clone())
                        };
                        crate::editor::tabs::open(&path, &name);
                    }
                }
            }
            *editor_open = true;
        }
    }
    true
}

// Line-graph mark: three nodes joined by edges, read as "the graph view".
fn icon_graph(d: &mut RaylibDrawHandle, cx: i32, cy: i32, c: Color) {
    let n: [Vector2; 3] = [
        Vector2::new(cx as f32, (cy - 9) as f32),
        Vector2::new((cx - 9) as f32, (cy + 8) as f32),
        Vector2::new((cx + 9) as f32, (cy + 8) as f32),
    ];
    d.draw_line_ex(n[0], n[1], 1.5, c);
    d.draw_line_ex(n[0], n[2], 1.5, c);
    d.draw_line_ex(n[1], n[2], 1.5, c);
    for node in n {
        d.draw_circle_v(node, 4.0, c);
    }
}

// Document mark: an outlined page with three lines of text.
fn icon_editor(d: &mut RaylibDrawHandle, cx: i32, cy: i32, c: Color) {
    d.draw_rectangle_lines_ex(Rectangle::new((cx - 10) as f32, (cy - 12) as f32, 20.0, 24.0), 1.5, c);
    d.draw_rectangle(cx - 6, cy - 5, 12, 2, c);
    d.draw_rectangle(cx - 6, cy - 1, 9, 2, c);
    d.draw_rectangle(cx - 6, cy + 3, 12, 2, c);
}

pub fn draw(d: &mut RaylibDrawHandle, editor_open: bool) {
    let (bx, by, bw, bh) = bar_bounds();
    let screen_mouse = d.get_mouse_position();
    let over = hit_bar(screen_mouse);

    // Strip background: solid black so it reads over the graph and the editor.
    d.draw_rectangle_rounded(
        Rectangle::new(bx as f32, by as f32, bw as f32, bh as f32),
        0.2,
        0,
        BAR_BG,
    );
    // Right edge divider so the strip reads as a dock.
    d.draw_line(bx + bw, by, bx + bw, by + bh, BAR_EDGE);

    for id in 0..BUTTON_COUNT {
        let (x, y, w, h) = button_bounds(id);
        let hovered = !over && button_at(screen_mouse) == Some(id);
        let active = match id {
            GRAPH_BUTTON => !editor_open,
            _ => editor_open,
        };

        if active {
            d.draw_rectangle_rounded(
                Rectangle::new(x as f32, y as f32, w as f32, h as f32),
                0.2,
                0,
                ACTIVE_BG,
            );
            // Left accent bar on the active button.
            d.draw_rectangle(x, y + 5, 3, h - 10, ACCENT);
        } else if hovered {
            d.draw_rectangle_rounded(
                Rectangle::new(x as f32, y as f32, w as f32, h as f32),
                0.2,
                0,
                HOVER_BG,
            );
        }

        let cx = x + w / 2;
        let cy = y + h / 2;
        let color = if active { ICON_ACTIVE } else { ICON_IDLE };
        match id {
            GRAPH_BUTTON => icon_graph(d, cx, cy, color),
            _ => icon_editor(d, cx, cy, color),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_vertically_centered() {
        let (_, y, _, h) = bar_bounds();
        assert_eq!((y + h / 2) * 2, config::HEIGHT);
    }

    #[test]
    fn buttons_inside_strip_and_non_overlapping() {
        let (bx, by, bw, bh) = bar_bounds();
        let (gx, gy, gw, gh) = button_bounds(GRAPH_BUTTON);
        let (ex, ey, ew, eh) = button_bounds(EDITOR_BUTTON);

        // Inside the strip.
        assert!(gx >= bx && gx + gw <= bx + bw);
        assert!(ex >= bx && ex + ew <= bx + bw);
        assert!(gy >= by && gy + gh <= by + bh);
        assert!(ey >= by && ey + eh <= by + bh);

        // No vertical overlap: editor button sits below the graph button.
        assert!(ey >= gy + gh);
    }

    #[test]
    fn button_hit_testing() {
        let (x, y, w, h) = button_bounds(GRAPH_BUTTON);
        assert_eq!(button_at(Vector2::new((x + w / 2) as f32, (y + h / 2) as f32)), Some(GRAPH_BUTTON));
        let (_, by, _, _) = bar_bounds();
        // Center of the divider between the two buttons.
        assert_eq!(
            button_at(Vector2::new((x + w / 2) as f32, (y + h + BUTTON_GAP / 2) as f32)),
            None
        );
        // Center of the strip's empty top padding.
        assert_eq!(
            button_at(Vector2::new((x + w / 2) as f32, (by + BAR_PAD_V / 2) as f32)),
            None
        );
    }
}