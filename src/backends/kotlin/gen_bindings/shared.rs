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
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "null",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "when",
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

/// Derive a payload-informed field name for sealed-class tuple variants.
///
/// For tuple variants with a single payload, this function derives smarter names:
/// - If the field name is positional (like `_0`), infer from the type:
///   - Named type `Pdf Metadata` with variant name `Pdf` → strip prefix "Pdf" → `metadata`
///   - Primitive type (String, Int, etc.) → use generic `value`
/// - If the field name is a struct field name (like `reason`), use it directly.
/// - For multiple tuple fields, use generic names: `value0`, `value1`, etc.
///
/// # Arguments
///
/// * `field_name` - Raw field name from IR (`_0`, `_1`, or named like `reason`)
/// * `field_idx` - Position in the variant's field list
/// * `field_type_name` - The simple type name (e.g., `PdfMetadata` from `TypeRef::Named`)
/// * `variant_name` - The variant name (e.g., `Pdf`)
/// * `total_fields` - Total number of fields in the variant
pub fn kotlin_field_name_with_type(
    field_name: &str,
    field_idx: usize,
    field_type_name: Option<&str>,
    variant_name: &str,
    total_fields: usize,
) -> String {
    let stripped = field_name.trim_start_matches('_');

    // If field name is non-positional (not `_N`), use it directly (struct variant).
    if !stripped.is_empty() && !stripped.chars().all(|c| c.is_ascii_digit()) {
        return escape_kotlin_keyword(&to_lower_camel(field_name));
    }

    // For positional fields, derive from type if available and single field.
    if total_fields == 1 {
        if let Some(type_name) = field_type_name {
            // Try to strip variant name as prefix. E.g., `Pdf` variant with `PdfMetadata` type.
            if let Some(remainder) = type_name.strip_prefix(variant_name) {
                // Convert `Metadata` to `metadata`
                let derived = to_lower_camel(remainder);
                if !derived.is_empty() {
                    return escape_kotlin_keyword(&derived);
                }
            }

            // For primitive types (String, Int, Long, etc.), use generic `value`.
            if is_primitive_or_stdlib_type(type_name) {
                return "value".to_string();
            }
        }
    }

    // For multiple fields or when inference fails, use generic names.
    if total_fields > 1 {
        return format!("value{}", field_idx);
    }

    // Fallback: use `value` for single non-inferred field.
    "value".to_string()
}

/// Check if a type name is a primitive or stdlib type (String, Int, Long, etc.).
fn is_primitive_or_stdlib_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "Byte"
            | "Short"
            | "Int"
            | "Long"
            | "Float"
            | "Double"
            | "Boolean"
            | "Unit"
            | "Char"
            | "Any"
            | "Nothing"
    )
}
