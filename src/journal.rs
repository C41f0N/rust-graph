// Date-keyed notes ("journals"): a note whose file name is a calendar date
// (`2026_10_11.md`) at the vault root, mirroring how the vault already keeps
// daily entries. `J` opens today's note, `[`/`]` step a day from an open
// journal note, and the Ctrl+K palette recognizes a typed date. Opening a
// date that has no file yet never touches the disk: it is a *pending* entry
// that only materializes (file + graph node) on the save that actually has
// content, so a blank J press leaves nothing behind. In every UI surface the
// date renders as "19th Aug 2025" rather than the raw `2026_08_19` stem. All
// date math is pure civil-date arithmetic (no date crate) so the tests are
// exact.

pub type Date = (i32, u32, u32);

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// Current local calendar date (DST-safe via localtime_r).
pub fn local_date() -> Date {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t: libc::time_t = now;
        libc::localtime_r(&t, &mut tm);
        (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
    }
}

// Howard Hinnant's days-from-civil algorithm: day number z since 1970-01-01.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

// Inverse of days_from_civil.
fn civil_from_days(z: i64) -> Date {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y } as i32, month, day)
}

// File-stem a journal note gets: underscores match the vault's existing
// dated notes (2026_10_11.md rather than a dashed locale format).
pub fn date_to_stem(d: Date) -> String {
    format!("{:04}_{:02}_{:02}", d.0, d.1, d.2)
}

// Day-ordinal suffix: 1st, 2nd, 3rd, 21st..., with the 11/12/13 teens as
// exceptions ("11th Dec").
fn ordinal(day: u32) -> &'static str {
    match day {
        11 | 12 | 13 => "th",
        d => match d % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    }
}

// Human-readable journal date, the form used across every UI surface:
// "19th Aug 2025".
pub fn pretty_date(d: Date) -> String {
    format!(
        "{}{} {} {:04}",
        d.2,
        ordinal(d.2),
        MONTHS[d.1 as usize - 1],
        d.0
    )
}

// Pretty-display a note name: a journal date stem renders as "19th Aug 2025",
// anything else passes through untouched. Callers wrap any label (graph node,
// document name, tab title, palette row) with this.
pub fn pretty_name(name: &str) -> String {
    match parse_date_stem(name) {
        Some(d) => pretty_date(d),
        None => name.to_string(),
    }
}

// Parse a dated file stem or filter into a real date. Both `_` and `-`
// separators are accepted (the palette is typed with a dash, files on disk
// use underscores); impossible dates (month 13, Feb 30, a non-leap Feb 29)
// fail by round-tripping the civil algorithms.
pub fn parse_date_stem(stem: &str) -> Option<Date> {
    let b = stem.as_bytes();
    if b.len() != 10 {
        return None;
    }
    let sep_ok =
        (b[4] == b'_' || b[4] == b'-') && (b[7] == b'_' || b[7] == b'-');
    let digits_ok = b[0..4]
        .iter()
        .chain(&b[5..7])
        .chain(&b[8..10])
        .all(|c| c.is_ascii_digit());
    if !sep_ok || !digits_ok {
        return None;
    }
    let year = b[0..4]
        .iter()
        .fold(0i32, |a, &c| a * 10 + (c - b'0') as i32);
    let month = (b[5] - b'0') as u32 * 10 + (b[6] - b'0') as u32;
    let day = (b[8] - b'0') as u32 * 10 + (b[9] - b'0') as u32;
    let d = (year, month, day);
    if civil_from_days(days_from_civil(year as i64, month, day)) == d {
        Some(d)
    } else {
        None
    }
}

// Palette-filter variant: trims whitespace and a trailing ".md" so typing
// either `2026-10-11`, `2026_10_11` or `2026-10-11.md` selects the row.
pub fn parse_date_filter(s: &str) -> Option<Date> {
    parse_date_stem(s.trim().trim_end_matches(".md"))
}

// Shift a date by `delta` days, renormalizing month/year boundaries.
pub fn step_date(d: Date, delta: i32) -> Date {
    civil_from_days(days_from_civil(d.0 as i64, d.1, d.2) + delta as i64)
}

// Open a journal note without touching the disk: an existing day's note
// opens like any other (nav mode), while one that doesn't exist yet opens
// as a blank *pending* document that only materializes on its first save
// (see materialize_if_pending). Returns true when the note is pending, so
// callers can drop straight into edit mode for a fresh entry. The caller
// must not be holding NODES or BUFFER.
pub fn open_date(d: Date) -> bool {
    let stem = date_to_stem(d);
    let filename = format!("{stem}.md");
    let dir = crate::graph::processing::DIR_PATH.read().unwrap().clone();
    let path = dir.join(&filename);
    let pending = !path.is_file();
    crate::editor::tabs::open(&path, &stem);
    pending
}

// Same rule, deferred through OPEN_NOTE_REQUESTED because the editor input
// handler runs its whole frame holding BUFFER's write lock, so the actual
// activate-and-load happens in main once that guard is dropped. When the
// open would land on a pending note, NAV_EDIT_REQUESTED asks main to drop
// straight into edit mode right after the load.
pub fn open_date_requested(d: Date) -> bool {
    let stem = date_to_stem(d);
    let filename = format!("{stem}.md");
    let dir = crate::graph::processing::DIR_PATH.read().unwrap().clone();
    let path = dir.join(&filename);
    let pending = !path.is_file();
    *crate::editor::OPEN_NOTE_REQUESTED.write().unwrap() = Some((path, stem));
    if pending {
        crate::editor::NAV_EDIT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pending
}

pub fn open_today() -> bool {
    open_date(local_date())
}

pub fn open_today_requested() -> bool {
    open_date_requested(local_date())
}

// Step a day forward/backward from the open journal note. Returns false when
// the active tab is not a dated note, so callers know the key was a no-op.
pub fn open_adjacent_requested(delta: i32) -> bool {
    let stem = crate::editor::tabs::active_path()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_default();
    let Some(d) = parse_date_stem(&stem) else {
        return false;
    };
    open_date_requested(step_date(d, delta));
    true
}

// A save targeting a journal note whose file is not on disk yet (opened via
// J, prev/next, or a palette date) creates the file and its graph node only
// when the buffer actually has content: nothing written, nothing born.
// Returns true when the save should proceed (existing file, non-journal
// path, or a just-materialized journal); false says skip the write and keep
// the blank entry unborn.
pub fn materialize_if_pending(path: &std::path::Path, has_content: bool) -> bool {
    if path.is_file() {
        return true;
    }
    let Some(filename) = path.file_name().map(|s| s.to_string_lossy().to_string()) else {
        return true;
    };
    if parse_date_stem(filename.trim_end_matches(".md")).is_none() {
        return true;
    }
    if !has_content {
        return false;
    }
    let dir = crate::graph::processing::DIR_PATH.read().unwrap().clone();
    crate::graph::processing::ensure_node_path(&dir, &filename);
    true
}

// True when a *file name* (not a path) sort of note marks it as a journal
// entry, used to decorate tabs/helpbar chips without owning the tab.
pub fn is_journal_file_name(name: &str) -> bool {
    parse_date_stem(name.trim_end_matches(".md")).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_round_trip() {
        assert_eq!(date_to_stem((2026, 10, 11)), "2026_10_11");
        assert_eq!(parse_date_stem("2026_10_11"), Some((2026, 10, 11)));
        assert_eq!(parse_date_stem("2026-10-11"), Some((2026, 10, 11)));
    }

    #[test]
    fn pretty_dates_render_human_readable() {
        assert_eq!(pretty_date((2025, 8, 19)), "19th Aug 2025");
        assert_eq!(pretty_date((2026, 10, 11)), "11th Oct 2026");
        assert_eq!(pretty_date((2026, 2, 3)), "3rd Feb 2026");
        assert_eq!(pretty_date((2026, 12, 12)), "12th Dec 2026");
        assert_eq!(pretty_date((2026, 12, 21)), "21st Dec 2026");
        assert_eq!(pretty_date((2026, 12, 22)), "22nd Dec 2026");
        assert_eq!(pretty_date((2026, 12, 13)), "13th Dec 2026");
        assert_eq!(pretty_name("2026_10_11"), "11th Oct 2026");
        assert_eq!(pretty_name("2026-08-19"), "19th Aug 2026");
        assert_eq!(pretty_name("hello"), "hello");
        assert_eq!(pretty_name("2026_13_01"), "2026_13_01");
    }

    #[test]
    fn filters_trim_and_drop_extension() {
        for s in ["2026_10_11.md", " 2026-10-11 ", "2026-10-11", "2026_10_11"] {
            assert_eq!(parse_date_filter(s), Some((2026, 10, 11)), "{s:?}");
        }
    }

    #[test]
    fn impossible_dates_fail_to_parse() {
        for bad in [
            "2026_13_01",
            "2026_00_10",
            "2026_02_30",
            "2026_02_29", // 2026 is not a leap year
            "2026_1_1",
            "2026-10",
            "20261011",
            "2026_10_11x",
            "notadate",
            "",
        ] {
            assert_eq!(parse_date_stem(bad), None, "{bad:?} must not parse");
        }
        assert_eq!(parse_date_stem("2024_02_29"), Some((2024, 2, 29)));
    }

    #[test]
    fn stepping_crosses_month_and_year_boundaries() {
        assert_eq!(step_date((2026, 1, 1), -1), (2025, 12, 31));
        assert_eq!(step_date((2026, 3, 1), -1), (2026, 2, 28));
        assert_eq!(step_date((2025, 12, 31), 1), (2026, 1, 1));
        assert_eq!(step_date((2026, 2, 28), 1), (2026, 3, 1));
        assert_eq!(step_date((2024, 2, 28), 1), (2024, 2, 29)); // leap year
    }

    #[test]
    fn local_date_is_real() {
        let d = local_date();
        assert!((2016..=2100).contains(&d.0));
        assert!(civil_from_days(days_from_civil(d.0 as i64, d.1, d.2)) == d);
    }
}