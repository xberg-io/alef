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
        let heading_level = line.chars().take_while(|&c| c == '#').count();
        if heading_level > 0 && heading_level <= 6 {
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

/// Demote markdown headings so the first heading starts at `target_level`.
///
/// This keeps rustdoc headings nested under a generated parent heading without
/// introducing skips when the source doc starts at `##` or deeper.
pub(crate) fn demote_headings_to_start_at(doc: &str, target_level: usize) -> String {
    let Some(first_level) = first_heading_level(doc) else {
        return doc.to_string();
    };
    let target_level = target_level.clamp(1, 6);
    if first_level >= target_level {
        return doc.to_string();
    }
    demote_headings(doc, target_level - first_level)
}

fn first_heading_level(doc: &str) -> Option<usize> {
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
        if heading_level > 0 && heading_level <= 6 {
            return Some(heading_level);
        }
    }

    None
}

/// Wrap bare `http://` and `https://` URLs in angle brackets to satisfy MD034.
/// Skips URLs already inside markdown links `[...](url)` or angle brackets `<url>`.
pub(crate) fn wrap_bare_urls(text: &str) -> String {
    let url_re = regex::Regex::new(r"(https?://[^\s)>\]]+)").unwrap();
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for mat in url_re.find_iter(text) {
        let start = mat.start();
        let preceding = if start > 0 { text.as_bytes()[start - 1] } else { b' ' };
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

    let doc = strip_rust_sections(doc);

    let doc = rust_links_to_plain(&doc);

    let doc = collapse_adjacent_code_spans(&doc);

    let doc = convert_doc_headings_to_bold(&doc);

    let doc = rust_paths_to_dot_notation(&doc, lang);

    let doc = replace_rust_type_terms(&doc, lang);

    let doc = normalize_feature_label_versions(&doc);

    let doc = replace_rust_terminology(&doc, lang);

    let doc = normalize_list_markers(&doc);

    let doc = ensure_blank_before_lists(&doc);

    doc.trim().to_string()
}

/// Strip patch/prerelease suffixes from prose feature labels.
///
/// This intentionally targets only feature provenance phrases. API page release
/// badges are generated elsewhere and must keep the full package version.
pub(crate) fn normalize_feature_label_versions(doc: &str) -> String {
    static FEATURE_LABEL_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = FEATURE_LABEL_RE.get_or_init(|| {
        regex::Regex::new(
            r"\b(Since|Changed in|Available by) v([0-9]+)\.([0-9]+)\.[0-9]+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?",
        )
        .expect("feature label version regex must compile")
    });
    re.replace_all(doc, "$1 v$2.$3").into_owned()
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
            matches!(bytes.get(1), Some(b' ') | Some(b'\t'))
        }
        Some(c) if c.is_ascii_digit() => {
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

        let trimmed_left = line.trim_start_matches(' ');
        let leading_spaces = line.len() - trimmed_left.len();

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

    let doc = doc.replace(
        "None when `output_format` is set to `OutputFormat.None`",
        "null/nil when in extraction-only mode",
    );

    let none_replacement = match lang {
        Language::Go | Language::Ruby | Language::Elixir => "`nil`",
        Language::Java | Language::Node | Language::Wasm | Language::Csharp | Language::Php => "`null`",
        Language::Python | Language::Rust => "`None`",
        Language::R | Language::Ffi | Language::C | Language::Jni => "`NULL`",
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => "`null`",
    };
    let doc = doc.replace("`None`", none_replacement);

    if lang == Language::Python {
        let doc = doc.replace("`true`", "`True`").replace("`false`", "`False`");
        return doc;
    }

    if lang != Language::Rust {
        let doc = doc.replace("`True`", "`true`").replace("`False`", "`false`");
        return doc;
    }

    doc
}

/// Replace common Rust type spellings in prose with language-native terms.
pub(crate) fn replace_rust_type_terms(doc: &str, lang: Language) -> String {
    if lang == Language::Rust {
        return doc.to_string();
    }

    let string_list = language_string_list_type(lang);
    let nested_string_list = language_nested_string_list_type(lang);
    let string_map = language_string_map_type(lang);
    let bytes = language_bytes_type(lang);

    map_non_code_lines(doc, |line| {
        let line = line
            .replace("Vec<Vec<String>>", nested_string_list)
            .replace("Vec<String>", string_list)
            .replace("Vec<u8>", bytes);
        replace_rust_generic_collection_terms(&line, lang)
            .replace("&BTreeMap<String, String>", string_map)
            .replace("BTreeMap<String, String>", string_map)
            .replace("&HashMap<String, String>", string_map)
            .replace("HashMap<String, String>", string_map)
            .replace("`std.io.Error`", "an operating-system I/O error")
            .replace("`std::io::Error`", "an operating-system I/O error")
            .replace("vec![", "[")
            .replace(".into()", "")
            .replace("empty vec", "empty list")
            .replace("this vec", "this list")
            .replace("Arc-wrapped ", "shared ")
            .replace("Arc semantics in-memory", "shared in-memory ownership")
            .replace("`NodeContext.with_lazy_attributes`", "lazy attribute extraction")
            .replace("`NodeContext::with_lazy_attributes`", "lazy attribute extraction")
            .replace(
                "`ConversionOptions.include_document_structure`",
                "the `include_document_structure` option",
            )
            .replace(
                "`ConversionOptions::include_document_structure`",
                "the `include_document_structure` option",
            )
            .replace("`Self.tables`", "the result's `tables` field")
            .replace("`Self::tables`", "the result's `tables` field")
            .replace(
                "Defaults to `PreprocessingOptions.default()`, which",
                "Defaults to the standard preprocessing options, which",
            )
            .replace(
                "Defaults to `PreprocessingOptions::default()`, which",
                "Defaults to the standard preprocessing options, which",
            )
            .replace(
                "or construct via `ConversionOptions.builder`",
                "or construct via the configuration builder",
            )
            .replace(
                "or construct via `ConversionOptions::builder`",
                "or construct via the configuration builder",
            )
            .replace(" (pub(crate))", "")
            .replace("(pub(crate))", "")
    })
}

fn replace_rust_generic_collection_terms(line: &str, lang: Language) -> String {
    let line = replace_wrapped_vec_terms(line.to_string(), "Vec<Arc<", ">>", lang);
    let line = replace_wrapped_vec_terms(line, "Vec<Box<dyn ", ">>", lang);
    replace_wrapped_vec_terms(line, "Vec<", ">", lang)
}

fn replace_wrapped_vec_terms(mut text: String, start: &str, end: &str, lang: Language) -> String {
    let mut search_start = 0;
    while let Some(relative_start) = text[search_start..].find(start) {
        let start_idx = search_start + relative_start;
        let inner_start = start_idx + start.len();
        let Some(relative_end) = text[inner_start..].find(end) else {
            break;
        };
        let end_idx = inner_start + relative_end;
        let inner = &text[inner_start..end_idx];
        if inner.is_empty() {
            search_start = inner_start;
            continue;
        }
        let replacement = language_list_of_type(lang, inner);
        let replacement_end = end_idx + end.len();
        text.replace_range(start_idx..replacement_end, &replacement);
        search_start = start_idx + replacement.len();
    }
    text
}

fn language_list_of_type(lang: Language, inner: &str) -> String {
    let inner = inner.trim();
    if inner == "String" {
        return language_string_list_type(lang).to_string();
    }
    if inner == "u8" {
        return language_bytes_type(lang).to_string();
    }
    if inner == "_" {
        return match lang {
            Language::Python => "list".to_string(),
            Language::Node | Language::Wasm => "Array".to_string(),
            Language::Go => "slice".to_string(),
            Language::Java | Language::Kotlin | Language::KotlinAndroid | Language::Dart => "List".to_string(),
            Language::Csharp => "List".to_string(),
            Language::Ruby | Language::Php | Language::Elixir | Language::R => "list".to_string(),
            Language::Ffi | Language::C | Language::Jni => "array".to_string(),
            Language::Swift => "Array".to_string(),
            Language::Gleam => "List".to_string(),
            Language::Zig => "slice".to_string(),
            Language::Rust => "Vec<_>".to_string(),
        };
    }
    match lang {
        Language::Python => format!("list[{inner}]"),
        Language::Node | Language::Wasm => format!("{inner}[]"),
        Language::Go => format!("[]{inner}"),
        Language::Java | Language::Kotlin | Language::KotlinAndroid | Language::Dart => format!("List<{inner}>"),
        Language::Csharp => format!("List<{inner}>"),
        Language::Ruby => format!("Array<{inner}>"),
        Language::Php => format!("array<{inner}>"),
        Language::Elixir => format!("list({inner})"),
        Language::R => "list".to_string(),
        Language::Ffi | Language::C | Language::Jni => format!("const {inner}*"),
        Language::Swift => format!("[{inner}]"),
        Language::Gleam => format!("List({inner})"),
        Language::Zig => format!("[]const {inner}"),
        Language::Rust => format!("Vec<{inner}>"),
    }
}

fn map_non_code_lines(doc: &str, mut map_line: impl FnMut(&str) -> String) -> String {
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
        } else if in_code_block {
            out.push_str(line);
        } else {
            out.push_str(&map_line(line));
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn language_string_list_type(lang: Language) -> &'static str {
    match lang {
        Language::Python => "list[str]",
        Language::Node | Language::Wasm => "string[]",
        Language::Go => "[]string",
        Language::Java => "List<String>",
        Language::Csharp => "List<string>",
        Language::Ruby => "Array<String>",
        Language::Php => "array<string>",
        Language::Elixir => "list(String.t())",
        Language::R => "list",
        Language::Ffi | Language::C | Language::Jni => "const char**",
        Language::Kotlin | Language::KotlinAndroid | Language::Dart => "List<String>",
        Language::Swift => "[String]",
        Language::Gleam => "List(String)",
        Language::Zig => "[]const []const u8",
        Language::Rust => "Vec<String>",
    }
}

fn language_nested_string_list_type(lang: Language) -> &'static str {
    match lang {
        Language::Python => "list[list[str]]",
        Language::Node | Language::Wasm => "string[][]",
        Language::Go => "[][]string",
        Language::Java => "List<List<String>>",
        Language::Csharp => "List<List<string>>",
        Language::Ruby => "Array<Array<String>>",
        Language::Php => "array<array<string>>",
        Language::Elixir => "list(list(String.t()))",
        Language::R => "list",
        Language::Ffi | Language::C | Language::Jni => "const char***",
        Language::Kotlin | Language::KotlinAndroid | Language::Dart => "List<List<String>>",
        Language::Swift => "[[String]]",
        Language::Gleam => "List(List(String))",
        Language::Zig => "[]const []const []const u8",
        Language::Rust => "Vec<Vec<String>>",
    }
}

fn language_string_map_type(lang: Language) -> &'static str {
    match lang {
        Language::Python => "dict[str, str]",
        Language::Node | Language::Wasm => "Record<string, string>",
        Language::Go => "map[string]string",
        Language::Java => "Map<String, String>",
        Language::Csharp => "Dictionary<string, string>",
        Language::Ruby => "Hash{String=>String}",
        Language::Php => "array<string, string>",
        Language::Elixir => "map()",
        Language::R => "list",
        Language::Ffi | Language::C | Language::Jni => "void*",
        Language::Kotlin | Language::KotlinAndroid => "Map<String, String>",
        Language::Swift => "[String: String]",
        Language::Dart => "Map<String, String>",
        Language::Gleam => "Dict(String, String)",
        Language::Zig => "std.StringHashMap([]const u8)",
        Language::Rust => "BTreeMap<String, String>",
    }
}

fn language_bytes_type(lang: Language) -> &'static str {
    match lang {
        Language::Python => "bytes",
        Language::Node | Language::Wasm => "Buffer",
        Language::Go => "[]byte",
        Language::Java => "byte[]",
        Language::Csharp => "byte[]",
        Language::Ruby => "String",
        Language::Php => "string",
        Language::Elixir => "binary()",
        Language::R => "raw",
        Language::Ffi | Language::C | Language::Jni => "const uint8_t*",
        Language::Kotlin | Language::KotlinAndroid => "ByteArray",
        Language::Swift => "Data",
        Language::Dart => "Uint8List",
        Language::Gleam => "BitArray",
        Language::Zig => "[]const u8",
        Language::Rust => "Vec<u8>",
    }
}

/// Replace Rust `Foo::bar()` path notation with `Foo.bar()` in prose (outside code blocks).
///
/// Rust and PHP static paths use `::`, so keep that separator for those languages.
pub(crate) fn rust_paths_to_dot_notation(doc: &str, lang: Language) -> String {
    let sep = if matches!(lang, Language::Rust | Language::Php) {
        "::"
    } else {
        "."
    };
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
        if line.trim_start().starts_with("```") {
            if in_code_block {
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

        if line.starts_with('#') {
            let header_text = line.trim_start_matches('#').trim().to_lowercase();
            if RUST_ONLY_SECTIONS.contains(&header_text.as_str()) {
                skip_section = true;
                continue;
            } else {
                skip_section = false;
            }
        }

        if skip_section {
            let trimmed = line.trim();
            let is_section_content = trimmed.is_empty()
                || trimmed.starts_with('*')
                || trimmed.starts_with('-')
                || trimmed.starts_with('+')
                || trimmed.starts_with("  ")
                || trimmed.starts_with('\t');
            if is_section_content {
                continue;
            }
            skip_section = false;
        }

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
    let first_line = content.lines().next().unwrap_or("");
    let fence_lang = first_line.trim_start_matches('`').trim().to_lowercase();
    if matches!(fence_lang.as_str(), "rust" | "rust,no_run" | "rust,ignore" | "") {
        for line in content.lines().skip(1) {
            if line.starts_with("use ")
                || line.contains("unwrap()")
                || line.contains("assert!")
                || line.contains("assert_eq!")
                || line.contains("Vec::new()")
                || line.contains("Default::default()")
                || line.contains("::new(")
                || line.contains(".collect::<")
                || line.contains(".to_string()")
                || line.contains("async fn ")
                || line.contains("-> Result<")
                || line.contains("&mut ")
                || line.contains("Ok(())")
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
            let trimmed = line.trim_start_matches(['*', '-', ' ']);
            let parsed = trimmed
                .find(" - ")
                .map(|pos| (pos, 3))
                .or_else(|| trimmed.find(": ").map(|pos| (pos, 2)));
            if let Some((sep_pos, sep_len)) = parsed {
                let raw_name = trimmed[..sep_pos].trim();
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
    let mut result = String::with_capacity(doc.len());
    let chars: Vec<char> = doc.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '`' {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() {
                let text: String = chars[start..j].iter().collect();
                if j + 1 < chars.len() && chars[j + 1] == '(' {
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

/// Collapse abutting single-backtick code spans into one span.
///
/// When two single-backtick code spans touch with no characters between them,
/// the source text contains a doubled backtick run (`` `a``b` ``). CommonMark
/// only matches a backtick run against a closing run of equal length, so the
/// doubled run is no longer parsed as inline code — it desyncs the inline-code
/// scanner and leaves the bracket characters inside exposed as literal text.
/// Strict Markdown link validators (e.g. Zensical `--strict`) then detect
/// later `[...]` spans as unresolved shortcut reference links.
///
/// This pass rewrites `` `a``b` `` → `` `ab` `` by dropping the two boundary
/// backticks between adjacent spans. It only removes a backtick pair when the
/// closing backtick of one span is immediately followed by the opening backtick
/// of the next, so genuine double-backtick spans (`` ``code with ` tick`` ``)
/// are left untouched — those have content, not another opening delimiter,
/// after their opening run.
fn collapse_adjacent_code_spans(doc: &str) -> String {
    let chars: Vec<char> = doc.chars().collect();
    let mut result = String::with_capacity(doc.len());
    let mut i = 0;
    let mut in_code_block = false;

    while i < chars.len() {
        if (i == 0 || chars[i - 1] == '\n')
            && chars[i] == '`'
            && chars.get(i + 1) == Some(&'`')
            && chars.get(i + 2) == Some(&'`')
        {
            in_code_block = !in_code_block;
            result.push_str("```");
            i += 3;
            continue;
        }

        if !in_code_block && chars[i] == '`' && chars.get(i + 1) != Some(&'`') {
            let mut content = String::new();
            let mut j = i + 1;
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == '`' {
                    if chars.get(j + 1) == Some(&'`') && chars.get(j + 2) != Some(&'`') {
                        j += 2;
                        continue;
                    }
                    closed = true;
                    break;
                }
                content.push(chars[j]);
                j += 1;
            }
            if closed {
                result.push('`');
                result.push_str(&content);
                result.push('`');
                i = j + 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

#[cfg(test)]
mod tests;
