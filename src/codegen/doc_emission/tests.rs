use super::*;

#[test]
fn test_emit_phpdoc() {
    let mut out = String::new();
    emit_phpdoc(&mut out, "Simple documentation", "    ", "TestException");
    assert!(out.contains("/**"));
    assert!(out.contains("Simple documentation"));
    assert!(out.contains("*/"));
}

#[test]
fn test_phpdoc_escaping() {
    let mut out = String::new();
    emit_phpdoc(&mut out, "Handle */ sequences", "", "TestException");
    assert!(out.contains("Handle * / sequences"));
}

#[test]
fn test_emit_csharp_doc() {
    let mut out = String::new();
    emit_csharp_doc(&mut out, "C# documentation", "    ", "TestException");
    assert!(out.contains("<summary>"));
    assert!(out.contains("C# documentation"));
    assert!(out.contains("</summary>"));
}

#[test]
fn test_csharp_xml_escaping() {
    let mut out = String::new();
    emit_csharp_doc(&mut out, "foo < bar & baz > qux", "", "TestException");
    assert!(out.contains("foo &lt; bar &amp; baz &gt; qux"));
}

#[test]
fn test_emit_elixir_doc() {
    let mut out = String::new();
    emit_elixir_doc(&mut out, "Elixir documentation");
    assert!(out.contains("@doc \"\"\""));
    assert!(out.contains("Elixir documentation"));
    assert!(out.contains("\"\"\""));
}

#[test]
fn test_elixir_heredoc_escaping() {
    let mut out = String::new();
    emit_elixir_doc(&mut out, "Handle \"\"\" sequences");
    assert!(out.contains("Handle \"\" \" sequences"));
}

#[test]
fn test_emit_roxygen() {
    let mut out = String::new();
    emit_roxygen(&mut out, "R documentation");
    assert!(out.contains("#' R documentation"));
}

#[test]
fn test_emit_swift_doc() {
    let mut out = String::new();
    emit_swift_doc(&mut out, "Swift documentation", "    ");
    assert!(out.contains("/// Swift documentation"));
}

#[test]
fn test_emit_javadoc() {
    let mut out = String::new();
    emit_javadoc(&mut out, "Java documentation", "    ");
    assert!(out.contains("/**"));
    assert!(out.contains("Java documentation"));
    assert!(out.contains("*/"));
}

#[test]
fn test_emit_kdoc() {
    let mut out = String::new();
    emit_kdoc(&mut out, "Kotlin documentation", "    ");
    assert!(out.contains("/**"));
    assert!(out.contains("Kotlin documentation"));
    assert!(out.contains("*/"));
}

#[test]
fn test_emit_dartdoc() {
    let mut out = String::new();
    emit_dartdoc(&mut out, "Dart documentation", "    ");
    assert!(out.contains("/// Dart documentation"));
}

#[test]
fn test_emit_gleam_doc() {
    let mut out = String::new();
    emit_gleam_doc(&mut out, "Gleam documentation", "    ");
    assert!(out.contains("/// Gleam documentation"));
}

#[test]
fn test_emit_zig_doc() {
    let mut out = String::new();
    emit_zig_doc(&mut out, "Zig documentation", "    ");
    assert!(out.contains("/// Zig documentation"));
}

#[test]
fn test_empty_doc_skipped() {
    let mut out = String::new();
    emit_phpdoc(&mut out, "", "", "TestException");
    emit_csharp_doc(&mut out, "", "", "TestException");
    emit_elixir_doc(&mut out, "");
    emit_roxygen(&mut out, "");
    emit_kdoc(&mut out, "", "");
    emit_dartdoc(&mut out, "", "");
    emit_gleam_doc(&mut out, "", "");
    emit_zig_doc(&mut out, "", "");
    assert!(out.is_empty());
}

#[test]
fn test_doc_first_paragraph_joined_single_line() {
    assert_eq!(doc_first_paragraph_joined("Simple doc."), "Simple doc.");
}

#[test]
fn test_doc_first_paragraph_joined_wrapped_sentence() {
    let doc = "Convert markup conversion,\nreturning a result.";
    assert_eq!(
        doc_first_paragraph_joined(doc),
        "Convert markup conversion, returning a result."
    );
}

#[test]
fn test_doc_first_paragraph_joined_stops_at_blank_line() {
    let doc = "First paragraph.\nStill first.\n\nSecond paragraph.";
    assert_eq!(doc_first_paragraph_joined(doc), "First paragraph. Still first.");
}

#[test]
fn test_doc_first_paragraph_joined_empty() {
    assert_eq!(doc_first_paragraph_joined(""), "");
}

#[test]
fn test_parse_rustdoc_sections_basic() {
    let doc = "Extracts text from a file.\n\n# Arguments\n\n* `path` - The file path.\n\n# Returns\n\nThe extracted text.\n\n# Errors\n\nReturns `SampleCrateError` on failure.";
    let sections = parse_rustdoc_sections(doc);
    assert_eq!(sections.summary, "Extracts text from a file.");
    assert_eq!(sections.arguments.as_deref(), Some("* `path` - The file path."));
    assert_eq!(sections.returns.as_deref(), Some("The extracted text."));
    assert_eq!(
        sections.errors.as_deref(),
        Some("Returns `SampleCrateError` on failure.")
    );
    assert!(sections.panics.is_none());
}

#[test]
fn test_parse_rustdoc_sections_example_with_fence() {
    let doc = "Run the thing.\n\n# Example\n\n```rust\nlet x = run();\n```";
    let sections = parse_rustdoc_sections(doc);
    assert_eq!(sections.summary, "Run the thing.");
    assert!(sections.example.as_ref().unwrap().contains("```rust"));
    assert!(sections.example.as_ref().unwrap().contains("let x = run();"));
}

#[test]
fn test_parse_rustdoc_sections_pound_inside_fence_is_not_a_heading() {
    let doc = "Summary.\n\n# Example\n\n```bash\n# install deps\nrun --foo\n```";
    let sections = parse_rustdoc_sections(doc);
    assert_eq!(sections.summary, "Summary.");
    assert!(sections.example.as_ref().unwrap().contains("# install deps"));
}

#[test]
fn test_parse_arguments_bullets_dash_separator() {
    let body = "* `path` - The file path.\n* `config` - Optional configuration.";
    let pairs = parse_arguments_bullets(body);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], ("path".to_string(), "The file path.".to_string()));
    assert_eq!(pairs[1], ("config".to_string(), "Optional configuration.".to_string()));
}

#[test]
fn test_parse_arguments_bullets_continuation_line() {
    let body = "* `path` - The file path,\n  resolved relative to cwd.\n* `mode` - Open mode.";
    let pairs = parse_arguments_bullets(body);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].1, "The file path, resolved relative to cwd.");
}

#[test]
fn test_replace_fence_lang_rust_to_typescript() {
    let body = "```rust\nlet x = run();\n```";
    let out = replace_fence_lang(body, "typescript");
    assert!(out.starts_with("```typescript"));
    assert!(out.contains("let x = run();"));
}

#[test]
fn test_replace_fence_lang_preserves_attrs() {
    let body = "```rust,no_run\nlet x = run();\n```";
    let out = replace_fence_lang(body, "typescript");
    assert!(out.starts_with("```typescript,no_run"));
}

#[test]
fn test_replace_fence_lang_no_fence_unchanged() {
    let body = "Plain prose with `inline code`.";
    let out = replace_fence_lang(body, "typescript");
    assert_eq!(out, "Plain prose with `inline code`.");
}

fn fixture_sections() -> RustdocSections {
    let doc = "Extracts text from a file.\n\n# Arguments\n\n* `path` - The file path.\n* `config` - Optional configuration.\n\n# Returns\n\nThe extracted text and metadata.\n\n# Errors\n\nReturns an error when the file is unreadable.\n\n# Example\n\n```rust\nlet result = extract(\"file.pdf\")?;\n```";
    parse_rustdoc_sections(doc)
}

#[test]
fn test_render_jsdoc_sections() {
    let sections = fixture_sections();
    let out = render_jsdoc_sections(&sections);
    assert!(out.starts_with("Extracts text from a file."));
    assert!(out.contains("@param path - The file path."));
    assert!(out.contains("@param config - Optional configuration."));
    assert!(out.contains("@returns The extracted text and metadata."));
    assert!(out.contains("@throws Returns an error when the file is unreadable."));
    assert!(!out.contains("@example"), "Rust example must not appear in TSDoc");
    assert!(!out.contains("```typescript"));
    assert!(!out.contains("```rust"));
}

#[test]
fn test_render_jsdoc_sections_preserves_typescript_example() {
    let doc = "Do something.\n\n# Example\n\n```typescript\nconst x = doSomething();\n```";
    let sections = parse_rustdoc_sections(doc);
    let out = render_jsdoc_sections(&sections);
    assert!(out.contains("@example"), "TypeScript example must be preserved");
    assert!(out.contains("```typescript"));
}

#[test]
fn test_render_javadoc_sections() {
    let sections = fixture_sections();
    let out = render_javadoc_sections(&sections, "SampleCrateRsException");
    assert!(out.contains("@param path The file path."));
    assert!(out.contains("@return The extracted text and metadata."));
    assert!(out.contains("@throws SampleCrateRsException Returns an error when the file is unreadable."));
    assert!(out.starts_with("Extracts text from a file."));
}

#[test]
fn test_render_csharp_xml_sections() {
    let sections = fixture_sections();
    let out = render_csharp_xml_sections(&sections, "SampleCrateException");
    assert!(out.contains("<summary>\nExtracts text from a file.\n</summary>"));
    assert!(out.contains("<param name=\"path\">The file path.</param>"));
    assert!(out.contains("<returns>The extracted text and metadata.</returns>"));
    assert!(out.contains("<exception cref=\"SampleCrateException\">"));
    assert!(out.contains("<example><code language=\"csharp\">"));
    assert!(out.contains("let result = extract"));
}

#[test]
fn test_render_phpdoc_sections() {
    let sections = fixture_sections();
    let out = render_phpdoc_sections(&sections, "SampleCrateException");
    assert!(out.contains("@param mixed $path The file path."));
    assert!(out.contains("@return The extracted text and metadata."));
    assert!(out.contains("@throws SampleCrateException"));
    assert!(!out.contains("```php"), "Rust example must not appear in PHPDoc");
    assert!(!out.contains("```rust"));
}

#[test]
fn test_render_phpdoc_sections_preserves_php_example() {
    let doc = "Do something.\n\n# Example\n\n```php\n$x = doSomething();\n```";
    let sections = parse_rustdoc_sections(doc);
    let out = render_phpdoc_sections(&sections, "MyException");
    assert!(out.contains("```php"), "PHP example must be preserved");
}

#[test]
fn test_render_doxygen_sections() {
    let sections = fixture_sections();
    let out = render_doxygen_sections(&sections);
    assert!(out.contains("\\param path The file path."));
    assert!(out.contains("\\return The extracted text and metadata."));
    assert!(out.contains("\\code"));
    assert!(out.contains("\\endcode"));
}

#[test]
fn test_emit_yard_doc_simple() {
    let mut out = String::new();
    emit_yard_doc(&mut out, "Simple Ruby documentation", "    ");
    assert!(out.contains("# Simple Ruby documentation"));
}

#[test]
fn test_emit_yard_doc_empty() {
    let mut out = String::new();
    emit_yard_doc(&mut out, "", "    ");
    assert!(out.is_empty());
}

#[test]
fn test_emit_yard_doc_with_sections() {
    let mut out = String::new();
    let doc = "Extracts text from a file.\n\n# Arguments\n\n* `path` - The file path.\n\n# Returns\n\nThe extracted text.\n\n# Errors\n\nReturns error on failure.";
    emit_yard_doc(&mut out, doc, "  ");
    assert!(out.contains("# Extracts text from a file."));
    assert!(out.contains("# @param path The file path."));
    assert!(out.contains("# @return The extracted text."));
    assert!(out.contains("# @raise Returns error on failure."));
}

#[test]
fn test_emit_c_doxygen_simple_prose() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "Free a string.", "");
    assert!(out.contains("/// Free a string."), "got: {out}");
}

#[test]
fn test_emit_c_doxygen_with_sections() {
    let mut out = String::new();
    let doc = "Extract content from a file.\n\n# Arguments\n\n* `path` - Path to the file.\n* `mode` - Read mode.\n\n# Returns\n\nA newly allocated string the caller owns.\n\n# Errors\n\nReturns null when the file is unreadable.";
    emit_c_doxygen(&mut out, doc, "");
    assert!(out.contains("/// Extract content from a file."));
    assert!(out.contains("/// \\param path Path to the file."));
    assert!(out.contains("/// \\param mode Read mode."));
    assert!(out.contains("/// \\return A newly allocated string the caller owns."));
    assert!(out.contains("/// \\note Returns null when the file is unreadable."));
}

#[test]
fn test_emit_c_doxygen_safety_section_maps_to_note() {
    let mut out = String::new();
    let doc = "Free a buffer.\n\n# Safety\n\nPointer must have been returned by this library.";
    emit_c_doxygen(&mut out, doc, "");
    assert!(out.contains("/// \\note SAFETY: Pointer must have been returned by this library."));
}

#[test]
fn test_emit_c_doxygen_example_renders_code_fence() {
    let mut out = String::new();
    let doc = "Demo.\n\n# Example\n\n```rust\nlet x = run();\n```";
    emit_c_doxygen(&mut out, doc, "");
    assert!(out.contains("/// \\code"));
    assert!(out.contains("/// \\endcode"));
    assert!(out.contains("let x = run();"));
}

#[test]
fn test_emit_c_doxygen_strips_markdown_links() {
    let mut out = String::new();
    let doc = "See [the docs](https://example.com/x) for details.";
    emit_c_doxygen(&mut out, doc, "");
    assert!(
        out.contains("the docs (https://example.com/x)"),
        "expected flattened link, got: {out}"
    );
    assert!(!out.contains("](https://"));
}

#[test]
fn test_emit_c_doxygen_word_wraps_long_lines() {
    let mut out = String::new();
    let long = "a ".repeat(80);
    emit_c_doxygen(&mut out, long.trim(), "");
    for line in out.lines() {
        let body = line.trim_start_matches("/// ");
        assert!(body.len() <= 100, "line too long ({}): {line}", body.len());
    }
}

#[test]
fn test_emit_c_doxygen_empty_input_is_noop() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "", "");
    emit_c_doxygen(&mut out, "   \n\t  ", "");
    assert!(out.is_empty());
}

#[test]
fn test_emit_c_doxygen_indent_applied() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "Hello.", "    ");
    assert!(out.starts_with("    /// Hello."));
}

#[test]
fn test_render_yard_sections() {
    let sections = fixture_sections();
    let out = render_yard_sections(&sections);
    assert!(out.contains("@param path The file path."));
    assert!(out.contains("@return The extracted text and metadata."));
    assert!(out.contains("@raise Returns an error when the file is unreadable."));
    assert!(!out.contains("@example"), "Rust example must not appear in YARD");
    assert!(!out.contains("```ruby"));
    assert!(!out.contains("```rust"));
}

#[test]
fn test_render_yard_sections_preserves_ruby_example() {
    let doc = "Do something.\n\n# Example\n\n```ruby\nputs :hi\n```";
    let sections = parse_rustdoc_sections(doc);
    let out = render_yard_sections(&sections);
    assert!(out.contains("@example"), "Ruby example must be preserved");
    assert!(out.contains("```ruby"));
}

#[test]
fn example_for_target_rust_fenced_suppressed_for_php() {
    let example = "```rust\nlet x = 1;\n```";
    assert_eq!(
        example_for_target(example, "php"),
        None,
        "rust-fenced example must be omitted for PHP target"
    );
}

#[test]
fn example_for_target_bare_fence_defaults_to_rust_suppressed_for_ruby() {
    let example = "```\nlet x = 1;\n```";
    assert_eq!(
        example_for_target(example, "ruby"),
        None,
        "bare fence is treated as Rust and must be omitted for Ruby target"
    );
}

#[test]
fn example_for_target_php_example_preserved_for_php() {
    let example = "```php\n$x = 1;\n```";
    let result = example_for_target(example, "php");
    assert!(result.is_some(), "PHP example must be preserved for PHP target");
    assert!(result.unwrap().contains("```php"));
}

#[test]
fn example_for_target_ruby_example_preserved_for_ruby() {
    let example = "```ruby\nputs :hi\n```";
    let result = example_for_target(example, "ruby");
    assert!(result.is_some(), "Ruby example must be preserved for Ruby target");
    assert!(result.unwrap().contains("```ruby"));
}

#[test]
fn render_phpdoc_sections_with_rust_example_emits_no_at_example_block() {
    let doc = "Convert HTML.\n\n# Arguments\n\n* `html` - The HTML input.\n\n# Example\n\n```rust\nlet result = convert(html, None)?;\n```";
    let sections = parse_rustdoc_sections(doc);
    let out = render_phpdoc_sections(&sections, "SampleMarkupException");
    assert!(!out.contains("```php"), "no PHP @example block for Rust source");
    assert!(!out.contains("```rust"), "raw Rust must not leak into PHPDoc");
    assert!(out.contains("@param"), "other sections must still be emitted");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_short_single_line() {
    let mut out = String::new();
    emit_kdoc_ktfmt_canonical(&mut out, "Simple doc.", "");
    assert_eq!(
        out, "/** Simple doc. */\n",
        "short single-line comment should collapse to canonical format"
    );
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_escapes_nested_block_comment_open() {
    let mut out = String::new();
    emit_kdoc_ktfmt_canonical(&mut out, "Prefix: `\"image/*\"` matches.", "    ");
    assert!(
        !out.contains("/*\""),
        "must not emit raw `/*` inside KDoc — would open nested block comment: {out}",
    );
    assert!(out.contains("/ *"), "must escape `/*` to `/ *`: {out}",);
}

#[test]
fn test_emit_kdoc_escapes_block_comment_close() {
    let mut out = String::new();
    emit_kdoc(&mut out, "Contains literal */ in middle.", "");
    assert!(!out.contains("*/ "), "must escape `*/` in KDoc body: {out}");
    assert!(out.contains("* /"), "must emit `* /` instead of `*/`: {out}");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_short_with_indent() {
    let mut out = String::new();
    emit_kdoc_ktfmt_canonical(&mut out, "Text node (most frequent - 100+ per document)", "    ");
    assert_eq!(out, "    /** Text node (most frequent - 100+ per document) */\n");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_long_comment_uses_multiline() {
    let mut out = String::new();
    let long_text = "This is a very long documentation comment that exceeds the 100-character line width limit and should therefore be emitted in multi-line format";
    emit_kdoc_ktfmt_canonical(&mut out, long_text, "");
    assert!(out.contains("/**\n"), "long comment should start with newline");
    assert!(out.contains(" * "), "long comment should use multi-line format");
    assert!(out.contains(" */\n"), "long comment should end with newline");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_multiline_comment() {
    let mut out = String::new();
    let doc = "First line.\n\nSecond paragraph.";
    emit_kdoc_ktfmt_canonical(&mut out, doc, "");
    assert!(out.contains("/**\n"), "multi-paragraph should use multi-line format");
    assert!(out.contains(" * First line."), "first paragraph preserved");
    assert!(out.contains(" *\n"), "blank line preserved");
    assert!(out.contains(" * Second paragraph."), "second paragraph preserved");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_empty_doc() {
    let mut out = String::new();
    emit_kdoc_ktfmt_canonical(&mut out, "", "");
    assert!(out.is_empty(), "empty doc should produce no output");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_fits_within_100_chars() {
    let mut out = String::new();
    let content = "a".repeat(93);
    emit_kdoc_ktfmt_canonical(&mut out, &content, "");
    let line = out.lines().next().unwrap();
    assert_eq!(
        line.len(),
        100,
        "should fit exactly at 100 chars and use single-line format"
    );
    assert!(out.starts_with("/**"), "should use single-line format");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_exceeds_100_chars() {
    let mut out = String::new();
    let content = "a".repeat(94);
    emit_kdoc_ktfmt_canonical(&mut out, &content, "");
    assert!(
        out.contains("/**\n"),
        "should use multi-line format when exceeding 100 chars"
    );
    assert!(out.contains(" * "), "multi-line format with ` * ` prefix");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_respects_indent() {
    let mut out = String::new();
    let content = "a".repeat(89);
    emit_kdoc_ktfmt_canonical(&mut out, &content, "    ");
    let line = out.lines().next().unwrap();
    assert_eq!(line.len(), 100, "should respect indent in 100-char calculation");
    assert!(line.starts_with("    /** "), "should include indent");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_real_world_enum_variant() {
    let mut out = String::new();
    emit_kdoc_ktfmt_canonical(&mut out, "Text node (most frequent - 100+ per document)", "    ");
    assert!(out.starts_with("    /** "), "should preserve 4-space indent");
    assert!(out.contains(" */\n"), "should end with newline");
    let line_count = out.lines().count();
    assert_eq!(line_count, 1, "should be single-line format");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_real_world_data_class_field() {
    let mut out = String::new();
    let doc = "Heading style to use in Markdown output (ATX `#` or Setext underline).";
    emit_kdoc_ktfmt_canonical(&mut out, doc, "    ");
    let line_count = out.lines().count();
    assert_eq!(line_count, 1, "should be single-line format");
    assert!(out.starts_with("    /** "), "should have correct indent");
}

#[path = "tests/sanitize.rs"]
mod sanitize;

#[test]
fn emit_c_doxygen_wraps_bare_bracket_references() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "Call [download()] to fetch data.", "");
    assert!(
        out.contains("`download()`"),
        "Bare bracket reference should be wrapped in backticks: {out}"
    );
    assert!(
        !out.contains("[download()]"),
        "Original [download()] should be converted to `download()`: {out}"
    );
}

#[test]
fn emit_c_doxygen_wraps_method_identifier_references() {
    let mut out = String::new();
    emit_c_doxygen(
        &mut out,
        "Use [configure] or [init] to set options. Call [Self::ensure_languages].",
        "",
    );
    assert!(out.contains("`configure`"), "Bare [configure] should be wrapped: {out}");
    assert!(out.contains("`init`"), "Bare [init] should be wrapped: {out}");
    assert!(
        out.contains("`Self.ensure_languages`"),
        "[Self::ensure_languages] should convert :: to . and wrap: {out}"
    );
}

#[test]
fn emit_c_doxygen_converts_colons_to_dots_in_references() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "See [Type::method] for details.", "");
    assert!(
        out.contains("`Type.method`"),
        "[Type::method] should become `Type.method`: {out}"
    );
    assert!(
        !out.contains("::"),
        ":: should be converted to . in wrapped references: {out}"
    );
}

#[test]
fn emit_c_doxygen_unwraps_intradoc_backtick_references() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "Use [`identifier`] as documented.", "");
    assert!(
        out.contains("`identifier`"),
        "Backtick-wrapped reference should be preserved: {out}"
    );
    assert!(
        !out.contains("``identifier``"),
        "Already-wrapped references should not be double-wrapped: {out}"
    );
    assert!(
        !out.contains("[`identifier`]"),
        "Outer brackets of intra-doc link form should be removed: {out}"
    );
}

#[test]
fn emit_c_doxygen_unwraps_intradoc_backtick_with_path_separator() {
    let mut out = String::new();
    emit_c_doxygen(&mut out, "Returns [`Error::LanguageNotFound`] if missing.", "");
    assert!(
        out.contains("`Error.LanguageNotFound`"),
        "Type::method form should be unwrapped and :: → . normalised: {out}"
    );
    assert!(
        !out.contains("[`Error"),
        "Outer brackets of intra-doc link form should be removed: {out}"
    );
}
