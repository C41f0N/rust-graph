// Keyboard graph navigation: a reticle pinned to the centre of the view stands
// in for the mouse while the user drives the graph from the keys. W/A/S/D pan
// the camera under the reticle with a short acceleration ramp, holding Q zooms
// in and E zooms out (also eased, anchored on the reticle), and Space / V / R
// act on the node under it (open, enter its sub-graph, context menu). Any real
// mouse motion (or Esc) hands the pointer back to the mouse, so the two never
// fight over the hover target.

use crate::config;
use crate::graph::renderer::{t_to_zoom, zoom_to_t, ZOOM_SCALE_MAX, ZOOM_SCALE_MIN};
use raylib::prelude::*;
use std::sync::RwLock;

// True while the keyboard owns the pointer: the renderer draws the reticle and
// the input handler picks the hovered node from the screen centre instead of
// the mouse.
pub static RETICLE_ACTIVE: RwLock<bool> = RwLock::new(false);

// On-screen pan speed cap for W/A/S/D, in pixels per second. The camera target
// advances by pan_px / zoom world units, so the view moves at a constant rate
// on screen no matter how far the camera is zoomed in or out.
pub const PAN_PX_PER_SEC: f32 = 560.0;

// How quickly the pan velocity ramps toward PAN_PX_PER_SEC (and back to zero
// on release), in pixels per second squared. The short ramp (~90 ms) keeps the
// middle of a held key at the cap while making the start and stop feel soft
// instead of snapping between 0 and full speed.
pub const PAN_ACCEL_PX_PER_SEC2: f32 = 6400.0;

// Continuous keyboard zoom speed cap: the fraction of the log zoom band
// crossed per second at full tilt while Q (in) / E (out) is held. The velocity
// eases up to this cap and back down to zero.
pub const KB_ZOOM_T_PER_SEC: f32 = 0.6;

// Zoom acceleration in log-band units per second squared. ~100 ms to reach the
// cap, mirroring the pan ramp.
pub const KB_ZOOM_ACCEL_T_PER_SEC2: f32 = 6.0;

// Holding Shift multiplies both keyboard speeds. The acceleration scales with
// it too, so the ramp to the faster cap still takes the same ~90-100 ms and
// only the sustained speed changes.
pub const SHIFT_SPEED_MULTIPLIER: f32 = 2.0;

// Longest frame step the motion integrator accepts. A window drag or
// breakpoint can otherwise produce a huge dt and teleport the camera.
pub const MAX_FRAME_DT: f32 = 0.05;

// Live camera velocities, in screen pixels/s and log-zoom units/s. They persist
// across frames so releasing a key glides to a stop instead of stopping dead.
pub static PAN_VELOCITY: RwLock<Vector2> = RwLock::new(Vector2 { x: 0.0, y: 0.0 });
pub static ZOOM_VELOCITY: RwLock<f32> = RwLock::new(0.0);

// One wheel notch scales the zoom destination by this factor. Zoom is felt
// logarithmically, so a fixed multiplier keeps a notch the same relative size
// at any zoom level; adding a constant would feel huge when zoomed in and
// barely visible when zoomed out.
pub const WHEEL_ZOOM_FACTOR: f32 = 1.1;

// Wheel easing: each notch retargets the tween and the camera eases in/out to
// the new destination. Keep the times short: a wheel notch is an impulse, so a
// long tween reads as input lag rather than smoothness. Even the slowest move
// settles well under a third of a second, and the cap lets rapid notches keep
// up instead of the camera trailing the destination.
const WHEEL_EASE_MIN_SECS: f32 = 0.06;
const WHEEL_EASE_MAX_SECS: f32 = 0.28;
const WHEEL_EASE_PEAK_T_PER_SEC: f32 = 1.2;

// A wheel zoom tween. `target` is the cumulative destination (one step per
// wheel notch); `start` is where the camera was when the latest notch landed;
// `elapsed`/`duration` drive a smooth ease-in-out between them.
pub struct ZoomTween {
    pub start: f32,
    pub target: f32,
    pub elapsed: f32,
    pub duration: f32,
}

pub static ZOOM_TWEEN: RwLock<Option<ZoomTween>> = RwLock::new(None);

// Clamp a frame step so a hitch cannot inject a huge velocity change.
pub fn frame_dt(dt: f32) -> f32 {
    dt.clamp(0.0, MAX_FRAME_DT)
}

// Move a scalar toward `target` by at most `max_delta` (no overshoot).
fn approach(current: f32, target: f32, max_delta: f32) -> f32 {
    if current < target {
        (current + max_delta).min(target)
    } else {
        (current - max_delta).max(target)
    }
}

// Vector form of `approach`: change the velocity by at most `max_delta` in
// magnitude, so diagonals accelerate as one vector instead of per axis.
fn approach_vec(current: Vector2, target: Vector2, max_delta: f32) -> Vector2 {
    let diff = target - current;
    let len = diff.length();
    if len <= max_delta || len == 0.0 {
        target
    } else {
        current + diff * (max_delta / len)
    }
}

// Advance one frame of pan velocity toward the capped input direction.
// `direction` need not be normalized; (0, 0) means "no key held" and lets the
// current velocity coast down to zero. `boost` scales the cap and acceleration
// (1.0 = normal, SHIFT_SPEED_MULTIPLIER = fast).
pub fn pan_velocity_step(current: Vector2, direction: Vector2, dt: f32, boost: f32) -> Vector2 {
    let len = direction.length();
    let target = if len > 0.0 {
        direction * (PAN_PX_PER_SEC * boost / len)
    } else {
        Vector2::zero()
    };
    approach_vec(current, target, PAN_ACCEL_PX_PER_SEC2 * boost * frame_dt(dt))
}

// Advance one frame of zoom velocity toward the capped Q/E direction (+1 in,
// -1 out, 0 = coast to a stop). `boost` scales the cap and acceleration.
pub fn zoom_velocity_step(current: f32, direction: f32, dt: f32, boost: f32) -> f32 {
    let target = if direction == 0.0 {
        0.0
    } else {
        direction.signum() * KB_ZOOM_T_PER_SEC * boost
    };
    approach(current, target, KB_ZOOM_ACCEL_T_PER_SEC2 * boost * frame_dt(dt))
}

// Apply `velocity` (log-band units/s) for one frame. Returns the new zoom and
// whether the band limit stopped it, so the caller can zero the velocity
// instead of leaning on the clamp.
pub fn advance_zoom(zoom: f32, velocity: f32, dt: f32) -> (f32, bool) {
    let raw = zoom_to_t(zoom) + velocity * frame_dt(dt);
    let clamped = raw.clamp(0.0, 1.0);
    (t_to_zoom(clamped), (raw - clamped).abs() > f32::EPSILON)
}

// Smoothstep: zero velocity at both ends, so a wheel step eases in and out.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// How long the tween should take. Smoothstep peaks at 1.5x the average speed,
// so scale the duration by the log-zoom distance to keep that peak near the
// keyboard zoom cap.
fn wheel_duration(from: f32, to: f32) -> f32 {
    let distance = (zoom_to_t(to) - zoom_to_t(from)).abs();
    (1.5 * distance / WHEEL_EASE_PEAK_T_PER_SEC)
        .clamp(WHEEL_EASE_MIN_SECS, WHEEL_EASE_MAX_SECS)
}

// Position of a tween at `elapsed` seconds, plus whether it has finished.
pub fn wheel_zoom_at(start: f32, target: f32, elapsed: f32, duration: f32) -> (f32, bool) {
    let t = (elapsed / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
    // Blend in log space: zoom is multiplicative, so interpolating the raw
    // values would cover far more ground in the zoomed-in half of the tween.
    let zoom = start * (target / start).powf(ease_in_out(t));
    (zoom, t >= 1.0)
}

// Wheel destination for one input: accumulate multiplicatively so every notch
// is the same relative step, independent of the current zoom. `wheel` may be
// fractional (trackpads) or negative (zoom out).
pub fn wheel_zoom_target(base: f32, wheel: f32) -> f32 {
    (base * WHEEL_ZOOM_FACTOR.powf(wheel)).clamp(ZOOM_SCALE_MIN, ZOOM_SCALE_MAX)
}

// Queue a wheel zoom. The destination accumulates from the pending tween (or
// the live zoom when none is running), so several notches count together; the
// tween itself always restarts from the camera's current position and eases
// in-out to that destination.
pub fn queue_wheel_zoom(zoom: f32, wheel: f32) {
    let base = {
        let pending = ZOOM_TWEEN.read().unwrap();
        pending.as_ref().map(|t| t.target).unwrap_or(zoom)
    };
    let target = wheel_zoom_target(base, wheel);
    let duration = wheel_duration(zoom, target);
    *ZOOM_VELOCITY.write().unwrap() = 0.0;
    *ZOOM_TWEEN.write().unwrap() = Some(ZoomTween {
        start: zoom,
        target,
        elapsed: 0.0,
        duration,
    });
}

// Advance the running wheel tween, if any, and return the new camera zoom.
pub fn step_wheel_zoom(dt: f32) -> Option<f32> {
    let mut pending = ZOOM_TWEEN.write().unwrap();
    let (next, done) = {
        let tween = pending.as_mut()?;
        tween.elapsed += frame_dt(dt);
        wheel_zoom_at(tween.start, tween.target, tween.elapsed, tween.duration)
    };
    if done {
        *pending = None;
    }
    Some(next)
}

// Drop any residual glide. Called when the camera is reframed or another input
// source takes over.
pub fn reset_motion() {
    *PAN_VELOCITY.write().unwrap() = Vector2::zero();
    *ZOOM_VELOCITY.write().unwrap() = 0.0;
    *ZOOM_TWEEN.write().unwrap() = None;
}

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

    #[test]
    fn pan_velocity_ramps_to_the_cap_and_glides_to_a_stop() {
        let dir = Vector2::new(1.0, 0.0);
        let dt = 1.0 / 60.0;
        let mut v = Vector2::zero();

        // The first frame is a small step, not an instant jump to full speed.
        v = pan_velocity_step(v, dir, dt, 1.0);
        assert!(v.x > 0.0 && v.x < PAN_PX_PER_SEC);

        for _ in 0..30 {
            v = pan_velocity_step(v, dir, dt, 1.0);
            assert!(v.length() <= PAN_PX_PER_SEC + 1e-3);
        }
        assert!((v.x - PAN_PX_PER_SEC).abs() < 1e-3);
        assert!(v.y.abs() < 1e-6);

        // Releasing the key decelerates monotonically to rest.
        for _ in 0..30 {
            let before = v.x;
            v = pan_velocity_step(v, Vector2::zero(), dt, 1.0);
            assert!(v.x >= -1e-6 && v.x <= before);
        }
        assert!(v.length() < 1e-3);
    }

    #[test]
    fn diagonal_pan_is_capped_like_a_cardinal_one() {
        let dt = 1.0 / 60.0;
        let mut v = Vector2::zero();
        for _ in 0..30 {
            v = pan_velocity_step(v, Vector2::new(1.0, 1.0), dt, 1.0);
        }
        assert!(v.length() <= PAN_PX_PER_SEC + 1e-3);
        assert!((v.length() - PAN_PX_PER_SEC).abs() < 1e-3);
        assert!((v.x - v.y).abs() < 1e-3);
    }

    #[test]
    fn zoom_velocity_ramps_and_stops() {
        let dt = 1.0 / 60.0;
        let mut v = 0.0;
        for _ in 0..30 {
            v = zoom_velocity_step(v, 1.0, dt, 1.0);
        }
        assert!((v - KB_ZOOM_T_PER_SEC).abs() < 1e-4);

        for _ in 0..30 {
            v = zoom_velocity_step(v, 0.0, dt, 1.0);
        }
        assert!(v.abs() < 1e-5);
    }

    #[test]
    fn shift_boost_raises_the_pan_and_zoom_caps() {
        let dt = 1.0 / 60.0;
        let dir = Vector2::new(1.0, 0.0);

        // The ramp accelerates twice as hard, so the first frame is 2x too.
        let normal = pan_velocity_step(Vector2::zero(), dir, dt, 1.0);
        let fast = pan_velocity_step(Vector2::zero(), dir, dt, SHIFT_SPEED_MULTIPLIER);
        assert!((fast.x - normal.x * SHIFT_SPEED_MULTIPLIER).abs() < 1e-3);

        let mut pan = Vector2::zero();
        let mut zoom = 0.0;
        for _ in 0..30 {
            pan = pan_velocity_step(pan, dir, dt, SHIFT_SPEED_MULTIPLIER);
            zoom = zoom_velocity_step(zoom, 1.0, dt, SHIFT_SPEED_MULTIPLIER);
        }
        assert!((pan.x - PAN_PX_PER_SEC * SHIFT_SPEED_MULTIPLIER).abs() < 1e-3);
        assert!((zoom - KB_ZOOM_T_PER_SEC * SHIFT_SPEED_MULTIPLIER).abs() < 1e-4);
    }

    #[test]
    fn zoom_advance_moves_in_log_space() {
        let (next, hit) = advance_zoom(1.0, KB_ZOOM_T_PER_SEC, 1.0 / 60.0);
        assert!(next > 1.0);
        assert!(!hit);
    }

    #[test]
    fn zoom_advance_clamps_at_the_band_limits() {
        // A long hold saturates at the wheel's own 0.1..10 bounds and reports
        // the limit so the caller can zero the velocity.
        let max = t_to_zoom(1.0);
        let min = t_to_zoom(0.0);
        let (z, hit) = advance_zoom(max, KB_ZOOM_T_PER_SEC, 1.0);
        assert!(hit);
        assert!((z - max).abs() < 1e-6);
        let (z, hit) = advance_zoom(min, -KB_ZOOM_T_PER_SEC, 1.0);
        assert!(hit);
        assert!((z - min).abs() < 1e-6);
    }

    #[test]
    fn wheel_easing_starts_and_ends_slowly() {
        assert!(ease_in_out(0.0).abs() < 1e-6);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-6);
        // The first 10% of the curve covers much less ground than the middle
        // 10%, which is the ease-in/ease-out shape.
        let start = ease_in_out(0.1) - ease_in_out(0.0);
        let middle = ease_in_out(0.55) - ease_in_out(0.45);
        assert!(middle > start);
    }

    #[test]
    fn wheel_tween_eases_to_the_destination_without_overshoot() {
        let dt = 1.0 / 60.0;
        let start = 1.0;
        for target in [WHEEL_ZOOM_FACTOR, 1.0 / WHEEL_ZOOM_FACTOR] {
            let duration = wheel_duration(start, target);
            assert!(duration >= WHEEL_EASE_MIN_SECS);

            let increasing = target > start;
            let mut elapsed = 0.0;
            let mut previous = start;
            let mut arrived = false;
            for _ in 0..120 {
                elapsed += dt;
                let (zoom, done) = wheel_zoom_at(start, target, elapsed, duration);
                if increasing {
                    assert!(zoom >= previous - 1e-6 && zoom <= target + 1e-5);
                } else {
                    assert!(zoom <= previous + 1e-6 && zoom >= target - 1e-5);
                }
                previous = zoom;
                if done {
                    arrived = true;
                    break;
                }
            }
            assert!(arrived, "wheel tween did not settle at {target}");
            assert!((previous - target).abs() < 1e-4);
        }
    }

    #[test]
    fn wheel_duration_grows_with_distance_and_is_bounded() {
        let one = wheel_duration(1.0, WHEEL_ZOOM_FACTOR);
        let many = wheel_duration(1.0, 2.0);
        assert!(one >= WHEEL_EASE_MIN_SECS);
        assert!(many > one);
        assert!(many <= WHEEL_EASE_MAX_SECS);
    }

    #[test]
    fn wheel_target_accumulates_multiplicatively() {
        // Every notch is the same relative step, whatever the current zoom.
        let once = wheel_zoom_target(2.0, 1.0);
        assert!((once - 2.0 * WHEEL_ZOOM_FACTOR).abs() < 1e-5);
        let twice = wheel_zoom_target(once, 1.0);
        assert!((twice - 2.0 * WHEEL_ZOOM_FACTOR * WHEEL_ZOOM_FACTOR).abs() < 1e-5);
        assert!((wheel_zoom_target(4.0, 1.0) / 4.0 - WHEEL_ZOOM_FACTOR).abs() < 1e-5);
        // Zooming out by one notch undoes zooming in by one.
        assert!((wheel_zoom_target(WHEEL_ZOOM_FACTOR, -1.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn wheel_tween_blends_in_log_space() {
        // Halfway through a 1 -> 4 tween the zoom is the geometric mean (2),
        // not the arithmetic mean (2.5).
        let (mid, _) = wheel_zoom_at(1.0, 4.0, 0.5, 1.0);
        assert!((mid - 2.0).abs() < 1e-5);
    }

    #[test]
    fn frame_dt_caps_long_hitches() {
        assert_eq!(frame_dt(-1.0), 0.0);
        assert_eq!(frame_dt(0.016), 0.016);
        assert_eq!(frame_dt(5.0), MAX_FRAME_DT);
    }
}
