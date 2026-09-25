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

// --- Serialized SDF atlas cache --------------------------------------------
//
// The shipped app bakes every bundled font into a byte-serialized SDF atlas at
// dev time (the `--dump-font-sdf` CLI pass, run against the TTFs in
// assets/fonts/) and embeds the result, so startup and font switches never run
// a rasterizer on the user's machine. The byte format mirrors the live raylib
// Font one-for-one, so rehydration is a pure pointer rebuild and raylib's own
// DrawTextEx / measure machinery keeps working unchanged.
//
// Layout (little-endian):
//   "SDF1"                     4-byte magic
//   version                   u32 (=1)
//   glyph_count               u32
//   atlas_width, atlas_height u32
//   format                    i32   raylib PixelFormat (GRAY_ALPHA = 6)
//   bytes_per_pixel           u32   (=2 for GRAY_ALPHA)
//   pixels                    width*height*bpp  (gray channel flush, SDF byte in alpha)
//   glyphs                    glyph_count * { value i32, offsetX i32, offsetY i32, advanceX i32 }
//   rects                     glyph_count * { x f32, y f32, width f32, height f32 }
pub const SDF_CACHE_MAGIC: [u8; 4] = *b"SDF1";
pub const SDF_CACHE_VERSION: u32 = 1;

// CPU-side rasterization of one family cut: a packed SDF atlas plus every
// glyph's metrics and atlas rect. Holds no GL resources, so the bake CLI can
// produce it without a live window.
pub struct RasterizedSdf {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: i32,
    pub bpp: u32,
    pub codepoints: Vec<i32>,
    pub offsets_x: Vec<i32>,
    pub offsets_y: Vec<i32>,
    pub advances: Vec<i32>,
    pub rects: Vec<Rectangle>,
}

// Rasterize a TTF/OTF's charset into one CPU-side SDF atlas (LoadFontData +
// GenImageFontAtlas, pure stb_truetype). Returns None for unparseable bytes or
// a charset that produces no glyphs.
pub fn rasterize_sdf_atlas(data: &[u8]) -> Option<RasterizedSdf> {
    let codepoints: Vec<i32> = charset().chars().map(|c| c as i32).collect();
    let mut glyph_count: std::ffi::c_int = 0;
    // SAFETY: as in load_sdf_font: data and codepoints outlive the call, and
    // the returned glyphs array is owned and freed below.
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
            unsafe { raylib::ffi::UnloadFontData(glyphs, glyph_count) };
        }
        return None;
    }

    let mut recs: *mut raylib::ffi::Rectangle = std::ptr::null_mut();
    // SAFETY: glyphs and glyph_count are valid from LoadFontData, and GRAY_ALPHA
    // output lets the shader read the SDF byte straight from the alpha channel.
    let atlas = unsafe {
        raylib::ffi::GenImageFontAtlas(glyphs, &mut recs, glyph_count, SDF_BASE_SIZE, SDF_PAD, 0)
    };
    if atlas.data.is_null() {
        // SAFETY: the array came from LoadFontData above.
        unsafe { raylib::ffi::UnloadFontData(glyphs, glyph_count) };
        return None;
    }

    let width = atlas.width as u32;
    let height = atlas.height as u32;
    // GRAY_ALPHA packs two bytes per texel: an opaque gray flush plus the
    // distance byte the shader reads from .a.
    let bpp: u32 = 2;
    let total = (width * height * bpp) as usize;
    let mut pixels = vec![0u8; total];
    // SAFETY: atlas.data points at `total` owned GRAY_ALPHA bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(atlas.data as *const u8, pixels.as_mut_ptr(), total);
    }

    let n = glyph_count as usize;
    let mut result = RasterizedSdf {
        pixels,
        width,
        height,
        format: atlas.format,
        bpp,
        codepoints: Vec::with_capacity(n),
        offsets_x: Vec::with_capacity(n),
        offsets_y: Vec::with_capacity(n),
        advances: Vec::with_capacity(n),
        rects: Vec::with_capacity(n),
    };
    for i in 0..n {
        // SAFETY: glyphs holds glyph_count live GlyphInfos; recs the same count
        // of live Rectangles from GenImageFontAtlas.
        let g = unsafe { *glyphs.add(i) };
        result.codepoints.push(g.value);
        result.offsets_x.push(g.offsetX);
        result.offsets_y.push(g.offsetY);
        result.advances.push(g.advanceX);
        let r = unsafe { *recs.add(i) };
        result.rects.push(r);
    }

    // SAFETY: glyphs (with its per-glyph images), recs and the atlas image are
    // all raylib-owned CPU allocations; free them now their bytes are copied.
    unsafe {
        raylib::ffi::UnloadFontData(glyphs, glyph_count);
        raylib::ffi::MemFree(recs as *mut std::ffi::c_void);
        raylib::ffi::UnloadImage(atlas);
    }
    Some(result)
}

// Flatten one family cut into the byte-cache format. Glyph metrics are i32 and
// rects f32, matching raylib's Font layout.
pub fn serialize_sdf_font_cache(sdf: &RasterizedSdf) -> Vec<u8> {
    let n = sdf.codepoints.len();
    let mut out = Vec::with_capacity(28 + sdf.pixels.len() + n * 16 * 2);
    out.extend_from_slice(&SDF_CACHE_MAGIC);
    out.extend_from_slice(&SDF_CACHE_VERSION.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&sdf.width.to_le_bytes());
    out.extend_from_slice(&sdf.height.to_le_bytes());
    out.extend_from_slice(&sdf.format.to_le_bytes());
    out.extend_from_slice(&sdf.bpp.to_le_bytes());
    out.extend_from_slice(&sdf.pixels);
    for i in 0..n {
        out.extend_from_slice(&sdf.codepoints[i].to_le_bytes());
        out.extend_from_slice(&sdf.offsets_x[i].to_le_bytes());
        out.extend_from_slice(&sdf.offsets_y[i].to_le_bytes());
        out.extend_from_slice(&sdf.advances[i].to_le_bytes());
    }
    for r in &sdf.rects {
        out.extend_from_slice(&r.x.to_le_bytes());
        out.extend_from_slice(&r.y.to_le_bytes());
        out.extend_from_slice(&r.width.to_le_bytes());
        out.extend_from_slice(&r.height.to_le_bytes());
    }
    out
}

fn take(cache: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    cache.get(off..off + len)
}

fn u32_at(cache: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(take(cache, off, 4)?.try_into().ok()?))
}

fn i32_at(cache: &[u8], off: usize) -> Option<i32> {
    Some(i32::from_le_bytes(take(cache, off, 4)?.try_into().ok()?))
}

fn f32_at(cache: &[u8], off: usize) -> Option<f32> {
    Some(f32::from_le_bytes(take(cache, off, 4)?.try_into().ok()?))
}

// Headless validity check for the serialized format: magic, version and a
// non-empty glyph table. Used by tests once the embedded catalog exists.
#[cfg(test)]
pub fn sdf_cache_ok(cache: &[u8]) -> bool {
    if cache.len() < 28 || take(cache, 0, 4) != Some(&SDF_CACHE_MAGIC) {
        return false;
    }
    match u32_at(cache, 4) {
        Some(SDF_CACHE_VERSION) => {}
        _ => return false,
    }
    u32_at(cache, 8).map(|n| n > 0).unwrap_or(false)
}

// Restore a serialized cache into a live SDF field-font. Glyphs and rect arrays
// come from raylib's own allocator so SdfFont's UnloadFont (RL_FREE on both)
// matches; the atlas re-uploads as a texture exactly like load_sdf_font does.
// Returns None for malformed or version-mismatched bytes.
pub fn load_sdf_font_from_bytes(cache: &[u8]) -> Option<SdfFont> {
    let glyph_count = u32_at(cache, 8)? as usize;
    let width = u32_at(cache, 12)? as usize;
    let height = u32_at(cache, 16)? as usize;
    let format = i32_at(cache, 20)?;
    let bpp = u32_at(cache, 24)? as usize;
    if glyph_count == 0 || width == 0 || height == 0 || bpp == 0 {
        return None;
    }

    let pixel_bytes = width * height * bpp;
    let glyph_block = glyph_count * 16;
    let pixels = take(cache, 28, pixel_bytes)?;
    let rect_off = 28 + pixel_bytes + glyph_block;
    if take(cache, rect_off, glyph_count * 16).is_none() {
        return None;
    }

    // SAFETY: everything allocated with raylib's allocator below is freed by
    // SdfFont's UnloadFont with matching RL_FREE. Zero-fill first so the
    // per-glyph `image` fields are null (UnloadImage(NULL) is a no-op).
    unsafe {
        let glyphs = raylib::ffi::MemAlloc(
            (std::mem::size_of::<raylib::ffi::GlyphInfo>() * glyph_count) as u32,
        ) as *mut raylib::ffi::GlyphInfo;
        let rects = raylib::ffi::MemAlloc(
            (std::mem::size_of::<raylib::ffi::Rectangle>() * glyph_count) as u32,
        ) as *mut raylib::ffi::Rectangle;
        std::ptr::write_bytes(glyphs, 0, glyph_count);
        std::ptr::write_bytes(rects, 0, glyph_count);
        let mut glyph_off = 28 + pixel_bytes;
        let mut rec_off = rect_off;
        for i in 0..glyph_count {
            let g = glyphs.add(i);
            (*g).value = i32_at(cache, glyph_off)?;
            (*g).offsetX = i32_at(cache, glyph_off + 4)?;
            (*g).offsetY = i32_at(cache, glyph_off + 8)?;
            (*g).advanceX = i32_at(cache, glyph_off + 12)?;
            glyph_off += 16;
            let r = rects.add(i);
            *r = raylib::ffi::Rectangle {
                x: f32_at(cache, rec_off)?,
                y: f32_at(cache, rec_off + 4)?,
                width: f32_at(cache, rec_off + 8)?,
                height: f32_at(cache, rec_off + 12)?,
            };
            rec_off += 16;
        }

        // SAFETY: pixels is a live slice; LoadTextureFromImage copies it into
        // VRAM before we return, so no borrow escapes the call.
        let image = raylib::ffi::Image {
            data: pixels.as_ptr() as *mut std::ffi::c_void,
            width: width as i32,
            height: height as i32,
            mipmaps: 1,
            format,
        };
        let tex = raylib::ffi::LoadTextureFromImage(image);
        // SAFETY: tex was just created; bilinear interpolates the distance
        // field when a glyph scales, matching load_sdf_font.
        raylib::ffi::SetTextureFilter(
            tex,
            raylib::ffi::TextureFilter::TEXTURE_FILTER_BILINEAR as std::ffi::c_int,
        );
        let font = raylib::ffi::Font {
            baseSize: SDF_BASE_SIZE,
            glyphCount: glyph_count as std::ffi::c_int,
            glyphPadding: SDF_PAD,
            texture: tex,
            recs: rects as *mut raylib::ffi::Rectangle,
            glyphs: glyphs as *mut raylib::ffi::GlyphInfo,
        };
        Some(SdfFont { font })
    }
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
    fn empty_font_bytes_rasterize_to_nothing() {
        // Invalid font bytes bail out before any GL call, so this runs headless.
        assert!(rasterize_sdf_atlas(b"\0\0\0").is_none());
    }

    #[test]
    fn garbage_bytes_rasterize_and_cache_load_to_nothing() {
        assert!(rasterize_sdf_atlas(b"\0\0\0").is_none());
        assert!(!sdf_cache_ok(b"garbage"));
        assert!(sdf_cache_ok(&serialize_sdf_font_cache(&RasterizedSdf {
            pixels: vec![0u8; 2],
            width: 1,
            height: 1,
            format: 6,
            bpp: 2,
            codepoints: vec![65],
            offsets_x: vec![0],
            offsets_y: vec![1],
            advances: vec![2],
            rects: vec![Rectangle::new(0.0, 0.0, 1.0, 1.0)],
        })));
    }
}