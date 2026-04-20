use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::to_go_name;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, DefaultValue, EnumDef, FieldDef, FunctionDef, MethodDef, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::fmt::Write;
use std::path::PathBuf;

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// Go structs require named fields, so these must be skipped.
fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Apply a serde `rename_all` strategy to a field name.
/// Returns the field name transformed according to the strategy, or the
/// original name if no strategy is set.
fn apply_serde_rename(field_name: &str, rename_all: Option<&str>) -> String {
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
fn needs_omitempty_pointer(field: &FieldDef) -> bool {
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

pub struct GoBackend;

impl GoBackend {
    /// Extract the package name from module path (last segment).
    /// Sanitize by removing hyphens and converting to lowercase.
    fn package_name(module_path: &str) -> String {
        module_path
            .split('/')
            .next_back()
            .unwrap_or("kreuzberg")
            .replace('-', "")
            .to_lowercase()
    }
}

impl Backend for GoBackend {
    fn name(&self) -> &str {
        "go"
    }

    fn language(&self) -> Language {
        Language::Go
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_path = config.go_module();
        let pkg_name = config
            .go
            .as_ref()
            .and_then(|g| g.package_name.clone())
            .unwrap_or_else(|| Self::package_name(&module_path));
        let ffi_prefix = config.ffi_prefix();

        let output_dir = resolve_output_dir(config.output.go.as_ref(), &config.crate_config.name, "packages/go/");

        let ffi_lib_name = config.ffi_lib_name();
        let ffi_header = config.ffi_header_name();
        // Derive the FFI crate directory from the output path (e.g., "crates/html-to-markdown-ffi/src/" → "crates/html-to-markdown-ffi")
        let ffi_crate_dir = config
            .output
            .ffi
            .as_ref()
            .and_then(|p| {
                let path = p.as_path();
                path.ancestors()
                    .find(|a| {
                        a.file_name()
                            .is_some_and(|n| n != "src" && n != "lib" && n != "include")
                    })
                    .map(|a| a.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| format!("crates/{ffi_lib_name}"));
        let content = strip_trailing_whitespace(&gen_go_file(
            api,
            &ffi_prefix,
            &pkg_name,
            &ffi_lib_name,
            &ffi_header,
            &ffi_crate_dir,
            &output_dir,
        ));

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Go)?;

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("binding.go"),
            content,
            generated_header: true,
        }])
    }

    /// Go bindings are already the public API (single .go file wrapping C FFI).
    /// This returns empty since the binding.go file serves as both the FFI layer
    /// and the high-level public API for consumers.
    fn generate_public_api(&self, _api: &ApiSurface, _config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Go's binding.go IS the public API — no additional wrapper needed.
        Ok(vec![])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "go",
            crate_suffix: "",
            depends_on_ffi: true,
            post_build: vec![],
        })
    }
}

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
fn strip_trailing_whitespace(content: &str) -> String {
    let mut result: String = content
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Generate the complete Go binding file wrapping the C FFI layer.
fn gen_go_file(
    api: &ApiSurface,
    ffi_prefix: &str,
    pkg_name: &str,
    ffi_lib_name: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    go_output_dir: &str,
) -> String {
    let mut out = String::with_capacity(4096);

    // Go convention: generated file marker must appear before package declaration.
    writeln!(out, "// Code generated by alef. DO NOT EDIT.").ok();
    writeln!(out).ok();

    // Compute relative path from Go output dir to project root.
    // go_output_dir is like "packages/go/", so we need "../../" to reach root.
    let depth = go_output_dir.trim_end_matches('/').matches('/').count() + 1;
    let to_root = "../".repeat(depth);

    // Package header and cgo directives
    writeln!(out, "package {}\n", pkg_name).ok();
    writeln!(out, "/*").ok();
    writeln!(out, "#cgo CFLAGS: -I${{SRCDIR}}/{to_root}{ffi_crate_dir}/include").ok();
    writeln!(
        out,
        "#cgo LDFLAGS: -L${{SRCDIR}}/{to_root}target/release -l{ffi_lib_name}"
    )
    .ok();
    writeln!(out, "#include \"{}\"", ffi_header).ok();
    writeln!(out, "*/\nimport \"C\"").ok();
    writeln!(out).ok();
    // Determine which imports are needed based on generated code.
    let has_opaque_types = api.types.iter().any(|t| t.is_opaque);
    // Functions that are not skipped (non-async or with non-Named returns) need json + unsafe.
    // Opaque-returning functions are no longer skipped, so check all non-async functions.
    let has_sync_functions = api.functions.iter().any(|f| !f.is_async);
    let has_non_static_methods = api.types.iter().any(|t| t.methods.iter().any(|m| !m.is_static));
    let needs_json_and_unsafe = has_sync_functions || has_non_static_methods;

    let mut imports = vec!["\"fmt\""];
    if needs_json_and_unsafe {
        imports.insert(0, "\"encoding/json\"");
        imports.push("\"unsafe\"");
    } else if has_opaque_types {
        // Opaque types need unsafe for pointer wrapping even without JSON serialization.
        imports.push("\"unsafe\"");
    }
    if !api.errors.is_empty() {
        imports.insert(1.min(imports.len()), "\"errors\"");
    }
    writeln!(
        out,
        "import (\n{}\n)\n",
        imports
            .iter()
            .map(|i| format!("    {}", i))
            .collect::<Vec<_>>()
            .join("\n")
    )
    .ok();

    // Error helper functions
    writeln!(out, "{}\n", gen_last_error_helper(ffi_prefix)).ok();

    // Generate error types (sentinel errors + structured error type)
    for error in &api.errors {
        writeln!(out, "{}\n", alef_codegen::error_gen::gen_go_error_types(error)).ok();
    }

    // Generate enum types and constants
    // Only unit enums map to `type X string` — data enums are generated as Go structs below.
    let unit_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();
    for enum_def in &api.enums {
        writeln!(out, "{}\n", gen_enum_type(enum_def)).ok();
    }

    // Error type names that are also opaque types — in this case the error struct emitted by
    // gen_go_error_types is the Go-side type and the opaque handle definition below would be a
    // duplicate. Skip re-generating the struct for such opaque types; the Free() method is still
    // generated separately.
    let error_names: std::collections::HashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

    // Collect opaque type names — these are pointer-wrapped handles, not JSON-serializable structs.
    let opaque_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();

    // Generate struct types
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque {
            // If an error type has the same name as this opaque type, the structured error
            // struct was already emitted by gen_go_error_types. Skip the duplicate struct
            // definition but still emit the Free() method.
            if error_names.contains(typ.name.as_str()) {
                writeln!(out, "{}\n", gen_opaque_type_free_only(typ, ffi_prefix)).ok();
            } else {
                writeln!(out, "{}\n", gen_opaque_type(typ, ffi_prefix)).ok();
            }
        } else {
            writeln!(out, "{}\n", gen_struct_type(typ, &unit_enum_names)).ok();
            // Generate functional options pattern if type has defaults.
            // Skip "Update" types (e.g., ConversionOptionsUpdate) — they are partial update
            // structs that share field names with the primary config type, producing duplicate
            // With* function declarations.
            if typ.has_default && !typ.name.ends_with("Update") {
                writeln!(out, "{}\n", gen_config_options(typ, &unit_enum_names)).ok();
            }
        }
    }

    // Generate free function wrappers.
    // Async functions are included — the underlying FFI uses block_on() for synchronous C calls.
    for func in &api.functions {
        writeln!(out, "{}\n", gen_function_wrapper(func, ffi_prefix, &opaque_names)).ok();
    }

    // Generate struct methods.
    // Skip static methods that return Named types (e.g., Default() constructors) —
    // these are redundant with the generated New*() functional options constructors,
    // and the opaque handle conversion pipeline is not yet implemented.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if method.is_static && matches!(method.return_type, TypeRef::Named(_)) {
                continue;
            }
            writeln!(out, "{}\n", gen_method_wrapper(typ, method, ffi_prefix, &opaque_names)).ok();
        }
    }

    out
}

/// Generate the lastError() helper function.
fn gen_last_error_helper(ffi_prefix: &str) -> String {
    // Note: ctx is a borrowed pointer into thread-local storage, NOT a heap allocation.
    // Do NOT call free_string on it — that causes a double-free crash on the next FFI call.
    format!(
        "// lastError retrieves the last error from the FFI layer.\nfunc lastError() error {{\n    \
         code := int32(C.{}_last_error_code())\n    \
         if code == 0 {{\n        return nil\n    }}\n    \
         ctx := C.{}_last_error_context()\n    \
         message := C.GoString(ctx)\n    \
         return fmt.Errorf(\"[%d] %s\", code, message)\n\
         }}",
        ffi_prefix, ffi_prefix
    )
}

/// Generate a Go enum type definition.
///
/// For unit enums (all variants have no fields): generates `type X string` with constants.
/// For data enums (any variant has fields): generates a flattened Go struct with all
/// variant fields collected and deduplicated, using pointer types for fields not present
/// in every variant.
fn gen_enum_type(enum_def: &EnumDef) -> String {
    let is_data_enum = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if is_data_enum {
        gen_data_enum_type(enum_def)
    } else {
        gen_unit_enum_type(enum_def)
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

/// Generate a Go unit enum as `type X string` with const block.
fn gen_unit_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);

    if !enum_def.doc.is_empty() {
        for line in enum_def.doc.lines() {
            writeln!(out, "// {}", line.trim()).ok();
        }
    } else {
        writeln!(out, "// {} is an enumeration type.", enum_def.name).ok();
    }
    writeln!(out, "type {} string", enum_def.name).ok();
    writeln!(out).ok();
    writeln!(out, "const (").ok();

    for variant in &enum_def.variants {
        let const_name = format!("{}{}", enum_def.name, variant.name.to_pascal_case());
        let wire_value = enum_variant_wire_value(variant, enum_def);
        if !variant.doc.is_empty() {
            for line in variant.doc.lines() {
                writeln!(out, "    // {}", line.trim()).ok();
            }
        }
        writeln!(out, "    {} {} = \"{}\"", const_name, enum_def.name, wire_value).ok();
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

    if !enum_def.doc.is_empty() {
        for line in enum_def.doc.lines() {
            writeln!(out, "// {}", line.trim()).ok();
        }
    } else {
        writeln!(
            out,
            "// {} is a tagged union type (discriminated by JSON tag).",
            enum_def.name
        )
        .ok();
    }
    writeln!(out, "// Variants: {}", variant_names.join(", ")).ok();
    writeln!(out, "type {} struct {{", enum_def.name).ok();

    // Emit the serde tag discriminator field first (e.g. `Type string \`json:"type"\``).
    // This ensures round-trip JSON serialization preserves the variant discriminator,
    // which Rust needs to deserialize the correct enum variant.
    if let Some(tag_name) = &enum_def.serde_tag {
        writeln!(out, "    {} string `json:\"{}\"`", to_go_name(tag_name), tag_name).ok();
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
                writeln!(out, "    // {}", line.trim()).ok();
            }
        }
        writeln!(out, "    {} {} `{}`", to_go_name(field_name), field_type, json_tag).ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate a Go opaque handle type wrapping an `unsafe.Pointer`.
///
/// Opaque types are not JSON-serializable — they are raw C pointers passed through
/// the FFI layer. The Go struct holds a pointer and exposes a `Free()` method.
fn gen_opaque_type(typ: &TypeDef, ffi_prefix: &str) -> String {
    let mut out = String::with_capacity(512);
    let type_snake = typ.name.to_snake_case();

    if !typ.doc.is_empty() {
        for line in typ.doc.lines() {
            writeln!(out, "// {}", line.trim()).ok();
        }
    } else {
        writeln!(out, "// {} is an opaque handle type.", typ.name).ok();
    }
    writeln!(out, "type {} struct {{", typ.name).ok();
    writeln!(out, "    ptr unsafe.Pointer").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Free method
    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), typ.name.to_pascal_case());
    writeln!(out, "// Free releases the resources held by this handle.").ok();
    writeln!(out, "func (h *{}) Free() {{", typ.name).ok();
    writeln!(out, "    if h.ptr != nil {{").ok();
    writeln!(
        out,
        "        C.{}_{}_free((*C.{})(h.ptr))",
        ffi_prefix, type_snake, c_type
    )
    .ok();
    writeln!(out, "        h.ptr = nil").ok();
    writeln!(out, "    }}").ok();
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
fn gen_opaque_type_free_only(typ: &TypeDef, _ffi_prefix: &str) -> String {
    // Nothing to emit — the structured error type already has its Error() method and
    // the C-level free function is invoked transparently inside the FFI layer.
    // Returning an empty string avoids a duplicate struct definition and a broken Free().
    let _ = typ;
    String::new()
}

/// Generate a Go struct type definition with json tags for marshaling.
fn gen_struct_type(typ: &TypeDef, enum_names: &std::collections::HashSet<&str>) -> String {
    let mut out = String::with_capacity(1024);

    if !typ.doc.is_empty() {
        for line in typ.doc.lines() {
            writeln!(out, "// {}", line.trim()).ok();
        }
    } else {
        writeln!(out, "// {} is a type.", typ.name).ok();
    }
    writeln!(out, "type {} struct {{", typ.name).ok();

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
                writeln!(out, "    // {}", line.trim()).ok();
            }
        }
        writeln!(out, "    {} {} `{}`", to_go_name(&field.name), field_type, json_tag).ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Returns true if any parameter in the list requires JSON marshaling (non-opaque Named, Vec, or Map).
///
/// Such parameters use `json.Marshal` internally, which is fallible. When the surrounding
/// function has no declared `error_type`, we must still propagate the marshal error rather
/// than panicking — so we synthesize an error return in the generated signature.
fn params_require_marshal(params: &[alef_core::ir::ParamDef], opaque_names: &std::collections::HashSet<&str>) -> bool {
    params.iter().any(|p| match &p.ty {
        TypeRef::Named(name) => !opaque_names.contains(name.as_str()),
        TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        _ => false,
    })
}

/// Generate a wrapper function for a free function.
fn gen_function_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    let func_go_name = to_go_name(&func.name);

    if !func.doc.is_empty() {
        for line in func.doc.lines() {
            writeln!(out, "// {}", line.trim()).ok();
        }
    } else {
        writeln!(out, "// {} calls the FFI function.", func_go_name).ok();
    }

    // A function that marshals parameters to JSON can fail even without a declared error_type.
    // Synthesize an error return in those cases so we never panic on marshal failure.
    let marshals_params = params_require_marshal(&func.params, opaque_names);
    let can_return_error = func.error_type.is_some() || marshals_params;

    let return_type = if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            format!("(*{}, error)", go_type(&func.return_type))
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        format!("*{}", go_type(&func.return_type))
    };

    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);

    write!(out, "func {}(", func_go_name).ok();

    // All optional params (wherever they appear) are represented as pointer types in the Go
    // signature so callers can pass nil to omit them.  This is simpler and more correct than
    // the earlier variadic approach which broke when more than one trailing optional existed.
    let mut param_strs: Vec<String> = Vec::new();
    for p in func.params.iter() {
        let param_type: String = if p.optional {
            go_optional_type(&p.ty).into_owned()
        } else if let TypeRef::Named(name) = &p.ty {
            if opaque_names.contains(name.as_str()) {
                // Opaque types are pointer wrappers — accept as pointer
                format!("*{}", go_type(&p.ty))
            } else {
                go_type(&p.ty).into_owned()
            }
        } else {
            go_type(&p.ty).into_owned()
        };
        param_strs.push(format!("{} {}", p.name, param_type));
    }
    write!(out, "{}", param_strs.join(", ")).ok();

    if return_type.is_empty() {
        writeln!(out, ") {{").ok();
    } else {
        writeln!(out, ") {} {{", return_type).ok();
    }

    // Convert parameters
    // Note: can_return_error is set above (includes synthesized error for marshal-requiring params).
    let returns_value_and_error = can_return_error && !matches!(func.return_type, TypeRef::Unit);
    for param in func.params.iter() {
        write!(
            out,
            "{}",
            gen_param_to_c(
                param,
                returns_value_and_error,
                can_return_error,
                ffi_prefix,
                opaque_names
            )
        )
        .ok();
    }

    // Build the C call with converted parameters
    let c_params: Vec<String> = func
        .params
        .iter()
        .map(|p| format!("c{}", p.name.to_pascal_case()))
        .collect();

    let c_call = format!("{}({})", ffi_name, c_params.join(", "));

    // Handle result and error.
    // When can_return_error is true (either from declared error_type or synthesized for
    // marshal-requiring params), emit lastError() checks. For synthesized-error functions
    // that have no declared error_type, the FFI call itself never sets a last error, so
    // lastError() will return nil and the return value flows through normally.
    if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            writeln!(out, "    {}", c_call).ok();
            if func.error_type.is_some() {
                writeln!(out, "    return lastError()").ok();
            } else {
                writeln!(out, "    return nil").ok();
            }
        } else {
            writeln!(out, "    ptr := {}", c_call).ok();
            if func.error_type.is_some() {
                writeln!(out, "    if err := lastError(); err != nil {{").ok();
                // Free the pointer if non-nil even on error, to avoid leaks
                if matches!(
                    func.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
                ) {
                    writeln!(out, "        if ptr != nil {{").ok();
                    writeln!(out, "            C.{}_free_string(ptr)", ffi_prefix).ok();
                    writeln!(out, "        }}").ok();
                }
                if let TypeRef::Named(name) = &func.return_type {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "        if ptr != nil {{").ok();
                    writeln!(out, "            C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                    writeln!(out, "        }}").ok();
                }
                writeln!(out, "        return nil, err").ok();
                writeln!(out, "    }}").ok();
            }
            // Free the FFI-allocated string after unmarshaling
            if matches!(
                func.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
            ) {
                writeln!(out, "    defer C.{}_free_string(ptr)", ffi_prefix).ok();
            }
            // For non-opaque Named types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &func.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "    defer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                }
            }
            writeln!(
                out,
                "    return {}, nil",
                go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
            )
            .ok();
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        writeln!(out, "    {}", c_call).ok();
    } else {
        writeln!(out, "    ptr := {}", c_call).ok();
        // Add defer free for C string returns
        if matches!(
            func.return_type,
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
        ) {
            writeln!(out, "    defer C.{}_free_string(ptr)", ffi_prefix).ok();
        }
        // For non-opaque Named types, free the handle after JSON extraction.
        // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
        if let TypeRef::Named(name) = &func.return_type {
            if !opaque_names.contains(name.as_str()) {
                let type_snake = name.to_snake_case();
                writeln!(out, "    defer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
            }
        }
        writeln!(
            out,
            "    return {}",
            go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
        )
        .ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate a wrapper method for a struct method.
fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);

    if !method.doc.is_empty() {
        for line in method.doc.lines() {
            writeln!(out, "// {}", line.trim()).ok();
        }
    } else {
        writeln!(out, "// {} is a method.", method_go_name).ok();
    }

    // A non-opaque, non-static method marshals its receiver to JSON — that is fallible.
    // Also include params that require marshaling.
    let receiver_requires_marshal = !method.is_static && !typ.is_opaque;
    let method_marshals = receiver_requires_marshal || params_require_marshal(&method.params, opaque_names);
    let method_can_return_error = method.error_type.is_some() || method_marshals;

    let return_type = if method_can_return_error {
        if matches!(method.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            format!("(*{}, error)", go_type(&method.return_type))
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        format!("*{}", go_type(&method.return_type))
    };

    let receiver_name = "r";
    let receiver_type = &typ.name;

    // Static methods become package-level functions (no receiver in Go)
    if method.is_static {
        write!(out, "func {}{}(", receiver_type, method_go_name).ok();
    } else {
        write!(out, "func ({} *{}) {}(", receiver_name, receiver_type, method_go_name).ok();
    }

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type: String = if p.optional {
                go_optional_type(&p.ty).into_owned()
            } else if let TypeRef::Named(name) = &p.ty {
                if opaque_names.contains(name.as_str()) {
                    format!("*{}", go_type(&p.ty))
                } else {
                    go_type(&p.ty).into_owned()
                }
            } else {
                go_type(&p.ty).into_owned()
            };
            format!("{} {}", p.name, param_type)
        })
        .collect();
    write!(out, "{}", params.join(", ")).ok();

    if return_type.is_empty() {
        writeln!(out, ") {{").ok();
    } else {
        writeln!(out, ") {} {{", return_type).ok();
    }

    {
        // Synchronous method - just convert params and call FFI
        // Note: method_can_return_error is set above (includes synthesized error for marshal-requiring methods).
        let returns_value_and_error = method_can_return_error && !matches!(method.return_type, TypeRef::Unit);
        for param in &method.params {
            write!(
                out,
                "{}",
                gen_param_to_c(
                    param,
                    returns_value_and_error,
                    method_can_return_error,
                    ffi_prefix,
                    opaque_names
                )
            )
            .ok();
        }

        let c_params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("c{}", p.name.to_pascal_case()))
            .collect();

        let type_snake = typ.name.to_snake_case();
        let method_snake = method.name.to_snake_case();
        let c_call = if method.is_static {
            // Static methods don't pass a receiver
            if c_params.is_empty() {
                format!("C.{}_{}_{}()", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{} ({})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        } else if typ.is_opaque {
            // Opaque types have a ptr field — cast it directly.
            let c_receiver = format!(
                "(*C.{}{})(unsafe.Pointer({}.ptr))",
                ffi_prefix.to_uppercase(),
                typ.name.to_pascal_case(),
                receiver_name
            );
            if c_params.is_empty() {
                format!("C.{}_{}_{} ({})", ffi_prefix, type_snake, method_snake, c_receiver)
            } else {
                format!(
                    "C.{}_{}_{} ({}, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_receiver,
                    c_params.join(", ")
                )
            }
        } else {
            // Non-opaque structs: marshal to JSON, create a temporary handle, use it, and free it.
            let err_prefix = if returns_value_and_error { "nil, " } else { "" };
            // method_can_return_error is always true here (receiver_requires_marshal is true for
            // non-opaque non-static methods), so we always emit fmt.Errorf, never panic.
            let err_action =
                format!("return {err_prefix}fmt.Errorf(\"failed to marshal receiver: %w\", err)");
            writeln!(
                out,
                "    jsonBytesRecv, err := json.Marshal({recv})\n    \
                 if err != nil {{\n        \
                 {err_action}\n    \
                 }}\n    \
                 tmpStrRecv := C.CString(string(jsonBytesRecv))\n    \
                 cRecv := C.{ffi_prefix}_{type_snake}_from_json(tmpStrRecv)\n    \
                 C.free(unsafe.Pointer(tmpStrRecv))\n    \
                 defer C.{ffi_prefix}_{type_snake}_free(cRecv)",
                recv = receiver_name,
                err_action = err_action,
                ffi_prefix = ffi_prefix,
                type_snake = type_snake,
            )
            .ok();
            if c_params.is_empty() {
                format!("C.{}_{}_{} (cRecv)", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{} (cRecv, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        };

        if method_can_return_error {
            if matches!(method.return_type, TypeRef::Unit) {
                writeln!(out, "    {}", c_call).ok();
                if method.error_type.is_some() {
                    writeln!(out, "    return lastError()").ok();
                } else {
                    writeln!(out, "    return nil").ok();
                }
            } else {
                writeln!(out, "    ptr := {}", c_call).ok();
                if method.error_type.is_some() {
                    writeln!(out, "    if err := lastError(); err != nil {{").ok();
                    // Free the pointer if non-nil even on error, to avoid leaks
                    if matches!(
                        method.return_type,
                        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
                    ) {
                        writeln!(out, "        if ptr != nil {{").ok();
                        writeln!(out, "            C.{}_free_string(ptr)", ffi_prefix).ok();
                        writeln!(out, "        }}").ok();
                    }
                    writeln!(out, "        return nil, err").ok();
                    writeln!(out, "    }}").ok();
                }
                // Free the FFI-allocated string after unmarshaling
                if matches!(
                    method.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
                ) {
                    writeln!(out, "    defer C.{}_free_string(ptr)", ffi_prefix).ok();
                }
                // For non-opaque Named return types, free the handle after JSON extraction.
                // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
                if let TypeRef::Named(name) = &method.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        writeln!(out, "    defer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                    }
                }
                writeln!(
                    out,
                    "    return {}, nil",
                    go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names)
                )
                .ok();
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "    {}", c_call).ok();
        } else {
            writeln!(out, "    ptr := {}", c_call).ok();
            // Add defer free for C string returns
            if matches!(
                method.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
            ) {
                writeln!(out, "    defer C.{}_free_string(ptr)", ffi_prefix).ok();
            }
            // For non-opaque Named return types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &method.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "    defer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                }
            }
            writeln!(
                out,
                "    return {}",
                go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names)
            )
            .ok();
        }
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate parameter conversion code from Go to C.
/// `returns_value_and_error` should be true when the enclosing function returns `(*T, error)`,
/// so that error paths emit `return nil, fmt.Errorf(...)` instead of `return fmt.Errorf(...)`.
/// `can_return_error` should be true when the enclosing function has `error` in its return type.
/// When false, marshal failures are handled with `panic` since the function signature has no error return.
fn gen_param_to_c(
    param: &alef_core::ir::ParamDef,
    returns_value_and_error: bool,
    can_return_error: bool,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(512);
    let c_name = format!("c{}", param.name.to_pascal_case());
    let err_return_prefix = if returns_value_and_error { "nil, " } else { "" };

    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            if param.optional {
                // Optional string param (ty=String, optional=true): the Go variable holds *string.
                writeln!(
                    out,
                    "    var {c_name} *C.char\n    if {param} != nil {{\n        \
                     {c_name} = C.CString(*{param})\n        defer C.free(unsafe.Pointer({c_name}))\n    \
                     }}",
                    c_name = c_name,
                    param = param.name,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "    {} := C.CString({})\n    defer C.free(unsafe.Pointer({}))",
                    c_name, param.name, c_name
                )
                .ok();
            }
        }
        TypeRef::Path => {
            if param.optional {
                writeln!(
                    out,
                    "    var {c_name} *C.char\n    if {param} != nil {{\n        \
                     {c_name} = C.CString(*{param})\n        defer C.free(unsafe.Pointer({c_name}))\n    \
                     }}",
                    c_name = c_name,
                    param = param.name,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "    {} := C.CString({})\n    defer C.free(unsafe.Pointer({}))",
                    c_name, param.name, c_name
                )
                .ok();
            }
        }
        TypeRef::Bytes => {
            writeln!(out, "    {} := (*C.uchar)(unsafe.Pointer(&{}[0]))", c_name, param.name).ok();
        }
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // Opaque types are pointer wrappers — cast the raw pointer to the C type.
                let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name.to_pascal_case());
                writeln!(
                    out,
                    "    {c_name} := (*C.{c_type})(unsafe.Pointer({param}.ptr))",
                    param = param.name,
                )
                .ok();
            } else {
                // Non-opaque Named types: marshal to JSON, create a handle via _from_json,
                // and pass that to the C function.
                let type_snake = name.to_snake_case();
                let err_action = if can_return_error {
                    format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
                } else {
                    "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
                };
                writeln!(
                    out,
                    "    jsonBytes{c_name}, err := json.Marshal({param})\n    \
                     if err != nil {{\n        \
                     {err_action}\n    \
                     }}\n    \
                     tmpStr{c_name} := C.CString(string(jsonBytes{c_name}))\n    \
                     {c_name} := C.{ffi_prefix}_{type_snake}_from_json(tmpStr{c_name})\n    \
                     C.free(unsafe.Pointer(tmpStr{c_name}))\n    \
                     defer C.{ffi_prefix}_{type_snake}_free({c_name})",
                    c_name = c_name,
                    param = param.name,
                    err_action = err_action,
                    ffi_prefix = ffi_prefix,
                    type_snake = type_snake,
                )
                .ok();
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Vec and Map types are serialized as JSON strings across the FFI boundary.
            let err_action = if can_return_error {
                format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
            } else {
                "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
            };
            writeln!(
                out,
                "    jsonBytes{c_name}, err := json.Marshal({param})\n    \
                 if err != nil {{\n        \
                 {err_action}\n    \
                 }}\n    \
                 {c_name} := C.CString(string(jsonBytes{c_name}))\n    \
                 defer C.free(unsafe.Pointer({c_name}))",
                c_name = c_name,
                param = param.name,
                err_action = err_action,
            )
            .ok();
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(
                        out,
                        "    var {} *C.char\n    if {} != nil {{\n        \
                         {} = C.CString(*{})\n        defer C.free(unsafe.Pointer({}))\n    \
                         }}",
                        c_name, param.name, c_name, param.name, c_name
                    )
                    .ok();
                }
                TypeRef::Named(name) if opaque_names.contains(name.as_str()) => {
                    // Optional opaque type: cast the raw pointer to the C type or pass nil.
                    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name.to_pascal_case());
                    writeln!(
                        out,
                        "    var {c_name} *C.{c_type}\n    if {param} != nil {{\n        \
                         {c_name} = (*C.{c_type})(unsafe.Pointer({param}.ptr))\n    \
                         }}",
                        c_name = c_name,
                        c_type = c_type,
                        param = param.name,
                    )
                    .ok();
                }
                TypeRef::Named(_) => {
                    writeln!(
                        out,
                        "    var {} *C.char\n    if {} != nil {{\n        \
                         jsonBytes, _ := json.Marshal({})\n        \
                         {} = C.CString(string(jsonBytes))\n        \
                         defer C.free(unsafe.Pointer({}))\n    \
                         }}",
                        c_name, param.name, param.name, c_name, c_name
                    )
                    .ok();
                }
                _ => {
                    // For other optional types, just pass nil or default
                    writeln!(out, "    var {} *C.char", c_name).ok();
                }
            }
        }
        TypeRef::Primitive(prim) if param.optional => {
            // Optional primitive: the Go param is a pointer (*T). Dereference it if non-nil,
            // otherwise pass the max-value sentinel (e.g. u64::MAX) so the FFI layer knows
            // the parameter was omitted.
            //
            // Declare the variable using the CGo type (e.g. C.uint64_t) so that CGo does
            // not reject the value when it is passed directly to the C function. Go's native
            // numeric types (uint64, uint32, …) are distinct from CGo types and cannot be
            // passed without an explicit cast — using the CGo type at declaration avoids a
            // second cast at every call-site.
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            let sentinel = primitive_max_sentinel(prim);
            writeln!(
                out,
                "    var {c_name} {cgo_ty} = {cgo_ty}({sentinel})\n    if {param} != nil {{\n        \
                 {c_name} = {cgo_ty}({go_ty}(*{param}))\n    }}",
                c_name = c_name,
                cgo_ty = cgo_ty,
                go_ty = go_ty,
                sentinel = sentinel,
                param = param.name,
            )
            .ok();
        }
        _ => {
            // Primitives and other types pass through directly
        }
    }

    if !out.is_empty() {
        writeln!(out).ok();
    }
    out
}

/// Return the CGo type name for a primitive type (e.g. `PrimitiveType::U64` → `"C.uint64_t"`).
///
/// CGo treats Go native types (`uint64`, `uint32`, …) and the corresponding C typedefs
/// (`C.uint64_t`, `C.uint32_t`, …) as distinct and will not implicitly convert between them
/// when passing values to C functions. Declaring optional-primitive temporaries with the CGo
/// type avoids an explicit cast at every call-site.
fn cgo_type_for_primitive(prim: &alef_core::ir::PrimitiveType) -> &'static str {
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
        PrimitiveType::Bool => "C.uchar",
    }
}

/// Return the Go expression for the maximum value of a primitive type, used as a sentinel
/// to signal "None" to FFI functions that use max-value sentinels for optional primitives.
fn primitive_max_sentinel(prim: &alef_core::ir::PrimitiveType) -> &'static str {
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
fn type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named(n) => n.to_pascal_case(),
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
fn go_return_expr(
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
                format!(
                    "&{go_type}{{ptr: unsafe.Pointer({var_name})}}",
                    go_type = name.to_pascal_case(),
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
                    go_type = name.to_pascal_case(),
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
            format!("unmarshalJSON({})", var_name)
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
fn gen_config_options(typ: &TypeDef, enum_names: &std::collections::HashSet<&str>) -> String {
    let mut out = String::with_capacity(2048);

    // ConfigOption type definition
    writeln!(out, "// {} option function", typ.name).ok();
    writeln!(out, "type {}Option func(*{})", typ.name, typ.name).ok();
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
            typ.name, field_go_name, field.name
        )
        .ok();
        writeln!(
            out,
            "func With{}{}(v {}) {}Option {{",
            typ.name, field_go_name, param_type, typ.name
        )
        .ok();
        // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults) both
        // store pointer types in the struct, so we must take the address of v when assigning.
        let use_ptr = field.optional || needs_omitempty_pointer(field);
        let assign_val = if use_ptr { "&v" } else { "v" };
        writeln!(
            out,
            "    return func(c *{}) {{ c.{} = {} }}",
            typ.name, field_go_name, assign_val
        )
        .ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
    }

    // Generate NewConfig constructor
    writeln!(
        out,
        "// New{} creates a {} with optional parameters.",
        typ.name, typ.name
    )
    .ok();
    writeln!(out, "func New{}(opts ...{}Option) *{} {{", typ.name, typ.name, typ.name).ok();
    writeln!(out, "    c := &{} {{", typ.name).ok();

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
                        val = format!("{}{{}}", name);
                    }
                }
            }
            val
        };
        writeln!(out, "        {}: {},", field_go_name, default_val).ok();
    }

    writeln!(out, "    }}").ok();
    writeln!(out, "    for _, opt := range opts {{").ok();
    writeln!(out, "        opt(c)").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "    return c").ok();
    writeln!(out, "}}").ok();

    out
}
