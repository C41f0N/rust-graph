use std::sync::Mutex;

pub static BUFFER: Mutex<String> = Mutex::new(String::new());
pub static CURSOR_INDEX: Mutex<i32> = Mutex::new(0);
pub static CURSOR_LINE: Mutex<i32> = Mutex::new(0);
