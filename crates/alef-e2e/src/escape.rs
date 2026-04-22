//! Language-specific string escaping for e2e test code generation.

/// Escape a string for embedding in a Python string literal.
pub fn escape_python(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
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

/// Format a string as a Go string literal (backtick or quoted).
pub fn go_string_literal(s: &str) -> String {
    if !s.contains('`') {
        format!("`{s}`")
    } else {
        format!("\"{}\"", escape_go(s))
    }
}

/// Escape a string for embedding in a Go double-quoted string.
pub fn escape_go(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a string for embedding in a Java string literal.
pub fn escape_java(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
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

/// Escape a string for embedding in a POSIX single-quoted shell string literal.
///
/// Wraps the string in single quotes and escapes embedded single quotes as `'\''`.
/// Single-quoted shell strings treat every character literally except `'`, so
/// no other escaping is needed.
pub fn escape_shell(s: &str) -> String {
    s.replace('\'', r"'\''")
}
