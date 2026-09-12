use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::RwLock;

// Slash-command picker: an unclosed word-initial "/" token in the buffer shows
// matching commands under the caret. The "/command" text itself stays in the
// buffer as ordinary characters while the user types (nothing is swallowed);
// pressing Enter removes that text and runs the selected command. Today there
// is exactly one entry (Add asset) but the plumbing is generic.
//
// The asset flow mirrors the graph's header-image picker: selecting the
// command hands a native file dialog over to main.rs (spawned on a worker
// thread so the app keeps redrawing). When a file comes back, main.rs copies
// it into the project's assets/ folder and publishes the relative target
// here; the editor input handler inserts `[[assets/name]]` on a new line below
// the caret.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    AddAsset,
}

static COMMANDS: &[(&str, Command)] = &[("Add asset", Command::AddAsset)];

pub struct CommandState {
    pub active: bool,
    pub filter: String,
    pub selected: usize,
    pub matches: Vec<(String, Command)>,
    // Mirrors the [[ autocomplete flags: ESC sets suppress (sticky until the
    // region content changes) and esc_consumed tells main's ESC handler that
    // the press was for the picker, not for closing the editor.
    pub suppress: bool,
    pub suppress_filter: String,
    pub esc_consumed: bool,
}

pub static COMMAND_PALETTE: RwLock<CommandState> = RwLock::new(CommandState {
    active: false,
    filter: String::new(),
    selected: 0,
    matches: Vec::new(),
    suppress: false,
    suppress_filter: String::new(),
    esc_consumed: false,
});

// Asset pick hand-off points between the input handler, main.rs and the rfd
// worker thread (same pattern as graph::processing's HEADER_PICK_*).
pub static ASSET_PICK_REQUEST: AtomicBool = AtomicBool::new(false);
pub static ASSET_PICK_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static ASSET_PICK_RESULT: RwLock<Option<PathBuf>> = RwLock::new(None);
// After the copy succeeds main.rs stores the project-relative target (e.g.
// "assets/pic_1.png"); the input handler consumes it next frame.
pub static ASSET_INSERT: RwLock<Option<String>> = RwLock::new(None);

// Commands matching `filter` (case-insensitive): a prefix of the name sorts
// first ("add"), then any substring of a word in the name ("asse" hits
// "Add asset" via "asset"). Empty filter returns every command.
pub fn refresh(filter: &str) -> Vec<(String, Command)> {
    let f = filter.to_lowercase();
    if f.is_empty() {
        return COMMANDS
            .iter()
            .map(|(n, c)| (n.to_string(), *c))
            .collect();
    }
    let mut prefix: Vec<(String, Command)> = Vec::new();
    let mut loose: Vec<(String, Command)> = Vec::new();
    for (name, cmd) in COMMANDS {
        let lower = name.to_lowercase();
        let hit = lower.starts_with(&f);
        let word_hit = !hit && lower.split_whitespace().any(|w| w.starts_with(&f));
        let contains_hit = !hit && !word_hit && lower.contains(&f);
        if hit {
            prefix.push((name.to_string(), *cmd));
        } else if word_hit {
            loose.push((name.to_string(), *cmd));
        } else if contains_hit {
            loose.push((name.to_string(), *cmd));
        }
    }
    // Whole commands before partial-word hits, keeping declaration order within
    // each bucket.
    prefix.extend(loose);
    prefix
}

// If the caret sits inside an unclosed word-initial "/" token, return the
// command filter: the text between the slash and the caret. Only the current
// word is considered; a slash in the middle of a word ("abc/def", "3/4" for a
// date or fraction) is prose, not a command, and a space after the slash ends
// the region (so everyday writing like "a/b" or "/ path" is left alone).
pub fn detect(line: &str, cursor_x: usize) -> Option<String> {
    let bytes = line.as_bytes();
    let upto = cursor_x.min(bytes.len());
    let mut tok_start = upto;
    while tok_start > 0 && !bytes[tok_start - 1].is_ascii_whitespace() {
        tok_start -= 1;
    }
    // The token must be non-empty and start with '/'.
    if tok_start >= upto || bytes[tok_start] != b'/' {
        return None;
    }
    Some(line[tok_start + 1..upto].to_string())
}

// Start the asset import: request the file dialog on the main thread.
pub fn request_asset_pick() {
    ASSET_PICK_REQUEST.store(true, std::sync::atomic::Ordering::Relaxed);
}

// Insert `[[target]]` as a fresh line directly below buffer line `y`,
// returning the new caret position (y, x). Pure so it can be unit-tested
// without raylib or the buffer globals.
pub fn insert_asset_link(buffer: &mut Vec<String>, y: usize, target: &str) -> (i32, i32) {
    let line = format!("[[{}]]", target);
    if buffer.is_empty() {
        buffer.push(line.clone());
        return (0, line.len() as i32);
    }
    let y = y.min(buffer.len() - 1);
    buffer.insert(y + 1, line.clone());
    (y as i32 + 1, line.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_word_initial_slash() {
        assert_eq!(detect("/add", 4), Some("add".to_string()));
        // A bare slash at line start is a region with an empty filter.
        assert_eq!(detect("/", 1), Some(String::new()));
        // Slash after leading whitespace.
        assert_eq!(detect("  /ass", 6), Some("ass".to_string()));
    }

    #[test]
    fn ignores_mid_word_and_prose_slashes() {
        // URL/date-style slashes in the middle of a word are not commands.
        assert_eq!(detect("a/b", 3), None);
        assert_eq!(detect("price 2026/09/12", 17), None);
        assert_eq!(detect("no slash here", 14), None);
    }

    #[test]
    fn space_after_slash_ends_region() {
        assert_eq!(detect("/ path", 6), None);
    }

    #[test]
    fn cursor_before_slash_is_not_a_command() {
        assert_eq!(detect("ab/cd", 1), None);
    }

    #[test]
    fn refresh_filters_by_prefix() {
        let all = refresh("");
        assert_eq!(all, vec![("Add asset".to_string(), Command::AddAsset)]);
        // Case-insensitive prefix.
        let m = refresh("add");
        assert_eq!(m, vec![("Add asset".to_string(), Command::AddAsset)]);
        let m2 = refresh("AD");
        assert_eq!(m2, vec![("Add asset".to_string(), Command::AddAsset)]);
        // Any word in the name also hits ("asse" ~ "asset"), so the command
        // stays up while the user types toward it.
        assert_eq!(refresh("asse"), vec![("Add asset".to_string(), Command::AddAsset)]);
        // Substring anywhere still matches.
        assert_eq!(refresh("set"), vec![("Add asset".to_string(), Command::AddAsset)]);
        // No match → empty (Enter then does nothing but still cleans the text).
        assert!(refresh("zzz").is_empty());
    }

    #[test]
    fn open_clears_filter_and_selection() {
        let mut st = COMMAND_PALETTE.write().unwrap();
        st.active = true;
        st.filter = "stale".into();
        st.selected = 0;
        st.matches = refresh("add");
        // Opening (what the input handler does on "/") wipes the old filter
        // and shows every command.
        st.filter.clear();
        st.selected = 0;
        st.matches = refresh("");
        assert!(st.active);
        assert!(st.filter.is_empty());
        assert_eq!(st.matches.len(), 1);
        assert_eq!(st.matches, vec![("Add asset".to_string(), Command::AddAsset)]);
    }

    #[test]
    fn insert_makes_own_line_below_caret() {
        let mut buf = vec!["sentence".to_string(), "".to_string()];
        let caret = insert_asset_link(&mut buf, 0, "assets/pic.png");
        assert_eq!(buf, vec!["sentence", "[[assets/pic.png]]", ""]);
        assert_eq!(caret, (1, "[[assets/pic.png]]".len() as i32));
    }

    #[test]
    fn insert_before_existing_below_line() {
        let mut buf = vec!["top".to_string(), "existing below".to_string()];
        let caret = insert_asset_link(&mut buf, 0, "assets/pdf");
        assert_eq!(buf, vec!["top", "[[assets/pdf]]", "existing below"]);
        assert_eq!(caret, (1, "[[assets/pdf]]".len() as i32));
    }

    #[test]
    fn insert_clamped_for_empty_buffer() {
        let mut buf: Vec<String> = Vec::new();
        let caret = insert_asset_link(&mut buf, 0, "assets/a.txt");
        assert_eq!(buf, vec!["[[assets/a.txt]]"]);
        assert_eq!(caret, (0, "[[assets/a.txt]]".len() as i32));
    }
}