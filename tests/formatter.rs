use topiary_markdown::{Formatter, verify, violations};

#[test]
fn emphasis_escapes_a_literal_leading_underscore() {
    let source = "*_Block*<br/>\n\nThe argument *_Nx* is here.\n";
    let fmt = Formatter::new().unwrap();
    let output = fmt.format(source).unwrap();
    assert_eq!(output, "_\\_Block_<br/>\n\nThe argument _\\_Nx_ is here.\n");
    verify(source, &output).unwrap();
    assert_eq!(fmt.format(&output).unwrap(), output);
    assert!(topiary_markdown::strict_violations(&output).is_empty());
}

#[test]
fn table_columns_settle_after_an_emphasis_escape() {
    let source = "| Account | Cost |\n| --- | --- |\n| Prefix *_Account* | 4 |\n";
    let fmt = Formatter::new().unwrap();
    let output = fmt.format(source).unwrap();
    assert!(output.contains("Prefix _\\_Account_ |"));
    verify(source, &output).unwrap();
    assert_eq!(fmt.format(&output).unwrap(), output);
    assert!(topiary_markdown::strict_violations(&output).is_empty());
}

#[test]
fn read_only_style_audit_rejects_preliminary_false_greens() {
    let source = "# Title\n\n* One\n* Two\n\nA *word* and a __strong word__ with a trailing space \ncontinuation. \n\n|a|bb|\n|---|---|\n|abc|d|\n\n* * *\n";
    let issues = topiary_markdown::strict_violations(source);
    for expected in [
        "unordered-list-marker",
        "emphasis-marker",
        "strong-marker",
        "prose-trailing-whitespace",
        "table-layout",
        "thematic-break",
    ] {
        assert!(
            issues.contains(&expected),
            "{expected} not found: {issues:?}"
        );
    }
    let canonical = "# Title\n\n- One\n- Two\n\nA _word_ and a **strong word**.  \nContinuation.\n\n| a   | bb  |\n| --- | --- |\n| abc | d   |\n\n---\n\n```\n  untouched \t\n```\n";
    assert_eq!(
        Vec::<&str>::new(),
        topiary_markdown::strict_violations(canonical)
    );
}

#[test]
fn pads_table_columns_without_rewriting_cells() {
    let before = "| a | b|\n|:-|--:|\n| `x\\|y`| 二|\n";
    let expected = "| a      |   b |\n| :----- | --: |\n| `x\\|y` |  二 |\n";
    let formatter = Formatter::new().unwrap();
    assert_eq!(expected, formatter.format(before).unwrap());
    assert_eq!(expected, formatter.format(expected).unwrap());
    assert!(topiary_markdown::strict_violations(expected).is_empty());
}

#[test]
fn table_padding_does_not_delay_block_layout_until_the_second_pass() {
    let input = "|  |  |\n| --- | --- |\n| long content | second col |\n  \n<a name=\"example\"></a>\n## Heading\n  Paragraph.\n  \n- Item\n";
    let formatter = Formatter::new().unwrap();
    let first = formatter.format(input).unwrap();
    assert!(first.contains("| long content | second col |"));
    assert_eq!(first, formatter.format(&first).unwrap());
    assert_eq!(Ok(()), verify(input, &first));
}

#[test]
fn emphasis_markers_do_not_change_inline_structure_or_math() {
    let input = "A *word*, __a strong word__, and another *word*.\n\nEmbedded alpha*beta*gamma cannot use underscores. Inline \\( * untouched * \\).\n";
    let formatter = Formatter::new().unwrap();
    let first = formatter.format(input).unwrap();
    assert!(
        first.contains("A _word_, **a strong word**, and another _word_."),
        "{first}"
    );
    assert!(first.contains("alpha*beta*gamma"));
    assert!(first.contains(r"\( * untouched * \)"));
    assert_eq!(first, formatter.format(&first).unwrap());
    assert_eq!(Ok(()), verify(input, &first));
}

#[test]
fn prose_whitespace_does_not_erase_hard_breaks_or_touch_opaque_text() {
    let source = "One trailing space \nNext line.    \nHard break above. \n\nEnd with spaces   \n\n```text\nKeep trailing spaces   \n```\n";
    let expected = "One trailing space\nNext line.  \nHard break above.\n\nEnd with spaces\n\n```text\nKeep trailing spaces   \n```\n";
    let formatter = Formatter::new().unwrap();
    assert_eq!(expected, formatter.format(source).unwrap());
    assert_eq!(expected, formatter.format(expected).unwrap());
    assert!(topiary_markdown::strict_violations(expected).is_empty());
}

#[test]
fn ordinary_block_layout_does_not_join_paragraphs() {
    let input = "#  Title ###\nFirst paragraph.\n\n\nSecond paragraph.\n\n- An item\n\nTrailing paragraph.\n";
    let expected =
        "# Title\n\nFirst paragraph.\n\nSecond paragraph.\n\n- An item\n\nTrailing paragraph.\n";
    let formatter = Formatter::new().unwrap();
    let actual = formatter.format(input).unwrap();
    assert_eq!(expected, actual);
    assert_eq!(Ok(()), verify(input, &actual));
    assert!(violations(&actual).is_empty());
    assert_eq!(actual, formatter.format(&actual).unwrap());
}

#[test]
fn opaque_leaves_keep_code_and_reference_spelling() {
    let input = "    indented\n\n#  Title\n[ref]: <example> \"quote\"\n\n[ref]\n\n```x\r\n  code\t \r\n```\r\n";
    let formatter = Formatter::new().unwrap();
    let output = formatter.format(input).unwrap();
    assert!(output.starts_with("    indented\n"));
    assert!(output.contains("\n[ref]: <example> \"quote\"\n\n[ref]\n"));
    assert!(output.ends_with("```x\r\n  code\t \r\n```\r\n"));
    assert_eq!(Ok(()), verify(input, &output));
    assert_eq!(output, formatter.format(&output).unwrap());
}

#[test]
fn adjacent_definitions_have_stable_spacing() {
    let input = "References: [first], [second], [third].\n\n[first]: https://example.org/one\n\n[second]: https://example.org/two\n\n[third]: https://example.org/three\n";
    let expected = "References: [first], [second], [third].\n\n[first]: https://example.org/one\n[second]: https://example.org/two\n[third]: https://example.org/three\n";
    let formatter = Formatter::new().unwrap();
    assert_eq!(formatter.format(input).unwrap(), expected);
    assert_eq!(formatter.format(expected).unwrap(), expected);
}

#[test]
fn latex_payloads_keep_blank_lines_and_spaces() {
    let input = "#   Title\nAn inline \\( x + y \\) and $ z +  w $.\n\n\\[\n\n\n  x + y \\tag{1}  \n\n\\]\n\n$$\n a +  b\n$$\n";
    let formatter = Formatter::new().unwrap();
    let output = formatter.format(input).unwrap();
    assert!(output.starts_with("# Title\n\n"));
    assert!(output.contains("\\[\n\n\n  x + y \\tag{1}  \n\n\\]"));
    assert!(output.contains(r"\( x + y \)"));
    assert!(output.contains("$$\n a +  b\n$$"));
    assert!(verify(input, &output.replace("x + y \\tag", "x  + y \\tag")).is_err());
    assert_eq!(output, formatter.format(&output).unwrap());
    assert!(violations(&output).is_empty());
}
#[test]
fn thematic_rule_inside_quote_does_not_become_a_heading() {
    let input = "Introduction\n____\n\n> Previous paragraph\n> * * *\n";
    let formatter = topiary_markdown::Formatter::new().unwrap();
    let output = formatter.format(input).unwrap();
    assert_eq!(
        output,
        "Introduction\n\n***\n\n> Previous paragraph\n> ***\n"
    );
    assert!(topiary_markdown::verification(input, &output).ast);
    assert_eq!(formatter.format(&output).unwrap(), output);
}
#[test]
fn nested_list_renumbering_settles_after_enclosing_markers_widen() {
    let input = format!(
        "{}1. Last top-level item\n    1. First subitem\n    1. Second subitem\n        - Child of second\n    1. Third subitem\n",
        "1. Repeated item\n".repeat(10)
    );
    let formatter = topiary_markdown::Formatter::new().unwrap();
    let output = formatter.format(&input).unwrap();
    assert!(output.contains("    2. Second subitem"), "{output}");
    topiary_markdown::verify(&input, &output).unwrap();
    assert_eq!(formatter.format(&output).unwrap(), output);
}
#[test]
fn aligns_nested_tables_without_touching_container_prefixes() {
    let input =
        "> |Name|Code|\n> |--|--:|\n> |large|0|\n\n- Example\n\n  |x|y|\n  |-|--|\n  |中文|row|\n";
    let formatter = topiary_markdown::Formatter::new().unwrap();
    let output = formatter.format(input).unwrap();
    assert!(
        output.contains("> | Name  | Code |\n> | ----- | ---: |"),
        "{output}"
    );
    assert!(
        output.contains("  | x    | y   |\n  | ---- | --- |"),
        "{output}"
    );
    topiary_markdown::verify(input, &output).unwrap();
    assert_eq!(formatter.format(&output).unwrap(), output);
    assert!(!topiary_markdown::strict_violations(&output).contains(&"table-layout"));
}
#[test]
fn related_emphasis_edits_settle_before_returning_output() {
    let input = "- **labels**<a name=\"refs-labels\"></a>: *object*\n  <br>*additional properties: false*\n  - **`^[a-z][a-z0-9_-]{0,62}$`**: *string*\n    <br>*pattern: ^[a-z0-9_-]{0,63}$*\n";
    let formatter = topiary_markdown::Formatter::new().unwrap();
    let output = formatter.format(input).unwrap();
    topiary_markdown::verify(input, &output).unwrap();
    assert!(output.contains("_string_"), "{output}");
    assert_eq!(formatter.format(&output).unwrap(), output);
}
#[test]
fn table_pipes_after_backslash_runs_stay_in_their_cells() {
    let source = "|Op|What|\n|---|---|\n|\\\\|Divides|\n|Mod|Remainder|\n";
    let formatter = Formatter::new().unwrap();
    let actual = formatter.format(source).unwrap();
    assert!(actual.contains("| \\\\|Divides |           |"), "{actual:?}");
    verify(source, &actual).unwrap();
    assert_eq!(formatter.format(&actual).unwrap(), actual);
}
#[test]
fn emphasis_can_split_from_the_closing_strong_run() {
    let source = "**Logic inside event handlers is *not reactive.***\n";
    let formatter = Formatter::new().unwrap();
    let actual = formatter.format(source).unwrap();
    assert_eq!(actual, "**Logic inside event handlers is _not reactive._**\n");
    verify(source, &actual).unwrap();
    assert_eq!(formatter.format(&actual).unwrap(), actual);
}
