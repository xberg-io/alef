use alef_core::config::Language;
use std::fmt::Write;

/// Rust doc section headers that should be stripped for all non-Rust output.
const RUST_ONLY_SECTIONS: &[&str] = &["example", "examples", "arguments", "fields"];

/// Wrap bare `http://` and `https://` URLs in angle brackets to satisfy MD034.
/// Skips URLs already inside markdown links `[...](url)` or angle brackets `<url>`.
pub(crate) fn wrap_bare_urls(text: &str) -> String {
    let url_re = regex::Regex::new(r"(https?://[^\s)>\]]+)").unwrap();
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for mat in url_re.find_iter(text) {
        let start = mat.start();
        // Check character before the URL
        let preceding = if start > 0 { text.as_bytes()[start - 1] } else { b' ' };
        // Skip if already inside parens (markdown link) or angle brackets
        if preceding == b'(' || preceding == b'<' {
            continue;
        }
        result.push_str(&text[last_end..start]);
        result.push('<');
        result.push_str(mat.as_str());
        result.push('>');
        last_end = mat.end();
    }
    result.push_str(&text[last_end..]);
    result
}

/// Clean up Rust doc strings for Markdown output.
///
/// - Strips `# Example`, `# Arguments`, `# Fields` sections (Rust-specific)
/// - Strips code blocks containing Rust-specific syntax
/// - Converts `` [`Foo`](Self::bar) `` → `` `Foo` ``
/// - Converts bare `` [`Foo`] `` → `` `Foo` ``
/// - Converts `# Errors` / `# Returns` headings to bold inline text
/// - Converts `Foo::bar()` Rust path syntax to `Foo.bar()` in prose
pub(crate) fn clean_doc(doc: &str, lang: Language) -> String {
    if doc.is_empty() {
        return String::new();
    }

    // Strip Rust-specific sections and their code blocks
    let doc = strip_rust_sections(doc);

    // Convert Rust-style links
    let doc = rust_links_to_plain(&doc);

    // Convert `# Errors` / `# Returns` headings to bold inline text
    // These are Rust doc conventions that render as H1 headings, which is wrong
    let doc = convert_doc_headings_to_bold(&doc);

    // Convert Rust path syntax `Foo::bar()` → `Foo.bar()` (or `Foo::bar()` for PHP) in prose
    let doc = rust_paths_to_dot_notation(&doc, lang);

    // Replace Rust-centric terminology
    let doc = replace_rust_terminology(&doc, lang);

    doc.trim().to_string()
}

/// Convert `# Errors` and `# Returns` section headings to bold inline text.
pub(crate) fn convert_doc_headings_to_bold(doc: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !in_code_block && line.starts_with('#') {
            let heading_text = line.trim_start_matches('#').trim();
            let lower = heading_text.to_lowercase();
            if lower == "errors"
                || lower == "returns"
                || lower == "panics"
                || lower == "safety"
                || lower == "notes"
                || lower == "note"
            {
                let _ = writeln!(out, "**{heading_text}:**");
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Replace Rust-centric terminology with language-neutral equivalents.
pub(crate) fn replace_rust_terminology(doc: &str, lang: Language) -> String {
    let doc = doc
        .replace("this crate", "this library")
        .replace("in this crate", "in this library")
        .replace("for this crate", "for this library")
        .replace(
            "Panic caught during conversion to prevent unwinding across FFI boundaries",
            "Internal error caught during conversion",
        );

    // Replace OutputFormat.None references with language-neutral phrasing
    let doc = doc.replace(
        "None when `output_format` is set to `OutputFormat.None`",
        "null/nil when in extraction-only mode",
    );

    // Replace `None` backtick references with the language-idiomatic null
    let none_replacement = match lang {
        Language::Go | Language::Ruby | Language::Elixir => "`nil`",
        Language::Java | Language::Node | Language::Wasm | Language::Csharp | Language::Php => "`null`",
        Language::Python | Language::Rust => "`None`", // keep as-is for Python and Rust
        Language::R | Language::Ffi => "`NULL`",
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => "`null`",
    };
    let doc = doc.replace("`None`", none_replacement);

    // For Python, normalise boolean literals in prose: `true` → `True`, `false` → `False`
    if lang == Language::Python {
        let doc = doc.replace("`true`", "`True`").replace("`false`", "`False`");
        return doc;
    }

    // For non-Python languages, normalise Rust/Python boolean literals: `True` → `true`, `False` → `false`
    if lang != Language::Rust {
        let doc = doc.replace("`True`", "`true`").replace("`False`", "`false`");
        return doc;
    }

    doc
}

/// Replace Rust `Foo::bar()` path notation with `Foo.bar()` in prose (outside code blocks).
///
/// For PHP, static method calls use `::` so we keep that separator.
pub(crate) fn rust_paths_to_dot_notation(doc: &str, lang: Language) -> String {
    // PHP uses `::` for static method calls; other languages use `.`
    let sep = if lang == Language::Php { "::" } else { "." };
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Replace `Foo::bar` patterns in prose
        // Common Rust-isms: Default::default(), ConversionOptions::default(), ConversionOptions::builder()
        let line = line
            .replace("Default::default()", "the default constructor")
            .replace("::", sep);
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Inline version that also strips newlines for use in table cells.
pub(crate) fn clean_doc_inline(doc: &str, lang: Language) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let cleaned = clean_doc(doc, lang);
    // Collapse to single line for table cells
    cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        // Escape pipe characters in table cells
        .replace('|', "\\|")
}

/// Strip Rust-specific doc sections (`# Example`, `# Arguments`, `# Fields`).
///
/// Also strips fenced code blocks that contain Rust-specific syntax
/// (use statements, unwrap(), assert!, etc.) regardless of which section they appear in.
pub(crate) fn strip_rust_sections(doc: &str) -> String {
    let mut out = String::new();
    let mut skip_section = false;
    let mut in_code_block = false;
    let mut code_block_buf = String::new();

    for line in doc.lines() {
        // Track code block boundaries
        if line.trim_start().starts_with("```") {
            if in_code_block {
                // End of code block — decide whether to emit it
                in_code_block = false;
                if !skip_section && !is_rust_code_block(&code_block_buf) {
                    out.push_str(&code_block_buf);
                    out.push_str(line);
                    out.push('\n');
                }
                code_block_buf.clear();
                continue;
            } else {
                in_code_block = true;
                if !skip_section {
                    code_block_buf.push_str(line);
                    code_block_buf.push('\n');
                }
                continue;
            }
        }

        if in_code_block {
            if !skip_section {
                code_block_buf.push_str(line);
                code_block_buf.push('\n');
            }
            continue;
        }

        // Outside code block: check for section headers
        if line.starts_with('#') {
            let header_text = line.trim_start_matches('#').trim().to_lowercase();
            if RUST_ONLY_SECTIONS.contains(&header_text.as_str()) {
                skip_section = true;
                continue;
            } else {
                // Any other section header ends the skip
                skip_section = false;
            }
        }

        if skip_section {
            // Blank lines and list items are part of the section — keep skipping
            let trimmed = line.trim();
            let is_section_content = trimmed.is_empty()
                || trimmed.starts_with('*')
                || trimmed.starts_with('-')
                || trimmed.starts_with('+')
                || trimmed.starts_with("  ") // indented continuation
                || trimmed.starts_with('\t');
            if is_section_content {
                continue;
            }
            // Non-list, non-blank line ends the skip
            skip_section = false;
        }

        // Skip lines that are clearly Rust-specific (unfenced imports/assertions)
        if is_rust_specific_line(line) {
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

/// Returns true if a code block's content contains Rust-specific patterns.
pub(crate) fn is_rust_code_block(content: &str) -> bool {
    // Opening fence line may declare "rust" or "no_run" etc.
    let first_line = content.lines().next().unwrap_or("");
    let fence_lang = first_line.trim_start_matches('`').trim().to_lowercase();
    if matches!(fence_lang.as_str(), "rust" | "rust,no_run" | "rust,ignore" | "") {
        // Check if content looks like Rust
        for line in content.lines().skip(1) {
            if line.starts_with("use ")
                || line.contains("unwrap()")
                || line.contains("assert!")
                || line.contains("assert_eq!")
                || line.contains("Vec::new()")
                || line.contains("Default::default()")
                || line.contains("::new(")
                || line.contains(".to_string()")
                || line.contains("html_to_markdown")
                || line.contains("r#\"")
            {
                return true;
            }
        }
    }
    false
}

/// Returns true if a plain (non-fenced) line is Rust-specific and should be removed.
pub(crate) fn is_rust_specific_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("# use ") || trimmed.starts_with("use ") && trimmed.ends_with(';')
}

/// Extract parameter descriptions from a `# Arguments` section in a doc string.
///
/// Parses lines like `* name - description` or `* name: description`.
/// Returns a map of parameter name → description.
pub(crate) fn extract_param_docs(doc: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut in_args = false;
    let mut in_code_block = false;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        if line.starts_with('#') {
            let header = line.trim_start_matches('#').trim().to_lowercase();
            in_args = matches!(header.as_str(), "arguments" | "args" | "parameters" | "params");
            continue;
        }

        if in_args {
            // Match "* `param_name` - description" or "* param_name - description"
            // or "* param_name: description"
            let trimmed = line.trim_start_matches(['*', '-', ' ']);
            // Try " - " separator first (3 chars), then ": " (2 chars)
            let parsed = trimmed
                .find(" - ")
                .map(|pos| (pos, 3))
                .or_else(|| trimmed.find(": ").map(|pos| (pos, 2)));
            if let Some((sep_pos, sep_len)) = parsed {
                let raw_name = trimmed[..sep_pos].trim();
                // Strip surrounding backticks if present (e.g. `` `html` `` → `html`)
                let param_name = raw_name.trim_matches('`');
                let desc = trimmed[sep_pos + sep_len..].trim();
                if !param_name.is_empty() && !desc.is_empty() {
                    map.insert(param_name.to_string(), desc.to_string());
                }
            }
        }
    }

    map
}

/// Convert `` [`text`](path) `` and bare `` [`text`] `` patterns to `` `text` ``.
pub(crate) fn rust_links_to_plain(doc: &str) -> String {
    // Pattern 1: [`text`](anything) → `text`
    // Pattern 2: [`text`] → `text`  (bare doc links)
    let mut result = String::with_capacity(doc.len());
    let chars: Vec<char> = doc.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Look for [`
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '`' {
            // Find closing `]`
            let start = i + 1; // position of opening `
            let mut j = start;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() {
                let text: String = chars[start..j].iter().collect();
                // Check if followed by `(` (linked form) or not (bare form)
                if j + 1 < chars.len() && chars[j + 1] == '(' {
                    // Linked form: find closing `)`
                    let mut k = j + 2;
                    while k < chars.len() && chars[k] != ')' {
                        k += 1;
                    }
                    if k < chars.len() {
                        result.push_str(&text);
                        i = k + 1;
                        continue;
                    }
                } else {
                    // Bare form: [`text`] — emit just the text
                    result.push_str(&text);
                    i = j + 1;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::Language;

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
        let doc = "See [`ConversionOptions`] for details.";
        let cleaned = clean_doc(doc, Language::Python);
        assert_eq!(cleaned, "See `ConversionOptions` for details.");
    }

    #[test]
    fn test_extract_param_docs() {
        let doc = "Convert HTML to Markdown.\n\n# Arguments\n\n* html - The HTML string to convert\n* options - Conversion options\n";
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
    fn test_clean_doc_inline_escapes_pipe_for_table_cells() {
        let doc = "Value between 0 | 1.";
        let result = clean_doc_inline(doc, Language::Python);
        assert!(result.contains("\\|"), "pipe must be escaped: {result}");
        assert!(!result.contains(" | "), "unescaped pipe must not remain: {result}");
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
}

// ---------------------------------------------------------------------------
// Ordering helpers
// ---------------------------------------------------------------------------
