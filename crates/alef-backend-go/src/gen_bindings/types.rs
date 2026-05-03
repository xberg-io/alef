use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_type_name, to_go_name};
use alef_core::ir::{DefaultValue, EnumDef, FieldDef, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::fmt::Write;

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// Go structs require named fields, so these must be skipped.
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Apply a serde `rename_all` strategy to a field name.
/// Returns the field name transformed according to the strategy, or the
/// original name if no strategy is set.
pub(super) fn apply_serde_rename(field_name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("camelCase") => field_name.to_lower_camel_case(),
        Some("PascalCase") => field_name.to_pascal_case(),
        Some("SCREAMING_SNAKE_CASE") => field_name.to_uppercase(),
        // snake_case is the Rust default — field names are already snake_case.
        _ => field_name.to_string(),
    }
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
pub(super) fn needs_omitempty_pointer(field: &FieldDef) -> bool {
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
pub(super) fn gen_unmarshal_bytes_helper() -> String {
    "// unmarshalBytes copies a C byte buffer into a Go []byte.\n\
     //\n\
     // The pointer is treated as a NUL-terminated C string; binary payloads\n\
     // that may contain interior NULs should be exposed by the FFI with an\n\
     // explicit length out-parameter instead.\n\
     func unmarshalBytes(ptr *C.uint8_t) *[]byte {\n\t\
         if ptr == nil {\n\t\t\
             return nil\n\t\
         }\n\t\
         v := []byte(C.GoString((*C.char)(unsafe.Pointer(ptr))))\n\t\
         return &v\n\
     }"
    .to_string()
}

/// Generate the lastError() helper function.
pub(super) fn gen_last_error_helper(ffi_prefix: &str) -> String {
    // Note: ctx is a borrowed pointer into thread-local storage, NOT a heap allocation.
    // Do NOT call free_string on it — that causes a double-free crash on the next FFI call.
    format!(
        "// lastError retrieves the last error from the FFI layer.\nfunc lastError() error {{\n\t\
         code := int32(C.{}_last_error_code())\n\t\
         if code == 0 {{\n\t\treturn nil\n\t}}\n\t\
         ctx := C.{}_last_error_context()\n\t\
         message := C.GoString(ctx)\n\t\
         return fmt.Errorf(\"[%d] %s\", code, message)\n\
         }}",
        ffi_prefix, ffi_prefix
    )
}

/// Emit Go-convention doc comment lines for an exported type into `out`.
///
/// Go's revive linter requires that the first line of a doc comment starts with
/// the exported name (with an optional leading article). This function rewrites
/// verbatim docs that begin with an article ("A ", "An ", "The ") by prepending
/// the type name, and falls back to a generated comment when no doc is present.
///
/// Examples:
/// - `"A chat message."` on `Message` → `"// Message is a chat message."`
/// - `"Message represents…"` on `Message` → `"// Message represents…"` (unchanged)
/// - empty doc on `Message` → `"// Message <fallback>."`
pub(super) fn emit_type_doc(out: &mut String, type_name: &str, doc: &str, fallback: &str) {
    if doc.is_empty() {
        writeln!(out, "// {} {}", type_name, fallback).ok();
        return;
    }
    let mut lines = doc.lines();
    if let Some(first) = lines.next() {
        let trimmed = first.trim();
        // Check whether the first line already starts with the type name.
        let already_starts = trimmed.starts_with(type_name);
        if already_starts {
            writeln!(out, "// {}", trimmed).ok();
        } else {
            // Strip leading articles and rewrite as "<TypeName> <rest>".
            let rest = trimmed
                .strip_prefix("A ")
                .or_else(|| trimmed.strip_prefix("An "))
                .or_else(|| trimmed.strip_prefix("The "))
                .unwrap_or(trimmed);
            // Lowercase the first letter of the rest so the sentence reads naturally
            // after the PascalCase type name prefix.
            let rest = if rest.is_empty() {
                fallback.to_string()
            } else {
                let mut chars = rest.chars();
                match chars.next() {
                    Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                    None => fallback.to_string(),
                }
            };
            writeln!(out, "// {} {}", type_name, rest).ok();
        }
        for line in lines {
            writeln!(out, "// {}", line.trim()).ok();
        }
    }
}

/// Generate a Go enum type definition.
///
/// For unit enums (all variants have no fields): generates `type X string` with constants.
/// For newtype-tuple enums (all data variant fields are positional tuple fields that would
/// be skipped in Go): generates `type X string` with constants for named variants plus
/// custom `MarshalJSON`/`UnmarshalJSON` that round-trips the string value unchanged — this
/// handles Rust enums like `enum Foo { A, B, Custom(String) }` where `Custom` carries an
/// arbitrary string payload.
/// For structural data enums (any variant has named fields): generates a flattened Go
/// struct with all variant fields collected and deduplicated, using pointer types for
/// fields not present in every variant.
pub(super) fn gen_enum_type(enum_def: &EnumDef) -> String {
    let is_data_enum = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if !is_data_enum {
        return gen_unit_enum_type(enum_def);
    }

    // Detect "newtype-tuple" pattern: a data enum whose data variants contain only
    // positional tuple fields (all of which `is_tuple_field` returns true for).
    // These are Rust enums like `enum Foo { A, B, Custom(String) }` where the
    // `Custom` variant wraps a single scalar.  Go cannot represent tuple fields in
    // a struct, so we fall back to the simpler `type Foo string` representation:
    // - Named (non-data) variants become string constants (e.g. `FooA`).
    // - The Custom/tuple variant becomes the "fallthrough": arbitrary string values
    //   that don't match a constant are accepted as-is (no extra UnmarshalJSON needed
    //   because the underlying type IS string).
    let all_data_fields_are_tuple = enum_def.variants.iter().all(|v| {
        v.fields.is_empty() || v.fields.iter().all(|f| is_tuple_field(f))
    });

    if all_data_fields_are_tuple {
        gen_newtype_tuple_enum_type(enum_def)
    } else {
        gen_data_enum_type(enum_def)
    }
}

/// Compute the wire value for a unit enum variant.
///
/// Priority order:
/// 1. Explicit `#[serde(rename = "...")]` on the variant (`serde_rename`).
/// 2. Enum-level `#[serde(rename_all = "...")]` applied to the variant name.
/// 3. Default: snake_case of the variant name.
fn enum_variant_wire_value(variant: &alef_core::ir::EnumVariant, enum_def: &EnumDef) -> String {
    if let Some(rename) = &variant.serde_rename {
        return rename.clone();
    }
    apply_serde_rename(&variant.name.to_snake_case(), enum_def.serde_rename_all.as_deref())
}

/// Generate a Go "newtype-tuple" enum as `type X string` with const block.
///
/// Used for Rust enums that have one or more unit variants plus one or more
/// "newtype" (single positional field) variants like `Custom(String)`.
/// The Go type is `type X string` — unit variants become named constants while
/// Custom/tuple variants are handled automatically because the underlying type
/// is `string` and any arbitrary string value round-trips through JSON as-is.
fn gen_newtype_tuple_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);
    let go_enum_name = go_type_name(&enum_def.name);
    emit_type_doc(&mut out, &go_enum_name, &enum_def.doc, "is an enumeration type.");
    writeln!(out, "type {} string", go_enum_name).ok();
    writeln!(out).ok();
    writeln!(out, "const (").ok();
    for variant in &enum_def.variants {
        // Only emit constants for unit (non-data) variants.
        // Tuple/data variants (e.g. Custom(String)) are represented as raw string values.
        if !variant.fields.is_empty() {
            continue;
        }
        let const_name = format!("{}{}", go_enum_name, to_go_name(&variant.name));
        let wire_value = enum_variant_wire_value(variant, enum_def);
        if !variant.doc.is_empty() {
            let mut lines = variant.doc.lines();
            if let Some(first) = lines.next() {
                let trimmed = first.trim();
                if trimmed.starts_with(&const_name) {
                    writeln!(out, "\t// {}", trimmed).ok();
                } else {
                    let rest = {
                        let mut chars = trimmed.chars();
                        match chars.next() {
                            Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                            None => trimmed.to_string(),
                        }
                    };
                    writeln!(out, "\t// {} {}", const_name, rest).ok();
                }
                for line in lines {
                    writeln!(out, "\t// {}", line.trim()).ok();
                }
            }
        } else {
            writeln!(
                out,
                "\t// {} is the {} variant of {}.",
                const_name, variant.name, enum_def.name
            )
            .ok();
        }
        writeln!(out, "\t{} {} = \"{}\"", const_name, go_enum_name, wire_value).ok();
    }
    writeln!(out, ")").ok();
    out
}

/// Generate a Go unit enum as `type X string` with const block.
fn gen_unit_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);

    let go_enum_name = go_type_name(&enum_def.name);
    emit_type_doc(&mut out, &go_enum_name, &enum_def.doc, "is an enumeration type.");
    writeln!(out, "type {} string", go_enum_name).ok();
    writeln!(out).ok();
    writeln!(out, "const (").ok();

    for variant in &enum_def.variants {
        let const_name = format!("{}{}", go_enum_name, to_go_name(&variant.name));
        let wire_value = enum_variant_wire_value(variant, enum_def);
        if !variant.doc.is_empty() {
            // revive requires the first comment line to start with the const name.
            // Prepend the const name if the existing doc doesn't already start with it.
            let mut lines = variant.doc.lines();
            if let Some(first) = lines.next() {
                let trimmed = first.trim();
                if trimmed.starts_with(&const_name) {
                    writeln!(out, "\t// {}", trimmed).ok();
                } else {
                    // Lowercase first char so the sentence reads naturally after the const name.
                    let rest = {
                        let mut chars = trimmed.chars();
                        match chars.next() {
                            Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                            None => trimmed.to_string(),
                        }
                    };
                    writeln!(out, "\t// {} {}", const_name, rest).ok();
                }
                for line in lines {
                    writeln!(out, "\t// {}", line.trim()).ok();
                }
            }
        } else {
            writeln!(
                out,
                "\t// {} is the {} variant of {}.",
                const_name, variant.name, enum_def.name
            )
            .ok();
        }
        writeln!(out, "\t{} {} = \"{}\"", const_name, go_enum_name, wire_value).ok();
    }

    writeln!(out, ")").ok();
    out
}

/// Generate a Go data enum as a flattened struct with JSON tags.
///
/// All fields from all variants are collected and deduplicated by name.
/// Fields that don't appear in every variant are made optional (pointer type).
fn gen_data_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);

    // Collect variant names for the doc comment
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
    let total_variants = enum_def.variants.len();
    let go_enum_name = go_type_name(&enum_def.name);

    emit_type_doc(
        &mut out,
        &go_enum_name,
        &enum_def.doc,
        "is a tagged union type (discriminated by JSON tag).",
    );
    writeln!(out, "// Variants: {}", variant_names.join(", ")).ok();
    writeln!(out, "type {} struct {{", go_enum_name).ok();
    writeln!(out, "\tVariant string `json:\"-\"`").ok();

    // Emit the serde tag discriminator field first (e.g. `Type string \`json:"type"\``).
    // This ensures round-trip JSON serialization preserves the variant discriminator,
    // which Rust needs to deserialize the correct enum variant.
    if let Some(tag_name) = &enum_def.serde_tag {
        writeln!(out, "\t{} string `json:\"{}\"`", to_go_name(tag_name), tag_name).ok();
    }

    // Collect and deduplicate fields across all variants.
    // Track: field name -> (FieldDef, count of variants containing it)
    let mut seen_fields: Vec<(String, FieldDef, usize)> = Vec::new();

    for variant in &enum_def.variants {
        for field in &variant.fields {
            if is_tuple_field(field) {
                continue;
            }
            if let Some(entry) = seen_fields.iter_mut().find(|(name, _, _)| *name == field.name) {
                entry.2 += 1;
            } else {
                seen_fields.push((field.name.clone(), field.clone(), 1));
            }
        }
    }

    for (field_name, field, count) in &seen_fields {
        // A field is optional if it's already marked optional OR if it doesn't appear in all variants
        let is_optional = field.optional || *count < total_variants;

        let field_type = if is_optional {
            go_optional_type(&field.ty)
        } else {
            go_type(&field.ty)
        };

        let json_tag = if is_optional {
            format!("json:\"{},omitempty\"", field_name)
        } else {
            format!("json:\"{}\"", field_name)
        };

        if !field.doc.is_empty() {
            for line in field.doc.lines() {
                writeln!(out, "\t// {}", line.trim()).ok();
            }
        }
        writeln!(out, "\t{} {} `{}`", to_go_name(field_name), field_type, json_tag).ok();
    }

    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func (e {go_enum_name}) String() string {{").ok();
    writeln!(out, "\treturn e.Variant").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func (e {go_enum_name}) MarshalJSON() ([]byte, error) {{").ok();
    writeln!(out, "\tif e.Variant != \"\" {{").ok();
    writeln!(out, "\t\treturn json.Marshal(e.Variant)").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\ttype alias {go_enum_name}").ok();
    writeln!(out, "\treturn json.Marshal(alias(e))").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func (e *{go_enum_name}) UnmarshalJSON(data []byte) error {{").ok();
    writeln!(out, "\tvar wire string").ok();
    writeln!(out, "\tif err := json.Unmarshal(data, &wire); err == nil {{").ok();
    writeln!(out, "\t\te.Variant = wire").ok();
    writeln!(out, "\t\treturn nil").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tvar tagged map[string]json.RawMessage").ok();
    writeln!(
        out,
        "\tif err := json.Unmarshal(data, &tagged); err == nil && len(tagged) == 1 {{"
    )
    .ok();
    writeln!(out, "\t\tfor variant, payload := range tagged {{").ok();
    writeln!(out, "\t\t\te.Variant = variant").ok();
    writeln!(out, "\t\t\tif string(payload) != \"null\" {{").ok();
    writeln!(out, "\t\t\t\ttype alias {go_enum_name}").ok();
    writeln!(out, "\t\t\t\tvar decoded alias").ok();
    writeln!(
        out,
        "\t\t\t\tif err := json.Unmarshal(payload, &decoded); err == nil {{"
    )
    .ok();
    writeln!(out, "\t\t\t\t\t*e = {go_enum_name}(decoded)").ok();
    writeln!(out, "\t\t\t\t\te.Variant = variant").ok();
    writeln!(out, "\t\t\t\t}}").ok();
    writeln!(out, "\t\t\t}}").ok();
    writeln!(out, "\t\t\treturn nil").ok();
    writeln!(out, "\t\t}}").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\ttype alias {go_enum_name}").ok();
    writeln!(out, "\tvar decoded alias").ok();
    writeln!(out, "\tif err := json.Unmarshal(data, &decoded); err != nil {{").ok();
    writeln!(out, "\t\treturn err").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\t*e = {go_enum_name}(decoded)").ok();
    if let Some(tag_name) = &enum_def.serde_tag {
        let tag_field = to_go_name(tag_name);
        writeln!(out, "\te.Variant = e.{tag_field}").ok();
    }
    writeln!(out, "\treturn nil").ok();
    writeln!(out, "}}").ok();
    out
}

/// Generate a Go opaque handle type wrapping an `unsafe.Pointer`.
///
/// Opaque types are not JSON-serializable — they are raw C pointers passed through
/// the FFI layer. The Go struct holds a pointer and exposes a `Free()` method.
/// Constructors are NOT emitted here — they are generated as free function wrappers
/// from `api.functions` entries that return this opaque type (e.g. `CreateClient`,
/// `CreateClientFromJson`). A zero-argument `New{TypeName}()` calling
/// `C.{prefix}_{type_snake}()` would reference a C function that does not exist in
/// the FFI layer.
pub(super) fn gen_opaque_type(typ: &TypeDef, ffi_prefix: &str) -> String {
    let mut out = String::with_capacity(512);
    let type_snake = typ.name.to_snake_case();
    let go_name = go_type_name(&typ.name);

    emit_type_doc(&mut out, &go_name, &typ.doc, "is an opaque handle type.");
    writeln!(out, "type {} struct {{", go_name).ok();
    writeln!(out, "\tptr unsafe.Pointer").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Free method. typ.name is already PascalCase from Rust IR; ToPascalCase
    // would mangle all-caps acronyms (GraphQL -> GraphQl) and disagree with
    // cbindgen's actual C type name.
    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), typ.name);
    writeln!(out, "// Free releases the resources held by this handle.").ok();
    writeln!(out, "func (h *{}) Free() {{", go_name).ok();
    writeln!(out, "\tif h.ptr != nil {{").ok();
    writeln!(out, "\t\tC.{}_{}_free((*C.{})(h.ptr))", ffi_prefix, type_snake, c_type).ok();
    writeln!(out, "\t\th.ptr = nil").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "}}").ok();

    out
}

/// Generate only the `Free()` method for an opaque handle type whose struct definition
/// was already emitted by `gen_go_error_types`.
///
/// Error types share their name with their corresponding opaque handle (the C layer allocates
/// a `LiterLlmError*` handle that the Go binding holds as an opaque pointer). However the Go
/// error struct uses `Code`/`Message` string fields rather than a raw `ptr unsafe.Pointer`, so
/// we cannot generate the normal `Free()` using `h.ptr`. Instead we emit an unexported stub
/// that references the C symbols to keep them from being pruned, but does nothing at runtime —
/// Go error values are not heap-allocated C objects from the binding's perspective.
pub(super) fn gen_opaque_type_free_only(typ: &TypeDef, _ffi_prefix: &str) -> String {
    // Nothing to emit — the structured error type already has its Error() method and
    // the C-level free function is invoked transparently inside the FFI layer.
    // Returning an empty string avoids a duplicate struct definition and a broken Free().
    let _ = typ;
    String::new()
}

/// Generate a Go struct type definition with json tags for marshaling.
pub(super) fn gen_struct_type(typ: &TypeDef, enum_names: &std::collections::HashSet<&str>) -> String {
    let mut out = String::with_capacity(1024);

    let go_name = go_type_name(&typ.name);
    emit_type_doc(&mut out, &go_name, &typ.doc, "is a type.");
    writeln!(out, "type {} struct {{", go_name).ok();

    for field in &typ.fields {
        if is_tuple_field(field) {
            continue;
        }

        // A non-optional field in a defaulted struct may still need pointer+omitempty when
        // the Go zero value differs from the Rust Default value (e.g., Duration, bool true, int != 0).
        let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);

        // Named types that map to Go string enums must also use omitempty (without pointer),
        // because the Go zero value "" is never a valid Rust enum variant. Without omitempty,
        // marshaling an empty struct sends `"field": ""` which fails Rust serde deserialization.
        let is_named_enum = !field.optional
            && !use_default_pointer
            && typ.has_default
            && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));

        let field_type = if field.optional {
            go_optional_type(&field.ty)
        } else if use_default_pointer {
            // Emit as pointer so that an unset field serializes as absent (omitempty),
            // letting Rust serde fill in the real default instead of seeing a zero value.
            go_optional_type(&field.ty)
        } else {
            go_type(&field.ty)
        };

        // Determine json tag - apply serde rename_all strategy.
        // Use omitempty for optional fields, slice/map types (nil slices serialize to null
        // in Go, which breaks Rust serde deserialization expecting an array), fields
        // where the Go zero value differs from the Rust Default value, and string enum
        // fields where "" is never a valid Rust enum variant.
        let json_name = apply_serde_rename(&field.name, typ.serde_rename_all.as_deref());
        let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
        let json_tag = if field.optional || is_collection || use_default_pointer || is_named_enum {
            format!("json:\"{},omitempty\"", json_name)
        } else {
            format!("json:\"{}\"", json_name)
        };

        if !field.doc.is_empty() {
            for line in field.doc.lines() {
                writeln!(out, "\t// {}", line.trim()).ok();
            }
        }
        writeln!(out, "\t{} {} `{}`", to_go_name(&field.name), field_type, json_tag).ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Return the CGo type name for a primitive type (e.g. `PrimitiveType::U64` → `"C.uint64_t"`).
///
/// CGo treats Go native types (`uint64`, `uint32`, …) and the corresponding C typedefs
/// (`C.uint64_t`, `C.uint32_t`, …) as distinct and will not implicitly convert between them
/// when passing values to C functions. Declaring optional-primitive temporaries with the CGo
/// type avoids an explicit cast at every call-site.
pub(super) fn cgo_type_for_primitive(prim: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType;
    match prim {
        PrimitiveType::U8 => "C.uint8_t",
        PrimitiveType::U16 => "C.uint16_t",
        PrimitiveType::U32 => "C.uint32_t",
        PrimitiveType::U64 => "C.uint64_t",
        PrimitiveType::Usize => "C.size_t",
        PrimitiveType::I8 => "C.int8_t",
        PrimitiveType::I16 => "C.int16_t",
        PrimitiveType::I32 => "C.int32_t",
        PrimitiveType::I64 => "C.int64_t",
        PrimitiveType::Isize => "C.ptrdiff_t",
        PrimitiveType::F32 => "C.float",
        PrimitiveType::F64 => "C.double",
        PrimitiveType::Bool => "C.int32_t",
    }
}

/// Return the Go expression for the maximum value of a primitive type, used as a sentinel
/// to signal "None" to FFI functions that use max-value sentinels for optional primitives.
pub(super) fn primitive_max_sentinel(prim: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType;
    match prim {
        PrimitiveType::U8 => "^uint8(0)",
        PrimitiveType::U16 => "^uint16(0)",
        PrimitiveType::U32 => "^uint32(0)",
        PrimitiveType::U64 => "^uint64(0)",
        PrimitiveType::Usize => "^uint(0)",
        PrimitiveType::I8 => "int8(127)",
        PrimitiveType::I16 => "int16(32767)",
        PrimitiveType::I32 => "int32(2147483647)",
        PrimitiveType::I64 => "int64(9223372036854775807)",
        PrimitiveType::Isize => "int(^uint(0) >> 1)",
        PrimitiveType::F32 => "float32(0)",
        PrimitiveType::F64 => "float64(0)",
        PrimitiveType::Bool => "false",
    }
}

/// Get a type name suitable for a function suffix (e.g., unmarshalFoo).
pub(super) fn type_name(ty: &TypeRef) -> String {
    match ty {
        // IR Named types are already PascalCase from Rust source. Avoid
        // ToPascalCase to preserve all-caps acronyms (GraphQL, JSON, HTTP).
        TypeRef::Named(n) => n.clone(),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Bytes".to_string(),
        TypeRef::Optional(inner) => type_name(inner),
        TypeRef::Vec(inner) => format!("List{}", type_name(inner)),
        TypeRef::Map(_, v) => format!("Map{}", type_name(v)),
        TypeRef::Json => "JSON".to_string(),
        TypeRef::Path => "Path".to_string(),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Duration => "U64".to_string(),
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "Bool".to_string(),
            alef_core::ir::PrimitiveType::U8 => "U8".to_string(),
            alef_core::ir::PrimitiveType::U16 => "U16".to_string(),
            alef_core::ir::PrimitiveType::U32 => "U32".to_string(),
            alef_core::ir::PrimitiveType::U64 => "U64".to_string(),
            alef_core::ir::PrimitiveType::I8 => "I8".to_string(),
            alef_core::ir::PrimitiveType::I16 => "I16".to_string(),
            alef_core::ir::PrimitiveType::I32 => "I32".to_string(),
            alef_core::ir::PrimitiveType::I64 => "I64".to_string(),
            alef_core::ir::PrimitiveType::F32 => "F32".to_string(),
            alef_core::ir::PrimitiveType::F64 => "F64".to_string(),
            alef_core::ir::PrimitiveType::Usize => "Usize".to_string(),
            alef_core::ir::PrimitiveType::Isize => "Isize".to_string(),
        },
    }
}

/// Generate a Go expression that converts a C return value (`ptr`) to the correct Go type.
///
/// For primitives like Bool, this produces inline conversion (e.g., `func() *bool { v := ptr != 0; return &v }()`).
/// For Named types (opaque handles), this uses `_to_json` to serialize then `json.Unmarshal` in Go.
/// For strings, this calls `C.GoString`.
/// The `ffi_prefix` is used to construct C type names for Named types.
pub(super) fn go_return_expr(
    ty: &TypeRef,
    var_name: &str,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    go_return_expr_inner(ty, var_name, ffi_prefix, opaque_names)
}

fn go_return_expr_inner(
    ty: &TypeRef,
    var_name: &str,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            alef_core::ir::PrimitiveType::Bool => {
                format!("func() *bool {{ v := {} != 0; return &v }}()", var_name)
            }
            _ => {
                // Numeric primitives: cast and take address
                let go_ty = go_type(ty);
                format!("func() *{go_ty} {{ v := {go_ty}({var_name}); return &v }}()")
            }
        },
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // Opaque types: wrap the raw C pointer in the Go handle struct.
                // IR name is already PascalCase from Rust; preserve all-caps
                // acronyms (GraphQLError stays GraphQLError, not GraphQlError).
                format!(
                    "&{go_type}{{ptr: unsafe.Pointer({var_name})}}",
                    go_type = name,
                    var_name = var_name,
                )
            } else {
                // Full conversion: serialize C handle to JSON, then unmarshal into Go struct
                let type_snake = name.to_snake_case();
                format!(
                    "func() *{go_type} {{\n\
                     \tjsonPtr := C.{ffi_prefix}_{type_snake}_to_json({var_name})\n\
                     \tif jsonPtr == nil {{ return nil }}\n\
                     \tdefer C.{ffi_prefix}_free_string(jsonPtr)\n\
                     \tvar result {go_type}\n\
                     \tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {{ return nil }}\n\
                     \treturn &result\n\
                     }}()",
                    go_type = name,
                    ffi_prefix = ffi_prefix,
                    type_snake = type_snake,
                    var_name = var_name,
                )
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            format!("func() *string {{ v := C.GoString({}); return &v }}()", var_name)
        }
        TypeRef::Json => {
            format!("func() *json.RawMessage {{ v := json.RawMessage(C.GoString({var_name})); return &v }}()")
        }
        TypeRef::Bytes => {
            format!("unmarshalBytes({})", var_name)
        }
        TypeRef::Optional(inner) => go_return_expr_inner(inner, var_name, ffi_prefix, opaque_names),
        TypeRef::Vec(inner) => {
            // Vec types are returned as JSON strings from FFI. Deserialize inline.
            let go_elem = go_type(inner);
            format!(
                "func() *[]{go_elem} {{\n\
                 \tif {var_name} == nil {{ return nil }}\n\
                 \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                 \tvar result []{go_elem}\n\
                 \tif err := json.Unmarshal([]byte(C.GoString({var_name})), &result); err != nil {{ return nil }}\n\
                 \treturn &result\n\
                 }}()",
                go_elem = go_elem,
                var_name = var_name,
                ffi_prefix = ffi_prefix,
            )
        }
        TypeRef::Map(k, v) => {
            // Map types are returned as JSON strings from FFI. Deserialize inline.
            let go_k = go_type(k);
            let go_v = go_type(v);
            format!(
                "func() *map[{go_k}]{go_v} {{\n\
                 \tif {var_name} == nil {{ return nil }}\n\
                 \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                 \tvar result map[{go_k}]{go_v}\n\
                 \tif err := json.Unmarshal([]byte(C.GoString({var_name})), &result); err != nil {{ return nil }}\n\
                 \treturn &result\n\
                 }}()",
                go_k = go_k,
                go_v = go_v,
                var_name = var_name,
                ffi_prefix = ffi_prefix,
            )
        }
        _ => format!("unmarshal{}({})", type_name(ty), var_name),
    }
}

/// Generate functional options pattern for Go config types with defaults.
/// Produces ConfigOption type and WithFieldName constructors.
pub(super) fn gen_config_options(typ: &TypeDef, enum_names: &std::collections::HashSet<&str>) -> String {
    let mut out = String::with_capacity(2048);

    // ConfigOption type definition
    let go_name = go_type_name(&typ.name);
    writeln!(out, "// {}Option is an option function for {}.", go_name, go_name).ok();
    writeln!(out, "type {}Option func(*{})", go_name, go_name).ok();
    writeln!(out).ok();

    // Generate WithFieldName constructors for each field
    for field in &typ.fields {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);
        // For the function parameter, always accept the direct type (not wrapped in optional)
        let param_type = go_type(&field.ty);

        writeln!(
            out,
            "// With{}{} sets the {} field.",
            go_name, field_go_name, field.name
        )
        .ok();
        writeln!(
            out,
            "func With{}{}(v {}) {}Option {{",
            go_name, field_go_name, param_type, go_name
        )
        .ok();
        // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults) both
        // store pointer types in the struct, so we must take the address of v when assigning.
        let use_ptr = field.optional || needs_omitempty_pointer(field);
        let assign_val = if use_ptr { "&v" } else { "v" };
        writeln!(
            out,
            "\treturn func(c *{}) {{ c.{} = {} }}",
            go_name, field_go_name, assign_val
        )
        .ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
    }

    // Generate NewConfig constructor
    writeln!(out, "// New{} creates a {} with optional parameters.", go_name, go_name).ok();
    writeln!(out, "func New{}(opts ...{}Option) *{} {{", go_name, go_name, go_name).ok();
    writeln!(out, "\tc := &{} {{", go_name).ok();

    // Set default values for fields
    for field in &typ.fields {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);
        let default_val = if field.optional || needs_omitempty_pointer(field) {
            // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults)
            // are pointer types. Set to nil so they serialize as absent, letting Rust serde
            // fill in the real default instead of seeing a Go zero value.
            "nil".to_string()
        } else {
            let mut val = alef_codegen::config_gen::default_value_for_field(field, "go");
            // config_gen returns "nil" for Named types with Empty default, but in Go
            // non-optional Named types are value types. Fix up based on whether the
            // Named type is a string-based enum or a struct.
            if val == "nil" {
                if let TypeRef::Named(name) = &field.ty {
                    if enum_names.contains(name.as_str()) {
                        // String-typed enum — zero value is empty string
                        val = "\"\"".to_string();
                    } else {
                        // Struct — zero value is TypeName{}
                        val = format!("{}{{}}", go_type_name(name));
                    }
                }
            }
            val
        };
        writeln!(out, "\t\t{}: {},", field_go_name, default_val).ok();
    }

    writeln!(out, "\t}}").ok();
    writeln!(out, "\tfor _, opt := range opts {{").ok();
    writeln!(out, "\t\topt(c)").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn c").ok();
    writeln!(out, "}}").ok();

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

    fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    #[test]
    fn test_is_tuple_field_detects_positional_names() {
        let positional = simple_field("_0", TypeRef::String);
        assert!(is_tuple_field(&positional));
        let named = simple_field("value", TypeRef::String);
        assert!(!is_tuple_field(&named));
    }

    #[test]
    fn test_apply_serde_rename_camel_case() {
        assert_eq!(apply_serde_rename("my_field", Some("camelCase")), "myField");
        assert_eq!(apply_serde_rename("my_field", None), "my_field");
    }

    #[test]
    fn test_gen_unit_enum_type_produces_type_string_and_const_block() {
        let enum_def = EnumDef {
            name: "Status".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_rename_all: None,
            variants: vec![EnumVariant {
                name: "Active".to_string(),
                doc: String::new(),
                fields: vec![],
                is_default: false,
                serde_rename: None,
                is_tuple: false,
            }],
        };
        let out = gen_unit_enum_type(&enum_def);
        assert!(out.contains("type Status string"));
        assert!(out.contains("const ("));
        assert!(out.contains("StatusActive"));
    }

    #[test]
    fn test_gen_struct_type_emits_json_tags() {
        let typ = TypeDef {
            name: "MyConfig".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            fields: vec![simple_field("timeout", TypeRef::Primitive(PrimitiveType::U64))],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            methods: vec![],
        };
        let out = gen_struct_type(&typ, &std::collections::HashSet::new());
        assert!(out.contains("type MyConfig struct"));
        assert!(out.contains("json:\"timeout\""));
    }
}
