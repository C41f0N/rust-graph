#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStyle {
    Plain,
    Link,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub style: SegmentStyle,
}

// Split a line into styled segments. Markers: [[wikilink]] (Link) and
// `inline code` (Code). Unclosed markers stay plain. Nested markers inside
// a link or code span are not parsed.
pub fn line_segments(line: &str) -> Vec<Segment> {
    let bytes = line.as_bytes();
    let mut segments = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut end = i + 2;
            let mut link_end = None;
            while end + 1 < bytes.len() {
                if bytes[end] == b']' && bytes[end + 1] == b']' {
                    link_end = Some(end + 2);
                    break;
                }
                end += 1;
            }
            if let Some(le) = link_end {
                if plain_start < i {
                    segments.push(Segment {
                        text: line[plain_start..i].to_string(),
                        style: SegmentStyle::Plain,
                    });
                }
                segments.push(Segment {
                    text: line[i..le].to_string(),
                    style: SegmentStyle::Link,
                });
                plain_start = le;
                i = le;
            } else {
                // Unclosed "[[" — treat the brackets as plain text.
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'`' {
            let mut end = i + 1;
            let mut code_end = None;
            while end < bytes.len() {
                if bytes[end] == b'`' {
                    code_end = Some(end);
                    break;
                }
                end += 1;
            }
            if let Some(ce) = code_end {
                if plain_start < i {
                    segments.push(Segment {
                        text: line[plain_start..i].to_string(),
                        style: SegmentStyle::Plain,
                    });
                }
                segments.push(Segment {
                    text: line[i..=ce].to_string(),
                    style: SegmentStyle::Code,
                });
                plain_start = ce + 1;
                i = ce + 1;
                continue;
            }
            // Unclosed backtick stays plain.
            i += 1;
            continue;
        }

        i += 1;
    }

    if plain_start < bytes.len() {
        segments.push(Segment {
            text: line[plain_start..].to_string(),
            style: SegmentStyle::Plain,
        });
    }

    segments
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
}