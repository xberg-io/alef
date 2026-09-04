use crate::backends::go::c_symbols;
use crate::core::ir::{DefaultValue, FieldDef, TypeDef, TypeRef};
use minijinja::context;

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// Go structs require named fields, so these must be skipped.
pub(in crate::backends::go::gen_bindings) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Returns true if a non-optional struct field should be emitted as a pointer type with
/// `omitempty`.
///
/// Gated on the field being *wire-optional*: either the field carries an actual
/// `#[serde(default...)]` (`field.default.is_some()`), or its container carries a
/// container-level `#[serde(default)]` (`TypeDef::serde_container_default`), which makes
/// every key of that struct fillable from the container's `Default`. Either way the Rust
/// side genuinely tolerates the key being absent from the JSON payload.
///
/// `TypeDef::has_default` (whether the container merely *has* a `Default` impl) is NOT a
/// substitute signal: a struct can implement `Default` while every individual field stays
/// required at the serde level, and treating `has_default` as if it meant "this field is
/// omittable" previously caused required fields (no `#[serde(default)]` at all) to be emitted
/// as pointer+omitempty, silently dropping them from `json.Marshal` output and breaking
/// deserialization on the Rust side (`missing field`). `serde_container_default` is the
/// narrower fact — the container carries the attribute, not just the impl — and is the only
/// container-level signal admissible here. ~keep
///
/// Once a field is confirmed wire-optional, pointer+omitempty is still only needed when the
/// Go zero value for the field differs from the Rust default — otherwise an unset Go value
/// already serializes to the same thing the Rust default would produce.
///
/// Cases that require pointer+omitempty:
/// - `TypeRef::Duration` — the Go zero (`DurationMillis(0)`) is a valid but essentially never
///   the real default (defaults are non-zero, e.g. 30s), so always prefer omitting when unset
/// - `BoolLiteral(true)` — Rust default is `true`, Go zero is `false`
/// - `IntLiteral(n)` where n != 0 — Rust default is n, Go zero is 0
/// - `FloatLiteral(f)` where f != 0.0 — Rust default is f, Go zero is 0.0
/// - `StringLiteral(s)` where !s.is_empty() — Rust default is s, Go zero is ""
/// - `EnumVariant(_)` — Rust default is a specific variant, Go zero is ""
/// - `Unresolved(_)` — alef could not read the real default, so it cannot be known to agree
///   with the Go zero; assume it does not, the same way `Duration` always does
/// - `TupleVariant(_, _)` / `StructVariant(_, _)` — a resolved enum-variant default this
///   renderer has no per-argument Go expression for, so it is unrenderable exactly like
///   `Unresolved` even though alef did read the value
/// - `FunctionCall(_)` / `PublicFunctionCall(_)` — `#[serde(default = "path")]`; alef records
///   the function's *name*, never its return value, so the Go zero is a claim about a value
///   alef does not have
///
/// A field whose type is itself a plain data struct — i.e. `field.ty` is `TypeRef::Named(name)`
/// and `name` is in `struct_names` (every non-opaque `TypeDef` emitted as a Go struct in this
/// binding) — always needs pointer+omitempty, independent of every check below. This is checked
/// first and unconditionally, not only when the field fails the wire-optional check, because a
/// struct field's Go zero value is a fully-populated substructure, never an "absent" marker,
/// whether or not the field itself carries a serde default:
///
/// - A required field (fails the wire-optional check below): leaving it a plain (non-pointer)
///   field would silently `json.Marshal` an all-zero payload that Rust's `serde` happily accepts
///   as genuinely-provided data — every leaf key is present, just wrong. Pointer+omitempty turns
///   an unset Go value into a dropped key instead, so Rust rejects the call with `missing field`
///   — a loud failure, and strictly better than a silent wrong value.
/// - A wire-optional field (passes the check below) whose `typed_default` folded to `Empty`:
///   `Empty` is only an assertion that `<FieldType>::default()` equals the *language-agnostic*
///   zero, not that it equals *Go's* zero — true when `FieldType` derives `#[derive(Default)]`,
///   but false whenever `FieldType` has a hand-written `impl Default` returning a non-zero value
///   (e.g. `NgramRange::default() -> Self { min: 1, max: 3 }`). The extractor's `Empty` fold
///   cannot distinguish the two cases (see `extract::extractor::defaults::expr_to_default_value`,
///   the `T::default()` arm), so this predicate cannot trust `Empty` for a struct-typed field the
///   way it does for a scalar. Pointer+omitempty is the safe answer in both branches: when the
///   nested type's true zero does happen to match, dropping the key is a no-op because Rust's own
///   default (or `#[serde(default)]` on the field) reconstructs the identical value; when it does
///   not match, dropping the key is what avoids shipping the wrong value. There is no case where
///   forcing the pointer is incorrect for a struct-typed field, so no case-split is needed here.
///
/// This is deliberately narrower than gating on `TypeDef::has_default` (the mistake
/// `serde_container_default` replaced, see above): it is keyed off `field.ty` itself, so it never
/// touches a scalar or enum field, however many other fields on `typ` are wire-optional. Scalar
/// zero values are the caller's own responsibility to set explicitly (no reason to force a
/// pointer); unit-enum fields already fail loud on their own — they render as Go strings, and
/// `""` is never a valid variant, so Rust already rejects a required enum field left at its Go
/// zero without needing this. ~keep
pub(crate) fn needs_omitempty_pointer(
    typ: &TypeDef,
    field: &FieldDef,
    struct_names: &std::collections::HashSet<&str>,
) -> bool {
    if matches!(&field.ty, TypeRef::Named(name) if struct_names.contains(name.as_str())) {
        return true;
    }
    if field.default.is_none() && !typ.serde_container_default {
        return false;
    }
    if matches!(field.ty, TypeRef::Duration) {
        return true;
    }
    match &field.typed_default {
        Some(DefaultValue::BoolLiteral(true)) => true,
        Some(DefaultValue::IntLiteral(n)) if *n != 0 => true,
        Some(DefaultValue::FloatLiteral(f)) if *f != 0.0 => true,
        Some(DefaultValue::StringLiteral(s)) if !s.is_empty() => true,
        Some(DefaultValue::EnumVariant(_)) => true,
        // `Unresolved`: alef could not read the real default out of `impl Default`, so there is
        // no way to know it agrees with the Go zero — assume it does not, the same way `Duration`
        // always does above. `TupleVariant`/`StructVariant`: alef read the value, but this
        // renderer has no Go expression for "construct enum variant X with these field values"
        // the way it does for a bare `EnumVariant` (which is unconditionally `true` just above).
        // Leaving either at `false` here reaches `default_value_for_field`'s type-zero table,
        // which then gets marshaled onto the wire as though the caller had chosen it — the exact
        // silent-wrong-data defect this predicate exists to prevent. ~keep
        Some(DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..)) => true,
        // SECURITY. `#[serde(default = "path")]`: alef records the *name* of the function, never
        // its return value, so the Go zero for this field is a claim about a value alef has not
        // read. Grouping these with `Empty` below broke exactly the scalar shapes — a `Vec`/`Map`
        // field already gets `,omitempty` from `go_struct_field_json_tag`'s `collection` rule and
        // so already dropped an unset key. A `string`/number/`bool` field got neither the pointer
        // nor the tag, so its Go zero (`""`, `0`, `false`) was marshaled onto the wire as though
        // the caller had chosen it and `path()` never ran. The same `false` also reaches
        // `config_gen::default_value_for_field(field, "go")` in the `New()` constructor, whose
        // `FunctionCall` arm answers `"nil"` — valid for a pointer field, not assignable to a
        // bare `string`/`int`, so after extraction stopped letting a container's
        // `#[derive(Default)]` overwrite `FunctionCall` with `Empty` the emitted Go stopped
        // compiling as well.
        //
        // Pointer+omitempty is the deferral: an unset Go value drops the key from
        // `json.Marshal`, which is precisely the condition under which serde runs `path()` and
        // supplies the real default. ~keep
        Some(DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_)) => true,
        Some(
            DefaultValue::BoolLiteral(false)
            | DefaultValue::IntLiteral(_)
            | DefaultValue::FloatLiteral(_)
            | DefaultValue::StringLiteral(_)
            | DefaultValue::ListLiteral(_)
            | DefaultValue::Empty
            | DefaultValue::None,
        )
        | None => false,
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

/// Generate the package-level `DurationMillis` type and its `MarshalJSON`/`UnmarshalJSON`
/// methods (see `duration_millis_type.jinja`).
///
/// Emitted exactly once per generated `binding.go`, only when the API surface has at
/// least one `Duration`-typed struct field that resolves to this type (see
/// `binding_file::api_has_duration_field`). Struct fields select it via
/// `type_map::go_field_type` instead of a bare `uint64` because `std::time::Duration`'s
/// *derived* serde shape is `{"secs":u64,"nanos":u32}`, not a plain integer — a field
/// carrying `#[serde(with = "...")]` overrides that derive and keeps the bare `uint64`.
pub(in crate::backends::go::gen_bindings) fn gen_duration_millis_helper() -> String {
    crate::backends::go::template_env::render("duration_millis_type.jinja", minijinja::Value::default())
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
pub(in crate::backends::go::gen_bindings) fn gen_last_error_helper(
    api: &crate::core::ir::ApiSurface,
    ffi_prefix: &str,
) -> String {
    let taxonomy = api.error_taxonomy();
    let error_codes: Vec<_> = taxonomy
        .iter()
        .map(|entry| {
            let error = api
                .errors
                .iter()
                .find(|error| error.rust_path == entry.error_type)
                .unwrap();
            (
                entry.code,
                crate::codegen::error_gen::go_error_sentinel_name(&api.errors, &error.name, &entry.variant),
            )
        })
        .collect();
    crate::backends::go::template_env::render(
        "last_error_helper.jinja",
        context! {
            last_error_code_fn => c_symbols::last_error_code_symbol(ffi_prefix),
            last_error_context_fn => c_symbols::last_error_context_symbol(ffi_prefix),
            error_codes => error_codes,
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

#[cfg(test)]
mod last_error_tests {
    use super::*;
    use crate::core::ir::{ErrorDef, ErrorVariant};

    #[test]
    fn typed_errors_dispatch_by_numeric_taxonomy_code() {
        let error = ErrorDef {
            name: "RequestError".to_string(),
            rust_path: "sample::RequestError".to_string(),
            variants: vec![ErrorVariant {
                error_code: Some(100),
                name: "InvalidInput".to_string(),
                is_unit: true,
                ..Default::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let api = crate::core::ir::ApiSurface {
            errors: vec![error],
            ..Default::default()
        };
        let code = api.error_taxonomy()[0].code;

        let helper = gen_last_error_helper(&api, "sample");

        assert!(helper.contains(&format!("case {code}:")));
        assert!(helper.contains("return ErrInvalidInput"));
        assert!(helper.contains("fmt.Errorf(\"[%d] %s\", code, message)"));
    }
}
