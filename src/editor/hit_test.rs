use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

use crate::editor::markdown;

// -----------------------------------------------------------------------
// Hit-test row published by the renderer every frame
// -----------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct HitRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub indent_px: i32,
    pub font_size: i32,
    pub top: i32,
    pub advance: i32,
    pub view_mode: bool,
    pub quote_skip: usize,
    pub image: bool,
    pub fm_bar: bool,
}

pub static VISUAL_HIT: std::sync::Mutex<Vec<HitRow>> = std::sync::Mutex::new(Vec::new());

pub static AUTOCOMPLETE_RECT: std::sync::Mutex<Option<(i32, i32, i32, i32)>> =
    std::sync::Mutex::new(None);

// -----------------------------------------------------------------------
// Mouse click state (used by the input handler)
// -----------------------------------------------------------------------

pub static MOUSE_DRAGGING: AtomicBool = AtomicBool::new(false);
pub static LAST_CLICK_TIME_MS: AtomicU64 = AtomicU64::new(0);
pub static LAST_CLICK_LINE: AtomicI32 = AtomicI32::new(0);
pub static LAST_CLICK_OFFSET: AtomicI32 = AtomicI32::new(0);
pub static CLICK_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn reset_click_state() {
    MOUSE_DRAGGING.store(false, Ordering::Relaxed);
    LAST_CLICK_TIME_MS.store(0, Ordering::Relaxed);
    LAST_CLICK_LINE.store(0, Ordering::Relaxed);
    LAST_CLICK_OFFSET.store(0, Ordering::Relaxed);
    CLICK_COUNT.store(0, Ordering::Relaxed);
}

// -----------------------------------------------------------------------
// Hit-testing helpers
// -----------------------------------------------------------------------

/// Find the row whose `[top, top+advance)` band contains `y`.
pub fn row_at_y(rows: &[HitRow], y: i32) -> Option<&HitRow> {
    rows.iter()
        .find(|r| y >= r.top && y < r.top + r.advance)
}

/// Map a content-relative X pixel to the byte offset the user clicked.
/// `px_from_padding` is measured from `editor_x + padding` (the content
/// origin).  The returned offset is always at a char boundary.
///
/// `measure` is injected so the function is unit-testable without raylib.
pub fn offset_at_px(
    line: &str,
    row: &HitRow,
    px_from_padding: i32,
    measure: impl Fn(&str, i32) -> i32,
) -> usize {
    if row.image {
        return row.start;
    }

    let origin_raw = row.start + row.quote_skip;
    let safe_end = row.end.min(line.len());
    if origin_raw >= safe_end {
        return row.end;
    }

    let eff = px_from_padding - row.indent_px;
    if eff <= 0 {
        return origin_raw;
    }

    let mut best = origin_raw;
    for (rel, _) in line[origin_raw..safe_end].char_indices() {
        let raw = origin_raw + rel;
        if raw == origin_raw {
            continue;
        }
        let w = measure(
            &markdown::measure_line(&line[origin_raw..raw], row.view_mode),
            row.font_size,
        );
        if w > eff {
            break;
        }
        best = raw;
    }

    let full_w = measure(
        &markdown::measure_line(&line[origin_raw..safe_end], row.view_mode),
        row.font_size,
    );
    if full_w <= eff {
        best = row.end;
    }

    best
}

/// Classify a mouse click as single, double or triple.  Returns 1..=3.
pub fn classify_click(now_ms: u64, last_ms: u64, same_pos: bool, prev_count: u32) -> u32 {
    if same_pos && now_ms.saturating_sub(last_ms) < 300 && prev_count < 3 {
        prev_count + 1
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pix(s: &str, _fsz: i32) -> i32 {
        10 * s.chars().count() as i32
    }

    fn plain_row() -> HitRow {
        HitRow {
            line: 0,
            start: 0,
            end: 5,
            indent_px: 0,
            font_size: 20,
            top: 0,
            advance: 20,
            view_mode: true,
            quote_skip: 0,
            image: false,
            fm_bar: false,
        }
    }

    #[test]
    fn plain_offset_before_start() {
        assert_eq!(offset_at_px("hello", &plain_row(), -5, pix), 0);
    }

    #[test]
    fn plain_offset_at_origin() {
        assert_eq!(offset_at_px("hello", &plain_row(), 0, pix), 0);
    }

    #[test]
    fn plain_offset_middle() {
        assert_eq!(offset_at_px("hello", &plain_row(), 25, pix), 2);
    }

    #[test]
    fn plain_offset_at_char_boundary() {
        assert_eq!(offset_at_px("hello", &plain_row(), 10, pix), 1);
    }

    #[test]
    fn plain_offset_past_end() {
        assert_eq!(offset_at_px("hello", &plain_row(), 999, pix), 5);
    }

    #[test]
    fn heading_skip() {
        let row = HitRow {
            start: 2,
            end: 7,
            indent_px: 0,
            font_size: 70,
            top: 0,
            advance: 70,
            view_mode: true,
            ..plain_row()
        };
        // "## Hi" – origin_raw=2, px 15 → eff 15, "H" = 10 ≤ 15, "Hi" = 20 > 15 → offset 3
        assert_eq!(offset_at_px("## Hi", &row, 15, pix), 3);
    }

    #[test]
    fn quote_skip_first_char() {
        let row = HitRow {
            start: 0,
            end: 8,
            indent_px: 20,
            font_size: 20,
            view_mode: true,
            quote_skip: 2,
            ..plain_row()
        };
        // px=30 → eff=10; origin_raw=2; "f" (10) ≤ 10 → best=3; "fo"(20)>10 → 3
        assert_eq!(offset_at_px("> foo", &row, 30, pix), 3);
    }

    #[test]
    fn quote_skip_before_indent() {
        let row = HitRow {
            start: 0,
            end: 8,
            indent_px: 20,
            font_size: 20,
            view_mode: true,
            quote_skip: 2,
            ..plain_row()
        };
        assert_eq!(offset_at_px("> foo", &row, 5, pix), 2);
    }

    #[test]
    fn image_returns_start() {
        let row = HitRow {
            image: true,
            ..plain_row()
        };
        assert_eq!(offset_at_px("[[pic.png]]", &row, 50, pix), 0);
    }

    #[test]
    fn empty_line() {
        let row = HitRow {
            start: 3,
            end: 3,
            indent_px: 0,
            top: 0,
            advance: 20,
            ..plain_row()
        };
        assert_eq!(offset_at_px("abc", &row, 0, pix), 3);
    }

    #[test]
    fn row_at_y_hits() {
        let rows = vec![
            HitRow { top: 0, advance: 20, ..plain_row() },
            HitRow { top: 22, advance: 20, ..plain_row() },
        ];
        assert_eq!(row_at_y(&rows, 10).unwrap().top, 0);
        assert_eq!(row_at_y(&rows, 22).unwrap().top, 22);
        assert!(row_at_y(&rows, 21).is_none());
    }

    #[test]
    fn classify_single() {
        assert_eq!(classify_click(100, 0, false, 0), 1);
    }

    #[test]
    fn classify_double() {
        assert_eq!(classify_click(200, 150, true, 1), 2);
    }

    #[test]
    fn classify_triple() {
        assert_eq!(classify_click(300, 250, true, 2), 3);
    }

    #[test]
    fn classify_quad_resets() {
        assert_eq!(classify_click(400, 350, true, 3), 1);
    }

    #[test]
    fn classify_timeout_resets() {
        assert_eq!(classify_click(500, 100, true, 2), 1);
    }

    #[test]
    fn classify_different_pos_resets() {
        assert_eq!(classify_click(200, 150, false, 1), 1);
    }
}
