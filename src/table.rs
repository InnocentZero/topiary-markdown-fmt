use std::ops::Range;

use pulldown_cmark::{Alignment, Event, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

use crate::{layout, parser, style::Edit};

pub(crate) fn edits(source: &str) -> Vec<Vec<Edit>> {
    let mut edits = Vec::new();
    let mut depth = 0;
    let mut nested: Option<(Vec<Alignment>, Vec<Range<usize>>)> = None;
    for (event, span) in parser(source).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(alignments)) if depth == 0 => {
                if let Some(table) = rows(source, span, &alignments)
                    && !table.is_empty()
                {
                    edits.push(table);
                }
            }
            Event::Start(Tag::Table(alignments)) => {
                nested = Some((alignments, Vec::new()));
                depth += 1;
            }
            Event::Start(Tag::TableHead | Tag::TableRow) if nested.is_some() => {
                nested.as_mut().unwrap().1.push(span);
                depth += 1;
            }
            Event::End(TagEnd::Table) if nested.is_some() => {
                let (alignments, rows) = nested.take().unwrap();
                if let Some(table) = nested_rows(source, rows, &alignments)
                    && !table.is_empty()
                {
                    edits.push(table);
                }
                depth -= 1;
            }
            Event::Start(_) => depth += 1,
            Event::End(TagEnd::Table) if depth == 0 => (),
            Event::End(_) => depth -= 1,
            _ => (),
        }
    }
    edits
}

fn cells(line: &str) -> Vec<&str> {
    let mut start = 0;
    let mut escaped = false;
    let mut result = Vec::new();
    let line = line.trim_matches([' ', '\t']);
    for (i, ch) in line.bytes().enumerate() {
        if ch == b'|' && !escaped {
            if i != 0 {
                result.push(line[start..i].trim_matches([' ', '\t']));
            }
            start = i + 1;
        }
        // GFM protects any pipe immediately after a backslash, including
        // after an even-length run; ordinary Markdown escapes use parity.
        escaped = ch == b'\\';
    }
    if start < line.len() {
        result.push(line[start..].trim_matches([' ', '\t']));
    }
    result
}

fn rows(source: &str, span: Range<usize>, alignments: &[Alignment]) -> Option<Vec<Edit>> {
    let start = layout::line_start(source, span.start);
    let mut offset = start;
    let mut lines = Vec::new();
    let mut inside_crlf = false;
    for (i, mut line) in source[start..span.end].split_inclusive('\n').enumerate() {
        let end = offset + line.len();
        if let Some(without) = line.strip_suffix('\n') {
            line = without.strip_suffix('\r').unwrap_or(without);
        } else if let Some(without) = line.strip_suffix('\r') {
            line = without;
            inside_crlf = true;
        }
        let raw = offset..offset + line.len();
        offset = end;
        lines.push(raw);
        if i == 1 && line.is_empty() {
            return None;
        }
    }
    // A range ending inside CRLF still has its original terminator outside the edits.
    if inside_crlf && !source[span.end..].starts_with('\n') || lines.len() < 2 {
        return None;
    }
    align(source, lines, alignments)
}

fn eol(source: &str, start: usize) -> usize {
    source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |n| start + n)
}

fn nested_rows(
    source: &str,
    mut rows: Vec<Range<usize>>,
    alignments: &[Alignment],
) -> Option<Vec<Edit>> {
    // The independent parser provides row starts after container prefixes.
    // Keep those exact prefixes, including quote markers and list indentation.
    for row in &mut rows {
        row.end = eol(source, row.start);
    }
    let after_head = rows.first()?.end;
    let linebreak = source[after_head..]
        .strip_prefix("\r\n")
        .map(|_| 2)
        .or_else(|| source[after_head..].starts_with(['\r', '\n']).then_some(1))?;
    let start = after_head + linebreak;
    let end = eol(source, start);
    let delim = &source[start..end];
    let prefix_len = delim.len() - delim.trim_start_matches([' ', '\t', '>']).len();
    rows.insert(1, start + prefix_len..end);
    align(source, rows, alignments)
}

fn align(source: &str, lines: Vec<Range<usize>>, alignments: &[Alignment]) -> Option<Vec<Edit>> {
    let mut widths = vec![3; alignments.len()];
    let mut parsed = Vec::with_capacity(lines.len());
    for (i, raw) in lines.into_iter().enumerate() {
        let mut cols = cells(&source[raw.clone()]);
        // Do not erase surplus cells which the AST ignores.
        if cols.len() > widths.len() || i <= 1 && cols.len() != widths.len() {
            return None;
        }
        cols.resize(widths.len(), "");
        if i == 1 {
            if !cols
                .iter()
                .all(|cell| cell.bytes().all(|c| matches!(c, b':' | b'-')) && cell.contains('-'))
            {
                return None;
            }
        } else {
            for (width, text) in widths.iter_mut().zip(&cols) {
                *width = (*width).max(text.width());
            }
        }
        parsed.push((raw, cols));
    }
    let mut results = Vec::with_capacity(parsed.len());
    for (i, (span, cols)) in parsed.into_iter().enumerate() {
        let mut replacement = String::from("|");
        for ((text, width), alignment) in cols.into_iter().zip(&widths).zip(alignments) {
            replacement.push(' ');
            let right = matches!(alignment, Alignment::Right | Alignment::Center);
            let left = matches!(alignment, Alignment::Left | Alignment::Center);
            if i == 1 {
                if left {
                    replacement.push(':');
                }
                replacement.extend(std::iter::repeat_n(
                    '-',
                    width - usize::from(left) - usize::from(right),
                ));
                if right {
                    replacement.push(':');
                }
            } else {
                let pad = width - text.width();
                let before = match alignment {
                    Alignment::Right => pad,
                    Alignment::Center => pad / 2,
                    _ => 0,
                };
                replacement.extend(std::iter::repeat_n(' ', before));
                replacement.push_str(text);
                replacement.extend(std::iter::repeat_n(' ', pad - before));
            }
            replacement.push_str(" |");
        }
        if source[span.clone()] != replacement {
            results.push(Edit {
                span,
                replacement,
                rule: "table-layout",
            });
        }
    }
    Some(results)
}
