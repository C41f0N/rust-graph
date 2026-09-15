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
                    text: display_link(&line[i..le]),
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

// The display form of a wikilink: the ".md" extension is hidden, so
// [[alpha.md]] draws as [[alpha]]. `render_line` and `strip_inline` both
// build on line_segments, so draw and measure stay in sync.
fn display_link(text: &str) -> String {
    if text.len() >= 4 && text.starts_with("[[") && text.ends_with("]]") {
        let inner = &text[2..text.len() - 2];
        let inner = inner.trim_end_matches(".md");
        format!("[[{}]]", inner)
    } else {
        text.to_string()
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

// Line-start structure ("prefix") of a raw line, parsed once and reduced to
// ordered marker units so every consumer (wrap, layout, draw, classification)
// sees the same answer. Units are display-only in view mode: blockquote
// rails, list markers and ATX heading markers are replaced or stripped when
// drawn. Adding a new line-start style means adding a unit + one consume arm
// here; every downstream layer picks it up automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixUnit {
    // Leading indentation, only counted as a marker when a block marker
    // follows it (otherwise it is plain content).
    Indent { cols: u32, len: usize },
    // A `>` rail run; each `>` optionally swallowed one following space.
    Quote { len: usize },
    // A list item marker (plus any whitespace before it).
    ListItem { len: usize },
    // An ATX heading marker: "#..." plus one optional space.
    Heading { level: u32, len: usize },
}

impl PrefixUnit {
    fn len(&self) -> usize {
        match self {
            PrefixUnit::Indent { len, .. }
            | PrefixUnit::Quote { len }
            | PrefixUnit::ListItem { len }
            | PrefixUnit::Heading { len, .. } => *len,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinePrefix {
    units: Vec<PrefixUnit>,
    indent_cols: u32,
    pub is_hr: bool,
}

impl LinePrefix {
    // Byte offset where visible content begins: past all leading display-only
    // markers (indent, quote rails, list marker, heading marker).
    pub fn content(&self) -> usize {
        self.units.iter().map(|u| u.len()).sum()
    }

    pub fn has_quote(&self) -> bool {
        self.units.iter().any(|u| matches!(u, PrefixUnit::Quote { .. }))
    }

    pub fn heading_level(&self) -> Option<u32> {
        self.units.iter().find_map(|u| match u {
            PrefixUnit::Heading { level, .. } => Some(*level),
            _ => None,
        })
    }

    // Display-only bytes just past the blockquote rails (leading indent
    // included). None when the line isn't a quote.
    fn blockquote_skip(&self) -> Option<usize> {
        if !self.has_quote() {
            return None;
        }
        let mut sum = 0;
        for u in &self.units {
            match u {
                PrefixUnit::Indent { len, .. } | PrefixUnit::Quote { len } => sum += len,
                _ => break,
            }
        }
        Some(sum)
    }

    // Display-only bytes up to and including the first list item marker.
    fn list_skip(&self) -> Option<usize> {
        let mut sum = 0;
        for u in &self.units {
            match u {
                PrefixUnit::ListItem { len } => return Some(sum + len),
                _ => sum += u.len(),
            }
        }
        None
    }

    // Display-only bytes up to (not including) the heading marker.
    fn content_before_heading(&self) -> usize {
        let mut sum = 0;
        for u in &self.units {
            match u {
                PrefixUnit::Heading { .. } => return sum,
                _ => sum += u.len(),
            }
        }
        sum
    }
}

// Parse the leading display-only markers of one line. Order: leading
// indentation, blockquote rails, a list item marker, then an ATX heading.
// Precedence matches classification: a thematic break is decided before any
// list/heading consumption, and a list/quote marker wins over a heading in
// the same line (`- ## x` is a list item, not a heading block).
pub(crate) fn parse_prefix(line: &str) -> LinePrefix {
    let bytes = line.as_bytes();

    let mut indent_len = 0;
    let mut indent_cols = 0u32;
    while indent_len < bytes.len() {
        match bytes[indent_len] {
            b' ' => {
                indent_cols += 1;
                indent_len += 1;
            }
            b'\t' => {
                indent_cols += 2;
                indent_len += 1;
            }
            _ => break,
        }
    }

    // Thematic break: past the indentation, the non-space body is 3+ of the
    // same marker char. Checked before list consumption so `- - -` is a rule
    // while `- item` is a list item.
    let body: Vec<char> = line[indent_len..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if body.len() >= 3
        && matches!(body[0], '-' | '*' | '_')
        && body.iter().all(|&c| c == body[0])
    {
        return LinePrefix {
            is_hr: true,
            ..Default::default()
        };
    }

    let mut units: Vec<PrefixUnit> = Vec::new();

    // Blockquote rails: a `>` run, each optionally followed by one space.
    let mut q = indent_len;
    let mut quote_len = 0;
    while q < bytes.len() && bytes[q] == b'>' {
        quote_len += 1;
        q += 1;
        if q < bytes.len() && bytes[q] == b' ' {
            quote_len += 1;
            q += 1;
        }
    }
    if quote_len > 0 {
        units.push(PrefixUnit::Quote { len: quote_len });
    }

    // A list item marker; loose whitespace before it is part of the marker
    // region (so `>  - x` parses the same as before a list marker).
    let mut li = q;
    while li < bytes.len() && (bytes[li] == b' ' || bytes[li] == b'\t') {
        li += 1;
    }
    if let Some(len) = list_marker_at(&bytes[li.min(bytes.len())..]) {
        units.push(PrefixUnit::ListItem {
            len: (li - q) + len,
        });
        q = li + len;
    }

    // An ATX heading after the markers, or as the first marker when the
    // leading indentation is at most 3 columns (tabs count 2, so `\t## x` is
    // a heading but `    ## x` is not).
    if let Some((level, hlen)) = heading_at(bytes, q) {
        if !units.is_empty() || indent_cols <= 3 {
            if units.is_empty() && indent_len > 0 {
                units.push(PrefixUnit::Indent {
                    cols: indent_cols,
                    len: indent_len,
                });
            }
            units.push(PrefixUnit::Heading { level, len: hlen });
        }
    }

    // Leading indentation counts as a marker only when a block marker follows.
    if !units.is_empty() && indent_len > 0 && !matches!(units[0], PrefixUnit::Indent { .. }) {
        units.insert(0, PrefixUnit::Indent {
            cols: indent_cols,
            len: indent_len,
        });
    }

    LinePrefix {
        units,
        indent_cols,
        is_hr: false,
    }
}

// A "- ", "* ", "+ " (2 bytes) or "N. "/"N) " list marker at the start of `b`.
fn list_marker_at(b: &[u8]) -> Option<usize> {
    if b.is_empty() {
        return None;
    }

    if matches!(b[0], b'-' | b'*' | b'+') {
        if b.len() >= 2 && matches!(b[1], b' ' | b'\t') {
            return Some(2);
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
        && matches!(b[digits + 1], b' ' | b'\t')
    {
        return Some(digits + 2);
    }
    None
}

// 1..=6 "#" then whitespace or end, at byte `p`. Returns (level, byte length
// of the "#..."+space marker).
fn heading_at(bytes: &[u8], p: usize) -> Option<(u32, usize)> {
    if p >= bytes.len() || bytes[p] != b'#' {
        return None;
    }
    let mut level = 0;
    while p + level < bytes.len() && bytes[p + level] == b'#' {
        level += 1;
    }
    if level > 6 {
        return None;
    }
    match bytes.get(p + level) {
        None => Some((level as u32, level)),
        Some(&c) if c == b' ' || c == b'\t' => Some((level as u32, level + 1)),
        _ => None,
    }
}

// An ATX heading at the start of the line (allowing up to 3 columns of leading
// indentation, tabs=2): return (level 1..=6, byte offset past the whitespace
// plus "#..."+space markers). `#foo` (no space) is not a heading.
pub fn heading_info(line: &str) -> Option<(u32, usize)> {
    match parse_prefix(line).units.as_slice() {
        [PrefixUnit::Heading { level, len }] => Some((*level, *len)),
        [PrefixUnit::Indent { len, .. }, PrefixUnit::Heading { level, len: hlen }] => {
            Some((*level, len + hlen))
        }
        _ => None,
    }
}

// Byte offset just past any leading display-only markers on a raw line: the
// blockquote rail(s) (with their indentation) and then a single list item
// marker. Examples: `- ` -> 2, `> ` -> 2, `> - ` -> 4, `1. ` -> 3. The
// content after the markers may still start with a heading.
pub(crate) fn marker_content_skip(line: &str) -> usize {
    parse_prefix(line).content_before_heading()
}

// Like heading_info but for a line whose heading sits after leading
// list/quote markers, e.g. `- ## hello`, `> ## hi`, `1. ## x` or `\t- ## x`.
// Returns the heading level and the byte offset to the heading's content
// (markers plus "#..."+space already accounted for). None when there is no
// such heading.
pub fn heading_after_markers(line: &str) -> Option<(u32, usize)> {
    let p = parse_prefix(line);
    p.heading_level().map(|level| (level, p.content()))
}

// A horizontal rule line: 3+ of the same char among '-', '*', '_'
// (whitespace between characters allowed).
pub fn is_horizontal_rule(line: &str) -> bool {
    parse_prefix(line).is_hr
}

pub fn is_blockquote(line: &str) -> bool {
    parse_prefix(line).has_quote()
}

// Byte length of the leading blockquote markers on a raw line: the leading
// whitespace plus each `>` (optionally followed by a single space), e.g.
// `> foo` -> 2, `>> bar` -> 3, `  > baz` -> 4. Content starts after this.
// Returns None when the line isn't a quote.
pub fn blockquote_marker_len(line: &str) -> Option<usize> {
    parse_prefix(line).blockquote_skip()
}

// A list item marker: "- ", "* ", "+ " or "1..9. "/"9) " (after optional
// indentation). Returns the byte offset where the visible content begins.
pub fn list_info(line: &str) -> Option<usize> {
    parse_prefix(line).list_skip()
}

// The on-screen text for a list marker: "- "/"+ " become "* " (indentation
// preserved), "N. " stays as-is. Returns (display_text, raw_marker_len).
// The display text always measures identical to the raw marker, so cursor
// and selection positions derived from raw offsets stay aligned.
pub fn list_marker_display(line: &str) -> Option<(String, usize)> {
    let skip = parse_prefix(line).list_skip()?;
    let marker = &line[..skip];
    // "- "+"+ " become "* " (indentation preserved); ordered "N. " stays.
    let disp: String = marker
        .chars()
        .map(|c| if c == '-' || c == '+' { '*' } else { c })
        .collect();
    Some((disp, skip))
}

// If `line` consists of exactly one image wikilink (optionally with leading
// list/quotation markers, e.g. `- [[assets/pic.png]]`), return the link
// target (e.g. "assets/pic.png"). Embedded links ("see [[pic.png]] here")
// are not images.
pub fn image_link_target(line: &str) -> Option<String> {
    let mut s = line.trim_start();
    // Consume blockquote rail(s): ">" optionally followed by a space.
    while let Some(rest) = s.strip_prefix('>') {
        s = rest.strip_prefix(' ').unwrap_or(rest).trim_start();
    }
    // Consume one list marker.
    if let Some(skip) = list_info(s) {
        s = s[skip..].trim_start();
    }
    if !(s.starts_with("[[") && s.ends_with("]]") && s.len() >= 5) {
        return None;
    }
    let inner = s[2..s.len() - 2].trim();
    if inner.is_empty() || !crate::editor::images::is_image_target(inner) {
        return None;
    }
    Some(inner.to_string())
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
        assert_eq!(segs[1].text, "[[alpha]]");
    }

    #[test]
    fn wikilink_hides_md_extension() {
        assert_eq!(display_link("[[alpha.md]]"), "[[alpha]]");
        assert_eq!(display_link("[[beta]]"), "[[beta]]");
        assert_eq!(display_link("[[cat.png]]"), "[[cat.png]]");
        assert_eq!(display_link("[[nested/deep.md]]"), "[[nested/deep]]");
        assert_eq!(display_link("plain"), "plain");
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
    fn heading_inside_list_and_quote_markers() {
        // A heading nested in a list/quote must be found past the markers and
        // report the byte offset to the actual content (markers consumed).
        assert_eq!(heading_after_markers("## plain"), Some((2, 3)));
        assert_eq!(heading_after_markers("- ## hi"), Some((2, 5)));
        assert_eq!(heading_after_markers("* ### hi"), Some((3, 6)));
        assert_eq!(heading_after_markers("> ## hi"), Some((2, 5)));
        assert_eq!(heading_after_markers("> - ## x"), Some((2, 7)));
        assert_eq!(heading_after_markers("1. ## hi"), Some((2, 6)));
        assert_eq!(heading_after_markers("- #hi"), None);
        assert_eq!(heading_after_markers("- plain"), None);
        assert_eq!(heading_after_markers("> plain"), None);
        assert_eq!(marker_content_skip("- ## hi"), 2);
        assert_eq!(marker_content_skip("> ## hi"), 2);
        assert_eq!(marker_content_skip("> - ## x"), 4);
        assert_eq!(marker_content_skip("## plain"), 0);
    }

    #[test]
    fn indented_headings() {
        // Up to 3 leading columns (tabs=2) still count as a heading; 4+ do not.
        assert_eq!(heading_info("## x"), Some((2, 3)));
        assert_eq!(heading_info("  ## x"), Some((2, 5)));
        assert_eq!(heading_info("\t## x"), Some((2, 4)));
        assert_eq!(heading_info("   ## x"), Some((2, 6)));
        assert_eq!(heading_info("    ## x"), None);
        assert_eq!(heading_info("\t\t## x"), None);
        assert_eq!(heading_info("\t# x"), Some((1, 3)));

        // Composition: indent + markers + heading all resolve to one offset.
        assert_eq!(heading_after_markers("\t## x"), Some((2, 4)));
        assert_eq!(heading_after_markers("\t- ## x"), Some((2, 6)));
        assert_eq!(heading_after_markers("  > ## x"), Some((2, 7)));
        // Only a single optional space may sit between marker and heading.
        assert_eq!(heading_after_markers("-   ## x"), None);
        assert_eq!(heading_after_markers(">  ## x"), None);
        assert_eq!(marker_content_skip("\t## x"), 1);
        assert_eq!(marker_content_skip("  > ## x"), 4);
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

        assert_eq!(blockquote_marker_len("> foo"), Some(2));
        assert_eq!(blockquote_marker_len(">"), Some(1));
        assert_eq!(blockquote_marker_len(">> bar"), Some(3));
        assert_eq!(blockquote_marker_len("  > baz"), Some(4));
        assert_eq!(blockquote_marker_len(">  spaced"), Some(2));
        assert_eq!(blockquote_marker_len("> > nested"), Some(4));
        assert_eq!(blockquote_marker_len("not a quote"), None);
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

    #[test]
    fn whole_line_image_links() {
        assert_eq!(image_link_target("[[cat.png]]"), Some("cat.png".to_string()));
        assert_eq!(
            image_link_target("[[assets/image.png]]"),
            Some("assets/image.png".to_string())
        );
        assert_eq!(
            image_link_target("- [[cat.png]]"),
            Some("cat.png".to_string())
        );
        assert_eq!(
            image_link_target("> [[cat.png]]"),
            Some("cat.png".to_string())
        );
        assert_eq!(
            image_link_target("  > [[img.jpeg]]"),
            Some("img.jpeg".to_string())
        );
        assert_eq!(
            image_link_target("[[cat.PNG]]"),
            Some("cat.PNG".to_string())
        );
    }

    #[test]
    fn non_image_lines() {
        assert_eq!(image_link_target("see [[cat.png]] here"), None);
        assert_eq!(image_link_target("[[cat.md]]"), None);
        assert_eq!(image_link_target("[[cat]]"), None);
        assert_eq!(image_link_target("[[cat.png]] extra"), None);
        assert_eq!(image_link_target("plain text"), None);
        assert_eq!(image_link_target(""), None);
    }
}