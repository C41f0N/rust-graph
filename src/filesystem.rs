use std::fs;
use std::path::{Path, PathBuf};

pub fn scan_directory(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" {
                        files.push(path);
                    }
                }
            }
        }
    }
    files.sort();
    files
}

pub fn read_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

pub fn write_file(path: &Path, content: &str) {
    let _ = fs::write(path, content);
}

pub fn create_file(path: &Path, content: &str) {
    let _ = fs::write(path, content);
}

pub fn delete_file(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn unique_filename(dir: &Path, stem: &str) -> String {
    let candidate = format!("{}.md", stem);
    if !dir.join(&candidate).exists() {
        return candidate;
    }
    for i in 1.. {
        let candidate = format!("{}_{}.md", stem, i);
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    unreachable!()
}
