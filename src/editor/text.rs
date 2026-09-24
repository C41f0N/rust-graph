use crate::editor::markdown::{Segment, SegmentStyle};
use raylib::core::drawing::RaylibShaderModeExt;
use raylib::core::shaders::Shader;
use raylib::core::text::RaylibFont;
use raylib::prelude::*;
use std::cell::RefCell;

// --- SDF field-text atlas --------------------------------------------------
//
// Text renders from a single-channel signed-distance-field atlas (raylib's own
// FONT_SDF rasterizer via stb_truetype). Because the field is a distance map,
// scaling the glyph quads to any em size interpolates smoothly instead of
// blurring, so the editor body, 70px headings and continuously-zoomed node
// labels all stay crisp from ONE atlas. Raylib packs the fields into a
// GRAY+ALPHA atlas (alpha channel carries the distance byte, gray is opaque)
// and its own DrawTextEx places/advances the glyphs, so this pipeline reuses
// raylib's battle-tested font machinery end to end.
//
// The font construction (rasterize + pack + upload) has no safe entry point in
// the stock raylib-rs 6.0 API: `Font::from_data` is private and the public
// `load_font_data` only surfaces the first glyph. Everything downstream of
// that (shader, shader mode, glph placement, measure) goes through the safe
// raylib API. Play the other way around and all the unsafe lives in this one
// construction block.

// Rasterization size of the distance fields. Bigger = more texels per glyph =
// smoother curves at large render sizes, at a one-time bake cost per family.
const SDF_BASE_SIZE: i32 = 64;
// Distance-field padding baked around each glyph; must match the pxRange
// uniform the shader interprets as the field range in atlas texels.
const SDF_PAD: i32 = 4;

// Single-channel SDF shader (raylib's official sdf.fs layout). The signed
// distance lives in the alpha channel; coverage per pixel comes from fwidth
// so antialiasing adapts to whatever size the camera scales the quads to.
const SDF_FS: &str = "#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
uniform sampler2D texture0;
uniform float pxRange;
out vec4 finalColor;

float screenPxRange() {
    vec2 unitRange = vec2(pxRange) / vec2(textureSize(texture0, 0));
    vec2 screenTexSize = vec2(1.0) / fwidth(fragTexCoord);
    return max(0.5 * dot(unitRange, screenTexSize), 1.0);
}

void main() {
    float dist = texture(texture0, fragTexCoord).a;
    float d = screenPxRange() * (dist - 0.5);
    finalColor = vec4(fragColor.rgb, fragColor.a * clamp(d + 0.5, 0.0, 1.0));
}
";

// A loaded family cut: raylib's Font owns the atlas texture (VRAM), the packed
// glyph rectangles and the glyph metrics (RAM). raylib handles all three in
// one UnloadFont call, so a single Drop manages the whole lifetime. It is
// neither Send nor Sync (GPU + raw pointers) and stays on the GL thread; the
// thread_local slots below model exactly that.
//
// Impls the raylib-rs `RaylibFont` trait (safe measure_text etc.), which only
// requires forwarding the raw Font by reference.
pub struct SdfFont {
    font: raylib::ffi::Font,
}

impl Drop for SdfFont {
    fn drop(&mut self) {
        // SAFETY: the Font was built on this thread and every allocation it
        // owns (texture, recs, glyph data) arrives from raylib's allocator,
        // so UnloadFont is the matching free for all three.
        unsafe {
            raylib::ffi::UnloadFont(self.font);
        }
    }
}

impl AsRef<raylib::ffi::Font> for SdfFont {
    fn as_ref(&self) -> &raylib::ffi::Font {
        &self.font
    }
}

impl raylib::core::AsRawMut<raylib::ffi::Font> for SdfFont {
    // SAFETY: SdfFont is the sole owner of `font`; the trait's caller (the
    // raylib-rs font extension methods) only reads metrics through it.
    unsafe fn as_raw_mut(&mut self) -> &mut raylib::ffi::Font {
        &mut self.font
    }
}

impl RaylibFont for SdfFont {}

// A family can contribute three field-fonts (upright, bold, italic) so markdown
// emphasis draws with real glyph shapes instead of faking the style. A missing
// style falls back to the upright font.
struct FontSet {
    regular: Option<SdfFont>,
    bold: Option<SdfFont>,
    italic: Option<SdfFont>,
}

impl Default for FontSet {
    fn default() -> Self {
        FontSet {
            regular: None,
            bold: None,
            italic: None,
        }
    }
}

// Everything below is main-thread-only GL state. The raylib types are not
// Send/Sync, so the slots are per-thread (thread_local!) instead of statics
// with hand-written Send/Sync impls.
thread_local! {
    static ACTIVE_FONT: RefCell<FontSet> = RefCell::new(FontSet::default());
    // raylib's built-in font, captured once at startup by `init_text`.
    static DEFAULT_FONT: RefCell<Option<WeakFont>> = RefCell::new(None);
    // The single SDF shader program, compiled once at startup by `init_text`.
    static SDF_SHADER: RefCell<Option<Shader>> = RefCell::new(None);
}

// One-time GL setup: raylib's built-in font for subpixel default drawing, and
// the compiled SDF shader with its pxRange uniform. Must run after window
// init, before any text draw.
pub fn init_text(rl: &mut RaylibHandle, thread: &RaylibThread) {
    let mut shader = rl.load_shader_from_memory(thread, None, Some(SDF_FS));
    let range_loc = shader.get_shader_location("pxRange");
    if range_loc >= 0 {
        shader.set_shader_value(range_loc, SDF_PAD as f32);
    }
    SDF_SHADER.with(|slot| *slot.borrow_mut() = Some(shader));
    DEFAULT_FONT.with(|slot| *slot.borrow_mut() = Some(rl.get_font_default()));
}

// Glyph set rasterized into every atlas: printable ASCII + Latin-1 + the
// punctuation sample used in markdown/notes.
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

// Rasterize every charset glyph of a font into one SDF field-texture.
// Returns None when the bytes are not a parseable font or produce no glyphs.
//
// This is the single unsafe construction site in the module: the SDF building
// blocks (LoadFontData, GenImageFontAtlas, the texture upload and the freed
// recs/glyph table) have no safe counterpart in the stock raylib-rs 6.0 API.
// Everything downstream is safe raylib-rs.
pub fn load_sdf_font(data: &[u8]) -> Option<SdfFont> {
    let codepoints: Vec<i32> = charset().chars().map(|c| c as i32).collect();
    let mut glyph_count: std::ffi::c_int = 0;
    // SAFETY: data is a live byte slice and codepoints a live array; raylib
    // copies both while rasterizing, and the returned array is passed to
    // GenImageFontAtlas/UnloadFont which own it from here on.
    let glyphs = unsafe {
        raylib::ffi::LoadFontData(
            data.as_ptr(),
            data.len() as std::ffi::c_int,
            SDF_BASE_SIZE,
            codepoints.as_ptr(),
            codepoints.len() as std::ffi::c_int,
            raylib::ffi::FontType::FONT_SDF as std::ffi::c_int,
            &mut glyph_count,
        )
    };
    if glyphs.is_null() || glyph_count <= 0 {
        if !glyphs.is_null() {
            // SAFETY: the array came from LoadFontData above.
            unsafe {
                raylib::ffi::UnloadFontData(glyphs, glyph_count);
            }
        }
        return None;
    }

    // Pack the per-glyph fields into one atlas image; recs (glyph rects in the
    // atlas) are raylib-allocated and freed together with everything else.
    let mut recs: *mut raylib::ffi::Rectangle = std::ptr::null_mut();
    // SAFETY: glyphs/glyphCount are valid from LoadFontData for the call.
    let atlas = unsafe {
        raylib::ffi::GenImageFontAtlas(
            glyphs,
            &mut recs,
            glyph_count,
            SDF_BASE_SIZE,
            SDF_PAD,
            0,
        )
    };
    if atlas.data.is_null() {
        // SAFETY: the array came from LoadFontData above.
        unsafe {
            raylib::ffi::UnloadFontData(glyphs, glyph_count);
        }
        return None;
    }

    // SAFETY: atlas is a valid raylib Image (GRAY_ALPHA) that stays untouched
    // for the call; LoadTextureFromImage copies it into VRAM.
    let tex = unsafe { raylib::ffi::LoadTextureFromImage(atlas) };
    // SAFETY: atlas is a CPU image we own; its pixels are already uploaded.
    unsafe {
        raylib::ffi::UnloadImage(atlas);
    }
    // SAFETY: tex was just created; bilinear (instead of nearest) lets the
    // distance field interpolate smoothly when a glyph is scaled in size.
    unsafe {
        raylib::ffi::SetTextureFilter(tex, raylib::ffi::TextureFilter::TEXTURE_FILTER_BILINEAR as std::ffi::c_int);
    }

    let font = raylib::ffi::Font {
        baseSize: SDF_BASE_SIZE,
        glyphCount: glyph_count,
        glyphPadding: SDF_PAD,
        texture: tex,
        recs,
        glyphs,
    };
    Some(SdfFont { font })
}

// Everything the editor needs to draw one family: the upright SDF font plus
// the bold/italic siblings the family ships (empty when a style has no file).
pub struct LoadedFontSet {
    pub regular: Option<SdfFont>,
    pub bold: Option<SdfFont>,
    pub italic: Option<SdfFont>,
}

// Install the family's field-fonts; unloads whatever was active before (each
// SdfFont's Drop runs UnloadFont).
pub fn set_active_fonts(set: LoadedFontSet) {
    ACTIVE_FONT.with(|slot| {
        *slot.borrow_mut() = FontSet {
            regular: set.regular,
            bold: set.bold,
            italic: set.italic,
        };
    });
}

// Drop all custom fonts and fall back to raylib's built-in font.
pub fn clear_active_font() {
    ACTIVE_FONT.with(|slot| *slot.borrow_mut() = FontSet::default());
}

// Field-font for a segment style; anything that is not emphasized draws
// upright. A missing style slot falls back to the upright font so measure and
// draw always agree with one another.
fn sdf_font(set: &FontSet, style: SegmentStyle) -> Option<&SdfFont> {
    let candidate = match style {
        SegmentStyle::Bold => &set.bold,
        SegmentStyle::Italic => &set.italic,
        _ => &set.regular,
    };
    candidate.as_ref().or_else(|| set.regular.as_ref())
}

// Measure a string as it will actually be drawn: through the active SDF font
// if one is loaded, otherwise through raylib's default font. The draw handle
// is unused (both measuring paths are font-global) but kept in the signature
// so call sites mirror `draw`.
pub fn measure<D>(_d: &D, text: &str, font_size: i32) -> i32 {
    measure_f(_d, text, font_size) as i32
}

// Style-aware variant: bold text measures through the bold font (glyphs run
// wider) so the wrap, the cursor and the selection stay glued to the drawn
// glyphs. Unknown/missing styles fall back to the upright font.
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
    ACTIVE_FONT.with(|active| {
        let set = active.borrow();
        match sdf_font(&set, style) {
            // Matches DrawTextEx glyph-for-glyph via raylib's own measure.
            Some(font) => font.measure_text(text, font_size as f32, 0.0).x,
            None => DEFAULT_FONT.with(|def| {
                let df = def.borrow();
                match df.as_ref() {
                    Some(font) => font.measure_text(text, font_size as f32, 0.0).x,
                    // init_text always runs before any draw, so this is dead.
                    None => 0.0,
                }
            }),
        }
    })
}

// Draw a run through the SDF font under the SDF shader. raylib's DrawTextEx
// places every glyph (advance, offset, per-size scaling) from the Font's own
// metrics, so nothing here touches atlas geometry. The RAII guard ends the
// shader mode when the scope closes.
fn draw_sdf_all(
    d: &mut impl RaylibDraw,
    font: &SdfFont,
    text: &str,
    x: f32,
    y: f32,
    font_size: i32,
    tint: raylib::ffi::Color,
) {
    SDF_SHADER.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(shader) = borrowed.as_mut() else {
            return;
        };
        // SAFETY of the mode itself lives in raylib-rs: the guard calls
        // EndShaderMode on drop, so draws inside stay shader-scoped.
        let mut shader_mode = d.begin_shader_mode(shader);
        shader_mode.draw_text_ex(
            font,
            text,
            Vector2::new(x, y),
            font_size as f32,
            0.0,
            tint,
        );
    });
}

// Draw text with the active SDF font (or the default font when none set).
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

// Style-aware draw: bold/italic segments go through their style font.
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
    let tint = color.into();
    ACTIVE_FONT.with(|active| {
        let set = active.borrow();
        if let Some(font) = sdf_font(&set, style) {
            draw_sdf_all(d, font, text, x, y, font_size, tint);
        } else {
            // No custom font: draw with the captured built-in font at subpixel
            // positions rather than raylib's integer draw_text, which would snap
            // the label to the pixel grid and strobe against moving nodes.
            DEFAULT_FONT.with(|def| {
                let df = def.borrow();
                match df.as_ref() {
                    Some(font) => d.draw_text_ex(
                        font,
                        text,
                        Vector2::new(x, y),
                        font_size as f32,
                        0.0,
                        tint,
                    ),
                    None => d.draw_text(text, x.round() as i32, y.round() as i32, font_size, tint),
                }
            });
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_font_bytes_load_to_nothing() {
        // Invalid font bytes bail out before any GL call, so this runs headless.
        assert!(load_sdf_font(b"\0\0\0").is_none());
    }
}