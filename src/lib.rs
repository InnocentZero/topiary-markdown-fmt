use std::{borrow::Cow, ops::Range};

use pulldown_cmark::{Event, Options, Parser, Tag};
use topiary_core::{Language, Operation, TopiaryQuery, formatter_str};

mod audit_style;
mod inline;
mod layout;
mod list;
pub use audit_style::violations as strict_violations;
mod math;
mod style;
mod table;
pub use style::violations;

pub struct Formatter {
    language: Language,
    upstream_language: Language,
}

impl Formatter {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            language: Self::language(include_str!("../queries/markdown.scm"))?,
            upstream_language: Self::language(topiary_queries::markdown())?,
        })
    }

    fn language(query: &str) -> Result<Language, String> {
        let grammar = tree_sitter_md::LANGUAGE.into();
        let formatting_query =
            TopiaryQuery::new(&grammar, query).map_err(|error| format!("{error:?}"))?;
        Ok(Language {
            name: "markdown".into(),
            formatting_query,
            injection_query: None,
            grammar,
            indent: None,
        })
    }

    pub fn upstream(&self, source: &str) -> Result<String, String> {
        Self::render(source, &self.upstream_language)
    }

    fn render(source: &str, language: &Language) -> Result<String, String> {
        let mut output = Vec::with_capacity(source.len());
        formatter_str(
            source,
            &mut output,
            language,
            Operation::Format {
                skip_idempotence: true,
                tolerate_parsing_errors: false,
                skip_stage: None,
            },
            None,
        )
        .map_err(|error| format!("{error:?}"))?;
        String::from_utf8(output).map_err(|error| error.to_string())
    }

    pub fn format(&self, source: &str) -> Result<String, String> {
        // Table padding may make the block grammars agree on boundaries they
        // previously disagreed on. Feed the canonical tables into Topiary,
        // rather than discovering those block breaks on the user's next run.
        let staged = normalize_tables(source)?;
        let mut inline_output = Cow::Borrowed(&*staged);
        let mut escaped_literal = false;
        for _ in 0..8 {
            let markers = inline::marker_edits(&inline_output);
            if !markers.is_empty() {
                let next = checked_markers(&inline_output, &markers, 0)?;
                if next != inline_output {
                    // An edited run can resolve an earlier delimiter ambiguity.
                    inline_output = Cow::Owned(next);
                    continue;
                }
            }
            let literal = inline::literal_underscore_edits(&inline_output);
            if literal.is_empty() {
                break;
            }
            let math = math::slash_ranges(&inline_output);
            let literal: Vec<_> = literal.into_iter()
                .filter(|group| !group.iter().any(|edit| math::overlaps(&math, &edit.span)))
                .collect();
            let next = checked_groups(&inline_output, &literal, &mut Vec::new(), 0)?;
            if next == inline_output {
                break;
            }
            escaped_literal = true;
            inline_output = Cow::Owned(next);
        }
        // In contrast to marker substitutions, adding an escape widens the
        // source cell. Settle its columns before returning the first output.
        let staged = if escaped_literal {
            normalize_tables(&inline_output)?
        } else {
            Cow::Borrowed(&*inline_output)
        };
        let mut block_output = Cow::Borrowed(&*staged);
        for _ in 0..8 {
            let mut blocks = list::reindent_marker_edits(&block_output);
            blocks.extend(style::thematic_edits(&block_output));
            let mut next = Cow::Borrowed(&*block_output);
            if !blocks.is_empty() {
                next = Cow::Owned(checked_groups(&block_output, &blocks, &mut Vec::new(), 0)?);
                if next == block_output {
                    let shifts = list::marker_edits(&block_output);
                    if !shifts.is_empty() {
                        next = Cow::Owned(checked_groups(&block_output, &shifts, &mut Vec::new(), 0)?);
                    }
                }
            }
            let continuations = list::prose_continuations(&next);
            if !continuations.is_empty() {
                next = Cow::Owned(checked_groups(&next, &continuations, &mut Vec::new(), 0)?);
            }
            if next == block_output {
                break;
            }
            // Enclosing marker widths can make nested edits safe that were
            // unsafe in the original columns. Normalized prose can likewise
            // establish a continuation column needed to move the marker.
            block_output = Cow::Owned(next.into_owned());
        }
        let staged = &*block_output;
        let mut edits = layout::edits(staged, &self.language)?;
        edits.extend(style::edits(staged));
        edits.extend(style::prose_edits(staged));
        let output = splice(staged, edits)?;
        verify(source, &output).map_err(str::to_owned)?;
        Ok(output)
    }
}

fn normalize_tables(source: &str) -> Result<Cow<'_, str>, String> {
    let tables = table::edits(source);
    if tables.is_empty() {
        return Ok(Cow::Borrowed(source));
    }
    let layout = splice(source, tables.iter().flatten().cloned().collect())?;
    if verify(source, &layout).is_ok() {
        return Ok(Cow::Owned(layout));
    }
    // A disputed table stays unformatted (and nonpassing in the style audit);
    // do not discard unrelated verified table edits.
    let mut current = source.to_owned();
    for edits in tables.into_iter().rev() {
        let next = splice(&current, edits)?;
        if verify(&current, &next).is_ok() {
            current = next;
        }
    }
    Ok(Cow::Owned(current))
}

fn checked_groups(
    source: &str,
    groups: &[Vec<style::Edit>],
    accepted: &mut Vec<style::Edit>,
    depth: u8,
) -> Result<String, String> {
    let keep = accepted.len();
    accepted.extend(groups.iter().flatten().cloned());
    // Enclosing and enclosed items can request incompatible moves on a line.
    // In that case try the groups separately on the original coordinates.
    let mut spans: Vec<_> = accepted.iter().map(|edit| &edit.span).collect();
    spans.sort_unstable_by_key(|range| (range.start, range.end));
    if !spans.windows(2).any(|pair| pair[1].start < pair[0].end) {
        let candidate = splice(source, accepted.clone())?;
        if verify(source, &candidate).is_ok() {
            return Ok(candidate);
        }
    }
    accepted.truncate(keep);
    if groups.len() == 1 || depth >= 7 {
        return splice(source, accepted.clone());
    }
    let (left, right) = groups.split_at(groups.len() / 2);
    checked_groups(source, left, accepted, depth + 1)?;
    checked_groups(source, right, accepted, depth + 1)
}

fn checked_markers(source: &str, groups: &[Vec<style::Edit>], depth: u8) -> Result<String, String> {
    let math = math::slash_ranges(source);
    let candidate = splice(
        source,
        groups
            .iter()
            .filter(|group| !group.iter().any(|edit| math::overlaps(&math, &edit.span)))
            .flatten()
            .cloned()
            .collect(),
    )?;
    if verify(source, &candidate).is_ok() {
        return Ok(candidate);
    }
    if groups.len() == 1 || depth >= 9 {
        return Ok(source.to_owned());
    }
    // Both marker spellings have the same byte length; offsets stay valid.
    let (left, right) = groups.split_at(groups.len() / 2);
    let left = checked_markers(source, left, depth + 1)?;
    checked_markers(&left, right, depth + 1)
}

fn splice(source: &str, mut edits: Vec<style::Edit>) -> Result<String, String> {
    let math = math::slash_ranges(source);
    edits.retain(|edit| !math::overlaps(&math, &edit.span));
    edits.sort_unstable_by_key(|edit| (edit.span.start, edit.span.end));
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for edit in edits {
        if edit.span.start < cursor {
            return Err(format!(
                "overlapping layout edits at byte {}",
                edit.span.start
            ));
        }
        output.push_str(&source[cursor..edit.span.start]);
        output.push_str(&edit.replacement);
        cursor = edit.span.end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

pub fn parser(source: &str) -> Parser<'_> {
    Parser::new_ext(
        source,
        Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_GFM
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
            | Options::ENABLE_MATH,
    )
}

fn references<'a>(parser: &Parser<'a>) -> Vec<(String, String, Option<String>)> {
    let mut definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(name, link)| {
            (
                name.to_lowercase(),
                link.dest.to_string(),
                link.title.as_deref().map(str::to_owned),
            )
        })
        .collect();
    definitions.sort_unstable();
    definitions
}

fn signature<'a>(source: &'a str, parser: Parser<'a>) -> (Vec<Event<'a>>, Vec<&'a str>) {
    let mut ast: Vec<Event<'a>> = Vec::new();
    let mut protected = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        if matches!(
            event,
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_))
                | Event::InlineHtml(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
        ) {
            protected.push(source_without_boundary(source, range));
        }
        match (ast.last_mut(), event) {
            (Some(Event::Text(prev)), Event::Text(next)) => {
                let mut merged = prev.to_string();
                merged.push_str(&next);
                *prev = merged.into();
            }
            (_, event) => ast.push(event),
        }
    }
    (ast, protected)
}

fn source_without_boundary(source: &str, range: Range<usize>) -> &str {
    // A final line terminator is a document boundary, not part of a closing delimiter.
    let raw = &source[range];
    raw.strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix(['\r', '\n']))
        .unwrap_or(raw)
}

pub struct Verification {
    pub ast: bool,
    pub protected: bool,
    pub issue: Option<&'static str>,
}

pub fn verification(source: &str, output: &str) -> Verification {
    if source == output {
        return Verification {
            ast: true,
            protected: true,
            issue: None,
        };
    }
    let (before, after) = (parser(source), parser(output));
    let reference_eq = references(&before) == references(&after);
    let (before_ast, before_protected) = signature(source, before);
    let (after_ast, after_protected) = signature(output, after);
    let ast_eq = before_ast == after_ast;
    let protected = before_protected == after_protected
        && math::slash_ranges(source)
            .into_iter()
            .map(|range| &source[range])
            .eq(math::slash_ranges(output)
                .into_iter()
                .map(|range| &output[range]));
    Verification {
        ast: reference_eq && ast_eq,
        protected,
        issue: if !reference_eq {
            Some("reference-definitions-changed")
        } else if !ast_eq {
            Some("markdown-ast-changed")
        } else if !protected {
            Some("protected-bytes-changed")
        } else {
            None
        },
    }
}

pub fn verify(source: &str, output: &str) -> Result<(), &'static str> {
    verification(source, output).issue.map_or(Ok(()), Err)
}
