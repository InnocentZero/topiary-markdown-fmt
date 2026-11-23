//! Read-only acceptance checks. These do not call a formatter or generate edits.
use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

use crate::{layout, math, parser};

fn horizontal(ch: char) -> bool {
    ch == ' ' || ch == '\t'
}

fn eol(source: &str, start: usize) -> usize {
    start
        + source[start..]
            .find(['\r', '\n'])
            .unwrap_or(source.len() - start)
}

pub fn violations(source: &str) -> Vec<&'static str> {
    let mut issues = crate::violations(source);
    let math = math::slash_ranges(source);
    let mut lists = Vec::new();
    let mut previous_list = None;
    let mut opaque = 0;
    let mut table_rows: Vec<Range<usize>> = Vec::new();
    let mut table_head = None;
    let mut in_table = false;
    let mut continuation_columns = Vec::new();
    let mut complex_container = 0;
    for (event, span) in parser(source).into_offset_iter() {
        match &event {
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_)) => opaque += 1,
            Event::End(TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_)) => {
                opaque -= 1
            }
            _ => {}
        }
        if opaque != 0 {
            continue;
        }
        match &event {
            Event::Start(Tag::BlockQuote(_) | Tag::Table(_) | Tag::Heading { .. }) => complex_container += 1,
            Event::End(TagEnd::BlockQuote(_) | TagEnd::Table | TagEnd::Heading(_)) => complex_container -= 1,
            Event::End(TagEnd::Item) => { continuation_columns.pop(); }
            _ => {}
        }
        let raw = &source[span.clone()];
        match event {
            Event::Start(Tag::List(start)) => {
                let adjacent = previous_list.is_some_and(|(depth, ordered, end)| {
                    depth == lists.len()
                        && ordered == start.is_some()
                        && end <= span.start
                        && source[end..span.start]
                            .trim_matches([' ', '\t', '\r', '\n', '>'])
                            .is_empty()
                });
                lists.push((start, adjacent));
            }
            Event::End(TagEnd::List(ordered)) => {
                lists.pop();
                previous_list = Some((lists.len(), ordered, span.end));
            }
            Event::Start(Tag::Item) => {
                let line = source[span.start..eol(source, span.start)].trim_start_matches(horizontal);
                let length = line.bytes().take_while(u8::is_ascii_digit).count();
                let marker_len = if length == 0 { 1 } else { length + 1 };
                let physical = &source[layout::line_start(source, span.start)..eol(source, span.start)];
                let first_content = physical.len().checked_sub(line.len()).and_then(|prefix| {
                    let after = line.get(marker_len..)?;
                    let padding = after.len() - after.trim_start_matches(' ').len();
                    ((1..=4).contains(&padding) && physical[..prefix].bytes().all(|b| b == b' '))
                        .then_some(prefix + marker_len + padding)
                });
                continuation_columns.push(first_content);
                let Some((number, adjacent)) = lists.last_mut() else {
                    continue;
                };
                if let Some(expected) = number {
                    if line.get(..marker_len) != Some(format!("{expected}.").as_str()) {
                        issues.push("ordered-list-marker");
                    }
                    *expected = expected.saturating_add(1);
                } else if line.starts_with('+') || line.starts_with('*') && !*adjacent {
                    // Adjacent lists of the same kind may need a distinct marker.
                    issues.push("unordered-list-marker");
                }
                if let Some(tail) = line.get(marker_len..) {
                    let rest = tail.trim_start_matches(horizontal);
                    if !rest.is_empty() && (!tail.starts_with(' ') || tail.len() - rest.len() != 1)
                    {
                        issues.push("list-marker-spacing");
                    }
                }
            }
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                table_rows.clear();
                table_head = None;
            }
            Event::Start(Tag::TableHead) => {
                table_head = Some(span.clone());
                table_rows.push(span);
            }
            Event::Start(Tag::TableRow) => table_rows.push(span),
            Event::End(TagEnd::Table) => {
                in_table = false;
                if let Some(head) = table_head.take() {
                    let delim_start = eol(source, head.start);
                    let delim_start = delim_start
                        + source[delim_start..]
                            .bytes()
                            .take_while(|b| matches!(b, b'\r' | b'\n'))
                            .count();
                    table_rows.push(delim_start..eol(source, delim_start));
                }
                let mut expected = None;
                for row in &table_rows {
                    if math::overlaps(&math, row) {
                        continue;
                    }
                    let line = source[row.start..eol(source, row.start)]
                        .trim_start_matches([' ', '\t', '>']);
                    let pipes = pipe_columns(line);
                    if !line.starts_with("| ")
                        || !line.ends_with(" |")
                        || pipes.len() < 2
                        || pipes.windows(2).any(|w| w[1] - w[0] < 3)
                        || expected.as_ref().is_some_and(|expected| expected != &pipes)
                    {
                        issues.push("table-layout");
                    }
                    expected = Some(pipes);
                }
            }
            Event::Start(Tag::Emphasis)
                if raw.starts_with('*')
                    && !raw.starts_with("***")
                    && !intraword_emphasis(source, &span)
                    && !math::overlaps(&math, &span) =>
            {
                issues.push("emphasis-marker")
            }
            Event::Start(Tag::Strong)
                if raw.starts_with("__")
                    && !raw.starts_with("___")
                    && !math::overlaps(&math, &span) =>
            {
                issues.push("strong-marker")
            }
            Event::Rule if !math::overlaps(&math, &span) => {
                if !matches!(raw.trim(), "---" | "***") {
                    issues.push("thematic-break");
                }
            }
            Event::SoftBreak | Event::HardBreak if !in_table => {
                let end = eol(source, span.start);
                let before = &source[layout::line_start(source, end)..end];
                let trimmed = before.trim_end_matches(horizontal);
                if !math::overlaps(&math, &(end - (before.len() - trimmed.len())..end)) {
                    let tail = &before[trimmed.len()..];
                    if (!tail.is_empty() && matches!(event, Event::SoftBreak))
                        || (matches!(event, Event::HardBreak) && !matches!(tail, "" | "  "))
                    {
                        issues.push("prose-trailing-whitespace");
                    }
                }
                if complex_container == 0 && let Some(&Some(column)) = continuation_columns.last() {
                    let following = source[end..].strip_prefix("\r\n")
                        .or_else(|| source[end..].strip_prefix(['\r', '\n']));
                    if let Some(following) = following {
                        let content = following.trim_start_matches(' ');
                        let indent = following.len() - content.len();
                        let start = source.len() - following.len();
                        if !content.is_empty() && !content.starts_with(['\r', '\n', '\t'])
                            && indent != column && !math::overlaps(&math, &(start..start + indent))
                        {
                            issues.push("list-prose-continuation");
                        }
                    }
                }
            }
            Event::End(TagEnd::Paragraph) => {
                let end = span.end
                    - raw
                        .bytes()
                        .rev()
                        .take_while(|b| matches!(b, b'\r' | b'\n'))
                        .count();
                let end = eol(source, end);
                let before = &source[layout::line_start(source, end)..end];
                let len = before.len() - before.trim_end_matches(horizontal).len();
                if len != 0 && !math::overlaps(&math, &(end - len..end)) {
                    issues.push("prose-trailing-whitespace");
                }
            }
            _ => {}
        }
    }
    issues.sort_unstable();
    issues.dedup();
    issues
}

fn intraword_emphasis(source: &str, span: &Range<usize>) -> bool {
    let Some(content) = source[span.clone()].strip_prefix('*').and_then(|s| s.strip_suffix('*')) else {
        return false;
    };
    // CommonMark does not permit underscores between two letters/digits as
    // emphasis delimiters. Prettier also keeps the asterisk in these cases.
    source[..span.start].chars().next_back().is_some_and(char::is_alphanumeric)
        && content.chars().next().is_some_and(char::is_alphanumeric)
        || content.chars().next_back().is_some_and(char::is_alphanumeric)
            && source[span.end..].chars().next().is_some_and(char::is_alphanumeric)
}

fn pipe_columns(line: &str) -> Vec<usize> {
    let mut width = 0;
    let mut segment = 0;
    let mut cols = Vec::new();
    for (i, ch) in line.char_indices() {
        if ch == '|' && (i == 0 || line.as_bytes()[i - 1] != b'\\') {
            // Measure whole runs: an emoji sequence is not the sum of its
            // constituent scalar widths.
            width += line[segment..i].width();
            segment = i;
            cols.push(width);
        }
    }
    cols
}

#[cfg(test)]
mod tests {
    #[test]
    fn checks_list_continuation_columns_but_not_protected_blocks() {
        let aligned = "- Item\n  more prose\n\n  ```sh\n  raw\n  ```\n- Last\n";
        assert!(super::violations(aligned).is_empty());
        for bad in ["- Item\nmore prose\n", "- Item\n    more prose\n"] {
            assert!(super::violations(bad).contains(&"list-prose-continuation"), "{bad:?}");
        }
    }
    #[test]
    fn style_keeps_required_intraword_asterisks_but_checks_standalone_emphasis() {
        for good in ["a*word*b\n", "a*word* only\n", "only *word*b\n", "走*好*呀\n", "_word_\n"] {
            assert!(!super::violations(good).contains(&"emphasis-marker"), "{good:?}");
        }
        assert!(super::violations("A *word* here.\n").contains(&"emphasis-marker"));
    }

    #[test]
    fn a_pipe_after_two_backslashes_is_not_a_column_boundary() {
        let good = "| Op         | What      |\n| ---------- | --------- |\n| \\\\|Divides |           |\n| Mod        | Remainder |\n";
        assert!(!super::violations(good).contains(&"table-layout"));
        assert!(super::violations(&good.replace("\\\\|Divides ", "\\\\|Divides  ")).contains(&"table-layout"));
    }

    #[test]
    fn ordered_markers_are_read_after_nested_indentation() {
        let good = "1. Parent\n    1. One\n    2. Two\n2. Sibling\n";
        let bad = "1. Parent\n    1. One\n    1. Two\n2. Sibling\n";
        assert!(!super::violations(good).contains(&"ordered-list-marker"));
        assert!(super::violations(bad).contains(&"ordered-list-marker"));
    }

    #[test]
    fn table_alignment_uses_display_columns_for_emoji_sequences() {
        let good = "| Cell | Col |\n| ---- | --- |\n| ✏️   | a   |\n| 👩‍💻   | b   |\n";
        assert!(!super::violations(good).contains(&"table-layout"));
        let bad = good.replace("✏️   |", "✏️    |");
        assert!(super::violations(&bad).contains(&"table-layout"));
    }
}
