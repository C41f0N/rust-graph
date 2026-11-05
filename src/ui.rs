// Shared visual language for every screen surface.
//
// The graph menus, editor chrome, settings dialog, palettes and helper bar all
// paint from this one palette so a new panel cannot drift into a different
// shade or border. Geometry stays in the feature modules; these are the ink
// and the reusable panel primitives.
use raylib::prelude::*;

// Near-black surfaces with a faint blue cast, so white text stays soft rather
// than glaring and the whole app reads as one material.
pub const PANEL: Color = Color::new(14, 16, 21, 244);
pub const PANEL_SOLID: Color = Color::new(9, 11, 15, 255);
pub const SURFACE: Color = Color::new(19, 22, 29, 250);
pub const SURFACE_RAISED: Color = Color::new(28, 32, 41, 255);
pub const SURFACE_HOVER: Color = Color::new(255, 255, 255, 14);

// Hairlines: BORDER for quiet seams, BORDER_STRONG for floating panels.
pub const BORDER: Color = Color::new(255, 255, 255, 28);
pub const BORDER_STRONG: Color = Color::new(255, 255, 255, 52);

// Type ramp.
pub const TEXT: Color = Color::new(237, 240, 246, 255);
pub const TEXT_SECONDARY: Color = Color::new(174, 181, 197, 255);
pub const TEXT_MUTED: Color = Color::new(116, 124, 144, 255);

// One accent (soft blue) for selection and affordances; green/red are reserved
// for create/destructive actions so the interface stays calm.
pub const ACCENT: Color = Color::new(104, 156, 242, 255);
pub const ACCENT_SOFT: Color = Color::new(104, 156, 242, 44);
pub const ACCENT_STRONG: Color = Color::new(104, 156, 242, 160);
pub const SUCCESS: Color = Color::new(122, 205, 158, 255);
pub const DANGER: Color = Color::new(230, 112, 112, 255);

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
