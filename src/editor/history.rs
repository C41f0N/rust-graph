use std::collections::VecDeque;
use std::sync::Mutex;

// Maximum number of undo (or redo) steps kept. The oldest entry is dropped
// once the cap is exceeded.
const MAX_ENTRIES: usize = 1000;

// Two plain character inserts coalesce into one undo step when they happen
// within this window at an unmoved caret (an uninterrupted typing burst), so
// Ctrl+Z rolls back a whole word instead of one character at a time.
const COALESCE_MS: u64 = 500;

#[derive(Clone, PartialEq, Debug)]
pub struct HistEntry {
    pub lines: Vec<String>,
    pub cursor: (i32, i32),
}

// What kind of edit is about to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    // A single typed character inserted at the caret.
    CharInsert,
    // Everything else: paste, delete, backspace, tab, autocomplete accept...
    Other,
}

// Two-stack undo history. Entries hold whole-buffer snapshots (the caller
// already holds the BUFFER lock, so every function takes the state by value
// rather than re-locking the buffer).
static UNDO: Mutex<VecDeque<HistEntry>> = Mutex::new(VecDeque::new());
static REDO: Mutex<VecDeque<HistEntry>> = Mutex::new(VecDeque::new());

// Coalescing burst marker: the caret position right after the most recent
// accepted character insert, and when it happened.
static RUN: Mutex<Option<(i32, i32, u64)>> = Mutex::new(None);

pub fn clear() {
    UNDO.lock().unwrap().clear();
    REDO.lock().unwrap().clear();
    *RUN.lock().unwrap() = None;
}

// Lift the whole undo/redo state out (for detaching it from the active tab
// when the editor switches documents). Returns ownership so the caller can
// store the stacks on the tab being left.
pub fn take() -> (VecDeque<HistEntry>, VecDeque<HistEntry>, Option<(i32, i32, u64)>) {
    let mut undo = UNDO.lock().unwrap();
    let mut redo = REDO.lock().unwrap();
    let mut run = RUN.lock().unwrap();
    let out = (std::mem::take(&mut *undo), std::mem::take(&mut *redo), *run);
    *run = None;
    out
}

// Reattach history state previously taken by `take()` (the tab being entered).
pub fn put(
    undo: VecDeque<HistEntry>,
    redo: VecDeque<HistEntry>,
    run: Option<(i32, i32, u64)>,
) {
    *UNDO.lock().unwrap() = undo;
    *REDO.lock().unwrap() = redo;
    *RUN.lock().unwrap() = run;
}

// Record that the user is about to edit `state` (the caller's locked buffer)
// with `caret_before` as the caret before the edit. `kind` and `now_ms` drive
// typing-burst coalescing. Whether the edit turns out to be a no-op at the
// buffer level is not known here, so a rare no-op undo step is accepted.
pub fn snapshot(state: &[String], caret_before: (i32, i32), kind: EditKind, now_ms: u64) {
    let mut run = RUN.lock().unwrap();
    if kind == EditKind::CharInsert {
        // A character insert extends the open burst when the caret has not
        // moved since the last accepted character and little time has passed.
        // Anything else (arrow key, mouse click, paste) breaks the burst.
        let continuing = match *run {
            Some((y, x, at)) => {
                y == caret_before.0
                    && x == caret_before.1
                    && now_ms.saturating_sub(at) <= COALESCE_MS
            }
            None => false,
        };
        // The caret advances by exactly one column per typed character (the
        // editor tracks byte/char offsets arithmetically), wherever the code
        // point that lands ends up byte-wise.
        *run = Some((caret_before.0, caret_before.1 + 1, now_ms));
        if !continuing {
            push(state, caret_before);
        }
    } else {
        *run = None;
        push(state, caret_before);
    }
}

fn push(state: &[String], caret: (i32, i32)) {
    let mut undo = UNDO.lock().unwrap();
    undo.push_back(HistEntry {
        lines: state.to_vec(),
        cursor: caret,
    });
    while undo.len() > MAX_ENTRIES {
        undo.pop_front();
    }
    REDO.lock().unwrap().clear();
}

// Pop the most recent pre-edit state. The caller's current state is pushed
// onto the redo stack so the undo can be reversed. Returns None when there is
// nothing to undo.
pub fn undo(current: &[String], caret: (i32, i32)) -> Option<HistEntry> {
    *RUN.lock().unwrap() = None;
    let mut undo = UNDO.lock().unwrap();
    let prev = undo.pop_back()?;
    let mut redo = REDO.lock().unwrap();
    redo.push_back(HistEntry {
        lines: current.to_vec(),
        cursor: caret,
    });
    while redo.len() > MAX_ENTRIES {
        redo.pop_front();
    }
    Some(prev)
}

// Reverse an undo: pop the most recent undone state and push the current
// state back onto the undo stack. Returns None when there is nothing to
// redo. `current` is the caller's currently locked state.
pub fn redo(current: &[String], caret: (i32, i32)) -> Option<HistEntry> {
    *RUN.lock().unwrap() = None;
    let mut redo = REDO.lock().unwrap();
    let next = redo.pop_back()?;
    let mut undo = UNDO.lock().unwrap();
    undo.push_back(HistEntry {
        lines: current.to_vec(),
        cursor: caret,
    });
    while undo.len() > MAX_ENTRIES {
        undo.pop_front();
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The stacks are global statics; serialize history tests so parallel
    // test threads cannot interleave snapshots/undos into each other.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn undo_restores_snapshot() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["hello".to_string()], (0, 5), EditKind::Other, 0);
        let u = undo(&["hello!".to_string()], (0, 6)).unwrap();
        assert_eq!(u.lines, vec!["hello".to_string()]);
        assert_eq!(u.cursor, (0, 5));
    }

    #[test]
    fn redo_reverses_undo() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["a".to_string()], (0, 1), EditKind::Other, 0);
        let u = undo(&["ab".to_string()], (0, 2)).unwrap();
        assert_eq!(u.lines, vec!["a".to_string()]);
        let r = redo(&["a".to_string()], (0, 1)).unwrap();
        assert_eq!(r.lines, vec!["ab".to_string()]);
        assert_eq!(r.cursor, (0, 2));
    }

    #[test]
    fn empty_stacks_return_none() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        assert!(undo(&["x".to_string()], (0, 1)).is_none());
        assert!(redo(&["x".to_string()], (0, 1)).is_none());
    }

    #[test]
    fn new_edit_clears_redo() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["a".to_string()], (0, 1), EditKind::Other, 0);
        undo(&["ab".to_string()], (0, 2));
        // A new edit after an undo discards the redo stack.
        snapshot(&["ab".to_string()], (0, 2), EditKind::Other, 0);
        assert!(redo(&["abc".to_string()], (0, 3)).is_none());
    }

    #[test]
    fn char_burst_coalesces_into_one_undo_step() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["".to_string()], (0, 0), EditKind::CharInsert, 0);
        snapshot(&["a".to_string()], (0, 1), EditKind::CharInsert, 100);
        snapshot(&["ab".to_string()], (0, 2), EditKind::CharInsert, 250);
        // One Ctrl+Z removes the whole "ab" run in a single step.
        let u = undo(&["ab".to_string()], (0, 2)).unwrap();
        assert_eq!(u.lines, vec!["".to_string()]);
        assert_eq!(u.cursor, (0, 0));
    }

    #[test]
    fn caret_move_breaks_burst() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["a".to_string()], (0, 1), EditKind::CharInsert, 0);
        // Typing continues on another line: a fresh snapshot is needed.
        snapshot(&["a".to_string(), "b".to_string()], (1, 1), EditKind::CharInsert, 50);
        let u = undo(
            &["a".to_string(), "b".to_string()],
            (1, 1),
        )
        .unwrap();
        assert_eq!(u.lines, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(u.cursor, (1, 1));
    }

    #[test]
    fn time_gap_breaks_burst() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["".to_string()], (0, 0), EditKind::CharInsert, 0);
        // 10 s later the same position starts a new burst and records its own
        // pre-state, so undoing once lands at "b" rather than the run start.
        snapshot(&["b".to_string()], (0, 1), EditKind::CharInsert, 10_000);
        let u = undo(&["b".to_string()], (0, 1)).unwrap();
        assert_eq!(u.lines, vec!["b".to_string()]);
        assert_eq!(u.cursor, (0, 1));
    }

    #[test]
    fn other_edit_breaks_burst_and_snapshots() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["a".to_string()], (0, 1), EditKind::CharInsert, 0);
        snapshot(&["ab".to_string()], (0, 2), EditKind::Other, 10);
        let u = undo(&["abc".to_string()], (0, 3)).unwrap();
        assert_eq!(u.lines, vec!["ab".to_string()]);
        assert_eq!(u.cursor, (0, 2));
    }

    #[test]
    fn undo_then_type_starts_fresh_burst() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        snapshot(&["".to_string()], (0, 0), EditKind::CharInsert, 0);
        snapshot(&["a".to_string()], (0, 1), EditKind::CharInsert, 100);
        // Undo, then keep typing at the restored caret: the burst must not
        // absorb the character typed after the undo.
        undo(&["a".to_string()], (0, 1));
        snapshot(&["".to_string()], (0, 0), EditKind::CharInsert, 200);
        let u = undo(&["x".to_string()], (0, 1)).unwrap();
        assert_eq!(u.lines, vec!["".to_string()]);
        assert_eq!(u.cursor, (0, 0));
    }

    #[test]
    fn cap_evicts_oldest() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        let over = MAX_ENTRIES + 5;
        for i in 0..over {
            let caret = (0, i as i32);
            snapshot(&[format!("l{i}")], caret, EditKind::Other, i as u64);
        }
        // The newest entry is the first one popped...
        let first = undo(&["tail".to_string()], (0, 0)).unwrap();
        assert_eq!(first.lines, vec![format!("l{}", over - 1)]);
        // ...and only MAX_ENTRIES survive the eviction (the newest was popped
        // above, so the loop can pop the remaining MAX_ENTRIES - 1).
        let mut count = 1;
        while undo(&["x".to_string()], (0, 0)).is_some() {
            count += 1;
        }
        assert_eq!(count, MAX_ENTRIES);
    }
}