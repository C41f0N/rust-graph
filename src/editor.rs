use std::sync::atomic::{AtomicBool, AtomicU64};

// Set by the X close button to request the editor close.
pub static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

// The buffer has unsaved edits since the last save.
pub static DIRTY: AtomicBool = AtomicBool::new(false);

// Timestamp (raylib GetTime seconds, scaled to millis) of the most recent
// edit, used to autosave after a typing pause (no edits for a while).
pub static LAST_EDIT_MILLIS: AtomicU64 = AtomicU64::new(0);

pub mod autocomplete;
pub mod blocks;
pub mod buffer;
pub mod images;
pub mod input_handler;
pub mod markdown;
pub mod renderer;
pub mod text;
