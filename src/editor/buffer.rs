use std::sync::Mutex;

pub static BUFFER: Mutex<Vec<String>> = Mutex::new(Vec::new());
pub static CURSOR_X: Mutex<i32> = Mutex::new(0);
pub static CURSOR_Y: Mutex<i32> = Mutex::new(0);
