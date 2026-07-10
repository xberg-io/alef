use super::{DocTarget, references::replace_intradoc_links, type_wrappers::replace_type_wrappers, utf8::advance_char};

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
pub(super) fn apply_prose_transforms(line: &str, target: DocTarget) -> String {
    let line = replace_intradoc_links(line, target);

    let line = replace_path_separator(&line);

    // Step 3: strip .unwrap() and .expect() everywhere, including inside backtick spans,
    let line = strip_unwrap_expect(&line);

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
            if i > start {
                segments.push((false, &line[start..i]));
            }
            let code_start = i + 1;
            let close = bytes[code_start..].iter().position(|&b| b == b'`');
            if let Some(offset) = close {
                let code_end = code_start + offset;
                segments.push((true, &line[code_start..code_end]));
                i = code_end + 1;
                start = i;
            } else {
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

    s = s.replace("pub fn ", "");
    s = s.replace("crate::", "");
    s = s.replace("&mut self", "");
    s = s.replace("&self", "");

    s = strip_lifetime_and_bounds(&s);

    s = replace_type_wrappers(&s, target);

    s = replace_some_calls(&s);

    s = replace_some_keyword_in_prose(&s);

    s = replace_none_keyword(&s, target);

    // Note: :: -> . and .unwrap()/.expect() stripping are applied to the full

    s
}

/// Strip inline `#[...]` attribute references (not on their own line — those
/// are handled as full-line drops in the main loop).
fn strip_inline_attributes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
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
    let mut out = s.to_string();
    out = regex_replace_all(&out, r"Send\s*\+\s*Sync", "");
    out = regex_replace_all(&out, r"Sync\s*\+\s*Send", "");
    out = regex_replace_word_boundary(&out, "Send", "");
    out = regex_replace_word_boundary(&out, "Sync", "");
    out = regex_replace_all(&out, r"'\s*static\b", "");
    out
}

/// Replace occurrences of `pattern` (treated as a simple substring pattern
/// with `\s*` only, no full regex) with `replacement` in `s`.
///
/// This is a lightweight regex-free replacement for simple patterns that
/// only need literal text or `\s*` between tokens.
fn regex_replace_all(s: &str, pattern: &str, replacement: &str) -> String {
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
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let op_chars: Vec<char> = op.chars().collect();

        if chars[i..].starts_with(&a_chars) {
            let mut j = i + a_chars.len();
            while j < total && chars[j] == ' ' {
                j += 1;
            }
            if j + op_chars.len() <= total && chars[j..].starts_with(&op_chars) {
                let mut k = j + op_chars.len();
                while k < total && chars[k] == ' ' {
                    k += 1;
                }
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
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let keyword = b"static";
            if bytes[j..].starts_with(keyword) {
                let end = j + keyword.len();
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
