use super::*;
use crate::core::config::Language;

#[test]
fn test_clean_doc_strips_examples() {
    let doc = "Does something.\n\n# Examples\n\n```rust\nfoo();\n```\n";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(!cleaned.contains("Examples"));
    assert!(!cleaned.contains("foo()"));
    assert!(cleaned.contains("Does something"));
}

#[test]
fn test_clean_doc_strips_arguments() {
    let doc = "Does something.\n\n# Arguments\n\n* html - The HTML string\n\nMore text.";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(!cleaned.contains("Arguments"));
    assert!(!cleaned.contains("html - The HTML string"));
    assert!(cleaned.contains("Does something"));
    assert!(cleaned.contains("More text"));
}

#[test]
fn test_clean_doc_rust_links() {
    let doc = "See [`field`](Self::field) for details.";
    let cleaned = clean_doc(doc, Language::Python);
    assert_eq!(cleaned, "See `field` for details.");
}

#[test]
fn test_clean_doc_bare_rust_links() {
    let doc = "See [`ParseOptions`] for details.";
    let cleaned = clean_doc(doc, Language::Python);
    assert_eq!(cleaned, "See `ParseOptions` for details.");
}

#[test]
fn test_collapse_adjacent_code_spans_merges_wrapped_link() {
    let input = "JSON-serialised `Vec<``StructuredOutput``>` (a JSON array).";
    let merged = collapse_adjacent_code_spans(input);
    assert_eq!(merged, "JSON-serialised `Vec<StructuredOutput>` (a JSON array).");
}

#[test]
fn test_collapse_adjacent_code_spans_leaves_separated_spans() {
    let input = "Returns `a` and `b` separately.";
    assert_eq!(collapse_adjacent_code_spans(input), input);
}

#[test]
fn test_collapse_adjacent_code_spans_preserves_double_backtick_span() {
    let input = "Use ``a ` b`` for a literal tick.";
    assert_eq!(collapse_adjacent_code_spans(input), input);
}

#[test]
fn test_collapse_adjacent_code_spans_ignores_fenced_code_block() {
    let input = "Text.\n\n```rust\nlet x = `a``b`;\n```\n\nMore `c``d` text.";
    let merged = collapse_adjacent_code_spans(input);
    assert!(merged.contains("let x = `a``b`;"));
    assert!(merged.contains("More `cd` text."));
}

#[test]
fn test_clean_doc_merges_wrapped_link_spans() {
    let doc = "JSON-serialised `Vec<`[`StructuredOutput`]`>` on success.";
    let cleaned = clean_doc(doc, Language::Zig);
    assert!(!cleaned.contains("``"), "no doubled backtick run: {cleaned}");
    assert!(cleaned.contains('`'), "code span preserved: {cleaned}");
}

#[test]
fn test_clean_doc_normalizes_feature_label_versions() {
    let doc = "Since v5.0.0.\n\nChanged in v1.6.2-rc.3.\n\nAvailable by v0.3.0+build.7.";
    let cleaned = clean_doc(doc, Language::Python);
    assert_eq!(cleaned, "Since v5.0.\n\nChanged in v1.6.\n\nAvailable by v0.3.");
}

#[test]
fn test_extract_param_docs() {
    let doc = "Convert markup conversion.\n\n# Arguments\n\n* html - The HTML string to convert\n* options - Conversion options\n";
    let params = extract_param_docs(doc);
    assert_eq!(
        params.get("html").map(String::as_str),
        Some("The HTML string to convert")
    );
    assert_eq!(params.get("options").map(String::as_str), Some("Conversion options"));
}

#[test]
fn test_clean_doc_empty_string_all_languages() {
    for lang in [Language::Python, Language::Go, Language::Node, Language::Rust] {
        assert_eq!(clean_doc("", lang), "", "empty doc for {lang:?} must stay empty");
    }
}

#[test]
fn test_clean_doc_multiline_prose_all_paragraphs_preserved() {
    let doc = "First line.\n\nSecond paragraph.\n\nThird paragraph.";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(cleaned.contains("First line."));
    assert!(cleaned.contains("Second paragraph."));
    assert!(cleaned.contains("Third paragraph."));
}

#[test]
fn test_clean_doc_none_becomes_nil_for_go_ruby_elixir() {
    let doc = "Returns `None` when nothing is found.";
    assert_eq!(clean_doc(doc, Language::Go), "Returns `nil` when nothing is found.");
    assert_eq!(clean_doc(doc, Language::Ruby), "Returns `nil` when nothing is found.");
    assert_eq!(clean_doc(doc, Language::Elixir), "Returns `nil` when nothing is found.");
}

#[test]
fn test_clean_doc_none_becomes_null_for_node_java_csharp_php() {
    let doc = "Returns `None` on failure.";
    assert_eq!(clean_doc(doc, Language::Node), "Returns `null` on failure.");
    assert_eq!(clean_doc(doc, Language::Java), "Returns `null` on failure.");
    assert_eq!(clean_doc(doc, Language::Csharp), "Returns `null` on failure.");
    assert_eq!(clean_doc(doc, Language::Php), "Returns `null` on failure.");
}

#[test]
fn test_clean_doc_none_stays_none_for_python_and_rust() {
    let doc = "Returns `None` when empty.";
    assert_eq!(clean_doc(doc, Language::Python), "Returns `None` when empty.");
    assert_eq!(clean_doc(doc, Language::Rust), "Returns `None` when empty.");
}

#[test]
fn test_clean_doc_none_becomes_null_uppercase_for_r_and_ffi() {
    let doc = "Returns `None` when empty.";
    assert_eq!(clean_doc(doc, Language::R), "Returns `NULL` when empty.");
    assert_eq!(clean_doc(doc, Language::Ffi), "Returns `NULL` when empty.");
}

#[test]
fn test_clean_doc_python_booleans_capitalised() {
    let doc = "Pass `true` to enable or `false` to disable.";
    let cleaned = clean_doc(doc, Language::Python);
    assert_eq!(cleaned, "Pass `True` to enable or `False` to disable.");
}

#[test]
fn test_clean_doc_non_python_booleans_lowercase_unchanged() {
    let doc = "Pass `true` to enable or `false` to disable.";
    assert_eq!(clean_doc(doc, Language::Go), doc);
    assert_eq!(clean_doc(doc, Language::Node), doc);
    assert_eq!(clean_doc(doc, Language::Java), doc);
}

#[test]
fn test_clean_doc_rust_path_becomes_dot_notation_for_python() {
    let doc = "Call `Foo::bar()` to create one.";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(cleaned.contains("Foo.bar()"), "expected dot notation: {cleaned}");
    assert!(!cleaned.contains("Foo::bar()"));
}

#[test]
fn test_clean_doc_rust_path_stays_double_colon_for_php() {
    let doc = "Call `Foo::bar()` to create one.";
    let cleaned = clean_doc(doc, Language::Php);
    assert!(cleaned.contains("Foo::bar()"), "PHP keeps :: notation: {cleaned}");
}

#[test]
fn test_clean_doc_rust_collection_terms_become_language_native() {
    let doc = "Returns `Vec<Vec<String>>`, accepts `&BTreeMap<String, String>`, and stores `Vec<u8>` bytes.";

    assert_eq!(
        clean_doc(doc, Language::Python),
        "Returns `list[list[str]]`, accepts `dict[str, str]`, and stores `bytes` bytes."
    );
    assert_eq!(
        clean_doc(doc, Language::Node),
        "Returns `string[][]`, accepts `Record<string, string>`, and stores `Buffer` bytes."
    );
    assert_eq!(
        clean_doc(doc, Language::Go),
        "Returns `[][]string`, accepts `map[string]string`, and stores `[]byte` bytes."
    );
}

#[test]
fn test_clean_doc_rust_collection_terms_keep_php_static_paths() {
    let doc = "Use `NodeContext::attributes` to read `BTreeMap<String, String>` values.";
    let cleaned = clean_doc(doc, Language::Php);
    assert_eq!(
        cleaned,
        "Use `NodeContext::attributes` to read `array<string, string>` values."
    );
}

#[test]
fn test_clean_doc_rust_collection_terms_stay_rust_for_rust_docs() {
    let doc = "Returns `Vec<String>` from `NodeContext::attributes`.";
    assert_eq!(clean_doc(doc, Language::Rust), doc);
}

#[test]
fn test_clean_doc_rust_internal_markers_become_clean_prose() {
    let doc = "Use `ConversionError::from(io_error)` to convert from `std::io::Error`. Prefer `Foo::bar` (pub(crate)) internally.";
    let cleaned = clean_doc(doc, Language::Python);

    assert_eq!(
        cleaned,
        "Use `ConversionError.from(io_error)` to convert from an operating-system I/O error. Prefer `Foo.bar` internally."
    );
}

#[test]
fn test_clean_doc_private_helper_references_become_neutral_prose() {
    let doc = "Access attributes through `NodeContext::with_lazy_attributes`. Populated when `ConversionOptions::include_document_structure` is `true` and exposed in `Self::tables`. Defaults to `PreprocessingOptions::default()`, which enables cleaning (or construct via `ConversionOptions::builder`).";
    let cleaned = clean_doc(doc, Language::Python);

    assert_eq!(
        cleaned,
        "Access attributes through lazy attribute extraction. Populated when the `include_document_structure` option is `True` and exposed in the result's `tables` field. Defaults to the standard preprocessing options, which enables cleaning (or construct via the configuration builder)."
    );
}

#[test]
fn test_clean_doc_vec_macros_become_list_examples() {
    let doc =
        "Example: `vec![\".cookie-banner\".into()]`. The length of this vec is bounded. An empty vec returns no cells.";
    let cleaned = clean_doc(doc, Language::Node);

    assert_eq!(
        cleaned,
        "Example: `[\".cookie-banner\"]`. The length of this list is bounded. An empty list returns no cells."
    );
}

#[test]
fn test_clean_doc_generic_rust_vec_terms_become_language_native() {
    let doc = "Uses Arc-wrapped tables: `Vec<Arc<Table>>`. Stores middleware in `Vec<Box<dyn ChunkMiddleware>>`. Serializes as Vec<Table> for JSON.";

    let python = clean_doc(doc, Language::Python);
    assert_eq!(
        python,
        "Uses shared tables: `list[Table]`. Stores middleware in `list[ChunkMiddleware]`. Serializes as list[Table] for JSON."
    );

    let typescript = clean_doc(doc, Language::Node);
    assert_eq!(
        typescript,
        "Uses shared tables: `Table[]`. Stores middleware in `ChunkMiddleware[]`. Serializes as Table[] for JSON."
    );
}

#[test]
fn test_clean_doc_strips_rust_fenced_blocks_with_collect_turbofish() {
    let doc = "Before.\n\n```rust\nasync fn process(result: &mut ExtractionResult) -> Result<()> {\n    result.content = result.content.split_whitespace().collect::<Vec<_>>().join(\" \");\n    Ok(())\n}\n```\n\nAfter.";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(cleaned.contains("Before."));
    assert!(cleaned.contains("After."));
    assert!(!cleaned.contains("collect::<Vec<_>>()"));
}

#[test]
fn test_clean_doc_non_rust_code_block_preserved() {
    let doc = "Example:\n\n```python\nresult = convert(html)\n```\n";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(cleaned.contains("```python"));
    assert!(cleaned.contains("result = convert(html)"));
}

#[test]
fn test_clean_doc_rust_code_block_stripped() {
    let doc = "Example:\n\n```rust\nuse foo::Bar;\nBar::new().unwrap();\n```\n\nAfter block.";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(!cleaned.contains("use foo::Bar"), "Rust use statement must be stripped");
    assert!(cleaned.contains("After block."));
}

#[test]
fn test_clean_doc_errors_section_heading_becomes_bold() {
    let doc = "Summary.\n\n# Errors\n\nMay fail.\n";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(cleaned.contains("**Errors:**"), "heading must become bold: {cleaned}");
    assert!(!cleaned.contains("# Errors"), "raw # heading must be gone: {cleaned}");
}

#[test]
fn test_clean_doc_returns_section_heading_becomes_bold() {
    let doc = "Summary.\n\n# Returns\n\nSome value.\n";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(cleaned.contains("**Returns:**"));
    assert!(!cleaned.contains("# Returns"));
}

#[test]
fn test_clean_doc_crate_references_replaced_with_library() {
    let doc = "Available in this crate as a public API.";
    assert_eq!(
        clean_doc(doc, Language::Python),
        "Available in this library as a public API."
    );
}

#[test]
fn test_clean_doc_inline_code_spans_survive_for_rust() {
    let doc = "Use `None` or `false` to skip.";
    let cleaned = clean_doc(doc, Language::Rust);
    assert!(cleaned.contains("`None`"));
    assert!(cleaned.contains("`false`"));
}

#[test]
fn test_clean_doc_inline_empty_string() {
    assert_eq!(clean_doc_inline("", Language::Python), "");
    assert_eq!(clean_doc_inline("", Language::Go), "");
}

#[test]
fn test_clean_doc_inline_collapses_multiline_to_single_line() {
    let doc = "First sentence.\nSecond sentence.";
    let result = clean_doc_inline(doc, Language::Python);
    assert!(!result.contains('\n'), "inline output must be single-line: {result}");
    assert!(result.contains("First sentence."));
    assert!(result.contains("Second sentence."));
}

#[test]
fn test_clean_doc_inline_does_not_escape_pipes() {
    let doc = "Value between 0 | 1.";
    let result = clean_doc_inline(doc, Language::Python);
    assert!(
        !result.contains("\\|"),
        "pipe must not be pre-escaped by clean_doc_inline: {result}"
    );
    assert!(
        result.contains(" | "),
        "raw pipe must be preserved for caller to escape: {result}"
    );
    let cell = crate::docs::formatting::escape_table_cell(&result);
    assert!(
        cell.contains("\\|"),
        "caller escape_table_cell must escape the pipe: {cell}"
    );
    assert!(!cell.contains("\\\\|"), "pipe must not be double-escaped: {cell}");
}

#[test]
fn test_clean_doc_inline_applies_language_terminology() {
    let doc = "Returns `None` when empty.";
    assert_eq!(clean_doc_inline(doc, Language::Go), "Returns `nil` when empty.");
    assert_eq!(clean_doc_inline(doc, Language::Node), "Returns `null` when empty.");
}

#[test]
fn test_clean_doc_inline_strips_argument_sections() {
    let doc = "Summary.\n\n# Arguments\n\n* foo - bar\n";
    let result = clean_doc_inline(doc, Language::Python);
    assert!(!result.contains("Arguments"));
    assert!(!result.contains("foo - bar"));
    assert!(result.contains("Summary."));
}

#[test]
fn test_clean_doc_inline_filters_blank_only_lines() {
    let doc = "\n\n  \n\nActual content.\n\n  \n";
    let result = clean_doc_inline(doc, Language::Python);
    assert_eq!(result, "Actual content.");
}

/// Regression test for MD056 double-escaping.
///
/// `clean_doc_inline` must NOT escape pipes itself. All call sites in
/// `lib.rs` pass the result through `escape_table_cell`, so internal pipe
/// escaping inside `clean_doc_inline` causes double-escaping:
///   `||`  →  `\|\|`  (clean_doc_inline)  →  `\\|\\|`  (escape_table_cell)
/// The CommonMark parser then sees `\\` as an escaped backslash (literal `\`)
/// followed by an unescaped `|` (cell separator), splitting one cell into two
/// and triggering MD056.
///
/// The correct output after the full pipeline is `\|\|` (each pipe escaped
/// exactly once by `escape_table_cell`).
#[test]
fn test_clean_doc_inline_does_not_double_escape_pipes_in_logical_or() {
    let doc = "The length of this vec is ≤ rows * cols. An empty table (rows == 0 || cols == 0) produces an empty vec.";
    let raw = clean_doc_inline(doc, Language::Python);
    assert!(
        !raw.contains("\\|"),
        "clean_doc_inline must not escape pipes (double-escaping bug): {raw}"
    );
    let cell = crate::docs::formatting::escape_table_cell(&raw);
    assert!(
        cell.contains("\\|\\|"),
        "after escape_table_cell the || must become \\|\\|, got: {cell}"
    );
    assert!(
        !cell.contains("\\\\|"),
        "double-escaped \\\\| must not appear, got: {cell}"
    );
}

#[test]
fn test_wrap_bare_urls_plain_https() {
    let text = "See https://example.com for details.";
    assert_eq!(wrap_bare_urls(text), "See <https://example.com> for details.");
}

#[test]
fn test_wrap_bare_urls_plain_http() {
    let text = "Visit http://example.com today.";
    assert_eq!(wrap_bare_urls(text), "Visit <http://example.com> today.");
}

#[test]
fn test_wrap_bare_urls_skips_already_angle_bracketed() {
    let text = "See <https://example.com> already wrapped.";
    assert_eq!(wrap_bare_urls(text), text);
}

#[test]
fn test_wrap_bare_urls_skips_markdown_link_url() {
    let text = "See [docs](https://example.com/docs) for more.";
    assert_eq!(wrap_bare_urls(text), text);
}

#[test]
fn test_wrap_bare_urls_multiple_bare_urls() {
    let text = "A: https://a.com B: https://b.com";
    assert_eq!(wrap_bare_urls(text), "A: <https://a.com> B: <https://b.com>");
}

#[test]
fn test_wrap_bare_urls_mixed_bare_and_already_wrapped() {
    let text = "Visit <https://wrapped.com> or https://bare.com";
    assert_eq!(
        wrap_bare_urls(text),
        "Visit <https://wrapped.com> or <https://bare.com>"
    );
}

#[test]
fn test_wrap_bare_urls_url_at_start_of_string() {
    let text = "https://example.com is the homepage.";
    assert_eq!(wrap_bare_urls(text), "<https://example.com> is the homepage.");
}

#[test]
fn test_wrap_bare_urls_url_at_end_of_string() {
    let text = "Homepage: https://example.com";
    assert_eq!(wrap_bare_urls(text), "Homepage: <https://example.com>");
}

#[test]
fn test_wrap_bare_urls_no_urls() {
    let text = "No links here, just prose.";
    assert_eq!(wrap_bare_urls(text), text);
}

#[test]
fn test_wrap_bare_urls_empty_string() {
    assert_eq!(wrap_bare_urls(""), "");
}

#[test]
fn test_demote_headings_single_level() {
    let doc = "# Heading 1\n\nSome text.\n\n## Heading 2";
    let demoted = demote_headings(doc, 1);
    assert!(demoted.contains("## Heading 1"), "H1 should become H2");
    assert!(demoted.contains("### Heading 2"), "H2 should become H3");
}

#[test]
fn test_demote_headings_multiple_levels() {
    let doc = "# Heading 1\n## Heading 2\n### Heading 3";
    let demoted = demote_headings(doc, 2);
    assert!(demoted.contains("### Heading 1"), "H1 should become H3");
    assert!(demoted.contains("#### Heading 2"), "H2 should become H4");
    assert!(demoted.contains("##### Heading 3"), "H3 should become H5");
}

#[test]
fn test_demote_headings_skips_code_blocks() {
    let doc = "# Heading\n\n```rust\n# Not a heading\n```\n\nMore text.";
    let demoted = demote_headings(doc, 1);
    assert!(demoted.contains("## Heading"), "H1 outside code should become H2");
    assert!(
        demoted.contains("# Not a heading"),
        "content inside code block should not be modified"
    );
}

#[test]
fn test_demote_headings_zero_levels_unchanged() {
    let doc = "# Heading\n## Subheading";
    let demoted = demote_headings(doc, 0);
    assert_eq!(demoted, doc, "zero demotion should return unchanged");
}

#[test]
fn test_demote_headings_caps_at_h6() {
    let doc = "##### Heading 5";
    let demoted = demote_headings(doc, 5);
    assert!(demoted.contains("###### Heading 5"), "should not exceed H6");
    let h6 = demote_headings("###### Heading 6", 1);
    assert!(h6.contains("###### Heading 6"), "H6 should stay at H6");
}

#[test]
fn test_demote_headings_preserves_trailing_content() {
    let doc = "# Title\n\nParagraph text.\n\n## Section\n\nMore text.";
    let demoted = demote_headings(doc, 1);
    assert!(demoted.contains("## Title"));
    assert!(demoted.contains("Paragraph text."));
    assert!(demoted.contains("### Section"));
    assert!(demoted.contains("More text."));
}

#[test]
fn test_demote_headings_to_start_at_demotes_h1_to_target() {
    let doc = "# Details\n\n## More Details";
    let demoted = demote_headings_to_start_at(doc, 5);
    assert!(demoted.contains("##### Details"));
    assert!(demoted.contains("###### More Details"));
}

#[test]
fn test_demote_headings_to_start_at_uses_first_heading_level() {
    let doc = "## Default Behavior\n\n### Edge Cases";
    let demoted = demote_headings_to_start_at(doc, 5);
    assert!(demoted.contains("##### Default Behavior"));
    assert!(demoted.contains("###### Edge Cases"));
    assert!(!demoted.contains("###### Default Behavior"));
}

#[test]
fn test_demote_headings_to_start_at_ignores_code_blocks() {
    let doc = "```markdown\n# Not a heading\n```\n\n## Real Heading";
    let demoted = demote_headings_to_start_at(doc, 5);
    assert!(demoted.contains("# Not a heading"));
    assert!(demoted.contains("##### Real Heading"));
}

#[test]
fn test_check_monotonic_headings_valid_increments() {
    let doc = "## Page\n\n### Section\n\n#### Item\n\n##### Subitem";
    assert!(check_monotonic_headings(doc).is_ok());
}

#[test]
fn test_check_monotonic_headings_valid_skips_down() {
    let doc = "## Page\n\n### Section\n\n## Another Section\n\nText.";
    assert!(check_monotonic_headings(doc).is_ok());
}

#[test]
fn test_check_monotonic_headings_detects_skip_up() {
    let doc = "## Page\n\n#### Item (skip H3)";
    let result = check_monotonic_headings(doc);
    assert!(result.is_err(), "should detect skip from H2 to H4");
    assert!(result.unwrap_err().contains("skip of 2"));
}

#[test]
fn test_check_monotonic_headings_ignores_code_blocks() {
    let doc = "## Page\n\n```markdown\n#### This is not a real heading\n```";
    assert!(
        check_monotonic_headings(doc).is_ok(),
        "headings in code blocks should be ignored"
    );
}

#[test]
fn test_demote_headings_maintains_monotonic_increments() {
    let doc = "## Sub-page\n\n### Section\n\n#### Item";
    let demoted = demote_headings(doc, 2);
    assert!(
        check_monotonic_headings(&demoted).is_ok(),
        "demoted headings should maintain monotonic increments"
    );
}

#[test]
fn test_doc_comment_with_internal_headings_demoted() {
    let doc_comment = "Main description.\n\n## Stream Limits\n\nDetailed info about limits.";
    let cleaned = clean_doc(doc_comment, Language::Python);
    let demoted = demote_headings(&cleaned, 2);
    assert!(
        demoted.contains("#### Stream Limits"),
        "internal heading should be demoted to #### (was ##)"
    );
    assert!(
        check_monotonic_headings(&demoted).is_ok(),
        "demoted doc comment should have monotonic heading increments"
    );
}

#[test]
fn test_ensure_blank_before_lists_inserts_blank_after_prose() {
    let doc = "For a typical element like `<div>`:\n1. Open tag\n2. Close tag\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(
        result, "For a typical element like `<div>`:\n\n1. Open tag\n2. Close tag\n",
        "blank line must be inserted before the ordered list"
    );
}

#[test]
fn test_ensure_blank_before_lists_unordered_after_prose() {
    let doc = "Available options:\n- one\n- two\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(result, "Available options:\n\n- one\n- two\n");
}

#[test]
fn test_ensure_blank_before_lists_preserves_existing_blank_line() {
    let doc = "Intro.\n\n- one\n- two\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(result, "Intro.\n\n- one\n- two\n", "must not add a second blank line");
}

#[test]
fn test_ensure_blank_before_lists_keeps_contiguous_list_items_tight() {
    let doc = "- one\n- two\n- three\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(result, doc, "contiguous list items must remain tight");
}

#[test]
fn test_ensure_blank_before_lists_ignores_lists_inside_fenced_code() {
    let doc = "Code:\n\n```\nintro\n- not a list\n```\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(result, doc, "content inside fenced code blocks must not be touched");
}

#[test]
fn test_ensure_blank_before_lists_does_not_split_emphasis_markers() {
    let doc = "Plain text.\n*not a list item*\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(result, "Plain text.\n*not a list item*\n");
}

#[test]
fn test_ensure_blank_before_lists_handles_ordered_with_paren() {
    let doc = "Steps:\n1) first\n2) second\n";
    let result = ensure_blank_before_lists(doc);
    assert_eq!(result, "Steps:\n\n1) first\n2) second\n");
}

#[test]
fn test_clean_doc_inserts_blank_line_before_list_md032() {
    let doc = "# Execution Order\n\nFor a typical element like `<div>`:\n1. Step one\n2. Step two\n";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(
        cleaned.contains(":\n\n1."),
        "blank line must separate prose from list: {cleaned}"
    );
}

#[test]
fn test_normalize_list_markers_converts_asterisk_to_dash() {
    let doc = "Items:\n* First item\n* Second item";
    let normalized = normalize_list_markers(doc);
    assert!(normalized.contains("- First item"), "* should be converted to -");
    assert!(normalized.contains("- Second item"), "* should be converted to -");
    assert!(!normalized.contains("* First"), "* marker should be replaced");
}

#[test]
fn test_normalize_list_markers_preserves_emphasis() {
    let doc = "Text with *emphasis* and *more emphasis*.";
    let normalized = normalize_list_markers(doc);
    assert!(normalized.contains("*emphasis*"), "emphasis markers must be preserved");
    assert!(!normalized.contains("- emphasis"), "emphasis must not become a list");
}

#[test]
fn test_normalize_list_markers_preserves_bold() {
    let doc = "Text with **bold** content.";
    let normalized = normalize_list_markers(doc);
    assert!(normalized.contains("**bold**"), "bold markers must be preserved");
}

#[test]
fn test_normalize_list_markers_indented_lists() {
    let doc = "Parent:\n* Item 1\n  * Nested item";
    let normalized = normalize_list_markers(doc);
    assert!(normalized.contains("- Item 1"));
    assert!(
        normalized.contains("- Nested item"),
        "indented asterisk must also convert"
    );
}

#[test]
fn test_normalize_list_markers_skips_code_blocks() {
    let doc = "Text:\n\n```markdown\n* Item in code block\n```\n\n* Real list item";
    let normalized = normalize_list_markers(doc);
    assert!(
        normalized.contains("* Item in code block"),
        "code block content must be preserved"
    );
    assert!(
        normalized.contains("- Real list item"),
        "real list items must be converted"
    );
}

#[test]
fn test_collapse_whitespace_multiline() {
    let s = "First line\nSecond line\nThird line";
    let collapsed = collapse_whitespace(s);
    assert_eq!(collapsed, "First line Second line Third line");
    assert!(!collapsed.contains('\n'));
}

#[test]
fn test_collapse_whitespace_with_empty_lines() {
    let s = "First\n\n\nSecond";
    let collapsed = collapse_whitespace(s);
    assert_eq!(collapsed, "First Second");
}

#[test]
fn test_collapse_whitespace_with_extra_spaces() {
    let s = "Text   with    multiple     spaces";
    let collapsed = collapse_whitespace(s);
    assert!(
        collapsed.contains("Text with multiple spaces") || collapsed.contains("Text  with"),
        "extra spaces should be normalized"
    );
}

#[test]
fn test_collapse_whitespace_empty() {
    assert_eq!(collapse_whitespace(""), "");
    assert_eq!(collapse_whitespace("   \n\n  "), "");
}

#[test]
fn test_field_default_with_multiline_collapsed() {
    let raw = "value_line_1\nvalue_line_2";
    let collapsed = collapse_whitespace(raw);
    let formatted = format!("`{collapsed}`");
    assert_eq!(formatted, "`value_line_1 value_line_2`");
    assert!(!formatted.contains('\n'), "backtick code span must be single-line");
}

#[test]
fn test_clean_doc_normalizes_asterisk_list_markers_to_dash() {
    let doc = "Summary.\n\n* First item\n* Second item";
    let cleaned = clean_doc(doc, Language::Python);
    assert!(
        cleaned.contains("- First item"),
        "asterisk lists should be normalized to dash: {cleaned}"
    );
    assert!(
        !cleaned.contains("* First"),
        "raw asterisk list markers should not remain"
    );
}
