// Shared visual language for every screen surface.
//
// The graph menus, editor chrome, settings dialog, palettes and helper bar all
// paint from this one palette so a new panel cannot drift into a different
// shade or border. Geometry stays in the feature modules; these are the ink
// and the reusable panel primitives.
//
// The theme is strictly monochrome: black and white with a ramp of neutral
// grays. No hue is introduced anywhere, so the graph and the chrome read as
// one material.
use raylib::prelude::*;

// Near-black neutral surfaces.
pub const PANEL: Color = Color::new(16, 16, 16, 244);
pub const PANEL_SOLID: Color = Color::new(10, 10, 10, 255);
pub const SURFACE: Color = Color::new(22, 22, 22, 250);
pub const SURFACE_RAISED: Color = Color::new(34, 34, 34, 255);
pub const SURFACE_HOVER: Color = Color::new(255, 255, 255, 14);

// Hairlines: BORDER for quiet seams, BORDER_STRONG for floating panels.
pub const BORDER: Color = Color::new(255, 255, 255, 28);
pub const BORDER_STRONG: Color = Color::new(255, 255, 255, 52);

// Type ramp.
pub const TEXT: Color = Color::new(240, 240, 240, 255);
pub const TEXT_SECONDARY: Color = Color::new(180, 180, 180, 255);
pub const TEXT_MUTED: Color = Color::new(126, 126, 126, 255);

// The accent is white: solid for marks and labels, translucent for pills and
// soft edges. Success/danger are expressed by brightness, not by hue.
pub const ACCENT: Color = Color::new(240, 240, 240, 255);
pub const ACCENT_SOFT: Color = Color::new(255, 255, 255, 24);
pub const ACCENT_STRONG: Color = Color::new(255, 255, 255, 40);
pub const SUCCESS: Color = Color::new(220, 220, 220, 255);
pub const DANGER: Color = Color::new(245, 245, 245, 255);

// A floating panel: rounded near-black fill with a one-pixel hairline.
pub fn panel(d: &mut impl RaylibDraw, rect: Rectangle, radius: f32) {
    d.draw_rectangle_rounded(rect, radius, 10, PANEL);
    d.draw_rectangle_rounded_lines_ex(rect, radius, 10, 1.0, BORDER_STRONG);
}

// A panel with an explicit fill, for raised bars that sit inside a panel.
pub fn surface(d: &mut impl RaylibDraw, rect: Rectangle, radius: f32, fill: Color) {
    d.draw_rectangle_rounded(rect, radius, 8, fill);
}

pub fn border(d: &mut impl RaylibDraw, rect: Rectangle, radius: f32, color: Color) {
    d.draw_rectangle_rounded_lines_ex(rect, radius, 10, 1.0, color);
}

// A padded selection/hover pill, drawn inset from a full row so consecutive
// rows stay visually separate.
pub fn row_pill(d: &mut impl RaylibDraw, rect: Rectangle, color: Color) {
    d.draw_rectangle_rounded(rect, 0.28, 6, color);
}

// A keycap used by the helper bar and the action menus.
pub fn keycap(d: &mut impl RaylibDraw, rect: Rectangle, fill: Color) {
    d.draw_rectangle_rounded(rect, 0.32, 5, fill);
    d.draw_rectangle_rounded_lines_ex(rect, 0.32, 5, 1.0, BORDER);
}
