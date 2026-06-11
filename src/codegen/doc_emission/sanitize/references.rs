use super::{DocTarget, utf8::advance_char};

/// Replace `` [`Type::method()`] `` and `` [`Foo`] `` intra-doc links with
/// backtick-wrapped identifiers, converting `::` to `.`.
pub(super) fn replace_intradoc_links(s: &str, _target: DocTarget) -> String {
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

/// Wrap or unwrap bracketed method/identifier references so rustdoc does not
/// treat them as intra-doc links.
///
/// Converts:
/// - `[identifier]`, `[method()]`, `[Type::method]` → `` `identifier` ``, `` `method()` ``, `` `Type.method` ``
/// - `` [`identifier`] ``, `` [`Type::method`] `` (rustdoc intra-doc-link form) → `` `identifier` ``, `` `Type.method` ``
///
/// The path separator `::` is normalised to `.` for foreign-language compatibility.
/// Both forms are unwrapped because the FFI / foreign-language emitters reference
/// core-crate items that are not in scope, so any intra-doc link form would
/// raise a rustdoc broken-intra-doc-link warning.
pub(crate) fn wrap_bare_bracket_references(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Already-backticked intra-doc link form: [`...`]
            if i + 1 < bytes.len() && bytes[i + 1] == b'`' {
                // Find closing `]
                let inner_start = i + 2;
                let mut j = inner_start;
                let mut found = false;
                while j + 1 < bytes.len() {
                    if bytes[j] == b'`' && bytes[j + 1] == b']' {
                        let inner = &s[inner_start..j];
                        out.push('`');
                        out.push_str(&inner.replace("::", "."));
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
