use std::ops::Range;

use pulldown_cmark::{Event, Tag};

use crate::{layout::line_start, parser};

pub(crate) fn slash_ranges(source: &str) -> Vec<Range<usize>> {
    if !source.contains(r"\(") && !source.contains(r"\[") {
        return Vec::new();
    }
    let mut opaque: Vec<_> = parser(source)
        .into_offset_iter()
        .filter_map(|(event, span)| {
            matches!(
                event,
                Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_))
                    | Event::InlineHtml(_)
                    | Event::Code(_)
                    | Event::InlineMath(_)
                    | Event::DisplayMath(_)
            )
            .then_some(span)
        })
        .collect();
    opaque.sort_unstable_by_key(|span| span.start);
    let mut out = Vec::new();
    let mut open = None;
    let mut opaque_index = 0;
    for (i, _) in source.match_indices('\\') {
        let Some(&next) = source.as_bytes().get(i + 1) else {
            break;
        };
        if !matches!(next, b'(' | b')' | b'[' | b']')
            || source.as_bytes()[..i]
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count()
                % 2
                != 0
        {
            continue;
        }
        let line = line_start(source, i);
        if let Some((b'(', start, _)) = open
            && start < line
        {
            open = None;
        }
        while opaque.get(opaque_index).is_some_and(|span| span.end <= i) {
            opaque_index += 1;
        }
        if opaque
            .get(opaque_index)
            .is_some_and(|span| span.contains(&i))
        {
            continue;
        }
        let prefix = &source[line..i];
        let layout_prefix = prefix.bytes().all(|b| matches!(b, b' ' | b'\t' | b'>'));
        match (open, next) {
            (None, b'(') => open = Some((next, i, line)),
            (None, b'[') if layout_prefix => open = Some((next, i, line)),
            (Some((b'(', start, _)), b')') if i > start + 2 => {
                out.push(start..i + 2);
                open = None;
            }
            (Some((b'[', start, opening_line)), b']')
                if i > start + 2
                    && (opening_line == line || layout_prefix)
                    && source[i + 2..]
                        .split(['\r', '\n'])
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .is_empty() =>
            {
                out.push(start..i + 2);
                open = None;
            }
            (Some((b'[', _, _)), b'[') if layout_prefix => open = Some((next, i, line)),
            _ => {}
        }
    }
    out
}

pub(crate) fn overlaps(ranges: &[Range<usize>], span: &Range<usize>) -> bool {
    ranges
        .get(ranges.partition_point(|range| range.end <= span.start))
        .is_some_and(|range| range.start < span.end)
}
