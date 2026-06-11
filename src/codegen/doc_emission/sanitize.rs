use super::sections::is_rust_fence_tag;

/// Target language for [`sanitize_rust_idioms`].
///
/// Each variant selects the idiomatic mapping for Rust constructs that do not
/// translate directly to foreign-language doc syntax.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DocTarget {
    /// PHPDoc (`/** ... */`), e.g. phpstan-typed prose.
    PhpDoc,
    /// Javadoc (`/** ... */`), e.g. OpenJDK-style annotations.
    JavaDoc,
    /// TSDoc (`/** ... */`), TypeScript variant of JSDoc.
    TsDoc,
    /// JSDoc (`/** ... */`), JavaScript variant.
    JsDoc,
    /// C# XML doc (`/// <summary>...</summary>`).
    ///
    /// Strips Rust code fences and section headings (`# Examples`,
    /// `# Arguments`, `# Returns`, etc.), drops Rust trait-bound prose,
    /// and XML-escapes any remaining `<` / `>` / `&` so the result is
    /// safe to embed inside a `<summary>` element.
    CSharpDoc,
}

/// Sanitize Rust-specific idioms in a prose string for the given foreign-language
/// documentation target.
///
/// Transformations are applied **outside** backtick spans and code fences only,
/// so inline code examples and fenced blocks are never mutated (except that
/// ` ```rust ` fences and unmarked ` ``` ` code blocks are dropped entirely
/// for all targets [`DocTarget::TsDoc`], [`DocTarget::JsDoc`], [`DocTarget::PhpDoc`],
/// [`DocTarget::JavaDoc`], and [`DocTarget::CSharpDoc`]).
///
/// # Transformations
///
/// - Intra-doc links `` [`Type::method`] `` → `` `Type.method` ``.
/// - `[`Foo`]` (backtick inside square brackets) → `` `Foo` ``.
/// - `None` (word boundary) → `null` (PHP/Java) or `undefined` (TS/JS).
/// - `Some(x)` → `the value (x)`.
/// - `Option<T>` → `T?` (PHP) / `T | null` (Java) / `T | undefined` (TS/JS).
/// - `Vec<u8>` → `string` (PHP) / `byte[]` (Java) / `Uint8Array` (TS/JS).
/// - `Vec<T>` → `T[]` (all targets).
/// - `HashMap<K, V>` → `array<K, V>` (PHP) / `Map<K, V>` (Java) / `Record<K, V>` (TS/JS).
/// - `Arc<T>`, `Box<T>`, `Mutex<T>`, `RwLock<T>`, `Rc<T>`, `Cell<T>`, `RefCell<T>` → `T`.
/// - `Send + Sync`, `Send`, `Sync`, `'static` → stripped.
/// - Standalone `::` between identifiers → `.`.
/// - `pub fn `, `crate::`, `&self`, `&mut self` → stripped.
/// - `#[…]` attribute macros on their own line or inline → stripped.
/// - `.unwrap()`, `.expect("…")` → stripped.
/// - ` ```rust ` and unmarked ` ``` ` code fences → dropped entirely.
pub fn sanitize_rust_idioms(text: &str, target: DocTarget) -> String {
    // For C# XML doc the default is to drop rustdoc section headings
    // (`# Examples`, `# Arguments`, …) and the remainder of the comment,
    // because those bodies routinely contain content that cannot be embedded
    // safely inside `<summary>`. Callers that have already extracted sections
    // (`emit_csharp_doc`) sanitise each section body via [`sanitize_rust_idioms_keep_sections`].
    sanitize_rust_idioms_inner(text, target, true)
}

/// Same as [`sanitize_rust_idioms`] but never drops rustdoc section headings.
///
/// Used by emitters that have already split the doc into sections and need to
/// sanitise each body fragment independently (e.g. C# XML doc emission with
/// per-section `<param>` / `<returns>` / `<exception>` tags).
pub fn sanitize_rust_idioms_keep_sections(text: &str, target: DocTarget) -> String {
    sanitize_rust_idioms_inner(text, target, false)
}

fn sanitize_rust_idioms_inner(text: &str, target: DocTarget, drop_csharp_sections: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_rust_fence = false;
    let mut in_other_fence = false;
    // For C# XML doc: once a `# Examples` / `# Arguments` / etc. heading is
    // encountered, drop the entire remainder of the comment. Rustdoc section
    // headings cannot be safely embedded inside `<summary>` and the per-section
    // content (code fences, intra-doc links, generics) is the leading cause
    // of CS1002/CS1519 leakage. The plain `<summary>` path collapses to the
    // top-level prose only.
    let mut csharp_section_dropped = false;

    for line in text.lines() {
        if csharp_section_dropped {
            continue;
        }
        let trimmed = line.trim_start();
        if drop_csharp_sections
            && matches!(target, DocTarget::CSharpDoc)
            && !in_rust_fence
            && !in_other_fence
            && is_rustdoc_section_heading(trimmed)
        {
            csharp_section_dropped = true;
            continue;
        }

        // Detect code fence boundaries.
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_rust_fence {
                // Closing fence of a rust block.
                in_rust_fence = false;
                match target {
                    DocTarget::TsDoc
                    | DocTarget::JsDoc
                    | DocTarget::CSharpDoc
                    | DocTarget::PhpDoc
                    | DocTarget::JavaDoc => {
                        // Entire rust block dropped — don't emit closing fence.
                    }
                }
                continue;
            }
            if in_other_fence {
                // Closing fence of a non-rust block.
                in_other_fence = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            // Opening fence — determine language.
            let lang = rest.split(',').next().unwrap_or("").trim();
            let is_rust = is_rust_fence_tag(lang);
            if is_rust {
                in_rust_fence = true;
                match target {
                    DocTarget::TsDoc
                    | DocTarget::JsDoc
                    | DocTarget::CSharpDoc
                    | DocTarget::PhpDoc
                    | DocTarget::JavaDoc => {
                        // Drop the entire rust fence block — skip opening line.
                        // Rust code examples are not portable to any of the target languages.
                    }
                }
                continue;
            }
            // Non-rust fence: pass through verbatim.
            in_other_fence = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Inside a rust fence.
        if in_rust_fence {
            match target {
                DocTarget::TsDoc | DocTarget::JsDoc | DocTarget::CSharpDoc | DocTarget::PhpDoc | DocTarget::JavaDoc => {
                    // Drop content of rust fences — all targets filter out Rust code examples.
                }
            }
            continue;
        }

        // Inside a non-rust fence: pass through verbatim.
        if in_other_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Check if this line is a bare `#[...]` attribute line.
        let stripped_indent = line.trim_start();
        if stripped_indent.starts_with("#[") && stripped_indent.ends_with(']') {
            // Attribute-only line — drop entirely.
            continue;
        }

        // Normal prose line: apply token-level transformations.
        let sanitized = apply_prose_transforms(line, target);
        out.push_str(&sanitized);
        out.push('\n');
    }

    // Trim trailing newline added by the loop (preserve internal newlines).
    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }

    // For JSDoc and TSDoc, escape any `*/` sequences so they don't prematurely
    // close the /** ... */ block. Replace `*/` with `* /` (backslash prevents
    // JS/TS comment-terminator matching but renders identically in docs).
    if matches!(target, DocTarget::TsDoc | DocTarget::JsDoc) {
        out = escape_jsdoc_block_close(&out);
    }

    // For C# XML doc, escape any remaining `<`, `>`, `&` so the result is
    // safe to embed inside `<summary>...</summary>`. By this point the
    // Rust-idiom substitutions have replaced `Vec<T>` / `Option<T>` /
    // `HashMap<K, V>` / `Result<T, E>` with their idiomatic forms, but
    // unrecognised generic constructs (e.g. trait-object references) may
    // still contain raw angle brackets that would break C# XML parsing.
    if matches!(target, DocTarget::CSharpDoc) {
        out = xml_escape_for_csharp(&out);
    }

    out
}

/// Return `true` if `line` (already left-trimmed) is a Rustdoc section heading
/// such as `# Examples`, `# Arguments`, `# Returns`, `# Errors`, `# Panics`,
/// or `# Safety`. Case-insensitive on the heading name.
fn is_rustdoc_section_heading(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("# ") else {
        return false;
    };
    let head = rest.trim().to_ascii_lowercase();
    matches!(
        head.as_str(),
        "arguments" | "args" | "returns" | "errors" | "panics" | "safety" | "example" | "examples"
    )
}

/// Escape JSDoc block-close sequences (`*/`) by replacing with `* /`.
///
/// JSDoc comments use `/** ... */` blocks. If rustdoc content contains a backtick
/// code span like `` `/* ... */` ``, the `*/` inside the backticks lands verbatim
/// in the emitted JSDoc and prematurely closes the comment block, breaking downstream
/// tools like oxfmt. This function replaces `*/` with `* /` (backslash breaks the
/// terminator matching) while preserving visual rendering in docs.
fn escape_jsdoc_block_close(s: &str) -> String {
    s.replace("*/", "* /")
}

/// XML-escape `<`, `>`, `&` for safe embedding inside a C# `<summary>` element.
///
/// `<` / `>` may legitimately appear in prose after Rust-idiom substitution
/// when the substitutions produce C#-friendly forms (e.g. `Dictionary<K, V>`).
/// Those are still XML-significant characters and must be entity-escaped for
/// XML parsers (Roslyn, doxygen) to accept the resulting `<summary>` block.
fn xml_escape_for_csharp(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Apply prose-level Rust-idiom transformations to a single line.
///
/// Some transformations span or precede backtick boundaries and must be applied
/// to the full line before tokenisation:
///
/// 1. Intra-doc links (`` [`...`] ``) — they wrap a backtick pair.
/// 2. `::` path separator — even inside backtick spans it should become `.`
///    for all foreign-language targets, since the target language uses `.` for
///    member access and package paths in code examples too.
///
/// All remaining transformations are applied only to literal (non-code) segments
/// after tokenisation.
fn apply_prose_transforms(line: &str, target: DocTarget) -> String {
    // Step 1: replace intra-doc links before tokenisation (they span backtick pairs).
    let line = replace_intradoc_links(line, target);

    // Step 2: replace :: everywhere (including inside backtick spans).
    // All targets use `.` as the member/package separator, so this is always safe.
    let line = replace_path_separator(&line);

    // Step 3: strip .unwrap() and .expect() everywhere, including inside backtick spans,
    // since these Rust error-handling idioms are meaningless in all target languages.
    let line = strip_unwrap_expect(&line);

    // Step 4: tokenise and apply remaining transforms only to literal segments.
    let segments = tokenize_backtick_spans(&line);
    let mut result = String::with_capacity(line.len());
    for (is_code, span) in segments {
        if is_code {
            result.push('`');
            result.push_str(span);
            result.push('`');
        } else {
            result.push_str(&transform_prose_segment(span, target));
        }
    }
    result
}

/// Split a line into alternating literal/code segments.
///
/// Returns `Vec<(is_code, &str)>` where `is_code` is true for the content
/// between a matched backtick pair. Unmatched backticks are treated as
/// literal characters (passed through as literal segments).
fn tokenize_backtick_spans(line: &str) -> Vec<(bool, &str)> {
    let mut segments = Vec::new();
    let bytes = line.as_bytes();
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Emit preceding literal segment.
            if i > start {
                segments.push((false, &line[start..i]));
            }
            // Find the closing backtick.
            let code_start = i + 1;
            let close = bytes[code_start..].iter().position(|&b| b == b'`');
            if let Some(offset) = close {
                let code_end = code_start + offset;
                segments.push((true, &line[code_start..code_end]));
                i = code_end + 1;
                start = i;
            } else {
                // No closing backtick — treat as literal from here.
                segments.push((false, &line[i..]));
                start = line.len();
                i = line.len();
            }
        } else {
            i += 1;
        }
    }
    if start < line.len() {
        segments.push((false, &line[start..]));
    }
    segments
}

/// Apply all prose-level Rust substitutions to a literal text segment.
///
/// Intra-doc links have already been replaced by `apply_prose_transforms`
/// before tokenisation; this function handles the remaining transformations.
fn transform_prose_segment(text: &str, target: DocTarget) -> String {
    let mut s = text.to_string();

    // 1. Strip #[derive(...)] and other inline attribute-style references.
    s = strip_inline_attributes(&s);

    // 2. Strip pub fn, crate::, &self, &mut self.
    s = s.replace("pub fn ", "");
    s = s.replace("crate::", "");
    s = s.replace("&mut self", "");
    s = s.replace("&self", "");

    // 3. Strip lifetime and bound markers.
    s = strip_lifetime_and_bounds(&s);

    // 4. Type substitutions (order matters — most specific first).
    s = replace_type_wrappers(&s, target);

    // 5. Some(x) -> the value (x).
    s = replace_some_calls(&s);

    // 5b. Bare "Some <lowercase>" in prose -> drop "Some ".
    s = replace_some_keyword_in_prose(&s);

    // 6. None -> null / undefined (word boundary, uppercase only).
    s = replace_none_keyword(&s, target);

    // Note: :: -> . and .unwrap()/.expect() stripping are applied to the full
    // line before tokenisation in apply_prose_transforms and therefore do not
    // need to be repeated here.

    s
}

/// Advance byte position `i` in `s` past one full UTF-8 character, push that
/// character to `out`, and return the new byte position.
///
/// All the byte-crawling helpers below look for ASCII special characters only.
/// When none matches, they must advance by one full character (not one byte)
/// to avoid splitting multi-byte UTF-8 sequences.
#[inline]
fn advance_char(s: &str, out: &mut String, i: usize) -> usize {
    // Safety: `i` must be a valid char boundary; callers guarantee this
    // because all branch points look for ASCII bytes which are always
    // single-byte char boundaries.
    let ch = s[i..].chars().next().expect("valid UTF-8 position");
    out.push(ch);
    i + ch.len_utf8()
}

/// Replace `` [`Type::method()`] `` and `` [`Foo`] `` intra-doc links with
/// backtick-wrapped identifiers, converting `::` to `.`.
fn replace_intradoc_links(s: &str, _target: DocTarget) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for [`
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'`' {
            // Find closing `]
            let search_start = i + 2;
            let mut found = false;
            let mut j = search_start;
            while j + 1 < bytes.len() {
                if bytes[j] == b'`' && bytes[j + 1] == b']' {
                    let inner = &s[search_start..j];
                    // Convert :: to . in the inner part.
                    let converted = inner.replace("::", ".");
                    out.push('`');
                    out.push_str(&converted);
                    out.push('`');
                    i = j + 2;
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                i = advance_char(s, &mut out, i);
            }
        } else {
            i = advance_char(s, &mut out, i);
        }
    }
    out
}

/// Wrap bare method/identifier references in brackets with backticks.
///
/// Converts `[identifier]`, `[method()]`, `[Type::method]` → `` `identifier` ``,
/// `` `method()` ``, `` `Type.method` `` respectively. This prevents rustdoc from
/// treating them as intra-doc links when emitting to FFI bindings, where those
/// identifiers may not be in scope.
///
/// Does not modify already-backtick-wrapped references like `` [`identifier`] ``.
pub(crate) fn wrap_bare_bracket_references(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Check if this is already a backtick-prefixed link: [`
            if i + 1 < bytes.len() && bytes[i + 1] == b'`' {
                // Already formatted as intra-doc link — emit as-is.
                i = advance_char(s, &mut out, i);
            } else {
                // Look for closing bracket to determine what's inside.
                let search_start = i + 1;
                if let Some(close_pos) = bytes[search_start..].iter().position(|&b| b == b']') {
                    let bracket_end = search_start + close_pos;
                    let inner = &s[search_start..bracket_end].trim();

                    // Only wrap if inner contains identifier-like characters (alphanumeric, underscore, ::, ::, ()).
                    if is_identifier_like(inner) {
                        out.push('`');
                        // Convert :: to . for foreign-language compatibility.
                        out.push_str(&inner.replace("::", "."));
                        out.push('`');
                        i = bracket_end + 1;
                    } else {
                        // Not an identifier reference — emit literally.
                        i = advance_char(s, &mut out, i);
                    }
                } else {
                    // No closing bracket — emit literally.
                    i = advance_char(s, &mut out, i);
                }
            }
        } else {
            i = advance_char(s, &mut out, i);
        }
    }

    out
}

/// Return `true` if `s` looks like a Rust identifier, method call, or path.
///
/// Matches patterns like:
/// - `identifier`
/// - `method()`
/// - `Type::method`
/// - `Self::method`
fn is_identifier_like(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Check if it starts with an identifier character (letter, underscore, or 'Self').
    let first_char = trimmed.chars().next().unwrap();
    if !first_char.is_alphabetic() && first_char != '_' {
        return false;
    }

    // Allow letters, digits, underscores, ::, (), and dots (for method chains).
    trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '(' || c == ')' || c == '.')
}

/// Strip inline `#[...]` attribute references (not on their own line — those
/// are handled as full-line drops in the main loop).
fn strip_inline_attributes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip until matching ']', handling nesting.
            let mut depth = 0usize;
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'[' {
                    depth += 1;
                } else if bytes[j] == b']' {
                    depth -= 1;
                    if depth == 0 {
                        i = j + 1;
                        break;
                    }
                }
                j += 1;
            }
            if depth != 0 {
                // Unmatched bracket: emit literally.
                i = advance_char(s, &mut out, i);
            }
        } else {
            i = advance_char(s, &mut out, i);
        }
    }
    out
}

/// Strip `'static`, `Send + Sync`, `Send`, `Sync` from prose text.
fn strip_lifetime_and_bounds(s: &str) -> String {
    // Order matters: match compound forms before simple forms.
    let mut out = s.to_string();
    // Strip `Send + Sync` (with optional spaces around `+`).
    out = regex_replace_all(&out, r"Send\s*\+\s*Sync", "");
    out = regex_replace_all(&out, r"Sync\s*\+\s*Send", "");
    // Strip standalone Send/Sync only at word boundaries.
    out = regex_replace_word_boundary(&out, "Send", "");
    out = regex_replace_word_boundary(&out, "Sync", "");
    // Strip 'static lifetime markers.
    out = regex_replace_all(&out, r"'\s*static\b", "");
    out
}

/// Replace occurrences of `pattern` (treated as a simple substring pattern
/// with `\s*` only, no full regex) with `replacement` in `s`.
///
/// This is a lightweight regex-free replacement for simple patterns that
/// only need literal text or `\s*` between tokens.
fn regex_replace_all(s: &str, pattern: &str, replacement: &str) -> String {
    // Inline tiny pattern compiler for the three patterns we actually use.
    match pattern {
        r"Send\s*\+\s*Sync" => replace_with_optional_spaces(s, "Send", "+", "Sync", replacement),
        r"Sync\s*\+\s*Send" => replace_with_optional_spaces(s, "Sync", "+", "Send", replacement),
        r"'\s*static\b" => replace_static_lifetime(s, replacement),
        _ => s.replace(pattern, replacement),
    }
}

/// Replace `word_boundary(keyword)` occurrences in `s` with `replacement`.
fn regex_replace_word_boundary(s: &str, keyword: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let klen = keyword.len();
    let bytes = s.as_bytes();
    let kbytes = keyword.as_bytes();
    if klen == 0 || klen > bytes.len() {
        return s.to_string();
    }
    let mut i = 0;
    while i + klen <= bytes.len() {
        if &bytes[i..i + klen] == kbytes {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok =
                i + klen >= bytes.len() || !bytes[i + klen].is_ascii_alphanumeric() && bytes[i + klen] != b'_';
            if before_ok && after_ok {
                out.push_str(replacement);
                i += klen;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    if i < bytes.len() {
        out.push_str(&s[i..]);
    }
    out
}

/// Replace `A <spaces> op <spaces> B` triplets with `replacement`.
fn replace_with_optional_spaces(s: &str, a: &str, op: &str, b: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();
    let total = chars.len();

    while i < total {
        // Try to match `a` at position i.
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let op_chars: Vec<char> = op.chars().collect();

        if chars[i..].starts_with(&a_chars) {
            let mut j = i + a_chars.len();
            // Skip spaces.
            while j < total && chars[j] == ' ' {
                j += 1;
            }
            // Match op.
            if j + op_chars.len() <= total && chars[j..].starts_with(&op_chars) {
                let mut k = j + op_chars.len();
                // Skip spaces.
                while k < total && chars[k] == ' ' {
                    k += 1;
                }
                // Match b.
                if k + b_chars.len() <= total && chars[k..].starts_with(&b_chars) {
                    out.push_str(replacement);
                    i = k + b_chars.len();
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replace `'static` lifetime markers (with optional spaces after `'`).
fn replace_static_lifetime(s: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            // Peek ahead skipping spaces.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let keyword = b"static";
            if bytes[j..].starts_with(keyword) {
                let end = j + keyword.len();
                // Must be followed by non-identifier char or end.
                let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_';
                if after_ok {
                    out.push_str(replacement);
                    i = end;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Replace Rust generic type wrappers in prose.
fn replace_type_wrappers(s: &str, target: DocTarget) -> String {
    // Order: most specific patterns first.
    let mut out = s.to_string();

    // Vec<u8> — must come before Vec<T>.
    let vec_u8_replacement = match target {
        DocTarget::PhpDoc => "string",
        DocTarget::JavaDoc => "byte[]",
        DocTarget::TsDoc | DocTarget::JsDoc => "Uint8Array",
        DocTarget::CSharpDoc => "byte[]",
    };
    out = replace_generic1(&out, "Vec", "u8", vec_u8_replacement);

    // HashMap<K, V> — must come before Vec<T> to avoid order-dependency issues.
    let map_replacement_fn = |k: &str, v: &str| match target {
        DocTarget::PhpDoc => format!("array<{k}, {v}>"),
        DocTarget::JavaDoc => format!("Map<{k}, {v}>"),
        DocTarget::TsDoc | DocTarget::JsDoc => format!("Record<{k}, {v}>"),
        DocTarget::CSharpDoc => format!("Dictionary<{k}, {v}>"),
    };
    out = replace_generic2(&out, "HashMap", &map_replacement_fn);

    // Vec<T> — generic.
    out = replace_generic1_passthrough(&out, "Vec", |inner| format!("{inner}[]"));

    // Option<T>.
    let option_replacement_fn = |inner: &str| match target {
        DocTarget::PhpDoc => format!("{inner}?"),
        DocTarget::JavaDoc => format!("{inner} | null"),
        DocTarget::TsDoc | DocTarget::JsDoc => format!("{inner} | undefined"),
        DocTarget::CSharpDoc => format!("{inner}?"),
    };
    out = replace_generic1_passthrough(&out, "Option", option_replacement_fn);

    // Result<T, E> — drop the error type, keep the success type.
    // C# has no Result type; the binding throws exceptions, so just the success type
    // is meaningful in prose. We do this for C# only; other targets historically left
    // `Result<T, E>` unchanged (their tests assert nothing about it).
    if matches!(target, DocTarget::CSharpDoc) {
        out = replace_generic2(&out, "Result", &|t: &str, _e: &str| t.to_string());
    }

    // Smart pointer wrappers: strip to inner type.
    for wrapper in &["Arc", "Box", "Mutex", "RwLock", "Rc", "Cell", "RefCell"] {
        out = replace_generic1_passthrough(&out, wrapper, |inner| inner.to_string());
    }

    out
}

/// Replace `Name<SingleArg>` where SingleArg is an exact literal (e.g. `Vec<u8>`).
fn replace_generic1(s: &str, name: &str, arg: &str, replacement: &str) -> String {
    let pattern = format!("{name}<{arg}>");
    s.replace(&pattern, replacement)
}

/// Replace `Name<T>` → `f(T)` for an arbitrary inner type expression.
///
/// Handles nested generics by counting angle-bracket depth.
fn replace_generic1_passthrough<F>(s: &str, name: &str, f: F) -> String
where
    F: Fn(&str) -> String,
{
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let prefix = format!("{name}<");
    let pbytes = prefix.as_bytes();
    let bytes = s.as_bytes();

    while i < bytes.len() {
        if bytes[i..].starts_with(pbytes) {
            // Check that the char before is not alphanumeric (word boundary).
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let inner_start = i + pbytes.len();
                // Find the matching '>'.
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'<' => depth += 1,
                        b'>' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let inner = &s[inner_start..j];
                    out.push_str(&f(inner));
                    i = j + 1;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Replace `Name<K, V>` → `f(K, V)` for two-argument generics (e.g. `HashMap`).
fn replace_generic2<F>(s: &str, name: &str, f: &F) -> String
where
    F: Fn(&str, &str) -> String,
{
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let prefix = format!("{name}<");
    let pbytes = prefix.as_bytes();
    let bytes = s.as_bytes();

    while i < bytes.len() {
        if bytes[i..].starts_with(pbytes) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let inner_start = i + pbytes.len();
                // Find the matching '>' respecting nesting.
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'<' => depth += 1,
                        b'>' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let inner = &s[inner_start..j];
                    // Split on the first ',' at depth 0.
                    let split = split_on_comma_at_top_level(inner);
                    if let Some((k, v)) = split {
                        out.push_str(&f(k.trim(), v.trim()));
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Split `s` on the first comma that is at angle-bracket depth 0.
fn split_on_comma_at_top_level(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some((&s[..idx], &s[idx + 1..])),
            _ => {}
        }
    }
    None
}

/// Replace `Some(x)` in prose with `the value (x)`.
fn replace_some_calls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let prefix = b"Some(";
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i..].starts_with(prefix) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let arg_start = i + prefix.len();
                // Find matching ')' respecting nesting.
                let mut depth = 1usize;
                let mut j = arg_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let arg = &s[arg_start..j];
                    out.push_str("the value (");
                    out.push_str(arg);
                    out.push(')');
                    i = j + 1;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Drop bare `Some ` when it appears as a Rust-idiom modifier in prose
/// ("(Some values)", "Some keys leave the previous", etc.). The `Some(...)`
/// call form is handled separately by [`replace_some_calls`].
///
/// Match shape: word-boundary `Some` + single ASCII space + ASCII-lowercase
/// letter. The "Some " prefix is dropped; the following word is preserved.
/// `SomeType`, `Some.method()`, `Some(x)`, and sentence-initial `Some `
/// followed by an uppercase noun stay untouched.
fn replace_some_keyword_in_prose(s: &str) -> String {
    let keyword = b"Some ";
    let klen = keyword.len();
    let bytes = s.as_bytes();
    if klen >= bytes.len() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i + klen < bytes.len() {
        if &bytes[i..i + klen] == keyword {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok = bytes[i + klen].is_ascii_lowercase();
            if before_ok && after_ok {
                i += klen;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    if i < bytes.len() {
        out.push_str(&s[i..]);
    }
    out
}

/// Replace `None` (at word boundaries, uppercase) with the target-appropriate nil.
fn replace_none_keyword(s: &str, target: DocTarget) -> String {
    let replacement = match target {
        DocTarget::PhpDoc | DocTarget::JavaDoc | DocTarget::CSharpDoc => "null",
        DocTarget::TsDoc | DocTarget::JsDoc => "undefined",
    };
    let keyword = b"None";
    let klen = keyword.len();
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    if klen > bytes.len() {
        return s.to_string();
    }
    let mut i = 0;

    while i + klen <= bytes.len() {
        if &bytes[i..i + klen] == keyword {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok =
                i + klen >= bytes.len() || !bytes[i + klen].is_ascii_alphanumeric() && bytes[i + klen] != b'_';
            if before_ok && after_ok {
                out.push_str(replacement);
                i += klen;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    if i < bytes.len() {
        out.push_str(&s[i..]);
    }
    out
}

/// Replace standalone `::` between identifiers with `.`.
fn replace_path_separator(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
            // Only replace if surrounded by identifier characters or end/start of string.
            let before_ok = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let after_ok = i + 2 < bytes.len() && (bytes[i + 2].is_ascii_alphanumeric() || bytes[i + 2] == b'_');
            if before_ok || after_ok {
                out.push('.');
                i += 2;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Strip `.unwrap()` and `.expect("...")` calls from prose.
fn strip_unwrap_expect(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Match .unwrap().
        if bytes[i..].starts_with(b".unwrap()") {
            i += b".unwrap()".len();
            continue;
        }
        // Match .expect(...).
        if bytes[i..].starts_with(b".expect(") {
            let arg_start = i + b".expect(".len();
            let mut depth = 1usize;
            let mut j = arg_start;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth == 0 {
                i = j + 1;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}
