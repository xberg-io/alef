//! Ruby (Magnus) binding generator backend for alef.

use std::borrow::Cow;

use crate::core::ir::FunctionDef;

pub(crate) mod gen_bindings;
mod gen_stubs;
pub(crate) mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::MagnusBackend;

pub(crate) fn ruby_public_function_name(func: &FunctionDef) -> &str {
    rust_path_leaf(&func.original_rust_path).unwrap_or(func.name.as_str())
}

pub(crate) fn ruby_native_function_name(func: &FunctionDef) -> Cow<'_, str> {
    if !func.is_async {
        return Cow::Borrowed(func.name.as_str());
    }

    if func.name.ends_with("_async") {
        Cow::Borrowed(func.name.as_str())
    } else {
        Cow::Owned(format!("{}_async", func.name))
    }
}

fn rust_path_leaf(path: &str) -> Option<&str> {
    let leaf = path.rsplit("::").next()?;
    let name = leaf.strip_prefix("r#").unwrap_or(leaf);
    if name.is_empty() { None } else { Some(name) }
}

/// Render `wire_value` as a Ruby symbol literal, quoting it only when required.
///
/// A bare Ruby symbol (`:foo`) may only contain identifier characters — a leading letter or
/// underscore followed by letters, digits, or underscores — with an optional single trailing
/// `?`, `!`, or `=` (Ruby's method-name suffixes, e.g. `:empty?`, `:freeze!`, `:foo=`; verified
/// bare-parseable by both `ruby` and the `rbs` gem). Anything else — a hyphen, a dot, a space, a
/// leading digit, or an empty string — is not a valid bare symbol and must be quoted
/// (`:"fine-tune"`). Quoting only changes how the literal is *written*; the symbol's value is
/// still `wire_value` unchanged, so callers must never transform `wire_value` itself (e.g.
/// hyphens must not become underscores) — that would silently corrupt the wire contract this
/// symbol represents. ~keep
///
/// Used for every generated Ruby/RBS symbol literal derived from a wire value (serde
/// `rename`/`rename_all`), so a value like `fine-tune` renders as `:"fine-tune"` instead of the
/// syntactically invalid bare `:fine-tune`.
pub(crate) fn ruby_symbol_literal(wire_value: &str) -> String {
    if is_bare_symbol_safe(wire_value) {
        format!(":{wire_value}")
    } else {
        let escaped = wire_value.replace('\\', "\\\\").replace('"', "\\\"");
        format!(":\"{escaped}\"")
    }
}

fn is_bare_symbol_safe(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    let body = chars.as_str();
    let core = body
        .strip_suffix('?')
        .or_else(|| body.strip_suffix('!'))
        .or_else(|| body.strip_suffix('='))
        .unwrap_or(body);

    core.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod ruby_symbol_literal_tests {
    use super::*;

    #[test]
    fn bare_safe_identifiers_stay_unquoted() {
        let cases = [
            ("plain_snake", ":plain_snake"),
            ("KeyValue", ":KeyValue"),
            ("_leading_underscore", ":_leading_underscore"),
            ("a1", ":a1"),
            ("foo?", ":foo?"),
            ("foo!", ":foo!"),
            ("foo=", ":foo="),
        ];
        for (input, expected) in cases {
            assert_eq!(ruby_symbol_literal(input), expected, "input: {input}");
        }
    }

    #[test]
    fn non_identifier_values_are_quoted_with_wire_value_preserved() {
        let cases = [
            ("fine-tune", ":\"fine-tune\""),
            ("a.b", ":\"a.b\""),
            ("2fast", ":\"2fast\""),
            ("with space", ":\"with space\""),
            ("", ":\"\""),
        ];
        for (input, expected) in cases {
            assert_eq!(ruby_symbol_literal(input), expected, "input: {input}");
        }
    }

    #[test]
    fn quoting_never_changes_the_underlying_wire_value() {
        for input in ["fine-tune", "a.b", "2fast", "with space", "", "plain_snake", "foo?"] {
            let literal = ruby_symbol_literal(input);
            let unquoted = literal.trim_start_matches(':').trim_matches('"');
            assert_eq!(unquoted, input, "wire value must survive quoting for: {input}");
        }
    }
}
