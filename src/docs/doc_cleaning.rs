use super::markdown_canonical::{FenceEvent, apply_fence_event, fence_event};
use crate::core::config::Language;

mod heading_levels;
mod terminology;

#[cfg(test)]
pub(crate) use heading_levels::check_monotonic_headings;
pub(crate) use heading_levels::demote_headings_to_start_at;
use terminology::replace_whole_word;

/// Rust doc section headers that should be stripped for all non-Rust output.
const RUST_ONLY_SECTIONS: &[&str] = &["example", "examples", "arguments", "fields"];

/// Trailing characters that terminate a URL when they appear at the very end of an
/// otherwise-greedy match: prose punctuation that follows a URL far more often than it is
/// actually part of one. Never trimmed from the *middle* of a match -- only repeatedly off
/// the end, so a URL whose path or query legitimately ends differently keeps every byte up
/// to the real terminator. ~keep
const URL_TRAILING_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', '"', '\''];

/// Trim trailing punctuation from a matched URL that cannot legitimately be the last
/// character of a URL, without disturbing punctuation that is genuinely part of it.
///
/// Two cases need different handling:
/// - Quotes/commas/periods/semicolons/etc. are stripped unconditionally off the end, in a
///   loop, since prose never ends a URL with them but a URL's path/query may contain them
///   earlier (`/a,b,c` stays untouched; only a trailing `,` is prose punctuation).
/// - A trailing `)` is only stripped when unbalanced against `(` earlier in the match, so a
///   Wikipedia-style `Foo_(disambiguation)` link keeps its real closing paren while `(see
///   https://example.com)` loses the parenthetical's closing paren.
///
/// A trailing `/` is never touched -- it is part of the URL, not punctuation.
fn trim_trailing_url_punctuation(url: &str) -> &str {
    let mut end = url.len();
    loop {
        let candidate = &url[..end];
        let Some(last) = candidate.chars().next_back() else {
            break;
        };
        if URL_TRAILING_PUNCTUATION.contains(&last) {
            end -= last.len_utf8();
            continue;
        }
        if last == ')' && candidate.matches(')').count() > candidate.matches('(').count() {
            end -= 1;
            continue;
        }
        break;
    }
    &url[..end]
}

/// Wrap bare `http://` and `https://` URLs in angle brackets to satisfy MD034.
/// Skips URLs already inside markdown links `[...](url)` or angle brackets `<url>`.
///
/// The match itself is greedy up to whitespace/`<`/`>`/`]`/quote characters -- quotes are
/// excluded from the match entirely (a URL is never legitimately quoted with no space
/// before the closing quote), which is what previously let a closing quote and everything
/// after it up to the next space get swallowed into the "URL". Trailing punctuation that
/// survives the character class (comma, period, unbalanced paren, ...) is trimmed by
/// [`trim_trailing_url_punctuation`] after the match. ~keep
pub(crate) fn wrap_bare_urls(text: &str) -> String {
    let url_re = regex::Regex::new(r#"(https?://[^\s>\]"']+)"#).unwrap();
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for mat in url_re.find_iter(text) {
        let start = mat.start();
        let preceding = if start > 0 { text.as_bytes()[start - 1] } else { b' ' };
        if preceding == b'(' || preceding == b'<' {
            continue;
        }
        let trimmed = trim_trailing_url_punctuation(mat.as_str());
        if trimmed.is_empty() {
            continue;
        }
        result.push_str(&text[last_end..start]);
        result.push('<');
        result.push_str(trimmed);
        result.push('>');
        last_end = start + trimmed.len();
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

    let doc = super::markdown_canonical::canonicalize_rustdoc_markdown(&doc);

    doc.trim().to_string()
}

/// Strip patch/prerelease suffixes from prose feature labels.
///
/// This intentionally targets only feature provenance phrases. API page release
/// badges are generated elsewhere and must keep the full package version. ~keep
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
    let mut fence_ticks = None;
    let mut prev_non_empty: Option<String> = None;
    let mut prev_was_blank = true;

    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            apply_fence_event(&mut fence_ticks, event);
            out.push_str(line);
            out.push('\n');
            prev_non_empty = Some(line.to_string());
            prev_was_blank = false;
            continue;
        }

        if fence_ticks.is_some() {
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
    let mut fence_ticks = None;
    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            apply_fence_event(&mut fence_ticks, event);
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if fence_ticks.is_none() && line.starts_with('#') {
            let heading_text = line.trim_start_matches('#').trim();
            let lower = heading_text.to_lowercase();
            if crate::codegen::doc_emission::BOLD_LABEL_ONLY_SECTION_NAMES.contains(&lower.as_str()) {
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
    let mut fence_ticks = None;
    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            apply_fence_event(&mut fence_ticks, event);
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if fence_ticks.is_some() {
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
    // "this crate" is a bare-word term, not delimited syntax -- a plain substring replace
    // would also match inside "crater"/"craters" (e.g. "this crater"), so it goes through the
    // boundary-aware helper instead of `.replace`. ~keep
    let doc = replace_whole_word(doc, "this crate", "this library");
    let doc = doc
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
        let line = replace_rust_generic_collection_terms(&line, lang)
            .replace("&BTreeMap<String, String>", string_map)
            .replace("BTreeMap<String, String>", string_map)
            .replace("&HashMap<String, String>", string_map)
            .replace("HashMap<String, String>", string_map)
            .replace("`std.io.Error`", "an operating-system I/O error")
            .replace("`std::io::Error`", "an operating-system I/O error")
            .replace("vec![", "[")
            .replace(".into()", "");
        // "empty vec"/"this vec" are bare-word terms, not `Vec<...>` type syntax -- a plain
        // substring replace here would also match inside "vector" (e.g. "empty vector"),
        // so these two use the boundary-aware helper instead of `.replace`. ~keep
        let line = replace_whole_word(&line, "empty vec", "empty list");
        let line = replace_whole_word(&line, "this vec", "this list");
        line.replace("Arc-wrapped ", "shared ")
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
    let mut fence_ticks = None;
    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            apply_fence_event(&mut fence_ticks, event);
            out.push_str(line);
        } else if fence_ticks.is_some() {
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
    let mut fence_ticks = None;
    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            apply_fence_event(&mut fence_ticks, event);
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if fence_ticks.is_some() {
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
/// Note: this function does NOT escape pipe characters. Every call site (in
/// `docs::language_pages::*` and `docs::shared_pages`) passes the result through
/// `escape_table_cell`, which handles pipe escaping exactly once. Escaping here
/// as well would double-escape `||` into `\\|\\|`, causing CommonMark parsers to
/// see an extra cell separator and trigger MD056 violations. ~keep
pub(crate) fn clean_doc_inline(doc: &str, lang: Language) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let cleaned = clean_doc(doc, lang);
    super::markdown_canonical::markdown_to_table_cell_inline(&cleaned)
}

/// Strip Rust-specific doc sections (`# Example`, `# Arguments`, `# Fields`).
///
/// Also strips fenced code blocks that contain Rust-specific syntax
/// (use statements, unwrap(), assert!, etc.) regardless of which section they appear in.
pub(crate) fn strip_rust_sections(doc: &str) -> String {
    let mut out = String::new();
    let mut skip_section = false;
    let mut fence_ticks = None;
    let mut code_block_buf = String::new();

    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            match event {
                FenceEvent::Close => {
                    fence_ticks = None;
                    if !skip_section && !is_rust_code_block(&code_block_buf) {
                        out.push_str(&code_block_buf);
                        out.push_str(line);
                        out.push('\n');
                    }
                    code_block_buf.clear();
                    continue;
                }
                FenceEvent::Open(ticks) => {
                    fence_ticks = Some(ticks);
                    if !skip_section {
                        code_block_buf.push_str(line);
                        code_block_buf.push('\n');
                    }
                    continue;
                }
            }
        }

        if fence_ticks.is_some() {
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
            // Indentation is read from the RAW line. The two continuation arms below used to test
            // the TRIMMED one, which cannot begin with whitespace by construction -- so they never
            // matched, and a wrapped bullet ended the skip and leaked its own tail into the page.
            // `* \`options\` — conversion options. Rust accepts bare [\`ConversionOptions\`],` wraps
            // onto two indented lines, and those two lines were published verbatim as a headless
            // paragraph, mid-sentence, in every generated reference page. ~keep
            let is_section_content = trimmed.is_empty()
                || trimmed.starts_with(['*', '-', '+'])
                || line.starts_with("  ")
                || line.starts_with('\t');
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
    let fence_info = first_line.trim_start_matches('`').trim().to_lowercase();
    // The fence info string is language plus optional rustdoc doctest attributes
    // (`rust,no_run`, a bare `no_run`, `rust,no_run,should_panic`, ...); only
    // `Language::from_fence_info` knows that vocabulary. Matching the raw string
    // against a handful of literal combinations (the previous approach) silently
    // stopped recognizing every combination it didn't happen to enumerate, which let
    // unrecognized-but-still-Rust blocks survive untouched into non-Rust language
    // pages. ~keep
    let fence_lang = fence_info.split_whitespace().next().unwrap_or("");
    if crate::snippets::types::Language::from_fence_info(fence_lang) == crate::snippets::types::Language::Rust {
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
    let mut fence_ticks = None;
    let mut current_param: Option<String> = None;

    for line in doc.lines() {
        if let Some(event) = fence_event(line, fence_ticks) {
            apply_fence_event(&mut fence_ticks, event);
            continue;
        }
        if fence_ticks.is_some() {
            continue;
        }

        if line.starts_with('#') {
            let header = line.trim_start_matches('#').trim().to_lowercase();
            in_args = matches!(header.as_str(), "arguments" | "args" | "parameters" | "params");
            current_param = None;
            continue;
        }

        if in_args {
            let trimmed = line.trim();
            let item = trimmed
                .strip_prefix('*')
                .or_else(|| trimmed.strip_prefix('-'))
                .or_else(|| trimmed.strip_prefix('+'))
                .map(str::trim_start);
            if let Some(item) = item {
                let parsed = item
                    .find(" - ")
                    .map(|position| (position, 3))
                    .or_else(|| item.find(": ").map(|position| (position, 2)));
                let Some((separator, separator_len)) = parsed else {
                    current_param = None;
                    continue;
                };
                let raw_name = item[..separator].trim();
                let param_name = raw_name.trim_matches('`');
                let desc = item[separator + separator_len..].trim();
                if !param_name.is_empty() && !desc.is_empty() {
                    map.insert(param_name.to_string(), desc.to_string());
                    current_param = Some(param_name.to_string());
                }
            } else if !trimmed.is_empty()
                && let Some(param_name) = current_param.as_ref()
                && let Some(description) = map.get_mut(param_name)
            {
                description.push(' ');
                description.push_str(trimmed);
            }
        }
    }

    map
}

/// Convert `` [`text`](path) `` and bare `` [`text`] `` patterns to `` `text` ``.
///
/// Also degrades a plain-text intra-doc link — `[text](Self::method)`, with no
/// backticks in the link text — to plain `text` when the target looks like a
/// Rust path. This must run before [`rust_paths_to_dot_notation`], which
/// otherwise idiom-translates `::` inside the link target and leaves a
/// relative link that resolves nowhere in the emitted doc. ~keep
pub(crate) fn rust_links_to_plain(doc: &str) -> String {
    let mut result = String::with_capacity(doc.len());
    let chars: Vec<char> = doc.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if i + 1 < chars.len() && chars[i + 1] == '`' {
                if let Some((text, next)) = parse_backticked_bracket_link(&chars, i) {
                    result.push_str(&text);
                    i = next;
                    continue;
                }
            } else if let Some((text, next)) = parse_plain_rust_path_link(&chars, i) {
                result.push_str(&text);
                i = next;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Parse `` [`text`](target) `` or bare `` [`text`] `` starting at the `[` at index
/// `i`. Returns the backticked text and the index following the whole match.
fn parse_backticked_bracket_link(chars: &[char], i: usize) -> Option<(String, usize)> {
    let start = i + 1;
    let mut j = start;
    while j < chars.len() && chars[j] != ']' {
        j += 1;
    }
    if j >= chars.len() {
        return None;
    }
    let text: String = chars[start..j].iter().collect();
    if j + 1 < chars.len() && chars[j + 1] == '(' {
        let mut k = j + 2;
        while k < chars.len() && chars[k] != ')' {
            k += 1;
        }
        if k < chars.len() {
            return Some((text, k + 1));
        }
        return None;
    }
    Some((text, j + 1))
}

/// Parse `[text](target)` (no backticks in `text`) starting at the `[` at index
/// `i`. Returns `(text, index-after-match)` only when `target` looks like a
/// rustdoc intra-doc path rather than a genuine Markdown link, so real relative
/// links, anchors, and URLs are left for the caller to emit untouched.
fn parse_plain_rust_path_link(chars: &[char], i: usize) -> Option<(String, usize)> {
    let start = i + 1;
    let mut j = start;
    while j < chars.len() && chars[j] != ']' && chars[j] != '[' {
        j += 1;
    }
    if j >= chars.len() || chars[j] != ']' || j + 1 >= chars.len() || chars[j + 1] != '(' {
        return None;
    }
    let target_start = j + 2;
    let mut k = target_start;
    while k < chars.len() && chars[k] != ')' {
        k += 1;
    }
    if k >= chars.len() {
        return None;
    }
    let target: String = chars[target_start..k].iter().collect();
    if !looks_like_rust_doc_path(&target) {
        return None;
    }
    let text: String = chars[start..j].iter().collect();
    Some((text, k + 1))
}

/// Return `true` when a `[text](target)` link `target` looks like a rustdoc
/// intra-doc path — `Self::foo`, `crate::Bar::baz`, or a bare identifier —
/// rather than a genuine Markdown document link, anchor, or URL.
fn looks_like_rust_doc_path(target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() || is_document_link_target(target) {
        return false;
    }
    let Some(first) = target.chars().next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    target
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '(' || c == ')')
}

/// Return `true` when `target` looks like a genuine Markdown document link,
/// anchor, or URL that must never be treated as a rustdoc path.
fn is_document_link_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with("www.")
        || target.contains('/')
        || lower.ends_with(".md")
        || lower.ends_with(".html")
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
/// after their opening run. ~keep
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
                if chars[j] != '`' {
                    content.push(chars[j]);
                    j += 1;
                    continue;
                }
                if chars.get(j + 1) == Some(&'`') && chars.get(j + 2) != Some(&'`') {
                    j += 2;
                    continue;
                }
                closed = true;
                break;
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
