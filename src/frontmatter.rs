// Leading YAML-subset frontmatter block: `---\n...\n---` at the very start of
// a note (byte 0). It holds note-level metadata as plain text inside the .md
// file so it round-trips through the editor untouched. Today the only
// meaningful key is `header`, which points another file/asset as this note's
// header; future keys (title, color, ...) slot into the same `fields` list.
//
// The delimiter `---` only means frontmatter when it is the FIRST line and is
// closed later; a lone `---` anywhere else remains a horizontal rule.

#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
    // Line index of the closing `---`. The block spans 0..=end_line.
    pub end_line: usize,
    // Byte offset just past the closing delimiter (for content slicing).
    // Only filled in by parse(); parse_lines() leaves it 0.
    pub end_byte: usize,
    // Every `key: value` pair in order (values trimmed, brackets kept).
    pub fields: Vec<(String, String)>,
    // The `header` target: a `[[wikilink]]` is unwrapped to its inner path,
    // otherwise the bare value as written.
    pub header: Option<String>,
}

// True for a frontmatter delimiter line: `---` (trailing whitespace allowed,
// and a UTF-8 BOM on the very first byte).
fn delimiter(line: &str) -> bool {
    line.trim().trim_start_matches('\u{feff}').trim() == "---"
}

fn unwrap_link(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix("[[") {
        if let Some(inner) = inner.strip_suffix("]]") {
            return inner.trim().to_string();
        }
    }
    value.to_string()
}

fn parse_slice(lines: &[&str]) -> Option<Frontmatter> {
    if !delimiter(lines.first()?) {
        return None;
    }

    let mut end_line = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if delimiter(line) {
            end_line = Some(i);
            break;
        }
    }
    let end_line = end_line?;

    let mut fields = Vec::new();
    for line in &lines[1..end_line] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim().to_string();
            let value = line[idx + 1..].trim().to_string();
            if !key.is_empty() {
                fields.push((key, value));
            }
        }
    }

    let header = fields
        .iter()
        .find(|(key, _)| key == "header")
        .map(|(_, value)| unwrap_link(value));

    Some(Frontmatter {
        end_line,
        end_byte: 0,
        fields,
        header,
    })
}

// Parse frontmatter from already-split editor buffer lines (the editor works
// on a Vec<String>, so this avoids re-joining). Returns None when the file
// does not start with a closed frontmatter block.
pub fn parse_lines(lines: &[String]) -> Option<Frontmatter> {
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    parse_slice(&refs)
}

// Parse frontmatter from raw file content. On top of parse_lines() this also
// records end_byte, the byte offset just past the closing delimiter, so the
// caller can slice `body` (the part after frontmatter) for content parsing.
pub fn parse(content: &str) -> Option<Frontmatter> {
    let refs: Vec<&str> = content.lines().collect();
    let mut fm = parse_slice(&refs)?;
    let mut off = 0;
    for (i, line) in refs.iter().enumerate() {
        off += line.len();
        if i == fm.end_line {
            break;
        }
        off += 1; // '\n'
    }
    fm.end_byte = off + 1; // past the closing delimiter's newline
    Some(fm)
}

fn join_frontmatter(lines: &[String], original: &str) -> String {
    let mut out = lines.join("\n");
    if original.ends_with('\n') {
        out.push('\n');
    }
    out
}

// Update (or insert) the `header:` field in the note's frontmatter, creating
// a frontmatter block when the note has none. `raw_value` is stored verbatim;
// callers usually pass a `[[wikilink]]`-wrapped target.
pub fn upsert_header(content: &str, raw_value: &str) -> String {
    let has_fm = content
        .lines()
        .next()
        .map_or(false, |l| delimiter(l.trim_start_matches('\u{feff}')));
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    if has_fm {
        let mut end = None;
        for (i, l) in lines.iter().enumerate().skip(1) {
            if delimiter(l) {
                end = Some(i);
                break;
            }
        }
        if let Some(end) = end {
            // Find an existing `header:` line (indices 1..end are the body
            // of the block).
            let mut header_pos = None;
            for (i, l) in lines.iter().enumerate().skip(1).take(end.saturating_sub(1)) {
                let trimmed = l.trim();
                if trimmed == "header:" || trimmed.strip_prefix("header:").is_some() {
                    header_pos = Some(i);
                    break;
                }
            }
            let new_line = format!("header: {}", raw_value);
            match header_pos {
                Some(pos) => lines[pos] = new_line,
                None => lines.insert(end, new_line),
            }
            return join_frontmatter(&lines, content);
        }
    }

    // No frontmatter block: prepend one.
    format!("---\nheader: {}\n---\n{}", raw_value, content)
}

// Inclusive range (start, end) of buffer line indices the frontmatter block
// occupies, when the buffer starts with a closed block. Used by the editor to
// treat the whole block as one editing unit (and hide it as one bar).
pub fn line_range(lines: &[String]) -> Option<(usize, usize)> {
    let first = lines.first()?;
    if !delimiter(first) {
        return None;
    }
    for (i, line) in lines.iter().enumerate().skip(1) {
        if delimiter(line) {
            return Some((0, i));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_header_wikilink() {
        let fm = parse_lines(&lines(&[
            "---",
            "header: [[assets/banner.png]]",
            "---",
            "# Title",
        ]))
        .unwrap();
        assert_eq!(fm.header.as_deref(), Some("assets/banner.png"));
        assert_eq!(fm.end_line, 2);
        assert_eq!(
            fm.fields,
            vec![("header".to_string(), "[[assets/banner.png]]".to_string())]
        );
    }

    #[test]
    fn parses_bare_and_multiple_fields() {
        let fm = parse_lines(&lines(&[
            "---",
            "title: Project Hub",
            "header: index.md",
            "---",
        ]))
        .unwrap();
        assert_eq!(fm.header.as_deref(), Some("index.md"));
        assert_eq!(fm.fields.len(), 2);
    }

    #[test]
    fn no_block_means_none() {
        assert!(parse_lines(&lines(&["# Title", "body"])).is_none());
        assert!(parse_lines(&lines(&[])).is_none());
        // A lone HR at the top is not frontmatter.
        assert!(parse_lines(&lines(&["---"])).is_none());
    }

    #[test]
    fn unclosed_block_is_none() {
        assert!(parse_lines(&lines(&["---", "header: [[x]]"])).is_none());
    }

    #[test]
    fn ignores_comments_and_empty_lines_inside() {
        let fm = parse_lines(&lines(&["---", "", "# note", "header: [[y]]", "---"])).unwrap();
        assert_eq!(fm.header.as_deref(), Some("y"));
        assert_eq!(fm.fields.len(), 1);
    }

    #[test]
    fn hr_later_does_not_trigger_frontmatter() {
        // First line is a heading, so the block below is content, not FM.
        assert!(parse_lines(&lines(&["# One", "---", "header: [[x]]", "---"])).is_none());
    }

    #[test]
    fn handles_bom() {
        let fm = parse_lines(&lines(&["\u{feff}---", "header: [[z]]", "---"])).unwrap();
        assert_eq!(fm.header.as_deref(), Some("z"));
    }

    #[test]
    fn content_parse_report_end_byte() {
        let content = "---\nheader: [[a.png]]\n---\n# Body\n[[b]]\n";
        let fm = parse(content).unwrap();
        assert_eq!(fm.header.as_deref(), Some("a.png"));
        assert_eq!(&content[fm.end_byte.min(content.len())..], "# Body\n[[b]]\n");
    }

    #[test]
    fn line_range_spans_block() {
        let b = lines(&["---", "header: [[x]]", "---", "body"]);
        assert_eq!(line_range(&b), Some((0, 2)));
        assert_eq!(line_range(&lines(&["# t", "---"])), None);
    }

    #[test]
    fn upsert_replaces_existing_header_line() {
        let out = upsert_header("---\nheader: [[old.png]]\n---\n# T\n", "[[assets/new.png]]");
        assert_eq!(out, "---\nheader: [[assets/new.png]]\n---\n# T\n");
    }

    #[test]
    fn upsert_inserts_header_when_block_has_none() {
        let out = upsert_header("---\ntitle: Hub\n---\n# T\n", "[[assets/pic.png]]");
        assert_eq!(out, "---\ntitle: Hub\nheader: [[assets/pic.png]]\n---\n# T\n");
    }

    #[test]
    fn upsert_prepends_block_when_no_frontmatter() {
        let out = upsert_header("# T\nbody\n", "[[a.png]]");
        assert_eq!(out, "---\nheader: [[a.png]]\n---\n# T\nbody\n");
    }
}