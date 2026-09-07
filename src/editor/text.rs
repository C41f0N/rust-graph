use raylib::prelude::*;
use std::ffi::CString;
use std::sync::RwLock;

// raylib's Font owns a GPU atlas and glyph tables, so it is neither Send nor
// Sync. Every load/unload and every draw happens on the main thread (the
// window's GL context thread), which is exactly the case this shared slot
// models, so the wrapper is sound.
struct FontSlot(RwLock<Vec<Option<Font>>>);
unsafe impl Send for FontSlot {}
unsafe impl Sync for FontSlot {}

static ACTIVE_FONT: FontSlot = FontSlot(RwLock::new(Vec::new()));

// One glyph atlas per size class actually drawn in the app. A single atlas
// rasterized at one size and scaled to the rest turns out blurry/pixelated
// (the editor body is 20px but markdown headings go up to 70px). Rasterizing
// each size class natively keeps every size crisp.
pub const FONT_ATLAS_SIZES: [i32; 7] = [16, 18, 20, 25, 33, 42, 52];

// Glyph set rasterized into every atlas: printable ASCII + Latin-1 + the
// punctuation sample used in markdown/notes. Keeps each atlas narrow enough to
// stay under raylib's max texture width even at the tallest sizes.
fn charset() -> String {
    let mut s: String = (32u8..=255).filter(|&c| c != 127).map(char::from).collect();
    s.push('\u{2013}'); // – en dash
    s.push('\u{2014}'); // — em dash
    s.push('\u{2018}'); // ' left single quote
    s.push('\u{2019}'); // ' right single quote
    s.push('\u{201c}'); // " left double quote
    s.push('\u{201d}'); // " right double quote
    s.push('\u{2022}'); // • bullet
    s.push('\u{2026}'); // … ellipsis
    s
}

// raylib default texture filtering is point/nearest, which makes any scaled
// glyph look blocky. Force bilinear so the 52px atlas upscaled to 70px
// headings (or zoomed node labels) stays smooth instead of pixelated.
fn set_bilinear_filter(font: &Font) {
    // SAFETY: font is a loaded Font whose atlas texture is a valid GPU texture.
    unsafe {
        let tex: &raylib::ffi::Texture2D = font.as_ref();
        raylib::ffi::SetTextureFilter(*tex, 1); // TEXTURE_FILTER_BILINEAR
    }
}

// Load every size class of a font file from disk. A None entry means that size
// failed to load and will fall back to raylib's built-in font.
pub fn load_font_set(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    path: &str,
) -> Vec<Option<Font>> {
    let cs = charset();
    FONT_ATLAS_SIZES
        .iter()
        .map(|&size| {
            let font = rl.load_font_ex(thread, path, size, Some(&cs)).ok()?;
            set_bilinear_filter(&font);
            Some(font)
        })
        .collect()
}

// Same as `load_font_set` but for raw font bytes in memory.
pub fn load_font_set_from_memory(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    file_type: &str,
    data: &[u8],
) -> Vec<Option<Font>> {
    let cs = charset();
    FONT_ATLAS_SIZES
        .iter()
        .map(|&size| {
            let font = rl
                .load_font_from_memory(thread, file_type, data, size, Some(&cs))
                .ok()?;
            set_bilinear_filter(&font);
            Some(font)
        })
        .collect()
}

// Install a full roster of per-size atlases; unloads whatever was active
// before (each Font's Drop runs UnloadFont).
pub fn set_active_font(fonts: Vec<Option<Font>>) {
    let mut slot = ACTIVE_FONT.0.write().unwrap();
    *slot = fonts;
}

// Drop all custom atlases and fall back to raylib's built-in font.
pub fn clear_active_font() {
    let mut slot = ACTIVE_FONT.0.write().unwrap();
    *slot = Vec::new();
}

fn nearest_size_index(size: i32) -> usize {
    FONT_ATLAS_SIZES
        .iter()
        .enumerate()
        .min_by_key(|(_, s)| (*s - size).abs())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// Measure a string as it will actually be drawn: through the closest custom
// atlas it one is loaded, otherwise through raylib's default font. The draw
// handle is unused (both measuring paths are font-global) but kept in the
// signature so call sites mirror `draw`.
pub fn measure<D>(_d: &D, text: &str, font_size: i32) -> i32 {
    let slot = ACTIVE_FONT.0.read().unwrap();
    match slot.get(nearest_size_index(font_size)).and_then(|f| f.as_ref()) {
        Some(font) => font.measure_text(text, font_size as f32, 0.0).x as i32,
        None => {
            let c_text = CString::new(text).unwrap();
            unsafe { raylib::ffi::MeasureText(c_text.as_ptr(), font_size) }
        }
    }
}

// Draw text with the closest custom atlas (or the default font when none set).
pub fn draw(
    d: &mut impl RaylibDraw,
    text: &str,
    x: i32,
    y: i32,
    font_size: i32,
    color: impl Into<ffi::Color>,
) {
    let slot = ACTIVE_FONT.0.read().unwrap();
    match slot.get(nearest_size_index(font_size)).and_then(|f| f.as_ref()) {
        Some(font) => d.draw_text_ex(
            font,
            text,
            Vector2::new(x as f32, y as f32),
            font_size as f32,
            0.0,
            color,
        ),
        None => d.draw_text(text, x, y, font_size, color),
    }
}