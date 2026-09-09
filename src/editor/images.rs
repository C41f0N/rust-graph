use raylib::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex, OnceLock, RwLock};

// raylib's Texture2D owns a GPU texture, so it is neither Send nor Sync.
// Every load/unload and every draw happens on the main thread (the window's
// GL context thread), which is exactly the case this cache models, so the
// wrapper is sound. Each Texture2D's Drop runs UnloadTexture, so a removed
// cache entry frees its GPU memory.
enum Slot {
    // Requested, but not yet decoded/uploaded by the background loader.
    Pending,
    // None means the load failed, cached as absent so it is not retried.
    Ready(Option<Texture2D>),
}
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

// An Image is only CPU pixel data, so handing it to another thread for the
// main-thread texture upload is sound. The wrapper makes the channel's Send
// bound explicit regardless of what raylib advertises for Image.
struct CpuImage(Image);
unsafe impl Send for CpuImage {}

// Full-resolution textures, decoded from the original file. Used by the
// editor, which draws whole [[file]] lines as their rectangular image. Only
// the files the caller asks for each frame end up here.
static CACHE: OnceLock<RwLock<HashMap<PathBuf, Slot>>> = OnceLock::new();

// General thumbnails (persisted next to the asset as
// <dir>/thumbnails/<name>.thumb.png), a pure aspect-keeping downscale of the
// source that any consumer can reuse; the graph disc-crops its own variant at
// load. Keyed by the original asset path.
static THUMB_CACHE: OnceLock<RwLock<HashMap<PathBuf, Slot>>> = OnceLock::new();

fn cache() -> &'static RwLock<HashMap<PathBuf, Slot>> {
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn thumb_cache() -> &'static RwLock<HashMap<PathBuf, Slot>> {
    THUMB_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

// --- Background loader ------------------------------------------------------
//
// Image decode, resize, mask and PNG export are CPU-only and safe off the
// render thread (raylib's rule is that only GL-touching calls need the main
// thread). So a single worker thread does all the slow work and hands finished
// CPU images back through a channel; each frame's sync() uploads them. The
// worker is spawned lazily on the first request.
enum Job {
    // Small general thumbnail (persisted next to the asset) plus the
    // disc-cropped variant the graph draws for node headers.
    Thumb { path: PathBuf },
    // Full-resolution image for editor [[file]] lines.
    Full { path: PathBuf },
}

enum ReadyImage {
    Thumb { path: PathBuf, image: Option<CpuImage> },
    Full { path: PathBuf, image: Option<CpuImage> },
}

static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
static RESULTS: OnceLock<Mutex<mpsc::Receiver<ReadyImage>>> = OnceLock::new();

fn worker_sender() -> &'static mpsc::Sender<Job> {
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let (rtx, rrx) = mpsc::channel();
        let _ = RESULTS.set(Mutex::new(rrx));
        std::thread::Builder::new()
            .name("image-loader".into())
            .spawn(move || worker_loop(rx, rtx))
            .expect("spawn image loader thread");
        tx
    })
}

fn worker_loop(rx: mpsc::Receiver<Job>, tx: mpsc::Sender<ReadyImage>) {
    while let Ok(job) = rx.recv() {
        let ready = match job {
            Job::Thumb { path } => ReadyImage::Thumb {
                image: build_node_thumb(&path).map(CpuImage),
                path,
            },
            Job::Full { path } => {
                let image = Image::load_image(&path.to_string_lossy()).ok().map(CpuImage);
                ReadyImage::Full { image, path }
            }
        };
        if tx.send(ready).is_err() {
            break;
        }
    }
}

// Upload whatever the worker finished since the last frame. This is the only
// place texture creation happens, so every upload runs on the main (GL) thread.
pub fn sync(rl: &mut RaylibHandle, thread: &RaylibThread) {
    let Some(receiver) = RESULTS.get() else {
        return;
    };
    let mut full = cache().write().unwrap();
    let mut thumbs = thumb_cache().write().unwrap();
    let rx = receiver.lock().unwrap();
    while let Ok(ready) = rx.try_recv() {
        match ready {
            ReadyImage::Full { path, image } => {
                let tex = image
                    .as_ref()
                    .and_then(|img| rl.load_texture_from_image(thread, &img.0).ok());
                full.insert(path, Slot::Ready(tex));
            }
            ReadyImage::Thumb { path, image } => {
                let tex = image
                    .as_ref()
                    .and_then(|img| rl.load_texture_from_image(thread, &img.0).ok());
                if let Some(t) = tex.as_ref() {
                    set_bilinear(t);
                }
                thumbs.insert(path, Slot::Ready(tex));
            }
        }
    }
}

// File extensions raylib can decode into a texture. A link whose target has
// one of these is treated as an inline image. The raylib build does not
// support TIFF, so tif/tiff are intentionally absent.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "tga", "ico"];

// Longest side, in pixels, of the persisted general thumbnail: a pure
// aspect-keeping downscale of the source, reusable by any consumer. The disc a
// header is drawn into is at most ~1.9 * max node radius (16) * max zoom (10)
// ~= 304 px, so 256 keeps it crisp while staying tiny to decode, mask and
// upload compared to a full-resolution source.
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

// Queue a full-resolution image decode for the editor's [[file]] lines. Does
// not block; the decoded image is uploaded by sync() once the worker finishes.
// Failed loads are cached as absent so they are not retried every frame; the
// editor falls back to rendering the raw link text for those.
pub fn request_full(path: &Path) {
    let mut c = cache().write().unwrap();
    if c.contains_key(path) {
        return;
    }
    c.insert(path.to_path_buf(), Slot::Pending);
    drop(c);
    worker_sender().send(Job::Full { path: path.to_path_buf() }).ok();
}

pub fn has_texture(path: &Path) -> bool {
    cache()
        .read()
        .unwrap()
        .get(path)
        .is_some_and(|slot| matches!(slot, Slot::Ready(Some(_))))
}

// (display_width, display_height) for a cached image scaled down (never up)
// to fit within max_w x max_h while keeping its aspect ratio.
pub fn fit(path: &Path, max_w: i32, max_h: i32) -> Option<(i32, i32)> {
    let cache = cache().read().unwrap();
    let tex = match cache.get(path)? {
        Slot::Ready(t) => t.as_ref()?,
        Slot::Pending => return None,
    };
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
    let tex = match cache.get(path)? {
        Slot::Ready(t) => t.as_ref()?,
        Slot::Pending => return None,
    };
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

// Queue a thumbnail for a node header image. Non-blocking; the worker
// generates the persisted general thumbnail if needed and disc-crops a variant
// for the graph, which sync() uploads. Failed loads are cached as absent.
pub fn request_thumb(path: &Path) {
    let mut c = thumb_cache().write().unwrap();
    if c.contains_key(path) {
        return;
    }
    c.insert(path.to_path_buf(), Slot::Pending);
    drop(c);
    worker_sender().send(Job::Thumb { path: path.to_path_buf() }).ok();
}

// Force bilinear filtering so a thumbnail scaled around a node (usually drawn
// larger than its 256px backing) reads smooth rather than blocky. raylib's
// default texture filter is point/nearest.
fn set_bilinear(tex: &Texture2D) {
    // SAFETY: tex is a loaded GPU texture owned by the main thread.
    unsafe {
        let t: &raylib::ffi::Texture2D = tex.as_ref();
        raylib::ffi::SetTextureFilter(*t, 1); // TEXTURE_FILTER_BILINEAR
    }
}

// Build the disc-cropped variant the graph draws for node headers: take the
// persisted general thumbnail, letterbox it onto a square black canvas (so an
// image's own border can bleed into the circle), then mask everything outside
// the inscribed disc to transparent.
fn disc_crop_image(source: &Image, size: i32) -> Image {
    let (nw, nh, ox, oy) = thumb_layout(source.width(), source.height(), size);
    let mut work = source.clone();
    work.resize(nw, nh);
    work.resize_canvas(size, size, ox, oy, Color::BLACK);
    let mut mask = Image::gen_image_color(size, size, Color::BLACK);
    mask.draw_circle(size / 2, size / 2, (size / 2).max(1), Color::WHITE);
    work.alpha_mask(&mask);
    work
}

// Downscale `w x h` to have its longest side fit `size`, preserving aspect
// ratio (both sides shrink out of the box). Pure so it is unit-testable.
fn thumb_scale(w: i32, h: i32, size: i32) -> (i32, i32) {
    if w <= 0 || h <= 0 || size <= 0 {
        return (1, 1);
    }
    if w <= size && h <= size {
        return (w, h);
    }
    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    (
        ((w as f32 * scale) as i32).max(1),
        ((h as f32 * scale) as i32).max(1),
    )
}

// Generate the persisted general thumbnail for `asset`: a pure aspect-keeping
// downscale to THUMB_SIZE, neither cropped nor masked, so it is a plain
// low-res copy of the source any consumer can reuse. Returns the scaled CPU
// image as well, so the caller does not need a disk round-trip after writing.
fn gen_general_thumb(asset: &Path, thumb: &Path) -> Option<Image> {
    let src = Image::load_image(&asset.to_string_lossy()).ok()?;
    if src.width() <= 0 || src.height() <= 0 {
        return None;
    }
    let (nw, nh) = thumb_scale(src.width(), src.height(), THUMB_SIZE as i32);
    let mut work = src;
    work.resize(nw, nh);
    if let Some(dir) = thumb.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    work.export_image(&thumb.to_string_lossy());
    Some(work)
}

// Produce the disc-cropped header image for a node. Regenerates the persisted
// general thumbnail when it is missing or the source file is newer on disk
// (cheap mtime compare), then disc-crops it for the graph.
fn build_node_thumb(asset: &Path) -> Option<Image> {
    let thumb = thumb_path(asset);
    let src_stamp = std::fs::metadata(asset).and_then(|m| m.modified()).ok();
    let thumb_stamp = std::fs::metadata(&thumb).and_then(|m| m.modified()).ok();
    let needs_gen = match (src_stamp, thumb_stamp) {
        (Some(s), Some(t)) => s > t,
        (Some(_), None) => true,
        _ => false,
    };
    let general = if needs_gen {
        gen_general_thumb(asset, &thumb)
            .or_else(|| Image::load_image(&thumb.to_string_lossy()).ok())
            .or_else(|| Image::load_image(&asset.to_string_lossy()).ok())?
    } else {
        Image::load_image(&thumb.to_string_lossy())
            .ok()
            .or_else(|| Image::load_image(&asset.to_string_lossy()).ok())?
    };
    Some(disc_crop_image(&general, THUMB_SIZE as i32))
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
        .is_some_and(|slot| matches!(slot, Slot::Ready(Some(_))))
}

// Draw the circular thumbnail for `path` as a disc of `side` pixels centred on
// (cx, cy). Coordinates and size are f32 (like the node positions they follow)
// so the disc tracks the node's smooth animation without pixel-grid snapping.
// Returns false if the thumbnail is not (yet) loaded.
pub fn draw_thumb<D: RaylibDraw>(d: &mut D, path: &Path, cx: f32, cy: f32, side: f32) -> bool {
    let cache = thumb_cache().read().unwrap();
    let tex = match cache.get(path) {
        Some(Slot::Ready(t)) => match t.as_ref() {
            Some(tex) => tex,
            None => return false,
        },
        Some(Slot::Pending) | None => return false,
    };
    let s = side.max(1.0);
    let (tw, th) = (tex.width, tex.height);
    d.draw_texture_pro(
        tex,
        Rectangle::new(0.0, 0.0, tw as f32, th as f32),
        Rectangle::new(cx - s / 2.0, cy - s / 2.0, s, s),
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

    #[test]
    fn thumb_scale_preserves_aspect_within_size() {
        for (w, h) in [(1920, 1080), (1080, 1920), (256, 256), (13, 7), (1, 1), (2048, 2048)] {
            let (nw, nh) = thumb_scale(w, h, 256);
            assert!(nw >= 1 && nh >= 1);
            assert!(nw <= 256 && nh <= 256);
            let a_src = w as f32 / h as f32;
            let a_dst = nw as f32 / nh as f32;
            assert!(
                (a_src - a_dst).abs() < 0.02,
                "{}x{} -> {}x{} broke the aspect ratio",
                w,
                h,
                nw,
                nh
            );
        }

        // Scale only kicks in when something exceeds the box; unchanged below it.
        assert_eq!(thumb_scale(256, 256, 256), (256, 256));
        assert_eq!(thumb_scale(512, 256, 256), (256, 128));
        assert_eq!(thumb_scale(256, 512, 256), (128, 256));
        assert_eq!(thumb_scale(100, 50, 256), (100, 50));
    }
}