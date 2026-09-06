use raylib::prelude::*;

pub const HEIGHT: i32 = 1080;
pub const WIDTH: i32 = 1920;

pub const EDITOR_FONT_SIZE: i32 = 20;
pub const EDITOR_FONT_SIZE_H1: i32 = 70;
pub const EDITOR_PADDING: i32 = 12;
pub const EDITOR_LINE_SPACING: i32 = 2;
pub const EDITOR_CURSOR_HEIGHT_RATIO: f32 = 0.8;

pub const EDITOR_FONT_COLOR: Color = Color::WHITE;
pub const EDITOR_SELECTION_COLOR: Color = Color::new(76, 128, 204, 102);

pub const AUTOCOMPLETE_MAX_VISIBLE: usize = 8;
pub const AUTOCOMPLETE_ITEM_HEIGHT: i32 = 18;
pub const AUTOCOMPLETE_BG: Color = Color::new(20, 20, 20, 235);
pub const AUTOCOMPLETE_SELECTED_BG: Color = Color::new(76, 128, 204, 160);
