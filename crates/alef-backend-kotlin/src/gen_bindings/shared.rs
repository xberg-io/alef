//! Shared string-conversion utilities for Kotlin code generation.
//!
//! Used by both the JVM and Native backends as well as the MPP backend.

/// Convert a `snake_case` or `kebab-case` name to `PascalCase`.
pub fn to_pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in name.chars() {
        if ch == '-' || ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Convert a `snake_case` or `kebab-case` name to `lowerCamelCase`.
pub fn to_lower_camel(name: &str) -> String {
    let pascal = to_pascal_case(name);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Convert a `PascalCase` name to `SCREAMING_SNAKE_CASE`.
pub fn to_screaming_snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_uppercase());
        }
    }
    out
}

/// Kotlin reserved keywords that must be backtick-escaped when used as
/// identifiers. Hard keywords cannot appear bare in any position; emitting
/// e.g. `val object: String` is a parse error. Wrapping in backticks
/// (`val \`object\`: String`) keeps the wire name intact while satisfying
/// the Kotlin grammar.
const KOTLIN_HARD_KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is", "null",
    "object", "package", "return", "super", "this", "throw", "true", "try", "typealias", "typeof", "val", "var", "when",
    "while",
];

fn escape_kotlin_keyword(name: &str) -> String {
    if KOTLIN_HARD_KEYWORDS.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

/// Field-name resolution for Kotlin record-style data class params. IR
/// positional fields use names like `_0`, `_1` which lowerCamelCase to `0`/`1`
/// — invalid Kotlin identifiers. Map them to `field0`, `field1`, ...
/// Names that collide with Kotlin hard keywords are backtick-escaped so they
/// remain wire-compatible without breaking the grammar.
pub fn kotlin_field_name(raw: &str, idx: usize) -> String {
    let stripped = raw.trim_start_matches('_');
    if stripped.is_empty() || stripped.chars().all(|c| c.is_ascii_digit()) {
        return format!("field{idx}");
    }
    escape_kotlin_keyword(&to_lower_camel(raw))
}
