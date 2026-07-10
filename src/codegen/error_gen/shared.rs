use crate::codegen::conversions::is_tuple_variant;
use crate::core::ir::ErrorVariant;

/// Generate a wildcard match pattern for an error variant.
/// Struct variants use `{ .. }`, tuple variants use `(..)`, unit variants have no suffix.
pub(super) fn error_variant_wildcard_pattern(rust_path: &str, variant: &ErrorVariant) -> String {
    if variant.is_unit {
        format!("{rust_path}::{}", variant.name)
    } else if is_tuple_variant(&variant.fields) {
        format!("{rust_path}::{}(..)", variant.name)
    } else {
        format!("{rust_path}::{} {{ .. }}", variant.name)
    }
}

/// Python builtin exception names that must not be shadowed (A004 compliance).
const PYTHON_BUILTIN_EXCEPTIONS: &[&str] = &[
    "ConnectionError",
    "TimeoutError",
    "PermissionError",
    "FileNotFoundError",
    "ValueError",
    "TypeError",
    "RuntimeError",
    "OSError",
    "IOError",
    "KeyError",
    "IndexError",
    "AttributeError",
    "ImportError",
    "MemoryError",
    "OverflowError",
    "StopIteration",
    "RecursionError",
    "SystemError",
    "ReferenceError",
    "BufferError",
    "EOFError",
    "LookupError",
    "ArithmeticError",
    "AssertionError",
    "BlockingIOError",
    "BrokenPipeError",
    "ChildProcessError",
    "FileExistsError",
    "InterruptedError",
    "IsADirectoryError",
    "NotADirectoryError",
    "ProcessLookupError",
    "UnicodeError",
];

/// Compute a prefix from the error type name by stripping a trailing "Error" suffix.
/// E.g. `"CrawlError"` -> `"Crawl"`, `"MyException"` -> `"MyException"`.
pub(super) fn error_base_prefix(error_name: &str) -> &str {
    error_name.strip_suffix("Error").unwrap_or(error_name)
}

/// Return the Python exception name for a variant, avoiding shadowing of Python builtins.
///
/// 1. Appends `"Error"` suffix if not already present (N818 compliance).
/// 2. If the resulting name shadows a Python builtin, prefixes it with the error type's base
///    name. E.g. for `CrawlError::Connection` -> `ConnectionError` (shadowed) -> `CrawlConnectionError`.
pub fn python_exception_name(variant_name: &str, error_name: &str) -> String {
    let candidate = if variant_name.ends_with("Error") {
        variant_name.to_string()
    } else {
        format!("{}Error", variant_name)
    };

    if PYTHON_BUILTIN_EXCEPTIONS.contains(&candidate.as_str()) {
        let prefix = error_base_prefix(error_name);
        if candidate.starts_with(prefix) {
            candidate
        } else {
            format!("{}{}", prefix, candidate)
        }
    } else {
        candidate
    }
}

/// Generate `pyo3::create_exception!` macros for each error variant plus the base error type.
/// Appends "Error" suffix to variant names that don't already have it (N818 compliance).
/// Prefixes names that would shadow Python builtins (A004 compliance).
pub(super) fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

pub(super) fn to_screaming_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_uppercase());
        } else {
            result.push(c.to_ascii_uppercase());
        }
    }
    result
}

/// Well-known acronyms recognised by the doc/error renderers.
///
/// When emitting human-readable Display strings (e.g. for Go sentinel
/// `errors.New("...")`), variant names like `IoError` must render as
/// "IO error" — not "iO error" (the result of naive `lowercase first
/// character` after `to_snake_case`).
const TECHNICAL_ACRONYMS: &[&str] = &[
    "API", "ASCII", "CPU", "CSS", "CSV", "DNS", "EOF", "FFI", "FTP", "GID", "GPU", "GUI", "HTML", "HTTP", "HTTPS",
    "ID", "IO", "IP", "JSON", "JWT", "LDAP", "MFA", "MIME", "OCR", "OS", "PDF", "PID", "PNG", "QPS", "RAM", "RGB",
    "RPC", "RTF", "SDK", "SLA", "SMTP", "SQL", "SSH", "SSL", "SVG", "TCP", "TLS", "TOML", "TTL", "UDP", "UI", "UID",
    "URI", "URL", "UTF8", "UUID", "VM", "XML", "XMPP", "XSRF", "XSS", "YAML", "ZIP",
];

/// Strip `thiserror`-style `{name}` placeholders from a Display template
/// without leaving stray punctuation.
///
/// Examples:
///
/// - `"OCR error: {message}"`           → `"OCR error"`
/// - `"plugin error in '{plugin_name}'"` → `"plugin error"`
/// - `"timed out after {elapsed_ms}ms (limit: {limit_ms}ms)"` → `"timed out"`
/// - `"I/O error: {0}"`                  → `"I/O error"`
///
/// Used by `variant_display_message` and binding error renderers
/// (Dart, Go, …) so the literal placeholder string never reaches
/// the runtime.
pub fn strip_thiserror_placeholders(template: &str) -> String {
    let mut without_placeholders = String::with_capacity(template.len());
    let mut depth = 0u32;
    for ch in template.chars() {
        match ch {
            '{' => depth = depth.saturating_add(1),
            '}' => depth = depth.saturating_sub(1),
            other if depth == 0 => without_placeholders.push(other),
            _ => {}
        }
    }
    let mut compacted = String::with_capacity(without_placeholders.len());
    let mut last_was_space = false;
    for ch in without_placeholders.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !compacted.is_empty() {
                compacted.push(' ');
            }
            last_was_space = true;
        } else {
            compacted.push(ch);
            last_was_space = false;
        }
    }
    let trimmed = compacted
        .trim()
        .trim_end_matches([':', ',', '-', ';', '(', '\'', '"', ' '])
        .trim();
    let cleaned = trimmed
        .replace("()", "")
        .replace("''", "")
        .replace("\"\"", "")
        .replace("  ", " ");
    cleaned.trim().to_string()
}

/// Convert a PascalCase variant name into a human readable phrase that
/// preserves canonical acronyms.
///
/// Examples:
/// - `"IoError"`           → `"IO error"`
/// - `"OcrError"`          → `"OCR error"`
/// - `"PdfParse"`          → `"PDF parse"`
/// - `"HttpRequestFailed"` → `"HTTP request failed"`
/// - `"Other"`             → `"other"`
pub fn acronym_aware_snake_phrase(variant_name: &str) -> String {
    if variant_name.is_empty() {
        return String::new();
    }
    let bytes = variant_name.as_bytes();
    let mut words: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for i in 1..bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            words.push(&variant_name[start..i]);
            start = i;
        }
    }
    words.push(&variant_name[start..]);

    let mut rendered: Vec<String> = Vec::with_capacity(words.len());
    for word in &words {
        let upper = word.to_ascii_uppercase();
        if TECHNICAL_ACRONYMS.contains(&upper.as_str()) {
            rendered.push(upper);
        } else {
            rendered.push(word.to_ascii_lowercase());
        }
    }
    rendered.join(" ")
}

/// Generate a human-readable message for an error variant.
///
/// Uses the `message_template` if present, otherwise falls back to a
/// space-separated version of the variant name (e.g. "ParseError" -> "parse error").
pub(super) fn variant_display_message(variant: &ErrorVariant) -> String {
    if let Some(tmpl) = &variant.message_template {
        let stripped = strip_thiserror_placeholders(tmpl);
        if stripped.is_empty() {
            return acronym_aware_snake_phrase(&variant.name);
        }
        let mut tokens = stripped.splitn(2, ' ');
        let head = tokens.next().unwrap_or("").to_string();
        let tail = tokens.next().unwrap_or("");
        let head_upper = head.to_ascii_uppercase();
        let head_rendered = if TECHNICAL_ACRONYMS.contains(&head_upper.as_str()) {
            head_upper
        } else {
            let mut chars = head.chars();
            match chars.next() {
                Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                None => head,
            }
        };
        if tail.is_empty() {
            head_rendered
        } else {
            format!("{} {}", head_rendered, tail)
        }
    } else {
        acronym_aware_snake_phrase(&variant.name)
    }
}
