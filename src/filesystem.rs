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

pub fn parse_links(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;

    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find the closing "]"
            let mut j = i + 2;
            while j + 1 < bytes.len() && !(bytes[j] == b']' && bytes[j + 1] == b']') {
                j += 1;
            }
            if j + 1 < bytes.len() && bytes[j] == b']' && bytes[j + 1] == b']' {
                let inner: String = content[i + 2..j].trim().to_string();
                if !inner.is_empty() {
                    links.push(inner);
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }

    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wikilinks() {
        let content = "see [[alpha]] and [[ beta ]] plus [[alpha]] again";
        assert_eq!(parse_links(content), vec!["alpha", "beta", "alpha"]);
    }

    #[test]
    fn ignores_unclosed_and_bare_brackets() {
        assert_eq!(parse_links("no [[links here"), Vec::<String>::new());
        assert_eq!(parse_links("[single] and plain text"), Vec::<String>::new());
        assert_eq!(parse_links("empty [[]]"), Vec::<String>::new());
    }
}
