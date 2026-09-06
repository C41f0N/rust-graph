// Derived block structure over the raw buffer.
//
// This layer is READ-ONLY and ephemeral: it is rebuilt from the raw lines
// every frame and never stored or persisted. The .md files stay plain text;
// save_to_file keeps joining the raw lines with "\n". It exists purely so
// the renderer can ask "which block does this line belong to?" for things
// a single line can't describe (fenced code spans, nested list depth).

use crate::editor::markdown;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    Paragraph,
    Heading { level: u8 },
    HorizontalRule,
    Blockquote,
    List { depth: u32 },
    FencedCode,
    FenceDelimiter,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    Paragraph,
    Heading { level: u8 },
    HorizontalRule,
    Blockquote,
    ListItem { depth: u32 },
    FencedCode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub start: usize, // inclusive buffer line index
    pub end: usize,   // exclusive buffer line index
}

// Depth of a list marker line: one level per 2 leading columns (tabs=2).
fn list_depth(line: &str) -> u32 {
    let mut cols = 0;
    for c in line.chars() {
        match c {
            ' ' => cols += 1,
            '\t' => cols += 2,
            _ => break,
        }
    }
    cols / 2
}

// A fenced code opening: 3+ backticks or tildes at the start (after lazy
// indentation). Returns the fence char and its run length.
fn fence_opening(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let mut n = 1;
    for c in chars {
        if c == first {
            n += 1;
        } else {
            break;
        }
    }
    if n >= 3 {
        Some((first, n))
    } else {
        None
    }
}

// Parse the buffer into a flat list of blocks.
pub fn parse_blocks(lines: &[String]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = &lines[i];

        if let Some((fence_char, fence_len)) = fence_opening(line) {
            // Fenced code: spans until a closing fence with the same char,
            // or to the end of the file if never closed.
            let mut j = i + 1;
            while j < lines.len() {
                if let Some((c, n)) = fence_opening(&lines[j]) {
                    if c == fence_char && n >= fence_len {
                        break;
                    }
                }
                j += 1;
            }
            let end = if j < lines.len() { j + 1 } else { lines.len() };
            blocks.push(Block {
                kind: BlockKind::FencedCode,
                start: i,
                end,
            });
            i = end;
            continue;
        }

        let kind = if line.trim().is_empty() {
            LineKind::Blank
        } else if markdown::is_blockquote(line) {
            LineKind::Blockquote
        } else if markdown::is_horizontal_rule(line) {
            LineKind::HorizontalRule
        } else if let Some((level, _)) = markdown::heading_info(line) {
            LineKind::Heading { level: level as u8 }
        } else if markdown::list_info(line).is_some() {
            LineKind::List { depth: list_depth(line) }
        } else {
            LineKind::Paragraph
        };

        if kind.is_blank() {
            i += 1;
            continue;
        }

        blocks.push(Block {
            kind: match kind {
                LineKind::Heading { level } => BlockKind::Heading { level },
                LineKind::List { depth } => BlockKind::ListItem { depth },
                LineKind::Blockquote => BlockKind::Blockquote,
                LineKind::HorizontalRule => BlockKind::HorizontalRule,
                _ => BlockKind::Paragraph,
            },
            start: i,
            end: i + 1,
        });
        i += 1;
    }

    merge_adjacent(blocks)
}

fn merge_adjacent(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for b in blocks {
        if let Some(last) = out.last_mut() {
            if last.kind == b.kind && last.end == b.start && !matches!(last.kind, BlockKind::ListItem { .. }) {
                last.end = b.end;
                continue;
            }
        }
        out.push(b);
    }
    out
}

impl LineKind {
    fn is_blank(self) -> bool {
        matches!(self, LineKind::Blank)
    }

    fn from_block(block: &Block, line: usize, lines: &[String]) -> LineKind {
        match &block.kind {
            BlockKind::Paragraph => LineKind::Paragraph,
            BlockKind::Heading { level } => LineKind::Heading { level: *level },
            BlockKind::HorizontalRule => LineKind::HorizontalRule,
            BlockKind::Blockquote => LineKind::Blockquote,
            BlockKind::ListItem { depth } => LineKind::List { depth: *depth },
            BlockKind::FencedCode => {
                let is_first = block.start == line;
                let is_last = line == block.end.saturating_sub(1);
                if is_first || (is_last && is_last_fence_delimiter(block, lines)) {
                    LineKind::FenceDelimiter
                } else {
                    LineKind::FencedCode
                }
            }
        }
    }
}

fn is_last_fence_delimiter(block: &Block, lines: &[String]) -> bool {
    if block.end <= block.start + 1 {
        return false;
    }
    let line = &lines[block.end - 1];
    let first_char = lines[block.start].trim_start().chars().next().unwrap_or('`');
    fence_opening(line).is_some_and(|(c, n)| {
        c == first_char && n >= 3
    })
}

// Per-line classification for the whole buffer, for O(1) lookups by the
// wrap and render passes.
pub fn classify(lines: &[String]) -> Vec<LineKind> {
    let blocks = parse_blocks(lines);
    let mut kinds = vec![LineKind::Blank; lines.len()];
    for block in &blocks {
        for line in block.start..block.end {
            kinds[line] = LineKind::from_block(block, line, lines);
        }
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn classifies_blank_and_paragraph() {
        let k = classify(&s(&["", "hello world", ""]));
        assert_eq!(k[0], LineKind::Blank);
        assert_eq!(k[1], LineKind::Paragraph);
        assert_eq!(k[2], LineKind::Blank);
    }

    #[test]
    fn classifies_headings_and_hr() {
        let k = classify(&s(&["# Title", "## Sub", "plain", "---"]));
        assert_eq!(k[0], LineKind::Heading { level: 1 });
        assert_eq!(k[1], LineKind::Heading { level: 2 });
        assert_eq!(k[2], LineKind::Paragraph);
        assert_eq!(k[3], LineKind::HorizontalRule);
    }

    #[test]
    fn classifies_list_depths() {
        let k = classify(&s(&["- one", "  - two", "    - three", "- four"]));
        assert_eq!(k[0], LineKind::List { depth: 0 });
        assert_eq!(k[1], LineKind::List { depth: 1 });
        assert_eq!(k[2], LineKind::List { depth: 2 });
        assert_eq!(k[3], LineKind::List { depth: 0 });
    }

    #[test]
    fn fenced_code_spans_blank_lines() {
        let k = classify(&s(&[
            "```rust",
            "fn main() {}",
            "",
            "still code",
            "```",
            "after",
        ]));
        assert_eq!(k[0], LineKind::FenceDelimiter);
        assert_eq!(k[1], LineKind::FencedCode);
        assert_eq!(k[2], LineKind::FencedCode);
        assert_eq!(k[3], LineKind::FencedCode);
        assert_eq!(k[4], LineKind::FenceDelimiter);
        assert_eq!(k[5], LineKind::Paragraph);
    }

    #[test]
    fn unclosed_fence_runs_to_end() {
        let k = classify(&s(&["```", "code", "more"]));
        assert_eq!(k[0], LineKind::FenceDelimiter);
        assert_eq!(k[1], LineKind::FencedCode);
        assert_eq!(k[2], LineKind::FencedCode);
    }

    #[test]
    fn blockquote_groups_are_quotes() {
        let k = classify(&s(&["> quote one", "> quote two", "plain", "> lone"]));
        assert_eq!(k[0], LineKind::Blockquote);
        assert_eq!(k[1], LineKind::Blockquote);
        assert_eq!(k[2], LineKind::Paragraph);
        assert_eq!(k[3], LineKind::Blockquote);
    }

    #[test]
    fn triple_dash_is_hr_not_list() {
        let k = classify(&s(&["---"]));
        assert_eq!(k[0], LineKind::HorizontalRule);
    }

    #[test]
    fn fences_merge_into_one_block() {
        let lines = s(&["```", "x", "```"]);
        let blocks = parse_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, BlockKind::FencedCode);
        assert_eq!(blocks[0].start, 0);
        assert_eq!(blocks[0].end, 3);
    }
}