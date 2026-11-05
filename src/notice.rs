// Transient user-facing notice drawn over every view.
//
// The editor uses this for file-open refusals (too large, unreadable, not
// UTF-8) so a rejected open is visible even when the editor itself never
// opens. The notice is non-blocking: it never consumes input and expires on
// its own.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use raylib::prelude::*;

use crate::config;
use crate::editor::text;
use crate::ui;

const NOTICE_LIFETIME: Duration = Duration::from_secs(8);

struct Notice {
    title: String,
    detail: String,
    shown_at: Instant,
}

static NOTICE: Mutex<Option<Notice>> = Mutex::new(None);

// Tests that assert on the process-global toast state must not race other
// tests that trigger a refusal (and therefore a toast).
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

pub fn push(title: impl Into<String>, detail: impl Into<String>) {
    *NOTICE.lock().unwrap() = Some(Notice {
        title: title.into(),
        detail: detail.into(),
        shown_at: Instant::now(),
    });
}

#[cfg(test)]
pub fn clear() {
    *NOTICE.lock().unwrap() = None;
}

// Fit a string into `max_w` pixels, appending an ellipsis when clipped. Pure
// display helper, kept separate so the geometry is simple to reason about.
fn fit(d: &RaylibDrawHandle, value: &str, font_size: i32, max_w: i32) -> String {
    if text::measure(d, value, font_size) <= max_w {
        return value.to_string();
    }
    let mut cut = value.to_string();
    loop {
        if cut.is_empty() {
            return "\u{2026}".to_string();
        }
        cut.pop();
        let candidate = format!("{cut}\u{2026}");
        if text::measure(d, &candidate, font_size) <= max_w {
            return candidate;
        }
    }
}

pub fn draw(d: &mut RaylibDrawHandle) {
    let now = Instant::now();
    let mut guard = NOTICE.lock().unwrap();
    let Some(notice) = guard.as_ref() else {
        return;
    };
    if now.duration_since(notice.shown_at) >= NOTICE_LIFETIME {
        *guard = None;
        return;
    }

    let screen_w = config::width();
    let screen_h = config::height();
    let title_size = config::scaled_size(16);
    let detail_size = config::scaled_size(13);
    let pad = config::scaled_size(14);
    let max_text_w = (screen_w - 2 * (pad * 3)).max(80);
    let title = fit(d, &notice.title, title_size, max_text_w);
    let detail = fit(d, &notice.detail, detail_size, max_text_w);
    let text_w = text::measure(d, &title, title_size).max(text::measure(d, &detail, detail_size));
    let box_w = (text_w + pad * 2).min(screen_w - 2 * pad).max(120);
    let box_h = title_size + detail_size + pad * 2 + config::scaled_size(4);
    let x = (screen_w - box_w) / 2;
    let y = config::scaled_size(20).min((screen_h - box_h).max(0));

    let panel = Rectangle::new(x as f32, y as f32, box_w as f32, box_h as f32);
    ui::panel(d, panel, 0.18);
    // A quiet accent edge ties the toast to the rest of the chrome.
    d.draw_rectangle_rounded(
        Rectangle::new(x as f32 + 3.0, (y + 10) as f32, 3.0, (box_h - 20).max(4) as f32),
        0.5,
        4,
        ui::ACCENT,
    );
    text::draw(d, &title, x + pad, y + pad, title_size, ui::TEXT);
    text::draw(
        d,
        &detail,
        x + pad,
        y + pad + title_size + config::scaled_size(4),
        detail_size,
        ui::TEXT_SECONDARY,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_replaces_and_clear_removes() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear();
        push("Cannot open note", "too large");
        assert!(NOTICE.lock().unwrap().is_some());
        push("Cannot open note", "unreadable");
        let detail = NOTICE.lock().unwrap().as_ref().unwrap().detail.clone();
        assert_eq!(detail, "unreadable");
        clear();
        assert!(NOTICE.lock().unwrap().is_none());
    }
}
