//! Language-specific string escaping for e2e test code generation.

/// Escape a string for embedding in a Python string literal.
pub fn escape_python(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Control character — emit \xHH escape so Python source remains valid.
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Escape a string for embedding in a Rust string literal.
pub fn escape_rust(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Compute the number of # needed for a Rust raw string literal.
pub fn raw_string_hashes(s: &str) -> usize {
    let mut max_hashes = 0;
    let mut current = 0;
    let mut after_quote = false;
    for ch in s.chars() {
        if ch == '"' {
            after_quote = true;
            current = 0;
        } else if ch == '#' && after_quote {
            current += 1;
            max_hashes = max_hashes.max(current);
        } else {
            after_quote = false;
            current = 0;
        }
    }
    max_hashes + 1
}

/// Format a string as a Rust raw string literal (r#"..."#).
pub fn rust_raw_string(s: &str) -> String {
    let hashes = raw_string_hashes(s);
    let h: String = "#".repeat(hashes);
    format!("r{h}\"{s}\"{h}")
}

/// Escape a string for embedding in a JavaScript/TypeScript double-quoted string literal.
///
/// `$` does not need escaping in double-quoted strings (only in template literals).
/// Escaping it would produce `\$` which Biome flags as `noUselessEscapeInString`.
pub fn escape_js(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a JavaScript/TypeScript template literal (backtick string).
///
/// Template literals interpolate `${...}` and use backtick delimiters, so both
/// `` ` `` and `$` must be escaped to prevent unintended interpolation.
pub fn escape_js_template(s: &str) -> String {
    s.replace('\\', "\\\\").replace('`', "\\`").replace('$', "\\$")
}

/// Returns `true` if the string must use a Go interpreted (double-quoted) literal
/// rather than a raw (backtick) literal.
///
/// Go raw string literals cannot contain backtick characters or NUL bytes, and
/// `\r` inside a raw string is passed through as a literal CR which gofmt rejects.
fn go_needs_quoted(s: &str) -> bool {
    s.contains('`') || s.bytes().any(|b| b == 0 || b == b'\r')
}

/// Format a string as a Go string literal (backtick or quoted).
///
/// Prefers backtick raw literals for readability, but falls back to double-quoted
/// interpreted literals when the string contains characters that raw literals
/// cannot represent: backtick `` ` ``, NUL (`\x00`), or carriage return (`\r`).
pub fn go_string_literal(s: &str) -> String {
    if go_needs_quoted(s) {
        format!("\"{}\"", escape_go(s))
    } else {
        format!("`{s}`")
    }
}

/// Escape a string for embedding in a Go double-quoted string.
///
/// Handles all characters that cannot appear literally in a Go interpreted string:
/// `\\`, `"`, `\n`, `\r`, `\t`, and NUL (`\x00`). Other non-printable bytes are
/// emitted as `\xNN` hex escape sequences.
pub fn escape_go(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0 => out.push_str("\\x00"),
            // Other control characters or non-ASCII bytes: hex escape.
            b if b < 0x20 || b == 0x7f => {
                out.push_str(&format!("\\x{b:02x}"));
            }
            _ => out.push(b as char),
        }
    }
    out
}

/// Escape a string for embedding in a Java string literal.
pub fn escape_java(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a Kotlin double-quoted string literal.
/// Like Java escaping but also escapes `$` which triggers Kotlin string interpolation.
pub fn escape_kotlin(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a C# string literal.
pub fn escape_csharp(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a PHP string literal.
pub fn escape_php(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a double-quoted Ruby string literal.
pub fn escape_ruby(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('#', "\\#")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a single-quoted Ruby string literal.
/// Single-quoted Ruby strings only interpret `\\` and `\'`.
pub fn escape_ruby_single(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Returns true if the string needs double quotes (contains control characters
/// that require escape sequences only available in double-quoted strings).
pub fn ruby_needs_double_quotes(s: &str) -> bool {
    s.contains('\n') || s.contains('\r') || s.contains('\t') || s.contains('\0')
}

/// Format a string as a Ruby literal, preferring single quotes.
pub fn ruby_string_literal(s: &str) -> String {
    if ruby_needs_double_quotes(s) {
        format!("\"{}\"", escape_ruby(s))
    } else {
        format!("'{}'", escape_ruby_single(s))
    }
}

/// Escape a string for embedding in an Elixir string literal.
pub fn escape_elixir(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('#', "\\#")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in an R string literal.
pub fn escape_r(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a C string literal.
pub fn escape_c(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Sanitize an identifier for use as a test function name.
/// Replaces non-alphanumeric characters with underscores, strips leading digits.
pub fn sanitize_ident(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    // Strip leading digits
    let trimmed = result.trim_start_matches(|c: char| c.is_ascii_digit());
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Convert a category name to a sanitized filename component.
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect::<String>()
        .to_lowercase()
}

/// Expand fixture template expressions in a string value.
///
/// Supported templates:
/// - `{{ repeat 'X' N times }}` — expands to the character X repeated N times
///
/// If no templates are found, the original string is returned unchanged.
pub fn expand_fixture_templates(s: &str) -> String {
    const PREFIX: &str = "{{ repeat '";
    const SUFFIX: &str = " times }}";

    let mut result = String::with_capacity(s.len());
    let mut remaining = s;

    while let Some(start) = remaining.find(PREFIX) {
        result.push_str(&remaining[..start]);
        let after_prefix = &remaining[start + PREFIX.len()..];

        // Expect character(s) followed by `' N times }}`
        if let Some(quote_pos) = after_prefix.find("' ") {
            let ch = &after_prefix[..quote_pos];
            let after_quote = &after_prefix[quote_pos + 2..];

            if let Some(end) = after_quote.find(SUFFIX) {
                let count_str = after_quote[..end].trim();
                if let Ok(count) = count_str.parse::<usize>() {
                    result.push_str(&ch.repeat(count));
                    remaining = &after_quote[end + SUFFIX.len()..];
                    continue;
                }
            }
        }

        // Template didn't match — emit the prefix literally and continue
        result.push_str(PREFIX);
        remaining = after_prefix;
    }
    result.push_str(remaining);
    result
}

/// Escape a string for embedding in a POSIX single-quoted shell string literal.
///
/// Wraps the string in single quotes and escapes embedded single quotes as `'\''`.
/// Single-quoted shell strings treat every character literally except `'`, so
/// no other escaping is needed.
pub fn escape_shell(s: &str) -> String {
    s.replace('\'', r"'\''")
}

/// Escape a string for embedding in a Gleam string literal.
pub fn escape_gleam(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a Zig string literal.
pub fn escape_zig(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go raw string literals (backticks) cannot contain NUL bytes — gofmt rejects them.
    /// Strings with NUL must fall back to a double-quoted interpreted literal with `\x00`.
    #[test]
    fn go_string_literal_nul_bytes_use_quoted_form() {
        let s = "Hello\x00World";
        let lit = go_string_literal(s);
        // Must not contain a raw NUL byte
        assert!(
            !lit.as_bytes().contains(&0u8),
            "go_string_literal emitted a NUL byte — gofmt would reject this: {lit:?}"
        );
        // Must be a double-quoted string, not a backtick raw string
        assert!(
            lit.starts_with('"'),
            "expected double-quoted string for NUL input, got: {lit:?}"
        );
        // The NUL must be represented as \\x00
        assert!(
            lit.contains("\\x00"),
            "expected \\x00 escape sequence for NUL byte, got: {lit:?}"
        );
    }

    /// Strings with carriage return must also use the double-quoted form
    /// because Go raw strings cannot represent `\r`.
    #[test]
    fn go_string_literal_carriage_return_uses_quoted_form() {
        let s = "line1\r\nline2";
        let lit = go_string_literal(s);
        assert!(
            !lit.as_bytes().contains(&b'\r'),
            "go_string_literal emitted a literal CR — gofmt would reject this: {lit:?}"
        );
        assert!(
            lit.starts_with('"'),
            "expected double-quoted string for CR input, got: {lit:?}"
        );
    }

    /// Strings with only printable chars and no backtick should still use the
    /// readable backtick form.
    #[test]
    fn go_string_literal_plain_string_uses_backtick() {
        let s = "Hello World\nwith newline";
        let lit = go_string_literal(s);
        assert!(
            lit.starts_with('`'),
            "expected backtick form for plain string, got: {lit:?}"
        );
    }

    /// Strings that contain a backtick must fall back to double-quoted form.
    #[test]
    fn go_string_literal_backtick_in_string_uses_quoted_form() {
        let s = "has `backtick`";
        let lit = go_string_literal(s);
        assert!(
            lit.starts_with('"'),
            "expected double-quoted form when string contains backtick, got: {lit:?}"
        );
    }
}
