use std::ops::Range;

use pulldown_cmark::{Event, Tag};

use crate::{layout, math, parser};

#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub span: Range<usize>,
    pub replacement: String,
    pub rule: &'static str,
}

pub(crate) fn edits(source: &str) -> Vec<Edit> {
    let mut edits = Vec::new();
    let mut depth = 0;
    for (event, span) in parser(source).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 && matches!(tag, Tag::Heading { .. }) {
                    if let Some(edit) = atx(source, span) {
                        edits.push(edit);
                    }
                }
                depth += 1;
            }
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
    if !source.is_empty() && !source.ends_with(['\n', '\r']) {
        edits.push(Edit {
            span: source.len()..source.len(),
            replacement: "\n".into(),
            rule: "final-newline",
        });
    }
    edits
}

pub(crate) fn prose_edits(source: &str) -> Vec<Edit> {
    let mut edits = Vec::new();
    for (event, span) in parser(source).into_offset_iter() {
        let (from, keep) = match event {
            Event::SoftBreak => (span.start, 0),
            Event::HardBreak if !source[span.clone()].starts_with('\\') => (span.start, 2),
            Event::End(pulldown_cmark::TagEnd::Paragraph) => (
                span.end
                    - source[span.clone()]
                        .bytes()
                        .rev()
                        .take_while(|b| matches!(b, b'\r' | b'\n'))
                        .count(),
                0,
            ),
            _ => continue,
        };
        let end = source[from..]
            .find(['\r', '\n'])
            .map_or(source.len(), |offset| from + offset);
        let line_start = layout::line_start(source, end);
        let line = &source[line_start..end];
        let body = line.trim_end_matches([' ', '\t']);
        let tail = &line[body.len()..];
        let replacement = if keep == 2 && !tail.is_empty() {
            "  "
        } else {
            ""
        };
        if !body.is_empty() && tail != replacement {
            edits.push(Edit {
                span: line_start + body.len()..end,
                replacement: replacement.into(),
                rule: "prose-trailing-whitespace",
            });
        }
    }
    edits.sort_unstable_by_key(|edit| edit.span.start);
    edits.dedup_by_key(|edit| edit.span.clone());
    edits
}

pub(crate) fn thematic_edits(source: &str) -> Vec<Vec<Edit>> {
    parser(source)
        .into_offset_iter()
        .filter_map(|(event, span)| {
            if !matches!(event, Event::Rule) {
                return None;
            }
            let raw = &source[span.clone()];
            let line = raw.trim_end_matches(['\n', '\r']);
            if matches!(line.trim(), "---" | "***") {
                return None;
            }
            // Asterisks cannot turn the preceding line into a Setext heading
            // or accidentally open a YAML metadata block at document start.
            Some(vec![Edit {
                span: span.start..span.start + line.len(),
                replacement: "***".into(),
                rule: "thematic-break",
            }])
        })
        .collect()
}

fn atx(source: &str, span: Range<usize>) -> Option<Edit> {
    let line_start = layout::line_start(source, span.start);
    let line = source[line_start..].split(['\r', '\n']).next()?;
    let unindented = line.trim_start_matches(' ');
    let count = unindented.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&count) {
        return None;
    }
    let rest = &unindented[count..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_end_matches([' ', '\t']);
    let without_hashes = rest.trim_end_matches('#');
    let text = if without_hashes.ends_with([' ', '\t']) {
        without_hashes
    } else {
        rest
    };
    let text = text.trim_matches([' ', '\t']);
    let replacement = if text.is_empty() {
        "#".repeat(count)
    } else {
        format!("{} {text}", "#".repeat(count))
    };
    (line != replacement).then_some(Edit {
        span: line_start..line_start + line.len(),
        replacement,
        rule: "atx-heading",
    })
}

pub fn violations(source: &str) -> Vec<&'static str> {
    let math = math::slash_ranges(source);
    let mut violations: Vec<_> = edits(source)
        .into_iter()
        .filter(|edit| !math::overlaps(&math, &edit.span))
        .map(|edit| edit.rule)
        .collect();
    if layout::gaps(source).iter().any(|gap| {
        !math::overlaps(&math, &gap.span) && source[gap.span.clone()] != *gap.replacement
    }) {
        violations.push("block-separator");
    }
    violations.sort_unstable();
    violations.dedup();
    violations
}
