use raylib::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

// raylib's Texture2D owns a GPU texture, so it is neither Send nor Sync.
// Every load/unload and every draw happens on the main thread (the window's
// GL context thread), which is exactly the case this cache models, so the
// wrapper is sound. Each Texture2D's Drop runs UnloadTexture, so a removed
// cache entry frees its GPU memory.
struct Slot(Option<Texture2D>);
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

// Full-resolution textures, decoded from the original file. Used by the
// editor, which draws whole [[file]] lines as their rectangular image. Only
// the files the caller asks for each frame end up here.
static CACHE: OnceLock<RwLock<HashMap<PathBuf, Slot>>> = OnceLock::new();

// Small circular thumbnails (persisted next to the asset as
// <dir>/thumbnails/<name>.thumb.png) used by the graph for node header
// images. Keyed by the original asset path.
static THUMB_CACHE: OnceLock<RwLock<HashMap<PathBuf, Slot>>> = OnceLock::new();

fn cache() -> &'static RwLock<HashMap<PathBuf, Slot>> {
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn thumb_cache() -> &'static RwLock<HashMap<PathBuf, Slot>> {
    THUMB_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

// File extensions raylib can decode into a texture. A link whose target has
// one of these is treated as an inline image. The raylib build does not
// support TIFF, so tif/tiff are intentionally absent.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "tga", "ico"];

// Side length, in pixels, of the square thumbnail persisted for node header
// images. The disc a header is drawn into is at most ~1.9 * max node radius
// (16) * max zoom (10) ~= 304 px, so 256 keeps it crisp while staying tiny to
// decode, mask and upload compared to a full-resolution source.
pub const THUMB_SIZE: u32 = 256;

const THUMB_DIR: &str = "thumbnails";

pub fn is_image_target(target: &str) -> bool {
    let t = target.trim();
    match t.rfind('.') {
        Some(i) => {
            let ext = &t[i + 1..].to_lowercase();
            IMAGE_EXTENSIONS.iter().any(|e| *e == ext)
        }
        None => false,
    }
}

// Resolve a wikilink target relative to the notes directory.
pub fn resolve_path(target: &str) -> Option<PathBuf> {
    let dir = crate::graph::processing::DIR_PATH.read().unwrap();
    let p = dir.join(target.trim());
    drop(dir);
    Some(p)
}

// Thumbnail path for an asset: next to the asset, under a `thumbnails/` dir.
// `assets/photo/20240513-231348.png` -> `assets/photo/thumbnails/20240513-231348.thumb.png`.
pub fn thumb_path(asset: &Path) -> PathBuf {
    let name = match asset.file_name().and_then(|n| n.to_str()) {
        Some(n) => match n.rfind('.') {
            Some(i) => format!("{}.thumb.png", &n[..i]),
            None => format!("{n}.thumb.png"),
        },
        None => "thumb.png".into(),
    };
    asset
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(THUMB_DIR)
        .join(name)
}

// Load an image file into the full-resolution cache, once. Failed loads are
// cached as absent so they are not retried every frame; the editor falls back
// to rendering the raw link text for those.
pub fn ensure_loaded(rl: &mut RaylibHandle, thread: &RaylibThread, path: &Path) {
    let mut c = cache().write().unwrap();
    if c.contains_key(path) {
        return;
    }
    let tex = Image::load_image(&path.to_string_lossy())
        .ok()
        .and_then(|img| rl.load_texture_from_image(thread, &img).ok());
    c.insert(path.to_path_buf(), Slot(tex));
}

pub fn has_texture(path: &Path) -> bool {
    cache()
        .read()
        .unwrap()
        .get(path)
        .is_some_and(|slot| slot.0.is_some())
}

// (display_width, display_height) for a cached image scaled down (never up)
// to fit within max_w x max_h while keeping its aspect ratio.
pub fn fit(path: &Path, max_w: i32, max_h: i32) -> Option<(i32, i32)> {
    let cache = cache().read().unwrap();
    let tex = &cache.get(path)?.0.as_ref()?;
    let (tw, th) = (tex.width, tex.height);
    if tw <= 0 || th <= 0 {
        return None;
    }
    let scale = ((max_w as f32 / tw as f32).min(max_h as f32 / th as f32)).min(1.0);
    Some((
        (tw as f32 * scale) as i32,
        (th as f32 * scale) as i32,
    ))
}

// Draw a cached image scaled into the available box (rectangular, as stored).
// Returns the drawn size.
pub fn draw<D: RaylibDraw>(d: &mut D, path: &Path, x: i32, y: i32, max_w: i32, max_h: i32) -> Option<(i32, i32)> {
    let (w, h) = fit(path, max_w, max_h)?;
    let cache = cache().read().unwrap();
    let tex = &cache.get(path)?.0.as_ref()?;
    let (tw, th) = (tex.width, tex.height);
    d.draw_texture_pro(
        tex,
        Rectangle::new(0.0, 0.0, tw as f32, th as f32),
        Rectangle::new(x as f32, y as f32, w as f32, h as f32),
        Vector2::zero(),
        0.0,
        Color::WHITE,
    );
    Some((w, h))
}

// Make sure a circular thumbnail exists and is loaded for `asset`, once. If a
// thumbnail file is already on disk the full-resolution image is never
// decoded again; otherwise the asset is decoded once, cropped to a square,
// masked to a disc and persisted as a PNG so future runs are cheap.
pub fn ensure_thumb_loaded(rl: &mut RaylibHandle, thread: &RaylibThread, asset: &Path) {
    let mut c = thumb_cache().write().unwrap();
    if c.contains_key(asset) {
        return;
    }
    let tex = build_thumb(rl, thread, asset);
    c.insert(asset.to_path_buf(), Slot(tex));
}

fn build_thumb(rl: &mut RaylibHandle, thread: &RaylibThread, asset: &Path) -> Option<Texture2D> {
    let thumb = thumb_path(asset);

    // Already generated on a previous run: load the small PNG directly.
    if thumb.exists() {
        if let Some(t) = rl.load_texture(thread, &thumb.to_string_lossy()).ok() {
            return Some(t);
        }
    }

    let img = Image::load_image(&asset.to_string_lossy()).ok()?;
    if img.width() <= 0 || img.height() <= 0 {
        return None;
    }
    let size = THUMB_SIZE as i32;

    // Fit the image, keeping its aspect, then expand the canvas to a square
    // so the inscribed-disc mask leaves no spurious crop.
    let (nw, nh, ox, oy) = thumb_layout(img.width(), img.height(), size);
    let mut work = img;
    work.resize(nw, nh);
    work.resize_canvas(size, size, ox, oy, Color::BLACK);

    let mut mask = Image::gen_image_color(size, size, Color::BLACK);
    mask.draw_circle(size / 2, size / 2, (size / 2).max(1), Color::WHITE);
    work.alpha_mask(&mask);

    if let Some(dir) = thumb.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    work.export_image(&thumb.to_string_lossy());

    // ExportImage has no return value; a failed export (e.g. unwritable dir)
    // is only detectable when the file fails to load back. In that case fall
    // back to the full-resolution image so the node still shows a header,
    // just as an unclipped square instead of a disc.
    match rl.load_texture(thread, &thumb.to_string_lossy()).ok() {
        Some(t) => Some(t),
        None => rl.load_texture(thread, &asset.to_string_lossy()).ok(),
    }
}

// Plane a `w x h` image onto a `size x size` square canvas: new dimensions
// keeping aspect plus centred canvas offsets. Pure so it is unit-testable.
fn thumb_layout(w: i32, h: i32, size: i32) -> (i32, i32, i32, i32) {
    if w <= 0 || h <= 0 || size <= 0 {
        return (1, 1, 0, 0);
    }
    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    let (nw, nh) = (((w as f32 * scale) as i32).max(1), ((h as f32 * scale) as i32).max(1));
    let (ox, oy) = ((size - nw) / 2, (size - nh) / 2);
    (nw, nh, ox, oy)
}

pub fn has_thumb(path: &Path) -> bool {
    thumb_cache()
        .read()
        .unwrap()
        .get(path)
        .is_some_and(|slot| slot.0.is_some())
}

// Draw the circular thumbnail for `path` as a disc of `side` pixels centred
// on (cx, cy). Returns false if the thumbnail is not (yet) loaded.
pub fn draw_thumb<D: RaylibDraw>(d: &mut D, path: &Path, cx: i32, cy: i32, side: i32) -> bool {
    let cache = thumb_cache().read().unwrap();
    let Some(slot) = cache.get(path) else {
        return false;
    };
    let Some(tex) = slot.0.as_ref() else {
        return false;
    };
    let s = side.max(1);
    let (tw, th) = (tex.width, tex.height);
    d.draw_texture_pro(
        tex,
        Rectangle::new(0.0, 0.0, tw as f32, th as f32),
        Rectangle::new((cx - s / 2) as f32, (cy - s / 2) as f32, s as f32, s as f32),
        Vector2::zero(),
        0.0,
        Color::WHITE,
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_image_extensions() {
        assert!(is_image_target("cat.png"));
        assert!(is_image_target("assets/image.png"));
        assert!(is_image_target("pic.jpg"));
        assert!(is_image_target("pic.jpeg"));
        assert!(is_image_target("anim.gif"));
        assert!(is_image_target("a.b.c.PNG"));
        assert!(is_image_target("icon.ico "));
    }

    #[test]
    fn rejects_unsupported_and_non_image_targets() {
        // raylib cannot decode TIFF; headers/links for it stay text.
        assert!(!is_image_target("scan.tif"));
        assert!(!is_image_target("scan.tiff"));
        assert!(!is_image_target("note.md"));
        assert!(!is_image_target("cat"));
        assert!(!is_image_target("cat.txt"));
        assert!(!is_image_target("dir/page.md"));
        assert!(!is_image_target(""));
    }

    #[test]
    fn thumbnail_path_sits_next_to_the_asset() {
        let a = Path::new("/home/x/graph-proj/assets/20240513-231348.png");
        let t = thumb_path(a);
        assert_eq!(t.parent().unwrap(), Path::new("/home/x/graph-proj/assets/thumbnails"));
        assert_eq!(t.file_name().unwrap(), "20240513-231348.thumb.png");

        let nested = thumb_path(Path::new("assets/sub/photo.jpg"));
        assert_eq!(nested.parent().unwrap(), Path::new("assets/sub/thumbnails"));
        assert_eq!(nested.file_name().unwrap(), "photo.thumb.png");
    }

    #[test]
    fn thumbnail_layout_fits_any_aspect_into_a_square() {
        // Square source fills the canvas with no offsets.
        let (nw, nh, ox, oy) = thumb_layout(256, 256, 256);
        assert_eq!((nw, nh), (256, 256));
        assert_eq!((ox, oy), (0, 0));

        // Wide image: letterboxed on top and bottom.
        let (nw, nh, ox, oy) = thumb_layout(1920, 1080, 256);
        assert_eq!((nw, nh), (256, 144));
        assert_eq!((ox, oy), (0, 56));

        // Tall image: letterboxed on the sides.
        let (nw, nh, ox, oy) = thumb_layout(1080, 1920, 256);
        assert_eq!((nw, nh), (144, 256));
        assert_eq!((ox, oy), (56, 0));

        // Everything stays inside the canvas.
        for (w, h) in [(1920, 1080), (40, 100), (13, 7), (1, 1), (2048, 2048)] {
            let (nw, nh, ox, oy) = thumb_layout(w, h, 256);
            assert!(nw >= 1 && nh >= 1);
            assert!(nw <= 256 && nh <= 256);
            assert!(ox >= 0 && oy >= 0);
            assert!(ox + nw <= 256 && oy + nh <= 256);
        }
    }
}