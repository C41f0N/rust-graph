use crate::config;
use crate::editor::text;
use crate::graph::processing::Node;
use crate::graph::renderer::CAMERA;
use raylib::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

// Hover action popup: after the mouse rests on a node for a moment, a small
// menu appears beside it listing the keyboard shortcuts for that node and
// acting as buttons for the same actions. It is a mouse affordance only; the
// keyboard reticle keeps its own Space/V/R flow and never opens this popup.
//
// Geometry, row order and timing live here so the renderer (which draws the
// popup) and the input handler (which hit-tests it) always agree.

// Seconds the pointer must stay on one node before the popup appears.
const DWELL_SECS: f32 = 0.45;
// Grace period while the pointer crosses the gap between the node and the
// popup, or briefly slips off both.
const OUT_GRACE_SECS: f32 = 0.18;

// Screen-space metrics (scaled by the global text zoom like other UI chrome).
const POPUP_W: i32 = 208;
const POPUP_ROW_H: i32 = 30;
const POPUP_PAD: i32 = 6;
const POPUP_GAP: i32 = 12;
const POPUP_MARGIN: i32 = 8;
const LABEL_INSET: i32 = 10;
const KEY_INSET: i32 = 6;

// The subset of node actions the hover popup exposes. "More..." hands off to
// the full right-click context menu, so rename/fold/copy/header stay reachable
// without duplicating that menu here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoverAction {
    Open,
    OpenSubGraph,
    Delete,
    More,
}

impl HoverAction {
    pub fn label(self) -> &'static str {
        match self {
            HoverAction::Open => "Open",
            HoverAction::OpenSubGraph => "Open Sub-Graph",
            HoverAction::Delete => "Delete",
            HoverAction::More => "More...",
        }
    }

    // Shortcut chip shown on the right of the row. These mirror the graph's
    // existing bindings exactly (Space opens, V enters the sub-graph, Del arms
    // the delete confirmation, R opens the context menu).
    pub fn key(self) -> &'static str {
        match self {
            HoverAction::Open => "Space",
            HoverAction::OpenSubGraph => "V",
            HoverAction::Delete => "Del",
            HoverAction::More => "R",
        }
    }

    fn hover_color(self) -> Color {
        match self {
            HoverAction::Open => Color::new(76, 128, 204, 160),
            HoverAction::OpenSubGraph => Color::new(76, 180, 120, 160),
            HoverAction::Delete => Color::new(200, 60, 60, 160),
            HoverAction::More => Color::new(120, 120, 130, 160),
        }
    }
}

// Ordered actions for one node. The sub-graph row only exists when the node
// actually owns one, matching the V binding and the context menu's gating.
pub fn hover_actions(node: &Node) -> Vec<HoverAction> {
    let mut rows = vec![HoverAction::Open];
    if node.has_subgraph {
        rows.push(HoverAction::OpenSubGraph);
    }
    rows.push(HoverAction::Delete);
    rows.push(HoverAction::More);
    rows
}

// One popup's timing state. The path, rather than the node index, identifies
// the target: node indices shift when the level regenerates, while the note is
// still the same note.
#[derive(Clone, Debug)]
pub struct HoverPopupState {
    pub path: PathBuf,
    pub dwell: f32,
    pub out_for: f32,
    pub visible: bool,
}

impl HoverPopupState {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            dwell: 0.0,
            out_for: 0.0,
            visible: false,
        }
    }
}

pub static HOVER_POPUP: RwLock<Option<HoverPopupState>> = RwLock::new(None);

pub fn clear() {
    *HOVER_POPUP.write().unwrap() = None;
}

// Advance the dwell/grace state machine. Kept free of raylib types so it can be
// tested directly; `inside_popup` is only meaningful once the popup is visible.
fn advance_state(
    state: &mut Option<HoverPopupState>,
    candidate: Option<&Path>,
    inside_popup: bool,
    suppressed: bool,
    dt: f32,
) {
    if suppressed {
        *state = None;
        return;
    }

    match (state.as_mut(), candidate) {
        (None, None) => {}
        (None, Some(path)) => {
            let mut next = HoverPopupState::new(path);
            next.dwell = dt;
            if next.dwell >= DWELL_SECS {
                next.visible = true;
            }
            *state = Some(next);
        }
        (Some(current), Some(path)) if current.path == path => {
            current.out_for = 0.0;
            if !current.visible {
                current.dwell += dt;
                if current.dwell >= DWELL_SECS {
                    current.visible = true;
                }
            }
        }
        (Some(_), Some(path)) => {
            *state = Some(HoverPopupState::new(path));
        }
        (Some(current), None) => {
            if inside_popup {
                current.out_for = 0.0;
            } else if current.visible {
                current.out_for += dt;
                if current.out_for >= OUT_GRACE_SECS {
                    *state = None;
                }
            } else {
                *state = None;
            }
        }
    }
}

// World position -> screen position for the current camera. Raylib's own
// helper needs a RaylibHandle; the popup updates from code that only has the
// camera, so reproduce the (simple, rotation-free) transform here.
pub fn world_to_screen(world: Vector2, camera: &Camera2D) -> Vector2 {
    Vector2::new(
        (world.x - camera.target.x) * camera.zoom + camera.offset.x,
        (world.y - camera.target.y) * camera.zoom + camera.offset.y,
    )
}

fn rect_contains(rect: Rectangle, point: Vector2) -> bool {
    point.x >= rect.x
        && point.x <= rect.x + rect.width
        && point.y >= rect.y
        && point.y <= rect.y + rect.height
}

pub struct PopupLayout {
    pub rect: Rectangle,
    pub rows: Vec<(HoverAction, Rectangle)>,
}

// Where the popup sits for a node: to the right when it fits, to the left when
// it would leave the viewport, vertically centred and clamped to the screen.
pub fn layout_for(node: &Node, camera: &Camera2D) -> PopupLayout {
    let rows = hover_actions(node);
    let w = config::scaled_size(POPUP_W);
    let row_h = config::scaled_size(POPUP_ROW_H);
    let pad = config::scaled_size(POPUP_PAD);
    let gap = config::scaled_size(POPUP_GAP);
    let margin = config::scaled_size(POPUP_MARGIN);
    let h = rows.len() as i32 * row_h + 2 * pad;

    let node_pos = world_to_screen(node.position, camera);
    let radius = node.radius * camera.zoom;

    let mut x = node_pos.x + radius + gap as f32;
    let right_limit = (config::width() - margin) as f32;
    if x + w as f32 > right_limit {
        x = node_pos.x - radius - gap as f32 - w as f32;
    }
    let max_x = (config::width() - margin - w).max(margin) as f32;
    let x = x.clamp(margin as f32, max_x);

    // Keep clear of the shortcut bar pinned to the bottom edge.
    let bottom = config::height() - config::scaled_size(config::HELPBAR_H) - margin;
    let max_y = (bottom - h).max(margin) as f32;
    let y = (node_pos.y - h as f32 / 2.0).clamp(margin as f32, max_y);

    let rect = Rectangle::new(x, y, w as f32, h as f32);
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(i, action)| {
            let row = Rectangle::new(
                x + pad as f32,
                y + pad as f32 + i as f32 * row_h as f32,
                (w - 2 * pad) as f32,
                row_h as f32,
            );
            (action, row)
        })
        .collect();

    PopupLayout { rect, rows }
}

// Resolve the visible popup's node against the current node slice, so a note
// that disappeared (navigation, fold, delete) simply stops drawing.
pub fn visible_index(nodes: &[Node]) -> Option<usize> {
    let state = HOVER_POPUP.read().unwrap();
    let state = state.as_ref()?;
    if !state.visible {
        return None;
    }
    nodes.iter().position(|n| n.path == state.path)
}

pub fn is_visible(nodes: &[Node]) -> bool {
    visible_index(nodes).is_some()
}

// Refresh the dwell/grace state for this frame. Called once per graph frame
// with the node under the mouse, if any.
pub fn update(
    nodes: &[Node],
    camera: &Camera2D,
    candidate: Option<usize>,
    mouse_screen: Vector2,
    suppressed: bool,
    dt: f32,
) {
    let mut state = HOVER_POPUP.write().unwrap();
    if suppressed {
        *state = None;
        return;
    }

    // Keep the popup alive while the pointer is over the popup itself, even
    // though it is no longer over the node.
    let inside_popup = match state.as_ref() {
        Some(current) if current.visible => nodes
            .iter()
            .find(|n| n.path == current.path)
            .map(|node| rect_contains(layout_for(node, camera).rect, mouse_screen))
            .unwrap_or(false),
        _ => false,
    };

    // The popup floats above the graph, so the node under the pointer while it
    // is over the popup is irrelevant: freeze the current target instead of
    // retargeting to a node the popup happens to cover.
    let candidate = if inside_popup { None } else { candidate };
    let candidate_path = candidate.map(|i| nodes[i].path.as_path());
    advance_state(&mut state, candidate_path, inside_popup, false, dt);
}

// The action row under the pointer, if the popup is visible. Drawing and
// hit-testing both go through layout_for, so a click can never target a row
// that was not drawn.
pub fn hit_test(
    nodes: &[Node],
    camera: &Camera2D,
    mouse_screen: Vector2,
) -> Option<(usize, HoverAction)> {
    let idx = visible_index(nodes)?;
    let layout = layout_for(&nodes[idx], camera);
    for (action, rect) in layout.rows {
        if rect_contains(rect, mouse_screen) {
            return Some((idx, action));
        }
    }
    None
}

// Draw the popup in screen space. The caller has already filtered out modal
// states; `nodes` is the renderer's existing read guard.
pub fn draw(d: &mut RaylibDrawHandle, nodes: &[Node]) {
    let Some(idx) = visible_index(nodes) else {
        return;
    };
    let camera = *CAMERA.read().unwrap();
    let layout = layout_for(&nodes[idx], &camera);
    let mouse = d.get_mouse_position();
    let font = config::scaled_size(15);
    let key_font = config::scaled_size(12);
    let label_inset = config::scaled_size(LABEL_INSET);
    let key_inset = config::scaled_size(KEY_INSET);

    d.draw_rectangle_rounded(layout.rect, 0.16, 8, Color::new(20, 20, 20, 235));
    d.draw_rectangle_rounded_lines_ex(
        layout.rect,
        0.16,
        8,
        1.0,
        Color::new(255, 255, 255, 36),
    );

    for (row_index, (action, rect)) in layout.rows.iter().enumerate() {
        // Separate the hand-off row from the direct actions.
        if *action == HoverAction::More && row_index > 0 {
            d.draw_line(
                rect.x as i32,
                rect.y as i32,
                (rect.x + rect.width) as i32,
                rect.y as i32,
                Color::new(255, 255, 255, 30),
            );
        }

        if rect_contains(*rect, mouse) {
            d.draw_rectangle_rec(*rect, action.hover_color());
        }

        text::draw(
            d,
            action.label(),
            rect.x as i32 + label_inset,
            rect.y as i32 + (rect.height as i32 - font) / 2,
            font,
            Color::WHITE,
        );

        let key = action.key();
        let key_w = text::measure(d, key, key_font);
        let chip_w = key_w + key_inset * 2;
        let chip_h = key_font + config::scaled_size(8);
        let chip = Rectangle::new(
            rect.x + rect.width - chip_w as f32 - config::scaled_size(8) as f32,
            rect.y + (rect.height - chip_h as f32) / 2.0,
            chip_w as f32,
            chip_h as f32,
        );
        d.draw_rectangle_rounded(chip, 0.3, 6, Color::new(255, 255, 255, 18));
        text::draw(
            d,
            key,
            chip.x as i32 + key_inset,
            chip.y as i32 + (chip_h - key_font) / 2,
            key_font,
            Color::new(225, 228, 235, 230),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, has_subgraph: bool) -> Node {
        Node {
            radius: 6.0,
            color: Color::WHITE,
            position: Vector2::new(100.0, 100.0),
            velocity: Vector2::zero(),
            name: name.to_string(),
            file_name: format!("{name}.md"),
            path: PathBuf::from(format!("/tmp/{name}.md")),
            header: None,
            has_subgraph,
            folder_backed: false,
        }
    }

    fn camera() -> Camera2D {
        Camera2D {
            target: Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0),
            offset: Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0),
            rotation: 0.0,
            zoom: 1.0,
        }
    }

    #[test]
    fn subgraph_row_is_gated_on_has_subgraph() {
        let plain = hover_actions(&node("plain", false));
        assert_eq!(
            plain,
            vec![HoverAction::Open, HoverAction::Delete, HoverAction::More]
        );
        let folder = hover_actions(&node("folder", true));
        assert_eq!(
            folder,
            vec![
                HoverAction::Open,
                HoverAction::OpenSubGraph,
                HoverAction::Delete,
                HoverAction::More
            ]
        );
    }

    #[test]
    fn dwell_shows_only_after_the_threshold() {
        let path = PathBuf::from("/tmp/a.md");
        let mut state = None;
        advance_state(&mut state, Some(&path), false, false, DWELL_SECS * 0.5);
        assert!(!state.as_ref().unwrap().visible);
        advance_state(&mut state, Some(&path), false, false, DWELL_SECS * 0.49);
        assert!(!state.as_ref().unwrap().visible);
        advance_state(&mut state, Some(&path), false, false, 0.02);
        assert!(state.as_ref().unwrap().visible);
    }

    #[test]
    fn moving_to_another_node_restarts_the_dwell() {
        let a = PathBuf::from("/tmp/a.md");
        let b = PathBuf::from("/tmp/b.md");
        let mut state = None;
        advance_state(&mut state, Some(&a), false, false, DWELL_SECS + 0.1);
        assert!(state.as_ref().unwrap().visible);
        advance_state(&mut state, Some(&b), false, false, 0.01);
        let state = state.as_ref().unwrap();
        assert_eq!(state.path, b);
        assert!(!state.visible);
        assert!(state.dwell < DWELL_SECS);
    }

    #[test]
    fn leaving_uses_a_short_grace_period() {
        let path = PathBuf::from("/tmp/a.md");
        let mut state = None;
        advance_state(&mut state, Some(&path), false, false, DWELL_SECS + 0.1);
        assert!(state.is_some());

        advance_state(&mut state, None, false, false, OUT_GRACE_SECS * 0.5);
        assert!(state.is_some(), "popup should survive the grace period");
        advance_state(&mut state, None, false, false, OUT_GRACE_SECS * 0.6);
        assert!(state.is_none(), "popup should close after the grace period");
    }

    #[test]
    fn entering_the_popup_keeps_it_open() {
        let path = PathBuf::from("/tmp/a.md");
        let mut state = None;
        advance_state(&mut state, Some(&path), false, false, DWELL_SECS + 0.1);
        for _ in 0..10 {
            advance_state(&mut state, None, true, false, OUT_GRACE_SECS);
            assert!(state.is_some());
        }
    }

    #[test]
    fn leaving_before_it_appears_closes_at_once() {
        let path = PathBuf::from("/tmp/a.md");
        let mut state = None;
        advance_state(&mut state, Some(&path), false, false, 0.1);
        advance_state(&mut state, None, false, false, 0.01);
        assert!(state.is_none());
    }

    #[test]
    fn suppression_clears_everything() {
        let path = PathBuf::from("/tmp/a.md");
        let mut state = None;
        advance_state(&mut state, Some(&path), false, false, DWELL_SECS + 0.1);
        advance_state(&mut state, Some(&path), false, true, 0.01);
        assert!(state.is_none());
    }

    #[test]
    fn layout_stays_inside_the_viewport() {
        let cam = camera();
        // A node near the right edge must flip the popup to its left.
        let mut right_edge = node("edge", false);
        right_edge.position = Vector2::new(
            config::width() as f32 - 4.0,
            config::height() as f32 / 2.0,
        );
        let layout = layout_for(&right_edge, &cam);
        assert!(layout.rect.x >= 0.0);
        assert!(layout.rect.x + layout.rect.width <= config::width() as f32);
        assert!(layout.rect.y >= 0.0);
        assert!(layout.rect.y + layout.rect.height <= config::height() as f32);
        // The popup sits to the left of the node's disc, not over it.
        let screen = world_to_screen(right_edge.position, &cam);
        assert!(layout.rect.x + layout.rect.width <= screen.x);
    }

    #[test]
    fn layout_rows_are_clickable_and_ordered() {
        let cam = camera();
        let plain = node("plain", false);
        let layout = layout_for(&plain, &cam);
        assert_eq!(layout.rows.len(), 3);
        assert_eq!(layout.rows[0].0, HoverAction::Open);
        assert_eq!(layout.rows[2].0, HoverAction::More);
        for (_, row) in &layout.rows {
            assert!(row.x >= layout.rect.x);
            assert!(row.x + row.width <= layout.rect.x + layout.rect.width);
        }
    }
}
