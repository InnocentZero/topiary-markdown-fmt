//! List markers are small edits; changing a whole list together avoids splits.
use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};

use crate::{layout, parser, style::Edit};

struct List {
    next_number: Option<u64>,
    alternate: bool,
    edits: Vec<Edit>,
}

pub(crate) fn marker_edits(source: &str) -> Vec<Vec<Edit>> {
    marker_edits_with_indent(source, false)
}

pub(crate) fn reindent_marker_edits(source: &str) -> Vec<Vec<Edit>> {
    marker_edits_with_indent(source, true)
}

pub(crate) fn prose_continuations(source: &str) -> Vec<Vec<Edit>> {
    let mut items = Vec::new();
    let mut unsupported = 0;
    let mut edits = Vec::new();
    for (event, span) in parser(source).into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => {
                let line_start = layout::line_start(source, span.start);
                let prefix = &source[line_start..span.start];
                let line = source[span.start..].split(['\r', '\n']).next().unwrap();
                let body = line.trim_start_matches(' ');
                let digits = body.bytes().take_while(u8::is_ascii_digit).count();
                let column = body.get(digits + 1..).and_then(|after| {
                    let padding = after.bytes().take_while(|&b| b == b' ').count();
                    (prefix.bytes().all(|b| b == b' ') && (1..=4).contains(&padding))
                        .then_some(prefix.len() + line.len() - body.len() + digits + 1 + padding)
                });
                items.push(column);
            }
            Event::End(TagEnd::Item) => { items.pop(); }
            Event::Start(Tag::BlockQuote(_) | Tag::Table(_) | Tag::Heading { .. }) => unsupported += 1,
            Event::End(TagEnd::BlockQuote(_) | TagEnd::Table | TagEnd::Heading(_)) => unsupported -= 1,
            Event::SoftBreak | Event::HardBreak if unsupported == 0 => {
                let Some(&Some(column)) = items.last() else { continue; };
                let rest = &source[span.start..];
                let Some(eol) = rest.find(['\r', '\n']) else { continue; };
                let start = span.start + eol + if rest[eol..].starts_with("\r\n") { 2 } else { 1 };
                let indent = source[start..].bytes().take_while(|&b| b == b' ').count();
                if indent != column && !source[start + indent..].starts_with(['\r', '\n', '\t']) && start + indent < source.len() {
                    edits.push(vec![Edit {
                        span: start..start + indent,
                        replacement: " ".repeat(column),
                        rule: "list-prose-continuation",
                    }]);
                }
            }
            _ => {}
        }
    }
    edits
}

fn marker_edits_with_indent(source: &str, reindent: bool) -> Vec<Vec<Edit>> {
    let mut stack: Vec<List> = Vec::new();
    let mut groups = Vec::new();
    let mut previous: Option<(usize, bool, usize, bool)> = None;
    let protected: Vec<_> = if reindent {
        parser(source)
            .into_offset_iter()
            .filter_map(|(event, span)| {
                matches!(
                    event,
                    Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_))
                )
                .then_some(span)
            })
            .collect()
    } else {
        Vec::new()
    };
    for (event, span) in parser(source).into_offset_iter() {
        match event {
            Event::Start(Tag::List(next_number)) => {
                let adjacent = previous.is_some_and(|(depth, ordered, end, _)| {
                    depth == stack.len()
                        && ordered == next_number.is_some()
                        && end <= span.start
                        && source[end..span.start]
                            .trim_matches([' ', '\t', '\r', '\n', '>'])
                            .is_empty()
                });
                let alternate = adjacent && previous.is_some_and(|(_, _, _, alt)| !alt);
                stack.push(List {
                    next_number,
                    alternate,
                    edits: Vec::new(),
                });
            }
            Event::Start(Tag::Item) => {
                let Some(list) = stack.last_mut() else {
                    continue;
                };
                let raw = source[span.start..].split(['\r', '\n']).next().unwrap();
                // Pulldown includes spare container indentation in some item
                // spans, especially four-space sublists of a `1. ` item.
                let line = raw.trim_start_matches([' ', '\t']);
                let marker_start = span.start + raw.len() - line.len();
                let digits = line.bytes().take_while(u8::is_ascii_digit).count();
                let Some(&marker) = line.as_bytes().get(digits) else {
                    continue;
                };
                let wanted = if let Some(number) = &mut list.next_number {
                    if !matches!(marker, b'.' | b')') || digits == 0 {
                        continue;
                    }
                    let text = format!("{number}.");
                    *number = number.saturating_add(1);
                    text
                } else {
                    if !matches!(marker, b'-' | b'+' | b'*') || digits != 0 {
                        continue;
                    }
                    if list.alternate { "*" } else { "-" }.to_owned()
                };
                let tail = &line[digits + 1..];
                let rest = tail.trim_start_matches([' ', '\t']);
                let end = line.len() - rest.len();
                let wanted = if rest.is_empty() {
                    wanted
                } else {
                    wanted + " "
                };
                if line[..end] != wanted {
                    if reindent && !rest.is_empty() && tail.len() - rest.len() <= 4 {
                        let wanted_len = wanted.len();
                        list.edits.extend(shift_indent(
                            source,
                            &(marker_start..span.end),
                            end,
                            wanted_len,
                            &protected,
                        ));
                    }
                    list.edits.push(Edit {
                        span: marker_start..marker_start + end,
                        replacement: wanted,
                        rule: "list-marker",
                    });
                }
            }
            Event::End(TagEnd::List(ordered)) => {
                if let Some(list) = stack.pop() {
                    previous = Some((stack.len(), ordered, span.end, list.alternate));
                    if !list.edits.is_empty() {
                        groups.push(list.edits);
                    }
                }
            }
            _ => {}
        }
    }
    groups
}

fn shift_indent(
    source: &str,
    item: &Range<usize>,
    old: usize,
    new: usize,
    protected: &[Range<usize>],
) -> Vec<Edit> {
    if old == new {
        return Vec::new();
    }
    let line = layout::line_start(source, item.start);
    let prefix = &source[line..item.start];
    if !prefix.bytes().all(|b| b == b' ') || source[item.start..item.start + old].contains('\t') {
        return Vec::new();
    }
    let column = prefix.len() + old;
    let mut edits = Vec::new();
    let mut anchored = false;
    let mut offset = item.start;
    for line in source[item.clone()].split_inclusive('\n') {
        let i = protected.partition_point(|range| range.end <= offset);
        let indent = line.bytes().take_while(|&b| b == b' ').count();
        if offset != item.start
            && !line.trim().is_empty()
            && indent >= column
            && !protected
                .get(i)
                .is_some_and(|range| range.start < offset + line.len())
        {
            anchored |= indent == column;
            let pos = offset + column;
            let (span, replacement) = if old > new {
                (pos - (old - new)..pos, String::new())
            } else {
                (pos..pos, " ".repeat(new - old))
            };
            edits.push(Edit {
                span,
                replacement,
                rule: "list-item-indent",
            });
        }
        offset += line.len();
    }
    // Don't infer a continuation baseline from deep indentation alone; it
    // can be intentional padding around a nested sibling list.
    if anchored || old < new {
        edits
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn prose_continuation_can_unlock_marker_padding_on_the_first_pass() {
        let source = "-  Start\n    continued\n\n      last\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(output, "- Start\n  continued\n\n     last\n");
        crate::verify(source, &output).unwrap();
        assert_eq!(formatter.format(&output).unwrap(), output);
    }
    #[test]
    fn list_prose_continuation_aligns_to_item_content_without_reflow() {
        let source = "- First paragraph\nlazy line\n     more text  \n last line\n- Next\n\n  ```text\n  keep me\n  ```\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(output, "- First paragraph\n  lazy line\n  more text  \n  last line\n- Next\n\n  ```text\n  keep me\n  ```\n");
        crate::verify(source, &output).unwrap();
        assert_eq!(formatter.format(&output).unwrap(), output);
    }
    #[test]
    fn canonical_list_markers_preserve_structure() {
        let input = "*   an item\n*   another item\n\n+ separate list\n\n3) third\n7) fourth\n";
        let formatter = crate::Formatter::new().unwrap();
        let actual = formatter.format(input).unwrap();
        assert_eq!(
            actual,
            "- an item\n- another item\n\n* separate list\n\n3. third\n4. fourth\n"
        );
        assert_eq!(formatter.format(&actual).unwrap(), actual);
        assert!(crate::strict_violations(&actual).is_empty());
    }

    #[test]
    fn changing_marker_does_not_unindent_code() {
        let source = "1.  first\n7.  second\n\n        indented code\n";
        let actual = crate::Formatter::new().unwrap().format(source).unwrap();
        crate::verify(source, &actual).unwrap();
        assert!(actual.contains("        indented code"));
    }

    #[test]
    fn renumbers_nested_markers_after_spare_container_indentation() {
        let source = "1. Parent\n    1. First child\n    1. Second child\n1. Sibling\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(
            output,
            "1. Parent\n    1. First child\n    2. Second child\n2. Sibling\n"
        );
        crate::verify(source, &output).unwrap();
        assert_eq!(formatter.format(&output).unwrap(), output);
    }

    #[test]
    fn marker_padding_shifts_continuation_blocks_with_it() {
        let source = "-   First\n\n    Continuation.\n\n    - Nested\n\n-   Second\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(
            output,
            "- First\n\n  Continuation.\n\n  - Nested\n\n- Second\n"
        );
        crate::verify(source, &output).unwrap();
        assert_eq!(formatter.format(&output).unwrap(), output);
    }

    #[test]
    fn widening_ordered_marker_keeps_children_and_continuation_in_the_item() {
        let source = "9. First\n1. Second\n\n   Continuation\n\n   - Child\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(
            output,
            "9. First\n10. Second\n\n    Continuation\n\n    - Child\n"
        );
        crate::verify(source, &output).unwrap();
        assert_eq!(formatter.format(&output).unwrap(), output);
    }

    #[test]
    fn shortening_item_padding_keeps_existing_nested_marker_columns() {
        let source = "+ Earlier\n\n+  Parent\n\n    + First\n    + Second\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(
            output,
            "- Earlier\n\n- Parent\n\n    - First\n    - Second\n"
        );
        crate::verify(source, &output).unwrap();
        assert!(crate::strict_violations(&output).is_empty());
        assert_eq!(formatter.format(&output).unwrap(), output);
    }
}
