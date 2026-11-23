use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};
use topiary_core::{Atom, Language, apply_query};

use crate::{parser, style::Edit};

#[derive(Debug)]
pub(crate) struct Gap {
    pub span: Range<usize>,
    pub next: Range<usize>,
    pub replacement: &'static str,
    tight_references: bool,
    footnote_boundary: bool,
}

pub(crate) fn line_start(source: &str, offset: usize) -> usize {
    source[..offset].rfind(['\r', '\n']).map_or(0, |i| i + 1)
}

pub(crate) fn gaps(source: &str) -> Vec<Gap> {
    let parsed = parser(source);
    let mut blocks: Vec<_> = parsed
        .reference_definitions()
        .iter()
        .map(|(_, definition)| (definition.span.clone(), true, false))
        .collect();
    let mut reference_starts: Vec<_> = blocks.iter().map(|(span, _, _)| span.start).collect();
    reference_starts.sort_unstable();
    let mut depth = 0;
    let mut last_item_end = None;
    for (event, range) in parsed.into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    blocks.push((range, false, matches!(tag, Tag::FootnoteDefinition(_))));
                }
                depth += 1;
            }
            Event::End(tag) => {
                if tag == TagEnd::Item && depth == 2 {
                    last_item_end = Some(range.end);
                } else if matches!(tag, TagEnd::List(_)) && depth == 1 {
                    // The parser can extend a list's span over the first of
                    // several unindented references after its last item.
                    // Use the actual item boundary for those references.
                    if let Some(end) = last_item_end.take()
                        && reference_starts
                            .get(reference_starts.partition_point(|&start| start < end))
                            .is_some_and(|&start| start < range.end)
                        && let Some((last, _, _)) = blocks.last_mut()
                    {
                        last.end = end;
                    }
                }
                depth -= 1;
            }
            Event::Rule | Event::DisplayMath(_) if depth == 0 => blocks.push((range, false, false)),
            _ => {}
        }
    }
    blocks.sort_unstable_by_key(|(span, _, _)| (span.start, span.end));
    let mut gaps = Vec::new();
    let mut previous: Option<(Range<usize>, bool, bool)> = None;
    for (next, reference, footnote) in blocks {
        if previous.as_ref().is_some_and(|(p, _, _)| p.end > next.start) {
            // A definition inside a visible container is not a block boundary.
            continue;
        }
        if let Some((prev, prev_reference, prev_footnote)) =
            previous.replace((next.clone(), reference, footnote))
        {
            let next_line = line_start(source, next.start);
            let body = source[prev.start..prev.end].trim_end_matches([' ', '\t', '\r', '\n']);
            let end = prev.start + body.len();
            let line_end = source[end..next_line.max(end)]
                .find(['\r', '\n'])
                .map_or(end, |i| end + i);
            // Keep the preceding line's trailing spaces (e.g. hard breaks) and
            // the next line's indentation. Ignore same-line and disputed gaps.
            if line_end < next_line {
                let gap = &source[line_end..next_line];
                if gap
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                {
                    let crlf = gap.starts_with("\r\n");
                    let cr = !crlf && gap.starts_with('\r');
                    let tight = prev_reference && reference;
                    let replacement = match (tight, crlf, cr) {
                        (true, true, _) => "\r\n",
                        (false, true, _) => "\r\n\r\n",
                        (true, _, true) => "\r",
                        (false, _, true) => "\r\r",
                        (true, _, _) => "\n",
                        (false, _, _) => "\n\n",
                    };
                    gaps.push(Gap {
                        span: line_end..next_line,
                        next: next_line..next.end,
                        replacement,
                        tight_references: tight,
                        footnote_boundary: prev_footnote || footnote,
                    });
                }
            }
        }
    }
    gaps
}

pub(crate) fn edits(source: &str, language: &Language) -> Result<Vec<Edit>, String> {
    let atoms = apply_query(source, &language.formatting_query, &language.grammar, true)
        .map_err(|err| format!("{err:?}"))?;
    let mut starts = vec![0];
    starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    let mut separator = false;
    let mut positions = Vec::new();
    for atom in &atoms[..] {
        match atom {
            Atom::Literal(text) if text == "\n\n" => separator = true,
            Atom::Leaf {
                original_position, ..
            } => {
                if separator && let Some(row) = starts.get(original_position.row as usize - 1) {
                    positions.push(row + original_position.column as usize - 1);
                }
                separator = false;
            }
            _ => {}
        }
    }
    positions.sort_unstable();
    let pending: Vec<_> = gaps(source)
        .into_iter()
        .filter(|gap| source[gap.span.clone()] != *gap.replacement)
        .collect();
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let mut code = false;
    let mut payloads = Vec::new();
    for (event, span) in parser(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code = true,
            Event::End(TagEnd::CodeBlock) => code = false,
            Event::Text(_) if code => payloads.push(span),
            _ => {}
        }
    }
    Ok(pending
        .into_iter()
        .filter(|gap| {
            // Tree-sitter can group adjacent reference definitions under
            // one leaf. Use the independent definition offsets within it.
            gap.tight_references
                || gap.footnote_boundary
                || positions
                    .get(positions.partition_point(|&p| p < gap.next.start))
                    .is_some_and(|p| {
                        *p < gap.next.end
                            && *p
                                < gap.next.start
                                    + source[gap.next.clone()]
                                        .find(['\r', '\n'])
                                        .unwrap_or(gap.next.len())
                    })
        })
        .filter_map(|gap| {
            let edit = Edit {
                span: gap.span,
                replacement: gap.replacement.into(),
                rule: "block-separator",
            };
            let next = payloads.partition_point(|span| span.end <= edit.span.start);
            if gap.footnote_boundary || payloads.get(next).is_some_and(|span| span.start < edit.span.end) {
                // The block grammar can fold footnotes into other leaves.
                // Use their independently verified boundaries.
                // Indented code can include a boundary terminator in its
                // span without treating more blank lines as code. An unclosed
                // fence, on the other hand, can consume every inserted line.
                let trial = crate::splice(source, vec![edit.clone()]).ok()?;
                crate::verify(source, &trial).ok()?;
            }
            Some(edit)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn adjacent_footnotes_get_blank_lines_like_prettier() {
        let source = "A[^one] and B[^two].\n\n[^one]: First.\n[^two]: Second.\n\n## Next\n";
        let fmt = crate::Formatter::new().unwrap();
        let output = fmt.format(source).unwrap();
        assert_eq!(output, source.replace("First.\n[^two]", "First.\n\n[^two]"));
        crate::verify(source, &output).unwrap();
        assert_eq!(fmt.format(&output).unwrap(), output);
    }
    #[test]
    fn safe_separator_after_an_indented_code_payload_is_still_formatted() {
        let source = "    raw\n# Heading\n";
        let fmt = crate::Formatter::new().unwrap();
        let output = fmt.format(source).unwrap();
        assert_eq!(output, "    raw\n\n# Heading\n");
        crate::verify(source, &output).unwrap();
        assert_eq!(fmt.format(&output).unwrap(), output);
    }
    #[test]
    fn does_not_add_a_separator_to_an_unclosed_nested_code_payload() {
        let source = "#  A heading\n\n1. Item\n\n    ```text\n    payload\n    ```\t\nOutside\n";
        let fmt = crate::Formatter::new().unwrap();
        let output = fmt.format(source).unwrap();
        assert_eq!(output, source.replacen("#  A heading", "# A heading", 1));
        crate::verify(source, &output).unwrap();
        assert_eq!(fmt.format(&output).unwrap(), output);
        assert!(crate::strict_violations(&output).contains(&"block-separator"));
    }
    #[test]
    fn reference_gap_repro() {
        let source = "- See [A][].\n- See [B][].\n\n[A]: ./one/\n[B]: https://example.org/\n";
        let formatter = crate::Formatter::new().unwrap();
        let output = formatter.format(source).unwrap();
        assert_eq!(source, output);
        crate::verify(source, &output).unwrap();
        assert_eq!(formatter.format(&output).unwrap(), output);
    }
}
