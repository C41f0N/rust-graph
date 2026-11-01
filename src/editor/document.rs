// Bounded file loading for the editor.
//
// Opening a note used to mean "read the whole file into a Vec<String> on the
// UI thread". That is fine for a note and catastrophic for a stray log dump: a
// huge file blocks the frame, and the renderer's per-line layout makes every
// subsequent frame expensive too. This module is the policy + bounded reader
// seam:
//
//   * small files      -> full editable load
//   * large files      -> one bounded chunk at a time, read-only preview
//   * enormous files   -> refused before the disk is touched
//
// Everything here is pure enough to unit-test without raylib: it only uses
// std::fs/std::io and plain strings.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::config;

// What `classify` decided to do with a path, before reading its contents.
// `bytes` is the on-disk size from metadata (0 for a not-yet-created note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenClass {
    Editable { bytes: u64 },
    Preview { bytes: u64 },
}

// A full editable read either fits the budget or has to be downgraded to the
// chunked preview path (e.g. a 300 KiB file containing one 200 KiB line).
#[derive(Debug)]
pub enum EditableLoad {
    Lines(Vec<String>),
    NeedsPreview,
}

// One bounded preview page. `next_byte` is the byte offset at the start of the
// following page (always a line boundary), so a page can be re-read later
// without scanning from the beginning of the file.
#[derive(Debug)]
pub struct PreviewChunk {
    pub lines: Vec<String>,
    pub next_byte: u64,
    pub eof: bool,
    pub truncated: bool,
}

// Reasons an open is refused. Sizes are user-facing; IO errors keep the
// underlying message so a permission problem is not silently turned into an
// empty document (the old `read_file` behaviour).
#[derive(Debug)]
pub enum OpenError {
    TooLarge { bytes: u64, limit: u64 },
    Io { path: PathBuf, message: String },
    NotText { path: PathBuf },
    NotAFile { path: PathBuf },
}

impl OpenError {
    fn io(path: &Path, error: io::Error) -> OpenError {
        OpenError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }

    // A one-line, user-facing description for a toast. Size errors do not
    // carry a path, so add the file name here; IO/encoding errors already do.
    pub fn describe(&self, path: &Path) -> String {
        match self {
            OpenError::TooLarge { .. } => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                format!("{name}: {self}")
            }
            _ => self.to_string(),
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::TooLarge { bytes, limit } => write!(
                f,
                "file is {}; the limit is {}",
                human_bytes(*bytes),
                human_bytes(*limit)
            ),
            OpenError::Io { path, message } => {
                write!(f, "cannot read {}: {message}", path.display())
            }
            OpenError::NotText { path } => {
                write!(f, "{} is not valid UTF-8 text", path.display())
            }
            OpenError::NotAFile { path } => {
                write!(f, "{} is not a regular file", path.display())
            }
        }
    }
}

// A compact human-readable byte count for the preview banner and refusal
// toast. One decimal place, no trailing ".0".
pub fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    let (value, unit) = if bytes >= GIB {
        (bytes as f64 / GIB as f64, "GiB")
    } else if bytes >= MIB {
        (bytes as f64 / MIB as f64, "MiB")
    } else if bytes >= KIB {
        (bytes as f64 / KIB as f64, "KiB")
    } else {
        return format!("{bytes} B");
    };
    if (value - value.round()).abs() < 0.05 {
        format!("{:.0} {unit}", value)
    } else {
        format!("{value:.1} {unit}")
    }
}

// Decide the policy tier from metadata only. A missing path is not an error:
// journal notes and ghost notes are legitimately created lazily and open as an
// empty editable document. A path that exists but is not a regular file is
// refused.
pub fn classify(path: &Path) -> Result<OpenClass, OpenError> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OpenClass::Editable { bytes: 0 });
        }
        Err(error) => return Err(OpenError::io(path, error)),
    };
    if !meta.is_file() {
        return Err(OpenError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    let bytes = meta.len();
    if bytes > config::EDITOR_MAX_PREVIEW_BYTES {
        return Err(OpenError::TooLarge {
            bytes,
            limit: config::EDITOR_MAX_PREVIEW_BYTES,
        });
    }
    if bytes > config::EDITOR_MAX_EDIT_BYTES {
        Ok(OpenClass::Preview { bytes })
    } else {
        Ok(OpenClass::Editable { bytes })
    }
}

// Read a whole file for the editable path, enforcing the byte, line-count and
// per-line limits while reading. The moment a limit is crossed the read stops
// and the caller switches to the preview path - the file is never buffered
// without a bound.
pub fn read_editable(path: &Path) -> Result<EditableLoad, OpenError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A pending note: keep the blank-document behaviour.
            return Ok(EditableLoad::Lines(Vec::new()));
        }
        Err(error) => return Err(OpenError::io(path, error)),
    };
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut bytes = 0u64;

    while let Some(line) = read_line_bounded(&mut reader, config::EDITOR_MAX_EDIT_LINE_BYTES, path)?
    {
        if line.truncated {
            return Ok(EditableLoad::NeedsPreview);
        }
        bytes = bytes.saturating_add(line.consumed);
        if bytes > config::EDITOR_MAX_EDIT_BYTES || lines.len() >= config::EDITOR_MAX_EDIT_LINES {
            return Ok(EditableLoad::NeedsPreview);
        }
        lines.push(line.text);
    }
    Ok(EditableLoad::Lines(lines))
}

// Read one bounded preview page starting at `start_byte` (a line boundary).
// Each line is clipped at the preview line limit, and the remainder of an
// overlong line is consumed without being stored, so a single 500 MiB line
// cannot blow up the page.
pub fn read_chunk(path: &Path, start_byte: u64) -> Result<PreviewChunk, OpenError> {
    let mut file = File::open(path).map_err(|error| OpenError::io(path, error))?;
    if start_byte > 0 {
        file.seek(SeekFrom::Start(start_byte))
            .map_err(|error| OpenError::io(path, error))?;
    }
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut bytes = 0u64;
    let mut eof = false;
    let mut truncated = false;

    while lines.len() < config::EDITOR_PREVIEW_CHUNK_LINES
        && bytes < config::EDITOR_PREVIEW_CHUNK_BYTES
    {
        let Some(mut line) =
            read_line_bounded(&mut reader, config::EDITOR_PREVIEW_LINE_BYTES, path)?
        else {
            eof = true;
            break;
        };
        bytes = bytes.saturating_add(line.consumed);
        if line.truncated {
            truncated = true;
            line.text.push_str(" \u{2026} [line truncated]");
        }
        lines.push(line.text);
        eof = line.at_eof;
        if eof {
            break;
        }
    }

    Ok(PreviewChunk {
        lines,
        next_byte: start_byte.saturating_add(bytes),
        eof,
        truncated,
    })
}

// --------------------------------------------------------------------------
// Bounded line reader
// --------------------------------------------------------------------------

struct BoundedLine {
    text: String,
    // Bytes consumed from the file, including the line terminator.
    consumed: u64,
    // The line had more content than the cap; the remainder was drained.
    truncated: bool,
    // No further bytes follow this line.
    at_eof: bool,
}

// Read one line, storing at most `keep` content bytes. The newline is consumed
// either way; an overlong line is drained in `fill_buf`-sized slices instead
// of being allocated in full. `\n` is stripped and a `\r` immediately before
// it is stripped too, matching `str::lines()`.
fn read_line_bounded(
    reader: &mut impl BufRead,
    keep: usize,
    path: &Path,
) -> Result<Option<BoundedLine>, OpenError> {
    let mut data: Vec<u8> = Vec::new();
    let mut consumed = 0u64;
    let mut truncated = false;
    let mut saw_any = false;
    let mut newline_found = false;

    loop {
        let buf = reader
            .fill_buf()
            .map_err(|error| OpenError::io(path, error))?;
        if buf.is_empty() {
            break;
        }
        saw_any = true;

        let newline = buf.iter().position(|&b| b == b'\n');
        let content_len = newline.unwrap_or(buf.len());
        let content = &buf[..content_len];

        if data.len() < keep {
            let room = keep - data.len();
            let take = content.len().min(room);
            data.extend_from_slice(&content[..take]);
            if take < content.len() {
                truncated = true;
            }
        } else if !content.is_empty() {
            truncated = true;
        }

        let consume_len = content_len + usize::from(newline.is_some());
        consumed = consumed.saturating_add(consume_len as u64);
        reader.consume(consume_len);

        if newline.is_some() {
            newline_found = true;
            break;
        }
    }

    if !saw_any {
        return Ok(None);
    }

    // A CR immediately before the consumed LF belongs to the terminator.
    if newline_found && !truncated && data.last() == Some(&b'\r') {
        data.pop();
    }

    let text = decode_line(&data, truncated, path)?;

    let at_eof = if newline_found {
        reader
            .fill_buf()
            .map_err(|error| OpenError::io(path, error))?
            .is_empty()
    } else {
        true
    };

    Ok(Some(BoundedLine {
        text,
        consumed,
        truncated,
        at_eof,
    }))
}

// Decode the stored bytes as UTF-8. A truncated line may end in the middle of
// a multi-byte character: only that tail is dropped. Invalid bytes earlier in
// the line mean the file is not a text file and is refused.
fn decode_line(bytes: &[u8], truncated: bool, path: &Path) -> Result<String, OpenError> {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Ok(text),
        Err(error) => {
            if truncated {
                let valid = error.utf8_error().valid_up_to();
                // More than a partial trailing codepoint (max 3 bytes) means
                // the input is genuinely invalid, not cut by our cap.
                if bytes.len().saturating_sub(valid) <= 3 {
                    return Ok(String::from_utf8_lossy(&bytes[..valid]).into_owned());
                }
            }
            Err(OpenError::NotText {
                path: path.to_path_buf(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rg_document_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn classifies_by_metadata_size() {
        let dir = temp_dir("classify");
        let small = dir.join("small.md");
        write(&small, b"hello");
        assert_eq!(classify(&small).unwrap(), OpenClass::Editable { bytes: 5 });

        let large = dir.join("large.md");
        File::create(&large)
            .unwrap()
            .set_len(config::EDITOR_MAX_EDIT_BYTES + 1)
            .unwrap();
        assert_eq!(
            classify(&large).unwrap(),
            OpenClass::Preview {
                bytes: config::EDITOR_MAX_EDIT_BYTES + 1
            }
        );

        let huge = dir.join("huge.md");
        File::create(&huge)
            .unwrap()
            .set_len(config::EDITOR_MAX_PREVIEW_BYTES + 1)
            .unwrap();
        assert!(matches!(classify(&huge), Err(OpenError::TooLarge { .. })));

        // A missing note is a legal pending document.
        assert_eq!(
            classify(&dir.join("pending.md")).unwrap(),
            OpenClass::Editable { bytes: 0 }
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_lines_with_crlf_and_trailing_newline() {
        let dir = temp_dir("crlf");
        let path = dir.join("a.md");
        write(&path, b"one\r\ntwo\n\n");
        let EditableLoad::Lines(lines) = read_editable(&path).unwrap() else {
            panic!("small file must stay editable");
        };
        assert_eq!(lines, vec!["one", "two", ""]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn downgrades_when_line_cap_is_crossed() {
        let dir = temp_dir("line_cap");
        let path = dir.join("long.md");
        write(&path, &vec![b'x'; config::EDITOR_MAX_EDIT_LINE_BYTES + 1]);
        assert!(matches!(
            read_editable(&path).unwrap(),
            EditableLoad::NeedsPreview
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_chunks_advance_at_line_boundaries() {
        let dir = temp_dir("chunks");
        let path = dir.join("big.md");
        let mut content = String::new();
        for i in 0..12 {
            content.push_str(&format!("line {i}\n"));
        }
        write(&path, content.as_bytes());

        // All 12 lines fit in one page, so the next offset is exactly EOF.
        let first = read_chunk(&path, 0).unwrap();
        assert_eq!(first.lines.first().map(String::as_str), Some("line 0"));
        assert_eq!(first.lines.len(), 12);
        assert!(first.eof);
        assert_eq!(first.next_byte, content.len() as u64);

        // Reading from EOF is a valid empty page, not an error.
        let empty = read_chunk(&path, first.next_byte).unwrap();
        assert!(empty.lines.is_empty());
        assert!(empty.eof);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_clips_an_overlong_line_and_keeps_going() {
        let dir = temp_dir("long_preview");
        let path = dir.join("one-line.md");
        let mut content = vec![b'a'; config::EDITOR_PREVIEW_LINE_BYTES + 100];
        content.extend_from_slice(b"\nshort\n");
        write(&path, &content);

        let chunk = read_chunk(&path, 0).unwrap();
        assert!(chunk.truncated);
        assert_eq!(chunk.lines.len(), 2);
        assert!(chunk.lines[0].ends_with("[line truncated]"));
        assert_eq!(chunk.lines[1], "short");
        assert!(chunk.eof);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_utf8_is_refused_not_silently_emptied() {
        let dir = temp_dir("invalid");
        let path = dir.join("bad.md");
        write(&path, &[0xff, 0xfe, 0xfd]);
        assert!(matches!(
            read_editable(&path),
            Err(OpenError::NotText { .. })
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn human_bytes_is_compact() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1 MiB");
    }
}
