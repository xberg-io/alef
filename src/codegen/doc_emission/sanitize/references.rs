use super::{DocTarget, utf8::advance_char};

/// Replace `` [`Type::method()`] `` and `` [`Foo`] `` intra-doc links with
/// backtick-wrapped identifiers, converting `::` to `.`. Also strips an
/// optional explicit-link target `(url)` suffix — e.g.
/// `` [`DataNode`](crate::DataNode) `` → `` `DataNode` `` — so docs that
/// reference items in the originating crate do not leak `crate::` paths
/// into foreign bindings where those paths are unresolvable.
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
                    // Strip an optional explicit-link target `(url)` so
                    // `[`X`](crate::X)` collapses to `` `X` ``.
                    if i < bytes.len() && bytes[i] == b'(' {
                        let mut depth = 1usize;
                        let mut k = i + 1;
                        while k < bytes.len() && depth > 0 {
                            match bytes[k] {
                                b'(' => depth += 1,
                                b')' => depth -= 1,
                                _ => {}
                            }
                            k += 1;
                        }
                        if depth == 0 {
                            i = k;
                        }
                    }
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

/// De-link rustdoc intra-doc references when emitting into a **Rust** binding
/// crate, preserving the referenced text verbatim (including `::` path
/// separators).
///
/// Rust core doc-comments contain intra-doc links such as
/// `` [`Error::LanguageNotFound`] ``, `` [`get_language`] ``, or
/// `` [`Self::ensure_languages`] ``. Those item paths resolve in the core crate
/// but **not** in the generated language binding crates, so `rustdoc`
/// invoked with `-D rustdoc::broken-intra-doc-links`
/// fails with `unresolved link to '...'`. A crate-level `#![allow(...)]` cannot
/// override a command-line `-D`, so the emitted doc text itself must not contain
/// broken intra-doc links.
///
/// This converts every intra-doc link form to a plain inline code span,
/// dropping the link but keeping the backticked text:
///
/// - `` [`X`] `` (shortcut reference)        → `` `X` ``
/// - `` [`X`](crate::X) `` (explicit target) → `` `X` ``
/// - `[X]` (identifier-like shortcut)        → `` `X` ``
///
/// Unlike [`wrap_bare_bracket_references`], the inner text is preserved exactly
/// — `::` is **not** rewritten to `.`, because the destination is Rust where
/// `` `Error::LanguageNotFound` `` is the correct, readable code span.
///
/// Legitimate Markdown links to URLs (`[text](https://…)`, `[text](http://…)`,
/// `[text](#anchor)`) are left untouched: only intra-doc references — those with
/// a non-URL target or the shortcut form — are de-linked.
pub(crate) fn unlink_intradoc_references(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Preserve existing inline code spans verbatim so a backtick pair
            // containing a `[` (e.g. `` `arr[0]` ``) is never misread as a link.
            // Slice the &str (not byte-by-byte) so multibyte UTF-8 is preserved.
            let inner_start = i + 1;
            let mut j = inner_start;
            while j < bytes.len() && bytes[j] != b'`' {
                j += 1;
            }
            out.push('`');
            out.push_str(&s[inner_start..j]);
            if j < bytes.len() {
                out.push('`');
                i = j + 1;
            } else {
                i = j;
            }
            continue;
        }

        if bytes[i] != b'[' {
            i = advance_char(s, &mut out, i);
            continue;
        }

        // Backticked intra-doc link form: [`...`]
        if i + 1 < bytes.len() && bytes[i + 1] == b'`' {
            let inner_start = i + 2;
            if let Some(close) = find_backtick_bracket_close(bytes, inner_start) {
                let inner = &s[inner_start..close];
                out.push('`');
                out.push_str(inner);
                out.push('`');
                i = close + 2;
                i = skip_link_target(bytes, i);
                continue;
            }
            i = advance_char(s, &mut out, i);
            continue;
        }

        // Bare-bracket form: [identifier] / [Type::method] / [method()]
        let search_start = i + 1;
        if let Some(close_pos) = bytes[search_start..].iter().position(|&b| b == b']') {
            let bracket_end = search_start + close_pos;
            let inner = s[search_start..bracket_end].trim();
            // Only de-link when the bracket holds an identifier-like reference
            // and there is no URL target — otherwise leave Markdown alone.
            let target_is_url = bytes.get(bracket_end + 1) == Some(&b'(') && link_target_is_url(bytes, bracket_end + 1);
            if is_identifier_like(inner) && !target_is_url {
                out.push('`');
                out.push_str(inner);
                out.push('`');
                i = bracket_end + 1;
                i = skip_link_target(bytes, i);
                continue;
            }
        }

        i = advance_char(s, &mut out, i);
    }

    out
}

/// Find the byte index of the backtick in a closing `` `] `` starting the scan
/// at `from`. Returns the index of the `` ` `` (so the `]` is at `idx + 1`).
fn find_backtick_bracket_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while j + 1 < bytes.len() {
        if bytes[j] == b'`' && bytes[j + 1] == b']' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// If a `(...)` link target immediately follows position `i`, skip past it and
/// return the new index; otherwise return `i` unchanged. Used to drop the
/// explicit target of a de-linked intra-doc reference.
fn skip_link_target(bytes: &[u8], i: usize) -> usize {
    if i >= bytes.len() || bytes[i] != b'(' {
        return i;
    }
    let mut depth = 1usize;
    let mut k = i + 1;
    while k < bytes.len() && depth > 0 {
        match bytes[k] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        k += 1;
    }
    if depth == 0 { k } else { i }
}

/// Return `true` if the `(...)` target starting at `open` (index of `(`) points
/// at a real URL/anchor that must be preserved as a genuine Markdown link
/// (`https://`, `http://`, `mailto:`, `www.`, or a `#anchor`).
fn link_target_is_url(bytes: &[u8], open: usize) -> bool {
    let start = open + 1;
    let Some(rel_close) = bytes[start..].iter().position(|&b| b == b')') else {
        return false;
    };
    let target = std::str::from_utf8(&bytes[start..start + rel_close])
        .unwrap_or("")
        .trim();
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with("www.")
        || target.starts_with('#')
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
