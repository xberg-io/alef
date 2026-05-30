use crate::core::config::Language;

/// Rust doc section headers that should be stripped for all non-Rust output.
const RUST_ONLY_SECTIONS: &[&str] = &["example", "examples", "arguments", "fields"];

/// Check if a markdown document has monotonic heading increments (no skips of >1 level).
///
/// Returns `Ok(())` if all headings increment by at most 1 level, or an error message
/// describing the first violation found.
#[cfg(test)]
pub(crate) fn check_monotonic_headings(doc: &str) -> Result<(), String> {
    let mut previous_level: Option<usize> = None;
    let mut in_code_block = false;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || !line.starts_with('#') {
            continue;
        }

        let heading_level = line.chars().take_while(|&c| c == '#').count();
        if heading_level == 0 || heading_level > 6 {
            continue;
        }

        if let Some(prev) = previous_level {
            let increment = heading_level.saturating_sub(prev);
            if increment > 1 {
                let heading_text = line.trim_start_matches('#').trim();
                return Err(format!(
                    "Heading increment violation: H{} → H{} (skip of {})\nHeading: {}",
                    prev, heading_level, increment, heading_text
                ));
            }
        }

        previous_level = Some(heading_level);
    }

    Ok(())
}

/// Demote all markdown headings by a given number of levels.
///
/// For example, with `levels=2`, all `#` become `###`, `##` become `####`, etc.
/// Headings inside code blocks are not modified.
pub(crate) fn demote_headings(doc: &str, levels: usize) -> String {
    if levels == 0 || doc.is_empty() {
        return doc.to_string();
    }
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_code_block || !line.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Count leading '#' characters
        let heading_level = line.chars().take_while(|&c| c == '#').count();
        if heading_level > 0 && heading_level <= 6 {
            // Add demotion levels
            let new_level = std::cmp::min(heading_level + levels, 6);
            let demoted_hashes = "#".repeat(new_level);
            let rest = &line[heading_level..];
            out.push_str(&demoted_hashes);
            out.push_str(rest);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

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
/// - Inserts a blank line before lists that follow prose (satisfies MD032)
pub fn clean_doc(doc: &str, lang: Language) -> String {
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

    // Normalize list markers from `*` to `-` for rumdl compliance
    let doc = normalize_list_markers(&doc);

    // Insert blank lines before lists that follow prose. Rust doc strings often
    // omit the blank line — CommonMark tolerates it, but rumdl's MD032 flags it.
    let doc = ensure_blank_before_lists(&doc);

    doc.trim().to_string()
}

/// Returns `true` if `line` starts a Markdown list item (`-`, `*`, `+`, or `N.`/`N)`).
///
/// Recognises up to three leading spaces of indentation, matching CommonMark.
fn is_list_item_start(line: &str) -> bool {
    let trimmed_left = line.trim_start_matches(' ');
    let leading_spaces = line.len() - trimmed_left.len();
    if leading_spaces > 3 {
        return false;
    }
    let bytes = trimmed_left.as_bytes();
    match bytes.first() {
        Some(b'-') | Some(b'*') | Some(b'+') => {
            // Must be followed by whitespace to be a list marker (not bold/italic).
            matches!(bytes.get(1), Some(b' ') | Some(b'\t'))
        }
        Some(c) if c.is_ascii_digit() => {
            // Ordered list: digits then `.` or `)` then whitespace.
            let mut idx = 1;
            while bytes.get(idx).is_some_and(|c| c.is_ascii_digit()) {
                idx += 1;
            }
            matches!(bytes.get(idx), Some(b'.') | Some(b')')) && matches!(bytes.get(idx + 1), Some(b' ') | Some(b'\t'))
        }
        _ => false,
    }
}

/// Insert a blank line before any list item that directly follows a non-blank
/// line that is itself not a list item. Satisfies rumdl's MD032.
pub(crate) fn ensure_blank_before_lists(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut in_code_block = false;
    let mut prev_non_empty: Option<String> = None;
    let mut prev_was_blank = true;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            prev_non_empty = Some(line.to_string());
            prev_was_blank = false;
            continue;
        }

        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
            prev_was_blank = true;
            continue;
        }

        let starts_list = is_list_item_start(line);
        let prev_was_list = prev_non_empty.as_deref().is_some_and(is_list_item_start);
        if starts_list && !prev_was_blank && !prev_was_list {
            out.push('\n');
        }

        out.push_str(line);
        out.push('\n');
        prev_non_empty = Some(line.to_string());
        prev_was_blank = false;
    }

    out
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
                out.push_str(&crate::docs::template_env::render(
                    "bold_heading.jinja",
                    minijinja::context! { text => heading_text },
                ));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Normalize list markers from `*` to `-` for consistency with rumdl style.
///
/// Replaces list marker `* ` with `- ` at line start (after indentation),
/// but avoids changing emphasis/bold markers like `*text*` or `**bold**` and
/// skips content inside fenced code blocks.
pub(crate) fn normalize_list_markers(doc: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        // Track code block boundaries
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Skip normalization inside code blocks
        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let trimmed_left = line.trim_start_matches(' ');
        let leading_spaces = line.len() - trimmed_left.len();

        // Only normalize `*` at the start of a list item (after indentation, followed by space)
        if trimmed_left.starts_with("* ") && leading_spaces <= 3 {
            out.push_str(&" ".repeat(leading_spaces));
            out.push_str("- ");
            out.push_str(&trimmed_left[2..]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Collapse multi-line and multi-space strings into a single line with normalized spacing.
///
/// Useful for field defaults that may contain embedded newlines or multiple consecutive spaces.
pub(crate) fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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
        Language::R | Language::Ffi | Language::C | Language::Jni => "`NULL`",
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => "`null`",
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
///
/// Note: this function does NOT escape pipe characters. Every call site in
/// `lib.rs` passes the result through `escape_table_cell`, which handles pipe
/// escaping exactly once. Escaping here as well would double-escape `||` into
/// `\\|\\|`, causing CommonMark parsers to see an extra cell separator and
/// trigger MD056 violations.
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
        let doc = "See [`ConversionOptions`] for details.";
        let cleaned = clean_doc(doc, Language::Python);
        assert_eq!(cleaned, "See `ConversionOptions` for details.");
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
        // clean_doc_inline must NOT escape pipes; callers use escape_table_cell for that.
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
        // The full pipeline (what lib.rs does) escapes exactly once:
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
        let doc =
            "The length of this vec is ≤ rows * cols. An empty table (rows == 0 || cols == 0) produces an empty vec.";
        let raw = clean_doc_inline(doc, Language::Python);
        // clean_doc_inline must NOT have pre-escaped the pipes
        assert!(
            !raw.contains("\\|"),
            "clean_doc_inline must not escape pipes (double-escaping bug): {raw}"
        );
        // The caller (lib.rs) escapes once via escape_table_cell
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
        // After demotion: #### Page, ##### Section, ###### Item
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
        // After demotion, ## becomes ####
        // Structure should be: (parent at ####) → (doc content at ####) → (internal heading at ####)
        assert!(
            demoted.contains("#### Stream Limits"),
            "internal heading should be demoted to #### (was ##)"
        );
        // Verify monotonic increments
        assert!(
            check_monotonic_headings(&demoted).is_ok(),
            "demoted doc comment should have monotonic heading increments"
        );
    }

    // --- MD032: lists preceded by blank line ---

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
        // `*bold*` without trailing space must NOT be treated as a list marker.
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
        // Regression: visitor trait docstrings emit lists without preceding blank lines.
        let doc = "# Execution Order\n\nFor a typical element like `<div>`:\n1. Step one\n2. Step two\n";
        let cleaned = clean_doc(doc, Language::Python);
        // After the prose line ending with `:`, a blank line must precede `1.`
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
        // split_whitespace handles multiple spaces
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

    // MD038 (spaces inside code span), MD055/MD056 (table cell count mismatch)
    #[test]
    fn test_field_default_with_multiline_collapsed() {
        // Simulates a default value with embedded newlines
        let raw = "value_line_1\nvalue_line_2";
        let collapsed = collapse_whitespace(raw);
        let formatted = format!("`{collapsed}`");
        assert_eq!(formatted, "`value_line_1 value_line_2`");
        assert!(!formatted.contains('\n'), "backtick code span must be single-line");
    }

    // MD004 (list marker style)
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
}

// ---------------------------------------------------------------------------
// Ordering helpers
// ---------------------------------------------------------------------------
