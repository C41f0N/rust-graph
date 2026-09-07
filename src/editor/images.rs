use raylib::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

// raylib's Texture2D owns a GPU texture, so it is neither Send nor Sync.
// Every load/unload and every draw happens on the main thread (the window's
// GL context thread), which is exactly the case this cache models, so the
// wrapper is sound. The texture stays in the cache until a removed entry is
// dropped (each Texture2D's Drop runs UnloadTexture).
struct TexSlot(Texture2D);
unsafe impl Send for TexSlot {}
unsafe impl Sync for TexSlot {}

static CACHE: OnceLock<RwLock<HashMap<PathBuf, Option<TexSlot>>>> = OnceLock::new();

fn cache() -> &'static RwLock<HashMap<PathBuf, Option<TexSlot>>> {
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

// File extensions raylib can decode into a texture. A link whose target has
// one of these is treated as an inline image.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "ico"];

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

// Load an image file into the cache. Failed loads are cached as None so they
// are not retried every frame; the editor falls back to rendering the raw
// link text for those.
pub fn ensure_loaded(rl: &mut RaylibHandle, thread: &RaylibThread, path: &Path) {
    let mut cache = cache().write().unwrap();
    if cache.contains_key(path) {
        return;
    }
    let slot = rl.load_texture(thread, &path.to_string_lossy()).ok().map(TexSlot);
    cache.insert(path.to_path_buf(), slot);
}

pub fn has_texture(path: &Path) -> bool {
    cache()
        .read()
        .unwrap()
        .get(path)
        .is_some_and(|slot| slot.is_some())
}

// (display_width, display_height) for a cached image scaled down (never up)
// to fit within max_w x max_h while keeping its aspect ratio.
pub fn fit(path: &Path, max_w: i32, max_h: i32) -> Option<(i32, i32)> {
    let cache = cache().read().unwrap();
    let tex = &cache.get(path)?.as_ref()?.0;
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

// Draw a cached image scaled into the available box. Returns the drawn size.
pub fn draw<D: RaylibDraw>(d: &mut D, path: &Path, x: i32, y: i32, max_w: i32, max_h: i32) -> Option<(i32, i32)> {
    let (w, h) = fit(path, max_w, max_h)?;
    let cache = cache().read().unwrap();
    let tex = &cache.get(path)?.as_ref()?.0;
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
    fn rejects_non_images() {
        assert!(!is_image_target("note.md"));
        assert!(!is_image_target("cat"));
        assert!(!is_image_target("cat.txt"));
        assert!(!is_image_target("dir/page.md"));
        assert!(!is_image_target(""));
    }
}