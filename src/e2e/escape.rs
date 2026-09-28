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

/// Returns `true` if the string must use a Rust quoted literal rather than a raw one.
///
/// A raw literal reproduces its content byte for byte, real newlines included, so a value
/// carrying whitespace immediately before a newline lands as *trailing whitespace on a physical
/// source line*. Every Rust formatter strips that, silently rewriting the literal to a
/// different value than the fixture asked for -- and a generated assertion then compares
/// against something nobody wrote. A Markdown two-space hard break (`"a  \nb"`) is exactly
/// this shape, so this is reachable from any fixture asserting one. Same class of problem as
/// [`go_needs_quoted`], and handled the same way. ~keep
fn rust_needs_quoted(s: &str) -> bool {
    s.contains(" \n") || s.contains("\t\n")
}

/// Format a string as a Rust string literal.
///
/// Prefers a raw literal (`r#"..."#`) for readability, but falls back to a quoted,
/// escaped literal when a raw one would not survive formatting -- see
/// [`rust_needs_quoted`].
pub fn rust_raw_string(s: &str) -> String {
    if rust_needs_quoted(s) {
        return format!("\"{}\"", escape_rust(s));
    }
    let hashes = raw_string_hashes(s);
    let h: String = "#".repeat(hashes);
    format!("r{h}\"{s}\"{h}")
}

/// Escape a string for embedding in a JavaScript/TypeScript double-quoted string literal.
///
/// `$` does not need escaping in double-quoted strings (only in template literals).
/// Escaping it would produce `\$` which formatters flag as an unnecessary escape.
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
///
/// A raw literal also reproduces its content byte for byte, real newlines included, so a
/// value carrying whitespace immediately before a newline lands as *trailing whitespace on a
/// physical source line*. gofmt -- and any trailing-whitespace tidy pass -- strips that,
/// silently rewriting the literal to a different value than the fixture asked for. A Markdown
/// two-space hard break (`"a  \nb"`) is exactly this shape. Same class of problem as
/// [`rust_needs_quoted`], and handled the same way. ~keep
fn go_needs_quoted(s: &str) -> bool {
    s.contains('`') || s.bytes().any(|b| b == 0 || b == b'\r') || s.contains(" \n") || s.contains("\t\n")
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
///
/// Delegates to [`crate::codegen::java_literal::escape_java_string_literal`], the single place
/// Java literal escaping is defined, so generated e2e sources and generated bindings cannot
/// disagree about what a hostile value looks like in Java source.
pub fn escape_java(s: &str) -> String {
    crate::codegen::java_literal::escape_java_string_literal(s)
}

/// Escape a string for embedding in a Swift double-quoted string literal.
///
/// Deliberately *not* the Java escaper: Swift has no octal escape at all and spells a Unicode
/// escape `\u{XXXX}`, so Java's `\uXXXX` and `\NNN` forms are compile errors in Swift source.
/// Swift files are read as UTF-8, so non-ASCII needs no escape at all. ~keep
pub fn escape_swift(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for character in s.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            other if other.is_control() => out.push_str(&format!("\\u{{{:X}}}", other as u32)),
            other => out.push(other),
        }
    }
    out
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

/// Escape a string for embedding in a single-quoted PHP string literal.
/// Single-quoted PHP strings only interpret `\\` and `\'`.
pub fn escape_php_single(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Build a PHP single-quoted PCRE pattern literal (`'/pattern/'`) matching `value` literally.
///
/// Escapes PCRE metacharacters (`. ^ $ * + ? ( ) [ ] { } |  \`) plus the `/`
/// delimiter, then escapes the resulting pattern text for a PHP single-quoted
/// string: every backslash introduced by the PCRE escaping must be doubled
/// (single-quoted PHP only collapses `\\` to one backslash — everything else
/// passes through unmodified) before quotes are escaped, or the pattern
/// arrives at `preg_match` with half its escapes stripped.
///
/// A newline, carriage return or tab in `value` becomes its PCRE escape rather than the raw
/// byte: a raw newline would split the generated PHP across physical source lines, and any
/// space or tab standing in front of it then reads as trailing whitespace that a formatter is
/// free to strip -- silently changing what the pattern matches. This is the same defect class
/// alef-task #557 found in the Go emitter, arriving through a delimited literal instead of a
/// raw one. ~keep
pub fn php_pcre_literal(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\n' => pattern.push_str("\\n"),
            '\r' => pattern.push_str("\\r"),
            '\t' => pattern.push_str("\\t"),
            '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' | '/' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    let delimited = format!("/{pattern}/");
    let php_escaped = delimited.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{php_escaped}'")
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

/// Build a Ruby regex literal (`/pattern/`) matching `value` literally.
///
/// Escapes Ruby regex metacharacters (`. ^ $ * + ? ( ) [ ] { } |  \`) plus the
/// `/` delimiter itself, so any characters in `value` — including regex
/// metacharacters a fixture author didn't intend as regex syntax — are matched
/// as plain text rather than interpreted as pattern syntax.
///
/// `#` is escaped too, because a Ruby regex literal interpolates `#{...}` exactly as a
/// double-quoted string does -- an unescaped `#{` in `value` would not merely mis-match, it
/// would fail to parse. A newline, carriage return or tab becomes its regex escape rather than
/// the raw byte, for the reason given on [`php_pcre_literal`]: a raw newline splits the literal
/// across physical source lines and leaves any preceding space or tab as strippable trailing
/// whitespace. ~keep
pub fn ruby_regex_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' | '/' | '#' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    format!("/{escaped}/")
}

/// Consume a run of ASCII alphanumeric or `_` characters from `chars`, stopping (without
/// consuming) at the first character that doesn't match.
fn take_ident_chars(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut ident = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' {
            ident.push(chars.next().unwrap());
        } else {
            break;
        }
    }
    ident
}

/// Convert a `{param}` template string to a Ruby double-quoted string with `#{param}` interpolation.
///
/// `{key}` placeholders are converted to `#{key}`. All other characters are escaped for
/// Ruby double-quoted strings. The returned value does NOT include the surrounding `"` quotes.
pub fn ruby_template_to_interpolation(template: &str) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                // Check if this is a {identifier} placeholder
                let is_ident_start = chars.peek().is_some_and(|&c| c.is_ascii_alphabetic() || c == '_');
                if is_ident_start {
                    // Collect the identifier
                    let ident = take_ident_chars(&mut chars);
                    if chars.peek() == Some(&'}') {
                        chars.next(); // consume '}'
                        out.push('#');
                        out.push('{');
                        out.push_str(&ident);
                        out.push('}');
                    } else {
                        // Not a valid {ident} placeholder — emit literally
                        out.push('{');
                        out.push_str(&ident);
                    }
                } else {
                    out.push('{');
                }
            }
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '#' => out.push_str("\\#"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Convert a `{param}` template string to an R expression using `paste0()`.
///
/// `{key}` placeholders are converted to variable references in a `paste0(...)` call.
/// Literal text segments are R string literals. If the template has no placeholders,
/// a plain R string literal is returned. If the template is a single bare placeholder
/// like `{text}`, just the variable name is returned.
///
/// Examples:
/// - `"[BTN:{text}]"` → `paste0("[BTN:", text, "]")`
/// - `"--- {text} ---"` → `paste0("--- ", text, " ---")`
/// - `"{text}"` → `text`
/// - `"static"` → `"static"`
pub fn r_template_to_paste0(template: &str) -> String {
    enum Seg {
        Lit(String),
        Param(String),
    }
    fn flush_lit(lit: &mut String, segments: &mut Vec<Seg>) {
        if !lit.is_empty() {
            segments.push(Seg::Lit(lit.clone()));
            lit.clear();
        }
    }
    let mut segments: Vec<Seg> = Vec::new();
    let mut lit = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let is_ident_start = chars.peek().is_some_and(|&c| c.is_ascii_alphabetic() || c == '_');
            if is_ident_start {
                let ident = take_ident_chars(&mut chars);
                if chars.peek() == Some(&'}') {
                    chars.next();
                    flush_lit(&mut lit, &mut segments);
                    segments.push(Seg::Param(ident));
                    continue;
                }
                lit.push('{');
                lit.push_str(&ident);
            } else {
                lit.push('{');
            }
        } else {
            lit.push(ch);
        }
    }
    if !lit.is_empty() {
        segments.push(Seg::Lit(lit));
    }
    match segments.as_slice() {
        [] => r#""""#.to_string(),
        [Seg::Param(p)] => p.clone(),
        segs => {
            let args: Vec<String> = segs
                .iter()
                .map(|s| match s {
                    Seg::Lit(l) => format!("\"{}\"", escape_r(l)),
                    Seg::Param(p) => p.clone(),
                })
                .collect();
            format!("paste0({})", args.join(", "))
        }
    }
}

/// Returns true if the string needs double quotes (contains control characters
/// that require escape sequences only available in double-quoted strings, or
/// apostrophes which are only properly escaped in double-quoted strings in Ruby).
pub fn ruby_needs_double_quotes(s: &str) -> bool {
    s.contains('\n') || s.contains('\r') || s.contains('\t') || s.contains('\0') || s.contains('\'')
}

/// Format a string as a Ruby literal, preferring single quotes but using double
/// quotes when the string contains apostrophes, control characters, or other
/// special chars that require escaping only available in double-quoted strings.
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

/// Build a quoted R regex-pattern string literal (`"pattern"`) matching `value` literally.
///
/// Escapes POSIX/PCRE regex metacharacters (`. ^ $ * + ? ( ) [ ] { } |  \`), then
/// runs the result through [`escape_r`] for double-quoted R string embedding.
/// The backslashes introduced by the regex escaping must themselves be doubled —
/// R double-quoted strings only recognize `\\` and `\"` — or `grepl()`/`regexp=`
/// receive a pattern with half its escapes stripped.
pub fn r_regex_literal(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
        ) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    format!("\"{}\"", escape_r(&pattern))
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
/// Replaces non-alphanumeric characters with underscores, strips leading digits,
/// and strips any underscores left dangling after the digit prefix so that
/// generators which prefix the result (e.g. `test_<ident>`) don't produce
/// double-underscore names like `test__foo` from fixture ids like `24_foo`.
pub fn sanitize_ident(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    // Strip leading digits.
    let after_digits = result.trim_start_matches(|c: char| c.is_ascii_digit());
    // If we stripped any digits, also strip the underscores left behind so
    // callers like `format!("test_{name}")` don't yield `test__foo`.
    let trimmed = if after_digits.len() < result.len() {
        after_digits.trim_start_matches('_')
    } else {
        after_digits
    };
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

    #[test]
    fn rust_raw_string_falls_back_to_quoted_when_whitespace_precedes_a_newline() {
        // A Markdown two-space hard break: a raw literal would put the two spaces at the end
        // of a physical source line, where the formatter strips them.
        let out = rust_raw_string("X `A`  \n`B`\n");
        assert_eq!(out, "\"X `A`  \\n`B`\\n\"");
        assert!(!out.contains(" \n"), "no real trailing whitespace may survive: {out}");
    }

    #[test]
    fn rust_raw_string_keeps_the_raw_form_when_no_whitespace_precedes_a_newline() {
        assert_eq!(rust_raw_string("a\nb"), "r#\"a\nb\"#");
        // Trailing whitespace at the very END of the value is safe -- nothing follows it on
        // the source line, so the formatter has nothing to strip.
        assert_eq!(rust_raw_string("abc "), "r#\"abc \"#");
    }
    use super::*;

    #[test]
    fn escape_php_single_escapes_backslash_and_quote_only() {
        assert_eq!(escape_php_single("it's a \\test"), "it\\'s a \\\\test");
    }

    #[test]
    fn php_pcre_literal_wraps_plain_value_in_slashes() {
        assert_eq!(php_pcre_literal("BadRequest"), "'/BadRequest/'");
    }

    /// The escaper this delegates to escapes control characters; the five-`replace` chain it
    /// replaced left everything outside `\n`/`\r`/`\t` raw, so a raw ESC proves the delegation.
    #[test]
    fn escape_java_delegates_to_the_central_java_literal_escaper() {
        assert_eq!(escape_java("escape\u{1b}[0m"), "escape\\033[0m");
        assert_eq!(escape_java("quote\"and\\slash"), "quote\\\"and\\\\slash");
    }

    /// Swift has no octal escape and spells Unicode escapes `\u{XXXX}`, so it must not inherit
    /// Java's forms — this aliased the Java escaper until the Java one learned them.
    #[test]
    fn escape_swift_uses_swift_escape_syntax_not_javas() {
        assert_eq!(escape_swift("escape\u{1b}[0m"), "escape\\u{1B}[0m");
        assert_eq!(escape_swift("caf\u{e9}"), "caf\u{e9}");
        assert_eq!(escape_swift("nul\0byte"), "nul\\0byte");
    }

    #[test]
    fn php_pcre_literal_escapes_metacharacters_with_doubled_backslashes() {
        assert_eq!(php_pcre_literal("field.name[0]"), "'/field\\\\.name\\\\[0\\\\]/'");
        assert_eq!(php_pcre_literal("1/2"), "'/1\\\\/2/'");
    }

    #[test]
    fn php_pcre_literal_escapes_single_quotes() {
        assert_eq!(php_pcre_literal("user's"), "'/user\\'s/'");
    }

    #[test]
    fn r_regex_literal_wraps_plain_value_in_double_quotes() {
        assert_eq!(r_regex_literal("BadRequest"), "\"BadRequest\"");
    }

    #[test]
    fn r_regex_literal_escapes_metacharacters_with_doubled_backslashes() {
        assert_eq!(r_regex_literal("field.name[0]"), "\"field\\\\.name\\\\[0\\\\]\"");
    }

    #[test]
    fn r_regex_literal_escapes_double_quotes() {
        assert_eq!(r_regex_literal("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn ruby_regex_literal_wraps_plain_value_in_slashes() {
        assert_eq!(ruby_regex_literal("BadRequest"), "/BadRequest/");
    }

    #[test]
    fn ruby_regex_literal_escapes_metacharacters() {
        assert_eq!(ruby_regex_literal("field.name[0]"), "/field\\.name\\[0\\]/");
        assert_eq!(ruby_regex_literal("a+b*c?"), "/a\\+b\\*c\\?/");
        assert_eq!(ruby_regex_literal("1/2"), "/1\\/2/");
    }

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

    /// A raw (backtick) Go literal reproduces its content byte for byte, real newlines
    /// included, so whitespace immediately before a newline lands as trailing whitespace on
    /// a physical source line -- which gofmt (and any trailing-whitespace tidy pass) strips.
    /// A Markdown two-space hard break is exactly this shape. Same class of problem as
    /// [`rust_needs_quoted`], and must be handled the same way: fall back to a quoted literal.
    #[test]
    fn go_string_literal_falls_back_to_quoted_when_whitespace_precedes_a_newline() {
        let s = "[Alpha  \n](https://example.com)Beta";
        let lit = go_string_literal(s);
        assert!(
            lit.starts_with('"'),
            "expected double-quoted form when a line ends in whitespace, got: {lit:?}"
        );
        assert_eq!(lit, "\"[Alpha  \\n](https://example.com)Beta\"");
        assert!(!lit.contains(" \n"), "no real trailing whitespace may survive: {lit}");
    }

    /// Regression net for the whole class alef-task #557 found in the Go emitter: any
    /// backend's string-literal function must never let a value's trailing whitespace before
    /// a newline survive as trailing whitespace on a physical source line. A file-level
    /// whitespace-trim gate was tried at the generated-file writer and reverted -- a real
    /// downstream regeneration run showed 409 of 409 trailing-whitespace hits there were
    /// ordinary layout (doc-comment continuations, parameter-list continuations), not fixture
    /// values, because a file-level check cannot distinguish the two. Only the emitter
    /// choosing a literal form can, which is exactly what `go_needs_quoted` does -- so the
    /// guard belongs here, at the literal-rendering functions themselves, one entry per
    /// backend that can reach this shape. A backend not listed here has no raw/verbatim
    /// literal form yet (every other `escape_*` in this module unconditionally escapes `\n`),
    /// so it cannot fail this test today; a future backend that adds one will, the moment its
    /// literal function is added to this table. ~keep
    #[test]
    fn every_backend_string_literal_function_survives_a_markdown_hard_line_break() {
        // The exact shape that broke Go: a value with two spaces immediately before a real
        // newline, embedded in surrounding text -- a Markdown hard line break.
        const VALUE: &str = "[Alpha  \n](https://example.com)Beta";

        // One entry per backend's primary "render this exact fixture value as a string
        // literal" function -- not every helper in this module (e.g. `*_regex_literal`,
        // `*_template_to_*`, `sanitize_*` build something other than a value literal, and are
        // out of scope for this particular guard).
        type LiteralFn = fn(&str) -> String;
        let literal_functions: &[(&str, LiteralFn)] = &[
            ("go", go_string_literal),
            ("rust", rust_raw_string),
            ("python", escape_python),
            ("javascript/typescript", escape_js),
            ("java", escape_java),
            ("swift", escape_swift),
            ("kotlin", escape_kotlin),
            ("csharp", escape_csharp),
            ("php", escape_php),
            ("ruby (double-quoted)", escape_ruby),
            ("ruby (single/double chooser)", ruby_string_literal),
            ("elixir", escape_elixir),
            ("r", escape_r),
            ("c", escape_c),
            ("gleam", escape_gleam),
            ("zig", escape_zig),
        ];

        for (backend, literal_fn) in literal_functions {
            let rendered = literal_fn(VALUE);
            for line in rendered.lines() {
                assert_eq!(
                    line,
                    line.trim_end(),
                    "{backend}: string-literal function let \"{VALUE:?}\" render with trailing \
                     whitespace on a physical source line -- rendered: {rendered:?}"
                );
            }
        }
    }

    /// The companion to the string-literal table above, for the helpers it explicitly declares
    /// out of scope. A regex literal is delimited source too, so a value carrying a Markdown
    /// hard line break leaks the same way: `/Alpha  \n/` puts two spaces at the end of a
    /// physical line, and whatever strips trailing whitespace next quietly turns the pattern
    /// into one that no longer matches the value it was built from. ~keep
    #[test]
    fn every_regex_literal_function_survives_a_markdown_hard_line_break() {
        const VALUE: &str = "[Alpha  \n](https://example.com)Beta";

        type LiteralFn = fn(&str) -> String;
        let regex_functions: &[(&str, LiteralFn)] = &[
            ("php", php_pcre_literal),
            ("ruby", ruby_regex_literal),
            ("r", r_regex_literal),
        ];

        for (backend, regex_fn) in regex_functions {
            let rendered = regex_fn(VALUE);
            assert!(
                !rendered.contains('\n'),
                "{backend}: regex literal for {VALUE:?} spans physical source lines: {rendered:?}"
            );
            for line in rendered.lines() {
                assert_eq!(
                    line,
                    line.trim_end(),
                    "{backend}: regex literal for {VALUE:?} rendered with trailing whitespace on a \
                     physical source line -- rendered: {rendered:?}"
                );
            }
        }
    }

    /// The two spaces are kept verbatim -- they are what the pattern must match -- while only
    /// the newline becomes an escape. PCRE sees `\n` because single-quoted PHP collapses the
    /// doubled backslash to one.
    #[test]
    fn php_pcre_literal_escapes_a_newline_rather_than_emitting_it() {
        assert_eq!(php_pcre_literal("Alpha  \nBeta"), "'/Alpha  \\\\nBeta/'");
        assert_eq!(php_pcre_literal("a\tb\r"), "'/a\\\\tb\\\\r/'");
    }

    #[test]
    fn ruby_regex_literal_escapes_a_newline_rather_than_emitting_it() {
        assert_eq!(ruby_regex_literal("Alpha  \nBeta"), "/Alpha  \\nBeta/");
        assert_eq!(ruby_regex_literal("a\tb\r"), "/a\\tb\\r/");
    }

    /// An unescaped `#{` in a Ruby regex literal interpolates: the generated spec would not
    /// parse at all, rather than merely match the wrong thing.
    #[test]
    fn ruby_regex_literal_escapes_the_interpolation_sigil() {
        assert_eq!(ruby_regex_literal("a#{b}c"), "/a\\#\\{b\\}c/");
        assert_eq!(ruby_regex_literal("issue #12"), "/issue \\#12/");
    }

    /// Fixture ids with a numeric prefix (`24_cookie_samesite_strict`) must not
    /// produce names like `_cookie_samesite_strict` that, when prefixed with
    /// `test_`, yield the invalid-looking `test__cookie_samesite_strict`.
    #[test]
    fn sanitize_ident_strips_leading_underscore_after_digit_prefix() {
        assert_eq!(sanitize_ident("24_cookie_samesite_strict"), "cookie_samesite_strict");
        assert_eq!(sanitize_ident("01_foo"), "foo");
        assert_eq!(sanitize_ident("9bar"), "bar");
    }

    /// Genuine leading underscores (no preceding digits) are preserved so
    /// fixture ids that intentionally start with `_` round-trip unchanged.
    #[test]
    fn sanitize_ident_preserves_leading_underscore_without_digits() {
        assert_eq!(sanitize_ident("_foo"), "_foo");
        assert_eq!(sanitize_ident("__bar"), "__bar");
    }

    /// Strings consisting only of digits (and the underscores left behind)
    /// collapse to the placeholder `_` since the result would otherwise be empty.
    #[test]
    fn sanitize_ident_all_digits_returns_underscore_placeholder() {
        assert_eq!(sanitize_ident("123"), "_");
        assert_eq!(sanitize_ident("12_"), "_");
    }

    /// Non-leading digits and underscores are untouched.
    #[test]
    fn sanitize_ident_preserves_interior_chars() {
        assert_eq!(sanitize_ident("foo_42_bar"), "foo_42_bar");
        assert_eq!(sanitize_ident("foo.bar-baz"), "foo_bar_baz");
    }

    /// Ruby strings with apostrophes must use double quotes to properly escape them.
    #[test]
    fn ruby_string_with_apostrophe_uses_double_quotes() {
        let s = "Tests JWT rejection when token is provided without 'Bearer ' prefix.";
        let result = ruby_string_literal(s);
        assert!(
            result.starts_with('"') && result.ends_with('"'),
            "String with apostrophe must use double quotes, got: {}",
            result
        );
        assert!(
            result.contains("Bearer"),
            "String content must be preserved, got: {}",
            result
        );
    }
}
