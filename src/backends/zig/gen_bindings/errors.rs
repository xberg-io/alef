use crate::core::ir::ErrorDef;

fn zig_error_variant_component(name: &str) -> String {
    crate::codegen::naming::public_host_identifier(
        crate::core::config::Language::Zig,
        crate::codegen::naming::PublicIdentifierKind::Type,
        name,
    )
}

/// Return the literal prefix of a thiserror `#[error("...")]` message template
/// up to the first interpolation placeholder (`{`). Strips any trailing
/// whitespace so the resulting string is suitable for `std.mem.startsWith`
/// comparison against the FFI error context.
///
/// Empty result means the template starts with a placeholder, in which case
/// no prefix-match dispatch is possible for that variant.
fn message_template_prefix(template: &str) -> String {
    let cut = template.find('{').unwrap_or(template.len());
    let mut idx = 0usize;
    let bytes = template.as_bytes();
    while idx < cut {
        if bytes[idx] == b'{' {
            break;
        }
        idx += 1;
    }
    template[..idx].trim_end().to_string()
}

/// Escape a string for inclusion as a Zig string literal. Mirrors the small
/// subset of escapes we may encounter in thiserror message templates.
fn zig_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn emit_error_set(error: &ErrorDef, out: &mut String) {
    if !error.doc.is_empty() {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_doc_block.jinja",
            minijinja::context! {
                error_doc_lines => error.doc.lines().collect::<Vec<_>>(),
            },
        ));
    }
    out.push_str(&crate::backends::zig::template_env::render(
        "error_set_header.jinja",
        minijinja::context! {
            error_name => &error.name,
        },
    ));
    for variant in &error.variants {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => zig_error_variant_component(&variant.name),
            },
        ));
    }
    if !error
        .variants
        .iter()
        .any(|v| zig_error_variant_component(&v.name) == "OutOfMemory")
    {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => "OutOfMemory",
            },
        ));
    }
    out.push_str("};\n");

    emit_from_ffi_msg_fn(error, out);
}

/// Emit `_from_ffi_msg_<ErrorName>(msg: ?[]const u8) <ErrorName>` — a helper
/// that maps an FFI error context string to a typed Zig error variant by
/// prefix-matching against the `#[error("...")]` template literals declared
/// on the Rust source enum.
///
/// Why: previously every FFI failure was reported as the first declared
/// variant, regardless of the actual cause. That made diagnostics actively
/// misleading (e.g. surfacing `error.LanguageNotFound` for unrelated parse,
/// config, or download failures). The per-variant prefix match restores
/// fidelity without requiring per-variant numeric codes from the FFI layer.
///
/// Variants whose `message_template` is missing or starts with a placeholder
/// (no literal prefix) are skipped; the final fallback returns the first
/// declared variant so the function always produces a typed error.
fn emit_from_ffi_msg_fn(error: &ErrorDef, out: &mut String) {
    out.push_str(&format!(
        "/// Map an FFI error message string to a `{}` variant by prefix-matching\n",
        error.name
    ));
    out.push_str("/// against the upstream `#[error(\"...\")]` templates. Falls back to the\n");
    out.push_str("/// first declared variant when no prefix matches.\n");
    out.push_str(&format!(
        "inline fn _from_ffi_msg_{}(msg_opt: ?[]const u8) {} {{\n",
        error.name, error.name
    ));
    out.push_str("    if (msg_opt) |msg| {\n");
    for variant in &error.variants {
        let Some(template) = variant.message_template.as_deref() else {
            continue;
        };
        let prefix = message_template_prefix(template);
        if prefix.is_empty() {
            continue;
        }
        let escaped = zig_string_escape(&prefix);
        let variant_ident = zig_error_variant_component(&variant.name);
        out.push_str(&format!(
            "        if (std.mem.startsWith(u8, msg, \"{escaped}\")) return error.{variant_ident};\n",
        ));
    }
    out.push_str("    }\n");
    out.push_str("    return _first_error(");
    out.push_str(&error.name);
    out.push_str(");\n");
    out.push_str("}\n");
}

/// Map a Rust error_type (e.g. `"anyhow::Error"`, `"SampleCrateError"`) to a
/// Zig error-set identifier. If the path's last segment matches a declared
/// error set, use it; otherwise fall back to the first declared error set
/// (the project's main error type).
pub(crate) fn resolve_zig_error_type(error_type: &str, declared: &[String]) -> String {
    let last = error_type.rsplit("::").next().unwrap_or(error_type);
    if declared.iter().any(|d| d == last) {
        return last.to_string();
    }
    declared.first().cloned().unwrap_or_else(|| "anyerror".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ErrorVariant;

    fn variant(name: &str, template: Option<&str>) -> ErrorVariant {
        ErrorVariant {
            name: name.to_string(),
            message_template: template.map(str::to_string),
            ..ErrorVariant::default()
        }
    }

    #[test]
    fn message_template_prefix_strips_placeholder_tail() {
        assert_eq!(message_template_prefix("Language '{0}' not found"), "Language '");
        assert_eq!(
            message_template_prefix("Dynamic library load error: {0}"),
            "Dynamic library load error:"
        );
        assert_eq!(
            message_template_prefix("Parse failed: parsing returned no tree"),
            "Parse failed: parsing returned no tree"
        );
        assert_eq!(
            message_template_prefix("Checksum mismatch for '{file}': expected {expected}, got {actual}"),
            "Checksum mismatch for '"
        );
    }

    #[test]
    fn message_template_prefix_empty_when_starts_with_placeholder() {
        assert_eq!(message_template_prefix("{0}: something"), "");
        assert_eq!(message_template_prefix(""), "");
    }

    #[test]
    fn zig_string_escape_handles_special_chars() {
        assert_eq!(zig_string_escape("plain"), "plain");
        assert_eq!(zig_string_escape("quote \" mid"), "quote \\\" mid");
        assert_eq!(zig_string_escape("back \\ slash"), "back \\\\ slash");
        assert_eq!(zig_string_escape("tab\tnewline\n"), "tab\\tnewline\\n");
    }

    #[test]
    fn emit_from_ffi_msg_dispatches_each_variant_by_prefix() {
        let error = ErrorDef {
            name: "Error".into(),
            rust_path: "x::Error".into(),
            original_rust_path: String::new(),
            variants: vec![
                variant("LanguageNotFound", Some("Language '{0}' not found")),
                variant("ParseFailed", Some("Parse failed: parsing returned no tree")),
                variant("NoTemplate", None),
                variant("PlaceholderFirst", Some("{0}: oops")),
            ],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut out = String::new();
        emit_from_ffi_msg_fn(&error, &mut out);

        assert!(
            out.contains("inline fn _from_ffi_msg_Error(msg_opt: ?[]const u8) Error {"),
            "missing function signature in:\n{out}"
        );
        assert!(
            out.contains("if (std.mem.startsWith(u8, msg, \"Language '\")) return error.LanguageNotFound;"),
            "missing LanguageNotFound dispatch in:\n{out}"
        );
        assert!(
            out.contains(
                "if (std.mem.startsWith(u8, msg, \"Parse failed: parsing returned no tree\")) return error.ParseFailed;"
            ),
            "missing ParseFailed dispatch in:\n{out}"
        );
        assert!(
            !out.contains("error.NoTemplate"),
            "NoTemplate (no template) should be skipped:\n{out}"
        );
        assert!(
            !out.contains("error.PlaceholderFirst"),
            "PlaceholderFirst (template starts with placeholder) should be skipped:\n{out}"
        );
        assert!(
            out.contains("return _first_error(Error);"),
            "missing fallback to _first_error:\n{out}"
        );
    }

    #[test]
    fn emit_error_set_emits_from_ffi_msg_helper() {
        let error = ErrorDef {
            name: "MyError".into(),
            rust_path: "x::MyError".into(),
            original_rust_path: String::new(),
            variants: vec![variant("Boom", Some("Boom happened: {0}"))],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut out = String::new();
        emit_error_set(&error, &mut out);

        assert!(out.contains("};\n"), "expected closing brace of error set:\n{out}");
        assert!(
            out.contains("inline fn _from_ffi_msg_MyError(msg_opt: ?[]const u8) MyError {"),
            "expected matcher emission after error set:\n{out}"
        );
        assert!(
            out.contains("if (std.mem.startsWith(u8, msg, \"Boom happened:\")) return error.Boom;"),
            "expected Boom dispatch (with whitespace-trimmed prefix):\n{out}"
        );
    }
}
