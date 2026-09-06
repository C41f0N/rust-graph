#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStyle {
    Plain,
    Link,
    Code,
    Bold,
    Italic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub style: SegmentStyle,
}

// Split a line into styled segments. Markers: [[wikilink]] (Link) and
// `inline code` (Code) keep their markers; emphasis (**bold**, __bold__,
// *italic*, _italic_) drops its markers into Bold/Italic segments so the
// content can be drawn with whichever font is available. Unclosed markers
// stay plain. Nested markers inside a link or code span are not parsed.
pub fn line_segments(line: &str) -> Vec<Segment> {
    let bytes = line.as_bytes();
    let mut segments = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some(le) = find_closer_bytes(bytes, i + 2, b']', b']') {
                flush_plain(&mut segments, line, plain_start, i);
                segments.push(Segment {
                    text: line[i..le].to_string(),
                    style: SegmentStyle::Link,
                });
                plain_start = le;
                i = le;
                continue;
            }
            i += 1;
            continue;
        }

if bytes[i] == b'`' {
        if let Some(ce) = find_closer_single(bytes, i + 1, b'`') {
            flush_plain(&mut segments, line, plain_start, i);
            segments.push(Segment {
                text: line[i..=ce].to_string(),
                style: SegmentStyle::Code,
            });
            plain_start = ce + 1;
            i = ce + 1;
            continue;
        }
        i += 1;
        continue;
    }

        let marker = emphasis_marker(bytes, i);
        if let Some((m_len, style)) = marker {
            if let Some(j) = find_emphasis_close(bytes, i + m_len, &line[i..i + m_len]) {
                flush_plain(&mut segments, line, plain_start, i);
                let inner = strip_inline(&line[i + m_len..j]);
                segments.push(Segment {
                    text: inner,
                    style,
                });
                let end = j + m_len;
                plain_start = end;
                i = end;
                continue;
            }
            i += 1;
            continue;
        }

        i += 1;
    }

    flush_plain(&mut segments, line, plain_start, bytes.len());
    segments
}

// Visible text of a line: markers stripped, links/code kept as-is.
// Defined via the tokenizer so rendering and measurement stay in sync.
pub fn strip_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for seg in line_segments(line) {
        out.push_str(&seg.text);
    }
    out
}

// Source view: no parsing at all. Used while the cursor is on the line so
// the original markdown is shown untouched for editing.
pub fn raw_segments(line: &str) -> Vec<Segment> {
    vec![Segment {
        text: line.to_string(),
        style: SegmentStyle::Plain,
    }]
}

// The segments to draw for `line`. `format` selects markdown rendering
// (cursor off the line); false shows the raw source (cursor on the line).
pub fn render_line(line: &str, format: bool) -> Vec<Segment> {
    if format {
        line_segments(line)
    } else {
        raw_segments(line)
    }
}

// The text whose measured width matches what render_line draws. Always use
// this (not the raw line) for cursor/selection/wrap measurements.
pub fn measure_line(line: &str, format: bool) -> String {
    if format {
        strip_inline(line)
    } else {
        line.to_string()
    }
}

fn flush_plain(segments: &mut Vec<Segment>, line: &str, start: usize, end: usize) {
    if start < end {
        segments.push(Segment {
            text: line[start..end].to_string(),
            style: SegmentStyle::Plain,
        });
    }
}

// Find the position just past a closing pair of `a`..`b`, starting at `from`.
fn find_closer_bytes(bytes: &[u8], from: usize, a: u8, b: u8) -> Option<usize> {
    let mut j = from;
    while j + 1 < bytes.len() {
        if bytes[j] == a && bytes[j + 1] == b {
            return Some(j + 2);
        }
        j += 1;
    }
    None
}

// Find the position of a single closing `c`, starting at `from`.
fn find_closer_single(bytes: &[u8], from: usize, c: u8) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == c {
            return Some(j);
        }
        j += 1;
    }
    None
}

// "**" or "__" -> (2, Bold), "*" or "_" -> (1, Italic).
fn emphasis_marker(bytes: &[u8], i: usize) -> Option<(usize, SegmentStyle)> {
    let c = bytes[i];
    if c != b'*' && c != b'_' {
        return None;
    }
    if i + 1 < bytes.len() && bytes[i + 1] == c {
        Some((2, SegmentStyle::Bold))
    } else {
        Some((1, SegmentStyle::Italic))
    }
}

// For emphasis, find a closer that is not part of a doubled marker.
fn find_emphasis_close(bytes: &[u8], from: usize, marker: &str) -> Option<usize> {
    let c = marker.as_bytes()[0];
    let doubled = marker.len() == 2;
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == c && (!doubled || (j + 1 < bytes.len() && bytes[j + 1] == c)) {
            return Some(j);
        }
        j += 1;
    }
    None
}

// For an ATX heading line return (level 1..=6, byte offset past the "#..."+
// following space marker). `#foo` (no space) is not a heading.
pub fn heading_info(line: &str) -> Option<(u32, usize)> {
    let bytes = line.as_bytes();
    let mut level = 0;
    while level < bytes.len() && bytes[level] == b'#' {
        level += 1;
    }
    if level == 0 || level > 6 {
        return None;
    }
    match line[level..].chars().next() {
        None => Some((level as u32, level)),
        Some(c) if c.is_whitespace() => Some((level as u32, level + 1)),
        _ => None,
    }
}

// A horizontal rule line: 3+ of the same char among '-', '*', '_'
// (whitespace between characters allowed).
pub fn is_horizontal_rule(line: &str) -> bool {
    let chars: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 3 {
        return false;
    }
    let first = chars[0];
    if first != '-' && first != '*' && first != '_' {
        return false;
    }
    chars.iter().all(|&c| c == first)
}

pub fn is_blockquote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

// A list item marker: "- ", "* ", "+ " or "1..9. "/"9) " (after optional
// indentation). Returns the byte offset where the visible content begins.
pub fn list_info(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let b = trimmed.as_bytes();

    if b.is_empty() {
        return None;
    }

    if matches!(b[0], b'-' | b'*' | b'+') {
        if b.len() >= 2 && (b[1] == b' ' || b[1] == b'\t') {
            return Some(indent + 2);
        }
        return None;
    }

    let mut digits = 0;
    while digits < b.len() && digits < 9 && b[digits].is_ascii_digit() {
        digits += 1;
    }
    if digits == 0 {
        return None;
    }
    if b.len() > digits
        && matches!(b[digits], b'.' | b')')
        && b.len() > digits + 1
        && (b[digits + 1] == b' ' || b[digits + 1] == b'\t')
    {
        return Some(indent + digits + 2);
    }
    None
}

// The on-screen text for a list marker: "- "/"+ " become "* " (indentation
// preserved), "N. " stays as-is. Returns (display_text, raw_marker_len).
// The display text always measures identical to the raw marker, so cursor
// and selection positions derived from raw offsets stay aligned.
pub fn list_marker_display(line: &str) -> Option<(String, usize)> {
    let skip = list_info(line)?;
    let marker = &line[..skip];
    let has_bullet = marker
        .trim_start()
        .chars()
        .next()
        .is_some_and(|c| c == '-' || c == '+');
    let disp: String = if has_bullet {
        marker
            .chars()
            .map(|c| if c == '-' || c == '+' { '*' } else { c })
            .collect()
    } else {
        marker.to_string()
    };
    Some((disp, skip))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(segs: &[Segment]) -> Vec<SegmentStyle> {
        segs.iter().map(|s| s.style).collect()
    }

    #[test]
    fn plain_line() {
        let segs = line_segments("just text here");
        assert_eq!(styles(&segs), vec![SegmentStyle::Plain]);
        assert_eq!(segs[0].text, "just text here");
    }

    #[test]
    fn wikilink_is_highlighted() {
        let segs = line_segments("see [[alpha.md]] here");
        assert_eq!(
            styles(&segs),
            vec![
                SegmentStyle::Plain,
                SegmentStyle::Link,
                SegmentStyle::Plain,
            ]
        );
        assert_eq!(segs[1].text, "[[alpha.md]]");
    }

    #[test]
    fn inline_code_and_link() {
        let segs = line_segments("use `x` then [[y]]");
        assert_eq!(
            styles(&segs),
            vec![
                SegmentStyle::Plain,
                SegmentStyle::Code,
                SegmentStyle::Plain,
                SegmentStyle::Link,
            ]
        );
        assert_eq!(segs[1].text, "`x`");
        assert_eq!(segs[3].text, "[[y]]");
    }

    #[test]
    fn unclosed_markers_stay_plain() {
        let segs = line_segments("broken [[link");
        assert_eq!(styles(&segs), vec![SegmentStyle::Plain]);

        let segs = line_segments("broken ` code");
        assert_eq!(styles(&segs), vec![SegmentStyle::Plain]);
    }

    #[test]
    fn link_then_code_adjacent() {
        let segs = line_segments("[[a]]`b`");
        assert_eq!(
            styles(&segs),
            vec![SegmentStyle::Link, SegmentStyle::Code]
        );
    }

    #[test]
    fn heading_levels() {
        assert_eq!(heading_info("# Title"), Some((1, 2)));
        assert_eq!(heading_info("## Sub"), Some((2, 3)));
        assert_eq!(heading_info("###### Deep"), Some((6, 7)));
        assert_eq!(heading_info("####### too many"), None);
        assert_eq!(heading_info("#foo"), None);
        assert_eq!(heading_info("plain"), None);
        assert_eq!(heading_info("#"), Some((1, 1)));
    }

    #[test]
    fn horizontal_rules() {
        assert!(is_horizontal_rule("---"));
        assert!(is_horizontal_rule("***"));
        assert!(is_horizontal_rule("___"));
        assert!(is_horizontal_rule("- - -"));
        assert!(is_horizontal_rule("---   "));
        assert!(!is_horizontal_rule("--"));
        assert!(!is_horizontal_rule("--- text"));
        assert!(!is_horizontal_rule(""));
    }

    #[test]
    fn blockquotes() {
        assert!(is_blockquote("> quote"));
        assert!(is_blockquote(">quote"));
        assert!(is_blockquote("  > indented quote"));
        assert!(!is_blockquote("no quote"));
        assert!(!is_blockquote(""));
    }

    #[test]
    fn bold_italic_emphasis() {
        let segs = line_segments("**bold** and *italic* mix");
        assert_eq!(
            segs.iter().map(|s| s.style).collect::<Vec<_>>(),
            vec![
                SegmentStyle::Bold,
                SegmentStyle::Plain,
                SegmentStyle::Italic,
                SegmentStyle::Plain,
            ]
        );
        assert_eq!(segs[0].text, "bold");
        assert_eq!(segs[2].text, "italic");
        assert_eq!(segs[1].text, " and ");
        assert_eq!(segs[3].text, " mix");
    }

    #[test]
    fn emphasis_markers_stripped() {
        assert_eq!(
            strip_inline("**bold**`code`_it_"),
            "bold`code`it"
        );
        assert_eq!(strip_inline("__bold__ word"), "bold word");
    }

    #[test]
    fn unclosed_emphasis_stays_plain() {
        let segs = line_segments("**unclosed");
        assert_eq!(
            styles(&segs),
            vec![SegmentStyle::Plain]
        );
        assert_eq!(segs[0].text, "**unclosed");
    }

    #[test]
    fn emphasis_keeps_cursor_measure_consistent() {
        assert_eq!(strip_inline("plain **b** plain"), "plain b plain");
        assert_eq!(strip_inline("# ## heading"), "# ## heading");
    }

    #[test]
    fn list_markers() {
        assert_eq!(list_info("- item"), Some(2));
        assert_eq!(list_info("* item"), Some(2));
        assert_eq!(list_info("+ item"), Some(2));
        assert_eq!(list_info("1. item"), Some(3));
        assert_eq!(list_info("12) item"), Some(4));
        assert_eq!(list_info("  - nested"), Some(4));
        assert_eq!(list_info("-"), None);
        assert_eq!(list_info("*item* (italic, not a list)"), None);
        assert_eq!(list_info("---is a rule"), None);
        assert_eq!(list_info("plain"), None);
        assert_eq!(list_info("1.item no space"), None);
    }

    #[test]
    fn list_marker_displays() {
        assert_eq!(list_marker_display("- item"), Some(("* ".to_string(), 2)));
        assert_eq!(list_marker_display("+ item"), Some(("* ".to_string(), 2)));
        assert_eq!(list_marker_display("* item"), Some(("* ".to_string(), 2)));
        assert_eq!(
            list_marker_display("  - indented"),
            Some(("  * ".to_string(), 4))
        );
        assert_eq!(
            list_marker_display("1. ordered"),
            Some(("1. ".to_string(), 3))
        );
        assert_eq!(list_marker_display("plain"), None);
    }

    #[test]
    fn raw_segments_are_unparsed() {
        let raw = raw_segments("**bold** [[x]] `c` - item");
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].style, SegmentStyle::Plain);
        assert_eq!(raw[0].text, "**bold** [[x]] `c` - item");
    }
}