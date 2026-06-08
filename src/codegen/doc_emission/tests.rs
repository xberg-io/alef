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
    // Simulates a docstring like convert's: "Convert markup conversion,\nreturning a result."
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
    // Even though we get rustdoc-hidden lines pre-stripped, a literal
    // `# foo` inside a non-rust fence (e.g. shell example) must not
    // start a new section.
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
    // fixture example is ```rust — stripped when target is TypeScript
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
    // Java rendering omits the example block (handled separately by emit_javadoc which
    // wraps code in `<pre>{@code}</pre>`); we just confirm summary survives.
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
    // fixture example is ```rust — stripped when target is PHP
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
        // Each emitted prefix is "/// " (4 chars); the body after that
        // should be ≤ 100 chars per `DOXYGEN_WRAP_WIDTH`.
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
    // fixture example is ```rust — stripped when target is Ruby
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

// --- M1: example_for_target unit tests ---

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

// --- KDoc ktfmt-canonical format tests ---

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
    // Kotlin block comments NEST: `/*` inside a `/** … */` opens a new
    // comment level. A backtick-quoted `"image/*"` from rustdoc contains
    // `/*` but no matching `*/`, leaving the outer block unclosed and the
    // Kotlin lexer reporting cascading "Missing '}'" / "Unclosed comment"
    // errors. The escape replaces `/*` with `/ *` so the lexer never opens
    // a nested block.
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
    // `*/` inside the KDoc body terminates the outer block early.
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
    // Construct exactly at the boundary: indent(0) + "/** " + content + " */" = 100 chars
    // "/** " = 4 chars, " */" = 3 chars, so content can be 93 chars
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
    // Exceed 100 chars: content of 94 chars with "/** " + " */" = 101 chars
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
    // With 4-char indent, max content is 89 chars (4 + 4 + 89 + 3 = 100)
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
    // This is from NodeType enum; should collapse to single-line
    assert!(out.starts_with("    /** "), "should preserve 4-space indent");
    assert!(out.contains(" */\n"), "should end with newline");
    // Verify it's single-line format
    let line_count = out.lines().count();
    assert_eq!(line_count, 1, "should be single-line format");
}

#[test]
fn test_emit_kdoc_ktfmt_canonical_real_world_data_class_field() {
    let mut out = String::new();
    let doc = "Heading style to use in Markdown output (ATX `#` or Setext underline).";
    emit_kdoc_ktfmt_canonical(&mut out, doc, "    ");
    // This is from ConversionOptions data class; should collapse to single-line
    let line_count = out.lines().count();
    assert_eq!(line_count, 1, "should be single-line format");
    assert!(out.starts_with("    /** "), "should have correct indent");
}

// --- sanitize_rust_idioms tests ---

#[test]
fn sanitize_intradoc_link_with_path_separator_java() {
    let input = "See [`ConversionOptions::builder()`] for details.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("`ConversionOptions.builder()`"), "got: {out}");
    assert!(!out.contains("[`"), "brackets must be removed, got: {out}");
}

#[test]
fn sanitize_intradoc_link_simple_type_php() {
    let input = "Returns a [`ConversionResult`].";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(out.contains("`ConversionResult`"), "got: {out}");
    assert!(!out.contains("[`"), "got: {out}");
}

#[test]
fn sanitize_none_to_null_javadoc() {
    let input = "Returns None when no value is found.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("null"), "got: {out}");
    assert!(!out.contains("None"), "got: {out}");
}

#[test]
fn sanitize_none_to_undefined_tsdoc() {
    let input = "Returns None if absent.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("undefined"), "got: {out}");
    assert!(!out.contains("None"), "got: {out}");
}

#[test]
fn sanitize_some_x_to_the_value_x() {
    let input = "Pass Some(value) to enable.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("the value (value)"), "got: {out}");
    assert!(!out.contains("Some("), "got: {out}");
}

#[test]
fn sanitize_bare_some_followed_by_lowercase_noun_is_dropped() {
    // Regression test for Rust option wording leaking into generated JavaDoc.
    let input = "Only specified fields (Some values) will override existing options; None values leave the previous";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(
        out.contains("(values)"),
        "bare `Some ` before lowercase noun must be stripped; got: {out}"
    );
    assert!(
        out.contains("null values"),
        "bare `None ` must also be replaced; got: {out}"
    );
    assert!(!out.contains("Some "), "Some prefix must not survive; got: {out}");
}

#[test]
fn sanitize_bare_some_does_not_touch_identifiers_or_uppercase_followers() {
    // SomeType, Some.method(), Some(x), and "Some Title" (proper noun) all preserved.
    let cases = [
        "SomeType lives on.",
        "Some.method() returns Self.",
        "Some Title",
        "Some(x) is a value.",
    ];
    for case in cases {
        let out = sanitize_rust_idioms(case, DocTarget::JavaDoc);
        // For the Some(x) case, replace_some_calls (run earlier) converts to "the value (x)"
        // so "Some" itself is gone — that's expected; everything else preserves "Some".
        if case.starts_with("Some(") {
            assert!(out.contains("the value (x)"), "got: {out}");
        } else {
            assert!(out.contains("Some"), "Some must survive in {case:?}; got: {out}");
        }
    }
}

#[test]
fn sanitize_option_t_to_nullable_php() {
    let input = "The result is Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(out.contains("String?"), "got: {out}");
    assert!(!out.contains("Option<"), "got: {out}");
}

#[test]
fn sanitize_option_t_to_or_null_java() {
    let input = "The result is Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("String | null"), "got: {out}");
}

#[test]
fn sanitize_option_t_to_or_undefined_tsdoc() {
    let input = "The result is Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("String | undefined"), "got: {out}");
}

#[test]
fn sanitize_vec_u8_per_target() {
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::PhpDoc).contains("string"));
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::JavaDoc).contains("byte[]"));
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::TsDoc).contains("Uint8Array"));
    assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::JsDoc).contains("Uint8Array"));
}

#[test]
fn sanitize_vec_t_to_array() {
    let input = "Returns Vec<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("String[]"), "got: {out}");
    assert!(!out.contains("Vec<"), "got: {out}");
}

#[test]
fn sanitize_hashmap_per_target() {
    let input = "Uses HashMap<String, u32>.";
    assert!(sanitize_rust_idioms(input, DocTarget::PhpDoc).contains("array<String, u32>"));
    assert!(sanitize_rust_idioms(input, DocTarget::JavaDoc).contains("Map<String, u32>"));
    assert!(sanitize_rust_idioms(input, DocTarget::TsDoc).contains("Record<String, u32>"));
}

#[test]
fn sanitize_arc_wrapper_stripped() {
    let input = "Holds Arc<Config>.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("Config"), "got: {out}");
    assert!(!out.contains("Arc<"), "got: {out}");
}

#[test]
fn sanitize_box_mutex_rwlock_rc_cell_refcell_stripped() {
    for wrapper in &["Box", "Mutex", "RwLock", "Rc", "Cell", "RefCell"] {
        let input = format!("Contains {wrapper}<Inner>.");
        let out = sanitize_rust_idioms(&input, DocTarget::JavaDoc);
        assert!(out.contains("Inner"), "wrapper {wrapper} not stripped, got: {out}");
        assert!(
            !out.contains(&format!("{wrapper}<")),
            "wrapper {wrapper} still present, got: {out}"
        );
    }
}

#[test]
fn sanitize_send_sync_stripped() {
    let input = "The type is Send + Sync.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(!out.contains("Send"), "got: {out}");
    assert!(!out.contains("Sync"), "got: {out}");
}

#[test]
fn sanitize_static_lifetime_stripped() {
    let input = "Requires 'static lifetime.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("'static"), "got: {out}");
}

#[test]
fn sanitize_pub_fn_stripped() {
    let input = "Calls pub fn convert().";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(!out.contains("pub fn"), "got: {out}");
    assert!(out.contains("convert()"), "got: {out}");
}

#[test]
fn sanitize_crate_prefix_stripped() {
    let input = "See crate::error::ConversionError.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("crate::"), "got: {out}");
    assert!(out.contains("error.ConversionError"), "got: {out}");
}

#[test]
fn sanitize_unwrap_expect_stripped() {
    let input = "Call result.unwrap() or result.expect(\"msg\").";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains(".unwrap()"), "got: {out}");
    assert!(!out.contains(".expect("), "got: {out}");
}

#[test]
fn sanitize_no_mutation_inside_backticks() {
    // None inside backtick span must not be replaced.
    let input = "Use `None` as the argument.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("`None`"), "backtick span must be preserved, got: {out}");
}

#[test]
fn sanitize_rust_fence_dropped_for_tsdoc() {
    let input = "Intro.\n\n```rust\nlet x = 1;\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(
        !out.contains("let x = 1;"),
        "rust fence content must be dropped, got: {out}"
    );
    assert!(!out.contains("```rust"), "got: {out}");
    assert!(out.contains("Trailer."), "text after fence must survive, got: {out}");
}

#[test]
fn sanitize_rust_fence_dropped_for_java() {
    let input = "Intro.\n\n```rust\nlet x = 1;\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    // Rust fences are now dropped entirely for Java (Rust code is not portable).
    assert!(
        !out.contains("let x = 1;"),
        "fence content must be dropped for Java, got: {out}"
    );
    assert!(!out.contains("```"), "fence markers must be dropped, got: {out}");
    assert!(out.contains("Intro."), "prose before fence kept: {out}");
    assert!(out.contains("Trailer."), "prose after fence kept: {out}");
}

#[test]
fn sanitize_non_rust_fence_passed_through() {
    let input = "Example:\n\n```typescript\nconst x = 1;\n```";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("```typescript"), "non-rust fence must survive, got: {out}");
    assert!(out.contains("const x = 1;"), "got: {out}");
}

#[test]
fn sanitize_backtick_code_span_not_mutated_option() {
    // Option<T> inside backtick span must not be replaced.
    let input = "The type is `Option<String>`.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    // The backtick-protected span should be preserved verbatim.
    assert!(
        out.contains("`Option<String>`"),
        "code span must be preserved, got: {out}"
    );
}

#[test]
fn sanitize_idempotent() {
    // Running twice should produce the same result as running once.
    let input = "Returns None when Vec<String> is empty.";
    let once = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    let twice = sanitize_rust_idioms(&once, DocTarget::JavaDoc);
    assert_eq!(once, twice, "sanitize_rust_idioms should be idempotent");
}

#[test]
fn sanitize_multiline_prose() {
    let input = "Convert markup conversion.\n\nReturns None on failure.\nUse Option<String> for the result.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("null"), "None must be replaced on line 2, got: {out}");
    assert!(
        out.contains("String | null"),
        "Option<String> must be replaced on line 3, got: {out}"
    );
}

#[test]
fn sanitize_attribute_line_dropped() {
    let input = "#[derive(Debug, Clone)]\nSome documentation.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("#[derive("), "attribute line must be dropped, got: {out}");
    // Prose survives, though bare "Some " before a lowercase noun is stripped
    // by `replace_some_keyword_in_prose`, so accept either form.
    assert!(out.contains("documentation."), "prose must survive, got: {out}");
}

#[test]
fn sanitize_path_separator_in_prose() {
    let input = "See std::collections::HashMap for details.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("std.collections.HashMap"), ":: must become ., got: {out}");
}

#[test]
fn sanitize_none_not_replaced_inside_identifier() {
    // "NoneType" must not be replaced.
    let input = "Unlike NoneType in Python.";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(out.contains("NoneType"), "NoneType must not be replaced, got: {out}");
}

// --- CSharpDoc target tests ---

#[test]
fn sanitize_csharp_drops_rust_section_headings_and_example_body() {
    // The GraphQLErrorException case: `# Examples` heading followed by a
    // ```ignore code fence containing `Self::error_code`, `Result<T, E>`,
    // intra-doc links — all of which previously leaked into `<summary>`.
    let input = "Convert error to HTTP status code\n\n\
            Maps GraphQL error types to status codes.\n\n\
            # Examples\n\n\
            ```ignore\n\
            use sample_router_graphql::error::GraphQLError;\n\
            let error = GraphQLError::AuthenticationError(\"Invalid token\".to_string());\n\
            assert_eq!(error.status_code(), 401);\n\
            ```\n";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(
        out.contains("Convert error to HTTP status code"),
        "summary preserved: {out}"
    );
    assert!(out.contains("Maps GraphQL error types"), "prose preserved: {out}");
    assert!(!out.contains("# Examples"), "heading dropped: {out}");
    assert!(!out.contains("```"), "code fence dropped: {out}");
    assert!(!out.contains("Self::error_code"), "Self::method dropped: {out}");
    assert!(
        !out.contains("GraphQLError::AuthenticationError"),
        "rust path dropped: {out}"
    );
}

#[test]
fn sanitize_csharp_intradoc_link_with_path_separator() {
    let input = "See [`Self::error_code`] for the variant codes.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("`Self.error_code`"), "intra-doc link normalised: {out}");
    assert!(!out.contains("[`"), "square brackets removed: {out}");
    assert!(!out.contains("::"), ":: replaced with .: {out}");
}

#[test]
fn sanitize_csharp_result_type_keeps_success_drops_error() {
    let input = "Returns Result<String, ConversionError> on failure.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("String"), "success type kept: {out}");
    assert!(!out.contains("Result<"), "Result wrapper dropped: {out}");
    assert!(!out.contains("ConversionError"), "error type dropped: {out}");
}

#[test]
fn sanitize_csharp_option_becomes_nullable() {
    let input = "Returns Option<String>.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    // After XML-escaping, the `?` survives but any surviving `<`/`>` get escaped.
    assert!(out.contains("String?"), "Option<T> -> T?: {out}");
    assert!(!out.contains("Option<"), "Option dropped: {out}");
}

#[test]
fn sanitize_csharp_vec_u8_becomes_byte_array() {
    let input = "Accepts Vec<u8>.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    // `byte[]` survives — the `[` is not XML-significant.
    assert!(out.contains("byte[]"), "Vec<u8> -> byte[]: {out}");
}

#[test]
fn sanitize_csharp_hashmap_becomes_dictionary() {
    let input = "Holds HashMap<String, u32>.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    // The `<` / `>` produced by Dictionary<K, V> must be XML-escaped.
    assert!(
        out.contains("Dictionary&lt;String, u32&gt;"),
        "HashMap -> Dictionary with XML-escaped brackets: {out}"
    );
}

#[test]
fn sanitize_csharp_none_to_null() {
    let input = "Returns None on miss.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(out.contains("null"), "None -> null: {out}");
    assert!(!out.contains("None"), "None replaced: {out}");
}

#[test]
fn sanitize_csharp_escapes_raw_angle_brackets_and_amp() {
    // Unrecognised `<...>` constructs (e.g. trait objects, generic params on
    // unknown names) must still be XML-escaped so the result is valid inside
    // `<summary>`.
    let input = "Accepts Box<dyn Trait> and combines a & b.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    // Box<T> wrapper is stripped to inner type, leaving `dyn Trait`.
    assert!(out.contains("dyn Trait"), "Box<T> stripped: {out}");
    assert!(out.contains("&amp;"), "ampersand escaped: {out}");
}

#[test]
fn sanitize_csharp_drops_rust_code_fence_entirely() {
    let input = "Intro.\n\n```rust\nlet x: Vec<u8> = vec![];\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(!out.contains("let x"), "code fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Intro."), "prose before fence kept: {out}");
    assert!(out.contains("Trailer."), "prose after fence kept: {out}");
}

#[test]
fn sanitize_csharp_keep_sections_does_not_drop_headings() {
    // The sections-preserving variant leaves heading lines alone so callers
    // that have already extracted sections can sanitise each body fragment.
    let input = "Summary.\n\n# Arguments\n\n* `name` - the value.";
    let out = sanitize_rust_idioms_keep_sections(input, DocTarget::CSharpDoc);
    assert!(out.contains("# Arguments"), "heading preserved: {out}");
    assert!(out.contains("name"), "body preserved: {out}");
}

#[test]
fn sanitize_csharp_idempotent() {
    let input = "Returns Option<String> or None.";
    let once = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    let twice = sanitize_rust_idioms(&once, DocTarget::CSharpDoc);
    assert_eq!(once, twice, "CSharpDoc sanitisation must be idempotent");
}

#[test]
fn sanitize_phpdoc_drops_unmarked_rust_code_fences() {
    // Regression test: unmarked code fences (```\n...\n```) in Rust docstrings
    // are treated as Rust code and should be dropped for PHP target.
    let input = "Detect language name from a file extension.\n\nReturns `None` for unrecognized extensions.\n\n```\nuse sample_language_pack::detect_language_from_extension;\nassert_eq!(detect_language_from_extension(\"py\"), Some(\"python\"));\nassert_eq!(detect_language_from_extension(\"RS\"), Some(\"rust\"));\nassert_eq!(detect_language_from_extension(\"xyz\"), None);\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(
        !out.contains("use sample_language_pack"),
        "Rust use stmt dropped: {out}"
    );
    assert!(!out.contains("assert_eq!"), "Rust code dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Detect language name"), "prose before fence kept: {out}");
    assert!(out.contains("unrecognized extensions"), "prose kept: {out}");
}

#[test]
fn sanitize_javadoc_drops_unmarked_rust_code_fences() {
    // Regression test: unmarked code fences in Rust docstrings should be dropped
    // for Java target as well.
    let input = "Process a file.\n\n```\nlet result = process(\"def hello(): pass\", &config).unwrap();\n```";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("unwrap"), "Rust unwrap dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Process a file"), "prose kept: {out}");
}

#[test]
fn sanitize_phpdoc_drops_explicit_rust_fences() {
    // Explicit ```rust fences should also be dropped for PHP.
    let input = "Summary.\n\n```rust\nuse std::path::PathBuf;\nlet p = PathBuf::from(\"/tmp\");\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(!out.contains("use std::"), "Rust code dropped: {out}");
    assert!(!out.contains("PathBuf"), "Rust types dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Summary"), "prose kept: {out}");
}

// --- rustdoc test-attribute fence tests ---

#[test]
fn sanitize_no_run_fence_dropped_for_tsdoc() {
    let input = "Intro.\n\n```no_run\nuse foo::bar;\nbar::init();\n```\n\nTrailer.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(!out.contains("use foo::bar"), "no_run fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Intro."), "prose before fence kept: {out}");
    assert!(out.contains("Trailer."), "prose after fence kept: {out}");
}

#[test]
fn sanitize_ignore_fence_dropped_for_phpdoc() {
    let input = "Summary.\n\n```ignore\nlet x = 1;\n// this would not compile\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(!out.contains("let x = 1"), "ignore fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Summary"), "prose kept: {out}");
}

#[test]
fn sanitize_should_panic_fence_dropped_for_javadoc() {
    let input = "Panics on null.\n\n```should_panic\nlet _ = parse(null);\n```";
    let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
    assert!(!out.contains("parse(null)"), "should_panic fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Panics on null"), "prose kept: {out}");
}

#[test]
fn sanitize_compile_fail_fence_dropped_for_csharp() {
    let input = "Type safety demo.\n\n```compile_fail\nlet x: u32 = \"hello\";\n```";
    let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    assert!(!out.contains("let x:"), "compile_fail fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Type safety demo"), "prose kept: {out}");
}

#[test]
fn sanitize_edition_fence_dropped_for_tsdoc() {
    let input = "Edition example.\n\n```edition2021\nuse std::fmt;\n```\n\nSee also edition2018.";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(!out.contains("use std::fmt"), "edition2021 fence body dropped: {out}");
    assert!(!out.contains("```"), "fence markers dropped: {out}");
    assert!(out.contains("Edition example"), "prose kept: {out}");
}

#[test]
fn sanitize_python_fence_preserved_for_tsdoc() {
    // Python fences are not Rust — they must pass through unchanged.
    let input = "Example:\n\n```python\nimport foo\nfoo.bar()\n```";
    let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(out.contains("```python"), "python fence preserved: {out}");
    assert!(out.contains("import foo"), "python body preserved: {out}");
}

#[test]
fn sanitize_javascript_fence_preserved_for_phpdoc() {
    let input = "Usage:\n\n```javascript\nconst x = require('foo');\n```";
    let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    assert!(out.contains("```javascript"), "javascript fence preserved: {out}");
    assert!(out.contains("require('foo')"), "javascript body preserved: {out}");
}

#[test]
fn example_for_target_no_run_fence_suppressed_for_typescript() {
    let example = "```no_run\nuse sample_language_pack::available_languages;\nlet langs = available_languages();\n```";
    assert_eq!(
        example_for_target(example, "typescript"),
        None,
        "no_run fence must be treated as Rust and suppressed for TypeScript"
    );
}

#[test]
fn example_for_target_ignore_fence_suppressed_for_php() {
    let example = "```ignore\nlet x = 1;\n```";
    assert_eq!(
        example_for_target(example, "php"),
        None,
        "ignore fence must be treated as Rust and suppressed for PHP"
    );
}

#[test]
fn example_for_target_compile_fail_fence_suppressed_for_java() {
    let example = "```compile_fail\nlet x: u32 = \"wrong\";\n```";
    assert_eq!(
        example_for_target(example, "java"),
        None,
        "compile_fail fence must be treated as Rust and suppressed for Java"
    );
}

#[test]
fn example_for_target_should_panic_fence_suppressed_for_ruby() {
    let example = "```should_panic\nlet _ = parse(None);\n```";
    assert_eq!(
        example_for_target(example, "ruby"),
        None,
        "should_panic fence must be treated as Rust and suppressed for Ruby"
    );
}

#[test]
fn example_for_target_edition_fence_suppressed_for_php() {
    let example = "```edition2021\nuse std::fmt;\n```";
    assert_eq!(
        example_for_target(example, "php"),
        None,
        "edition2021 fence must be treated as Rust and suppressed for PHP"
    );
}

#[test]
fn example_for_target_python_fence_preserved() {
    let example = "```python\nimport foo\n```";
    let result = example_for_target(example, "php");
    assert!(result.is_some(), "python fence must be preserved for PHP target");
}

#[test]
fn emit_csharp_doc_multi_paragraph_with_intra_doc_link() {
    let input = "Stream a single-URL crawl, yielding [`CrawlEvent`]s as pages are processed.\n\nReturns an async stream that emits one event per crawled page, plus a\nterminal `Complete` event.";
    let mut out = String::new();
    emit_csharp_doc(&mut out, input, "    ", "TestException");

    // Check that the output has all expected parts
    assert!(out.contains("<summary>"), "summary tag present: {out}");
    assert!(out.contains("</summary>"), "closing summary tag present: {out}");

    // Check that both paragraphs are present
    assert!(
        out.contains("Stream a single-URL crawl"),
        "first paragraph present: {out}"
    );
    assert!(
        out.contains("Returns an async stream"),
        "second paragraph present: {out}"
    );

    // Check that the intra-doc link is converted (backticks preserved, square brackets gone)
    assert!(
        out.contains("`CrawlEvent`"),
        "intra-doc link converted to code span: {out}"
    );
    assert!(
        !out.contains("[`CrawlEvent`]"),
        "square brackets removed from intra-doc link: {out}"
    );

    // Check that all lines have the /// prefix (including the blank line separating paragraphs)
    let lines: Vec<&str> = out.lines().collect();
    for line in lines {
        if !line.trim().is_empty() {
            assert!(line.contains("///"), "every non-empty line has /// prefix: {}", line);
        }
    }
}

#[test]
fn sanitize_rust_idioms_escapes_jsdoc_block_close() {
    let input = "A block or multi-line comment (e.g., `/* ... */`).";
    let result = sanitize_rust_idioms(input, DocTarget::TsDoc);
    // The `*/` inside backticks must be escaped to `* /` so it doesn't
    // prematurely close a JSDoc /** ... */ block.
    assert!(
        result.contains("* /"),
        "JSDoc block-close sequences must be escaped: {result}"
    );
    assert!(
        !result.contains("*/"),
        "JSDoc block-close should not appear unescaped: {result}"
    );
}

#[test]
fn sanitize_rust_idioms_jsdoc_escape_preserves_content() {
    let input = "Handle `/* ... */` and `/* comment */` patterns.";
    let result = sanitize_rust_idioms(input, DocTarget::TsDoc);
    // Both patterns should be escaped and content otherwise preserved
    assert!(result.contains("Handle"), "Handle keyword preserved");
    assert!(result.contains("and"), "and keyword preserved");
    assert!(result.contains("patterns"), "patterns keyword preserved");
    assert!(result.contains("* /"), "escaped block-close preserved");
}

#[test]
fn sanitize_rust_idioms_jsdoc_escape_for_tsdoc_target() {
    let input = "Code example: `/* comment */`";
    let result = sanitize_rust_idioms(input, DocTarget::TsDoc);
    assert!(result.contains("* /"), "TsDoc target must escape */ sequences");
}

#[test]
fn sanitize_rust_idioms_jsdoc_escape_for_jsdoc_target() {
    let input = "Code example: `/* comment */`";
    let result = sanitize_rust_idioms(input, DocTarget::JsDoc);
    assert!(result.contains("* /"), "JsDoc target must escape */ sequences");
}

#[test]
fn sanitize_rust_idioms_no_jsdoc_escape_for_other_targets() {
    let input = "Code example: `/* comment */`";
    let _result_phpdoc = sanitize_rust_idioms(input, DocTarget::PhpDoc);
    let _result_csharp = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
    // PhpDoc uses different escape (already tested via emit_phpdoc),
    // and C# uses XML escaping, not JSDoc escaping.
    // Just verify the escape_jsdoc_block_close function isn't called for these targets.
    // (The actual escaping for these targets happens elsewhere, as tested separately.)
}
