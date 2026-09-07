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

pub fn create_dir(path: &Path) {
    let _ = fs::create_dir_all(path);
}

pub fn is_dir(path: &Path) -> bool {
    path.is_dir()
}

// The companion sub-graph folder for a note: same directory, same stem,
// no extension. `note.md` owns the nested graph in `note/`.
pub fn subgraph_dir(note_path: &Path) -> PathBuf {
    let stem = note_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    note_path.parent().unwrap_or(Path::new(".")).join(stem)
}

pub fn rename_dir(old: &Path, new_name: &str) -> bool {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return false;
    }
    if let Some(dir) = old.parent() {
        let new_path = dir.join(new_name);
        if new_path.exists() {
            return false;
        }
        return fs::rename(old, &new_path).is_ok();
    }
    false
}

pub fn rename_file(old: &Path, new_stem: &str) -> bool {
    let new_stem = new_stem.trim();
    if new_stem.is_empty() {
        return false;
    }
    let stem = new_stem.trim_end_matches(".md");
    if stem.is_empty() {
        return false;
    }
    if let Some(dir) = old.parent() {
        let new_path = dir.join(format!("{}.md", stem));
        if new_path.exists() {
            return false;
        }
        return fs::rename(old, &new_path).is_ok();
    }
    false
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
    fn renames_file_and_appends_extension() {
        let dir = std::env::temp_dir().join("rg_rename_test");
        let _ = std::fs::create_dir_all(&dir);
        let old = dir.join("old.md");
        std::fs::write(&old, "hi").unwrap();

        assert!(rename_file(&old, "new"));
        assert!(dir.join("new.md").exists());
        assert!(!old.exists());

        // Sanitizes a trailing explicit extension.
        std::fs::write(&dir.join("a.md"), "x").unwrap();
        assert!(rename_file(&dir.join("a.md"), "b.md"));
        assert!(dir.join("b.md").exists());

        // Rejects empty names.
        assert!(!rename_file(&dir.join("b.md"), "   "));
        // Rejects colliding with an existing file.
        std::fs::write(&dir.join("c.md"), "x").unwrap();
        std::fs::write(&dir.join("b.md"), "x").unwrap();
        assert!(!rename_file(&dir.join("c.md"), "b"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_unclosed_and_bare_brackets() {
        assert_eq!(parse_links("no [[links here"), Vec::<String>::new());
        assert_eq!(parse_links("[single] and plain text"), Vec::<String>::new());
        assert_eq!(parse_links("empty [[]]"), Vec::<String>::new());
    }

    #[test]
    fn creates_and_detects_directories() {
        let dir = std::env::temp_dir().join("rg_is_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!is_dir(&dir));
        create_dir(&dir);
        assert!(is_dir(&dir));
        // A nested path also counts as a directory once created.
        let child = dir.join("sub");
        create_dir(&child);
        assert!(is_dir(&child));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subgraph_dir_names_folder_after_note_stem() {
        let dir = std::env::temp_dir().join("rg_subgraph_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        create_dir(&dir);

        let note = dir.join("alpha.md");
        assert_eq!(subgraph_dir(&note), dir.join("alpha"));

        // Creating that companion folder makes the note "have" a sub-graph.
        create_dir(&dir.join("alpha"));
        assert!(is_dir(&subgraph_dir(&note)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renames_directories_same_folder() {
        let dir = std::env::temp_dir().join("rg_rename_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        create_dir(&dir);
        create_dir(&dir.join("old"));

        assert!(rename_dir(&dir.join("old"), "new"));
        assert!(is_dir(&dir.join("new")));
        assert!(!dir.join("old").exists());

        // Rejects empty names.
        assert!(!rename_dir(&dir.join("new"), "  "));
        // Rejects colliding with an existing folder.
        create_dir(&dir.join("other"));
        assert!(!rename_dir(&dir.join("new"), "other"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
