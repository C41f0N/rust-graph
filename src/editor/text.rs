use crate::editor::markdown::{Segment, SegmentStyle};
use raylib::prelude::*;
use std::ffi::CString;
use std::sync::RwLock;

// raylib's Font owns a GPU atlas and glyph tables, so it is neither Send nor
// Sync. Every load/unload and every draw happens on the main thread (the
// window's GL context thread), which is exactly the case this shared slot
// models, so the wrapper is sound.
//
// A family can contribute three rosters of per-size atlases (upright, bold,
// italic) so markdown emphasis draws with real glyphs instead of faking the
// style. A missing style falls back to the upright roster.
struct FontSet {
    regular: Vec<Option<Font>>,
    bold: Vec<Option<Font>>,
    italic: Vec<Option<Font>>,
}

impl Default for FontSet {
    fn default() -> Self {
        FontSet {
            regular: Vec::new(),
            bold: Vec::new(),
            italic: Vec::new(),
        }
    }
}

struct FontSlot(RwLock<FontSet>);
unsafe impl Send for FontSlot {}
unsafe impl Sync for FontSlot {}

static ACTIVE_FONT: FontSlot = FontSlot(RwLock::new(FontSet {
    regular: Vec::new(),
    bold: Vec::new(),
    italic: Vec::new(),
}));

// raylib's built-in font, captured once at startup. Like the atlases above it
// is not Send/Sync (it references the GL context), modelled the same way.
struct DefaultFontSlot(RwLock<Option<WeakFont>>);
unsafe impl Send for DefaultFontSlot {}
unsafe impl Sync for DefaultFontSlot {}

static DEFAULT_FONT: DefaultFontSlot = DefaultFontSlot(RwLock::new(None));

// Record the built-in font (from rl.get_font_default()) so text can be drawn
// at subpixel positions even when no custom atlas is active.
pub fn capture_default_font(font: WeakFont) {
    *DEFAULT_FONT.0.write().unwrap() = Some(font);
}

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

// Everything the editor needs to draw one family: the upright roster plus the
// bold/italic siblings the family ships (empty when a style has no file).
pub struct LoadedFontSet {
    pub regular: Vec<Option<Font>>,
    pub bold: Vec<Option<Font>>,
    pub italic: Vec<Option<Font>>,
}

// Install the full roster of style atlases; unloads whatever was active before
// (each Font's Drop runs UnloadFont).
pub fn set_active_fonts(set: LoadedFontSet) {
    let mut slot = ACTIVE_FONT.0.write().unwrap();
    *slot = FontSet {
        regular: set.regular,
        bold: set.bold,
        italic: set.italic,
    };
}

// Drop all custom atlases and fall back to raylib's built-in font.
pub fn clear_active_font() {
    let mut slot = ACTIVE_FONT.0.write().unwrap();
    *slot = FontSet::default();
}

fn nearest_size_index(size: i32) -> usize {
    FONT_ATLAS_SIZES
        .iter()
        .enumerate()
        .min_by_key(|(_, s)| (*s - size).abs())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// Roster for a segment style; anything that is not emphasized draws upright.
// A missing style slot at the requested size falls back to the upright atlas
// so measure and draw always agree with one another.
fn atlas<'a>(set: &'a FontSet, style: SegmentStyle, index: usize) -> Option<&'a Font> {
    let roster = match style {
        SegmentStyle::Bold => &set.bold,
        SegmentStyle::Italic => &set.italic,
        _ => &set.regular,
    };
    roster
        .get(index)
        .and_then(|f| f.as_ref())
        .or_else(|| set.regular.get(index).and_then(|f| f.as_ref()))
}

// Measure a string as it will actually be drawn: through the closest custom
// atlas if one is loaded, otherwise through raylib's default font. The draw
// handle is unused (both measuring paths are font-global) but kept in the
// signature so call sites mirror `draw`.
pub fn measure<D>(_d: &D, text: &str, font_size: i32) -> i32 {
    measure_f(_d, text, font_size) as i32
}

// Style-aware variant: bold text measures through the bold atlas (glyphs run
// wider) so the wrap, the cursor and the selection stay glued to the drawn
// glyphs. Unknown/missing styles fall back to the upright atlas.
pub fn measure_styled<D>(_d: &D, text: &str, font_size: i32, style: SegmentStyle) -> i32 {
    measure_f_styled(_d, text, font_size, style) as i32
}

// Summed width of a run of styled segments, matching what draw_styled paints.
pub fn segments_measure<D>(_d: &D, segments: &[Segment], font_size: i32) -> i32 {
    segments
        .iter()
        .map(|s| measure_styled(_d, &s.text, font_size, s.style))
        .sum()
}

// f32-precise measure for call sites that position via fractional pixels (a
// node label centred on a moving node must not round before it even draws).
pub fn measure_f<D>(_d: &D, text: &str, font_size: i32) -> f32 {
    measure_f_styled(_d, text, font_size, SegmentStyle::Plain)
}

fn measure_f_styled<D>(_d: &D, text: &str, font_size: i32, style: SegmentStyle) -> f32 {
    let slot = ACTIVE_FONT.0.read().unwrap();
    match atlas(&slot, style, nearest_size_index(font_size)) {
        Some(font) => font.measure_text(text, font_size as f32, 0.0).x,
        None => match DEFAULT_FONT.0.read().unwrap().as_ref() {
            Some(font) => {
                let c_text = CString::new(text).unwrap();
                // SAFETY: `font` is raylib's built-in font, valid for the
                // program's whole lifetime.
                unsafe {
                    raylib::ffi::MeasureTextEx(*font.as_ref(), c_text.as_ptr(), font_size as f32, 0.0)
                        .x
                }
            }
            None => {
                let c_text = CString::new(text).unwrap();
                unsafe { raylib::ffi::MeasureText(c_text.as_ptr(), font_size) as f32 }
            }
        },
    }
}

// Draw text with the closest custom atlas (or the default font when none set).
// `x`/`y` are i32 for screen-space UI.
pub fn draw(
    d: &mut impl RaylibDraw,
    text: &str,
    x: i32,
    y: i32,
    font_size: i32,
    color: impl Into<ffi::Color>,
) {
    draw_f(d, text, x as f32, y as f32, font_size, color);
}

// Style-aware draw: bold/italic segments go through their style atlas.
pub fn draw_styled(
    d: &mut impl RaylibDraw,
    text: &str,
    x: i32,
    y: i32,
    font_size: i32,
    color: impl Into<ffi::Color>,
    style: SegmentStyle,
) {
    draw_f_styled(d, text, x as f32, y as f32, font_size, color, style);
}

fn draw_f_styled(
    d: &mut impl RaylibDraw,
    text: &str,
    x: f32,
    y: f32,
    font_size: i32,
    color: impl Into<ffi::Color>,
    style: SegmentStyle,
) {
    let slot = ACTIVE_FONT.0.read().unwrap();
    match atlas(&slot, style, nearest_size_index(font_size)) {
        Some(font) => d.draw_text_ex(
            font,
            text,
            Vector2::new(x, y),
            font_size as f32,
            0.0,
            color,
        ),
        // No custom atlas: draw with the captured built-in font at subpixel
        // positions rather than raylib's integer draw_text, which would snap
        // the label to the pixel grid and strobe against moving nodes.
        None => match DEFAULT_FONT.0.read().unwrap().as_ref() {
            Some(font) => d.draw_text_ex(font, text, Vector2::new(x, y), font_size as f32, 0.0, color),
            None => d.draw_text(text, x.round() as i32, y.round() as i32, font_size, color),
        },
    }
}

// Draw text at subpixel positions without snapping. Used for node labels so
// they track the node's f32 animation smoothly instead of strobing on the
// pixel grid as the camera pans/zooms.
pub fn draw_f(
    d: &mut impl RaylibDraw,
    text: &str,
    x: f32,
    y: f32,
    font_size: i32,
    color: impl Into<ffi::Color>,
) {
    draw_f_styled(d, text, x, y, font_size, color, SegmentStyle::Plain);
}