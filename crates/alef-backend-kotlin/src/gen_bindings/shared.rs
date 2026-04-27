//! Shared string-conversion utilities for Kotlin code generation.
//!
//! Used by both the JVM and Native backends as well as the MPP backend.

/// Convert a `snake_case` or `kebab-case` name to `PascalCase`.
pub(crate) fn to_pascal_case(name: &str) -> String {
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
pub(crate) fn to_lower_camel(name: &str) -> String {
    let pascal = to_pascal_case(name);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Convert a `PascalCase` name to `SCREAMING_SNAKE_CASE`.
pub(crate) fn to_screaming_snake(name: &str) -> String {
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

/// Field-name resolution for Kotlin record-style data class params. IR
/// positional fields use names like `_0`, `_1` which lowerCamelCase to `0`/`1`
/// — invalid Kotlin identifiers. Map them to `field0`, `field1`, ...
pub(crate) fn kotlin_field_name(raw: &str, idx: usize) -> String {
    let stripped = raw.trim_start_matches('_');
    if stripped.is_empty() || stripped.chars().all(|c| c.is_ascii_digit()) {
        return format!("field{idx}");
    }
    to_lower_camel(raw)
}
