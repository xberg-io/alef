use crate::core::ir::{DefaultValue, FieldDef, TypeRef};
use minijinja::context;

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// Go structs require named fields, so these must be skipped.
pub(in crate::backends::go::gen_bindings) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Returns true if a non-optional struct field should be emitted as a pointer type with
/// `omitempty` in a struct that has `has_default: true`.
///
/// This is necessary when the Go zero value for a field differs from the Rust `Default` value.
/// Without pointer+omitempty, unset fields serialize as their Go zero value (0, false, ""), which
/// the Rust FFI layer may reject or misinterpret (e.g., `request_timeout: 0` is invalid).
///
/// Cases that require pointer+omitempty:
/// - `TypeRef::Duration` — Duration zero is always invalid; real defaults are non-zero (e.g., 30s)
/// - `BoolLiteral(true)` — Rust default is `true`, Go zero is `false`
/// - `IntLiteral(n)` where n != 0 — Rust default is n, Go zero is 0
/// - `FloatLiteral(f)` where f != 0.0 — Rust default is f, Go zero is 0.0
/// - `StringLiteral(s)` where !s.is_empty() — Rust default is s, Go zero is ""
/// - `EnumVariant(_)` — Rust default is a specific variant, Go zero is ""
pub(in crate::backends::go::gen_bindings) fn needs_omitempty_pointer(field: &FieldDef) -> bool {
    // Duration fields always need pointer+omitempty: zero duration is invalid in Rust
    if matches!(field.ty, TypeRef::Duration) {
        return true;
    }
    match &field.typed_default {
        Some(DefaultValue::BoolLiteral(true)) => true,
        Some(DefaultValue::IntLiteral(n)) if *n != 0 => true,
        Some(DefaultValue::FloatLiteral(f)) if *f != 0.0 => true,
        Some(DefaultValue::StringLiteral(s)) if !s.is_empty() => true,
        Some(DefaultValue::EnumVariant(_)) => true,
        _ => false,
    }
}

/// Generate the package-level `unmarshalBytes` helper.
///
/// Emitted exactly once per generated `binding.go`. Methods and functions
/// returning `TypeRef::Bytes` reference this helper by name. The helper takes
/// a `*C.uint8_t` aliasing pointer (typically returned by an FFI accessor
/// that hands out a borrowed view into a parent handle's buffer) and produces
/// a freshly-allocated `*[]byte` copy. The caller MUST keep the parent handle
/// alive across the helper call; the returned slice is detached.
///
/// The helper does not free the input pointer because the FFI surface aliases
/// internal storage; freeing here would corrupt the parent handle.
pub(in crate::backends::go::gen_bindings) fn gen_unmarshal_bytes_helper() -> String {
    crate::backends::go::template_env::render("unmarshal_bytes_helper.jinja", minijinja::Value::default())
}

/// Generate the package-level `Ptr` generic helper.
///
/// Emitted exactly once per generated `binding.go`. Used by data DTOs to construct
/// pointers for optional fields without the functional-options pattern boilerplate.
/// Typical usage: `&MyStruct{Field: Ptr("value"), OtherField: Ptr(42)}`
pub(in crate::backends::go::gen_bindings) fn gen_ptr_helper() -> String {
    crate::backends::go::template_env::render("ptr_helper.jinja", minijinja::Value::default())
}

/// Generate the lastError() helper function.
pub(in crate::backends::go::gen_bindings) fn gen_last_error_helper(ffi_prefix: &str) -> String {
    // Note: ctx is a borrowed pointer into thread-local storage, NOT a heap allocation.
    // Do NOT call free_string on it — that causes a double-free crash on the next FFI call.
    crate::backends::go::template_env::render(
        "last_error_helper.jinja",
        context! {
            ffi_prefix => ffi_prefix,
        },
    )
}

/// Emit Go-convention doc comment lines for an exported symbol into `out`.
///
/// Go's revive linter requires that the first line of a doc comment starts with
/// the exported name (with an optional leading article). This function rewrites
/// verbatim docs that begin with an article ("A ", "An ", "The ") by prepending
/// the symbol name, and falls back to a generated comment when no doc is present.
///
/// Used for both types and methods/functions: the symbol name appears at the
/// start of the comment so `go doc`, `godoc`, and `pkg.go.dev` recognise the
/// item description.
///
/// Rustdoc sections are translated into Godoc-friendly prose:
/// - `# Arguments` → `// Arguments:` followed by `//   - name: desc` bullets
/// - `# Returns`   → `// Returns ...`
/// - `# Errors`    → `// Errors are returned when ...` (verbatim body if it
///   already reads naturally)
/// - `# Example` / `# Examples` → `//\n// Example:\n//   <indented code>`
///
/// Examples:
/// - `"A chat message."` on `Message` → `"// Message is a chat message."`
/// - `"Message represents…"` on `Message` → `"// Message represents…"` (unchanged)
/// - empty doc on `Message` → `"// Message <fallback>."`
pub(in crate::backends::go::gen_bindings) fn emit_type_doc(
    out: &mut String,
    type_name: &str,
    doc: &str,
    fallback: &str,
) {
    if doc.is_empty() {
        out.push_str(&crate::backends::go::template_env::render(
            "type_doc_header.jinja",
            context! {
                type_name => type_name,
                doc => fallback,
            },
        ));
        return;
    }
    let sections = crate::codegen::doc_emission::parse_rustdoc_sections(doc);
    let summary = sections.summary.trim();
    if summary.is_empty() {
        // No summary prose, only sections — synthesise a header line then
        // append sections so the symbol still has a name-prefixed doc line.
        out.push_str(&crate::backends::go::template_env::render(
            "type_doc_header.jinja",
            context! {
                type_name => type_name,
                doc => fallback,
            },
        ));
    } else {
        emit_godoc_summary(out, type_name, summary);
    }
    emit_godoc_sections(out, &sections);
}

/// Emit the summary prose with the symbol name prefixed onto the first line.
///
/// Subsequent lines of the summary are emitted as plain `// <line>` continuation
/// comments. Article-stripping is applied only to the first sentence so
/// "A foo" becomes "Name is a foo".
fn emit_godoc_summary(out: &mut String, symbol_name: &str, summary: &str) {
    let mut lines = summary.lines();
    let first = lines.next().unwrap_or("").trim();
    // The template prepends `// {{ symbol_name }} `, so strip a leading
    // occurrence of `{symbol_name}` (plus an optional separator space) from
    // the rendered body — otherwise summaries that already start with the
    // exported name produce `// Name Name does ...` double-prefixes.
    let body = if let Some(rest) = first.strip_prefix(symbol_name) {
        rest.trim_start().to_string()
    } else {
        let rest = first
            .strip_prefix("A ")
            .or_else(|| first.strip_prefix("An "))
            .or_else(|| first.strip_prefix("The "))
            .unwrap_or(first);
        if rest.is_empty() {
            String::new()
        } else {
            let mut chars = rest.chars();
            match chars.next() {
                Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    };
    out.push_str(&crate::backends::go::template_env::render(
        "type_doc_header.jinja",
        context! {
            type_name => symbol_name,
            doc => &body,
        },
    ));
    for line in lines {
        out.push_str(&crate::backends::go::template_env::render(
            "go_doc_comment_line.jinja",
            context! { line => line.trim() },
        ));
    }
}

/// Push a blank `//` separator line if one isn't already at the end of `out`.
fn push_godoc_blank(out: &mut String) {
    if !out.ends_with("//\n") {
        out.push_str("//\n");
    }
}

/// Push `// <text>` line, or `//` when `text` is empty.
fn push_godoc_line(out: &mut String, text: &str) {
    if text.is_empty() {
        out.push_str("//\n");
    } else {
        out.push_str("// ");
        out.push_str(text);
        out.push('\n');
    }
}

/// Sanitize Rust-specific syntax and idioms from code examples for Go consumption.
/// Removes or translates patterns like .unwrap(), use statements, and Rust module syntax.
fn sanitize_rust_code_for_go(line: &str) -> String {
    let mut result = line.to_string();
    // Remove Rust use statements entirely.
    if result.trim().starts_with("use ") {
        return String::new();
    }
    // Remove .unwrap() and .expect(_) calls — Go idiom is explicit error handling.
    result = result.replace(".unwrap()", "").replace(".expect(\"", "/* error: ");
    if result.contains(".expect") {
        result = result.replace("\")", " */");
    }
    result
}

/// Emit a section body prefixed with `lead` on the first line.
///
/// If the body already starts with the lead phrase (case-insensitive) the body
/// is emitted verbatim. Otherwise the first content word's leading character is
/// lowercased so `Returns` + `"The root node"` reads as `Returns the root node`
/// rather than `Returns The root node`.
fn emit_prefixed_section(out: &mut String, body: &str, lead: &str) {
    let trimmed = body.trim();
    let lead_first_word = lead.split_whitespace().next().unwrap_or(lead);
    let starts_with_lead = trimmed
        .split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case(lead_first_word));
    if starts_with_lead {
        for line in trimmed.lines() {
            push_godoc_line(out, line.trim());
        }
        return;
    }
    let mut lines = trimmed.lines();
    if let Some(first) = lines.next() {
        let first = first.trim();
        let first_lc = first
            .chars()
            .next()
            .map(|c| c.to_lowercase().to_string() + &first[c.len_utf8()..])
            .unwrap_or_default();
        push_godoc_line(out, &format!("{} {}", lead, first_lc));
    }
    for line in lines {
        push_godoc_line(out, line.trim());
    }
}

/// Emit `# Arguments`, `# Returns`, `# Errors`, `# Example` sections of a
/// rustdoc block as Godoc-friendly prose. Each section is separated from
/// preceding output by a blank `//` line so godoc tooling renders paragraphs.
fn emit_godoc_sections(out: &mut String, sections: &crate::codegen::doc_emission::RustdocSections) {
    if let Some(body) = sections.arguments.as_deref() {
        push_godoc_blank(out);
        push_godoc_line(out, "Arguments:");
        let bullets = crate::codegen::doc_emission::parse_arguments_bullets(body);
        if bullets.is_empty() {
            for line in body.lines() {
                push_godoc_line(out, line.trim());
            }
        } else {
            for (name, desc) in bullets {
                let bullet = if desc.is_empty() {
                    format!("  - {}", name)
                } else {
                    format!("  - {}: {}", name, desc)
                };
                push_godoc_line(out, &bullet);
            }
        }
    }
    if let Some(body) = sections.returns.as_deref() {
        push_godoc_blank(out);
        emit_prefixed_section(out, body, "Returns");
    }
    if let Some(body) = sections.errors.as_deref() {
        push_godoc_blank(out);
        emit_prefixed_section(out, body, "Errors are returned when");
    }
    if let Some(body) = sections.example.as_deref() {
        push_godoc_blank(out);
        push_godoc_line(out, "Example:");
        // Godoc renders indented blocks as preformatted code. Strip a single
        // ``` fence pair if present, sanitize Rust-specific syntax, then indent each line with two spaces.
        let mut in_fence = false;
        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if line.trim().is_empty() {
                out.push_str("//\n");
            } else {
                let sanitized = sanitize_rust_code_for_go(line.trim_end());
                // Skip empty lines that result from stripping use statements.
                if !sanitized.trim().is_empty() {
                    out.push_str("//   ");
                    out.push_str(&sanitized);
                    out.push('\n');
                }
            }
            let _ = in_fence;
        }
    }
}
