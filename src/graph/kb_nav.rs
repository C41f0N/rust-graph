// Keyboard graph navigation: a reticle pinned to the centre of the view stands
// in for the mouse while the user drives the graph from the keys. W/A/S/D pan
// the camera under the reticle, C zooms out and Shift+C zooms in (both anchored
// on the reticle), and Space / V / R act on the node under it (open, enter its
// sub-graph, context menu). Any real mouse motion (or Esc) hands the pointer
// back to the mouse, so the two never fight over the hover target.

use crate::config;
use raylib::prelude::*;
use std::sync::RwLock;

// True while the keyboard owns the pointer: the renderer draws the reticle and
// the input handler picks the hovered node from the screen centre instead of
// the mouse.
pub static RETICLE_ACTIVE: RwLock<bool> = RwLock::new(false);

// Moving the mouse this many screen pixels in one frame hands the pointer back
// to it, so reaching for the mouse always wins over a latched reticle.
pub const RETICLE_HANDOFF_PX: f32 = 3.0;

// On-screen pan speed for W/A/S/D, in pixels per second. The camera target
// advances by pan_px / zoom world units, so the view glides at a constant rate
// on screen no matter how far the camera is zoomed in or out.
pub const PAN_PX_PER_SEC: f32 = 560.0;

// The centre of the viewport, where the reticle sits.
pub fn screen_center() -> (f32, f32) {
    (config::width() as f32 / 2.0, config::height() as f32 / 2.0)
}

// The world point currently under the reticle
// (world = (screen - offset) / zoom + target).
pub fn reticle_world(cam: &Camera2D) -> Vector2 {
    let (cx, cy) = screen_center();
    Vector2::new(
        (cx - cam.offset.x) / cam.zoom + cam.target.x,
        (cy - cam.offset.y) / cam.zoom + cam.target.y,
    )
}

// The camera target that keeps `world` pinned under the reticle at the
// camera's current zoom and offset — the anchor for a keyboard zoom step.
pub fn target_for_center(world: Vector2, cam: &Camera2D) -> Vector2 {
    let (cx, cy) = screen_center();
    Vector2::new(
        world.x - (cx - cam.offset.x) / cam.zoom,
        world.y - (cy - cam.offset.y) / cam.zoom,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(zoom: f32, tx: f32, ty: f32) -> Camera2D {
        let (cx, cy) = screen_center();
        Camera2D {
            target: Vector2::new(tx, ty),
            offset: Vector2::new(cx, cy),
            rotation: 0.0,
            zoom,
        }
    }

    #[test]
    fn reticle_reports_the_world_point_under_the_centre() {
        // With the offset exactly on the centre, the reticle is the target.
        let c = cam(2.0, 10.0, -5.0);
        let w = reticle_world(&c);
        assert!((w.x - 10.0).abs() < 1e-4);
        assert!((w.y + 5.0).abs() < 1e-4);
    }

    #[test]
    fn target_for_center_round_trips() {
        // Anchoring a world point must leave the reticle sitting on it.
        let world = Vector2::new(123.0, -47.0);
        for zoom in [0.1, 0.3, 1.0, 3.0, 10.0] {
            let mut c = cam(zoom, 0.0, 0.0);
            c.target = target_for_center(world, &c);
            let back = reticle_world(&c);
            assert!((back.x - world.x).abs() < 1e-3, "zoom {zoom}");
            assert!((back.y - world.y).abs() < 1e-3, "zoom {zoom}");
        }
    }

    #[test]
    fn anchoring_holds_a_world_point_through_a_zoom_change() {
        // A zoom step anchored at the centre keeps the reticle's world point.
        let c = cam(1.0, 30.0, 20.0);
        let world = reticle_world(&c);
        let mut next = c;
        next.zoom = 3.0;
        next.target = target_for_center(world, &next);
        let after = reticle_world(&next);
        assert!((after.x - world.x).abs() < 1e-3);
        assert!((after.y - world.y).abs() < 1e-3);
    }
}
