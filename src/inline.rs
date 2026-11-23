use pulldown_cmark::{Event, Tag};

use crate::{parser, style::Edit};

pub(crate) fn marker_edits(source: &str) -> Vec<Vec<Edit>> {
    let mut edits = Vec::new();
    for (event, span) in parser(source).into_offset_iter() {
        let (old, new) = match event {
            Event::Start(Tag::Emphasis) => ("*", "_"),
            Event::Start(Tag::Strong) => ("__", "**"),
            _ => continue,
        };
        let raw = &source[span.clone()];
        if raw.len() <= 2 * old.len() || !raw.starts_with(old) || !raw.ends_with(old) {
            continue;
        }
        let starts_touch =
            source[..span.start].ends_with(['_', '*']) || raw[old.len()..].starts_with(['_', '*']);
        let ends_touch = source[span.end..].starts_with(['_', '*'])
            || raw[..raw.len() - old.len()].ends_with(['_', '*']);
        if starts_touch && ends_touch || old == "__" && (starts_touch || ends_touch) {
            // A fully shared delimiter run needs coordinated handling. An
            // emphasis with one independent end can be split safely if the
            // complete candidate preserves the AST.
            continue;
        }
        edits.push(vec![
            Edit {
                span: span.start..span.start + old.len(),
                replacement: new.into(),
                rule: "inline-markers",
            },
            Edit {
                span: span.end - old.len()..span.end,
                replacement: new.into(),
                rule: "inline-markers",
            },
        ]);
    }
    edits
}

pub(crate) fn literal_underscore_edits(source: &str) -> Vec<Vec<Edit>> {
    parser(source)
        .into_offset_iter()
        .filter_map(|(event, span)| {
            if !matches!(event, Event::Start(Tag::Emphasis)) {
                return None;
            }
            let body = source[span.clone()].strip_prefix('*')?.strip_suffix('*')?;
            let first = body.starts_with('_');
            let last = body.ends_with('_');
            if (!first && !last) || body.len() < 2 {
                return None;
            }
            // Keep a literal underscore at the edge of the emphasis.
            // AST verification rejects the edit if this is nested markup.
            Some(vec![
                Edit {
                    span: span.start..span.start + 1 + usize::from(first),
                    replacement: if first { "_\\_" } else { "_" }.into(),
                    rule: "inline-literal-underscore",
                },
                Edit {
                    span: span.end - 1 - usize::from(last)..span.end,
                    replacement: if last { "\\__" } else { "_" }.into(),
                    rule: "inline-literal-underscore",
                },
            ])
        })
        .collect()
}
