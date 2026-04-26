use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_param_name, go_type_name, to_go_name};
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AdapterPattern, AlefConfig, Language, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, DefaultValue, EnumDef, FieldDef, FunctionDef, MethodDef, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::HashSet;
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
        // Collect bridge param names from trait_bridges config so we can strip them
        // from generated function signatures and emit ConvertWithVisitor instead.
        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        // Also collect type aliases used as bridge params (e.g. "VisitorHandle").
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        // Determine if any bridge is configured for the visitor pattern.
        // Requires both trait_bridges to be present AND visitor_callbacks = true in [ffi].
        let visitor_callbacks_enabled = config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks);
        let has_visitor_bridge = !config.trait_bridges.is_empty() && visitor_callbacks_enabled;

        // Collect streaming adapter method names — their FFI signature uses callbacks
        // which Go's CGO wrappers can't call directly.
        let streaming_methods: HashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .map(|a| a.name.clone())
            .collect();

        // Collect functions excluded from FFI generation. Go bindings call C symbols directly
        // via cgo, so any function excluded from the FFI header must also be excluded here.
        let ffi_exclude_functions: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        let content = format_go_code(&strip_trailing_whitespace(&gen_go_file(
            api,
            &ffi_prefix,
            &pkg_name,
            &ffi_lib_name,
            &ffi_header,
            &ffi_crate_dir,
            &output_dir,
            &bridge_param_names,
            &bridge_type_aliases,
            &streaming_methods,
            &ffi_exclude_functions,
        )));

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Go)?;

        // Compute relative path from Go output dir to project root.
        let depth = output_dir.trim_end_matches('/').matches('/').count() + 1;
        let to_root = "../".repeat(depth);

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("binding.go"),
            content,
            generated_header: true,
        }];

        // Generate visitor.go when a visitor bridge is configured.
        if has_visitor_bridge {
            let visitor_content = strip_trailing_whitespace(&crate::gen_visitor::gen_visitor_file(
                &pkg_name,
                &ffi_prefix,
                &ffi_header,
                &ffi_crate_dir,
                &to_root,
            ));
            files.push(GeneratedFile {
                path: PathBuf::from(&output_dir).join("visitor.go"),
                content: visitor_content,
                generated_header: true,
            });

            // Generate trait_bridges.go only for plugin-style bridges that have a register_fn.
            // Per-call bridges (no register_fn) use visitor.go callbacks via convert() instead.
            let has_plugin_bridges = config.trait_bridges.iter().any(|b| b.register_fn.is_some());
            if has_plugin_bridges {
                let trait_bridges_content = strip_trailing_whitespace(&super::trait_bridge::gen_trait_bridges_file(
                    api,
                    config,
                    &pkg_name,
                    &ffi_prefix,
                    &ffi_header,
                    &ffi_crate_dir,
                    &to_root,
                    &config.crate_config.name,
                ));
                if !trait_bridges_content.trim().is_empty() && trait_bridges_content.len() > 100 {
                    files.push(GeneratedFile {
                        path: PathBuf::from(&output_dir).join("trait_bridges.go"),
                        content: trait_bridges_content,
                        generated_header: true,
                    });
                }
            }
        }

        Ok(files)
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
            build_dep: BuildDependency::Ffi,
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

/// Run `gofmt -s` on generated Go code. Falls back to the original if gofmt is unavailable.
fn format_go_code(code: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let child = Command::new("gofmt")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut c) => {
            if let Some(ref mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(code.as_bytes());
            }
            match c.wait_with_output() {
                Ok(output) if output.status.success() => {
                    String::from_utf8(output.stdout).unwrap_or_else(|_| code.to_string())
                }
                _ => code.to_string(),
            }
        }
        Err(_) => code.to_string(),
    }
}

/// Returns true if a `TypeRef::Named` type comes from `api.enums` (either unit or data enum)
/// and therefore does not have `_from_json`/`_to_json`/`_free` FFI helpers.
///
/// Only types in `api.types` (non-opaque struct types) have these helpers in the C header.
fn is_ffi_enum_type(name: &str, ffi_enum_names: &HashSet<String>) -> bool {
    ffi_enum_names.contains(name)
}

/// Returns true if a function references an enum type (from `api.enums`) as a parameter type
/// or return type, for which the FFI header lacks `_from_json`/`_to_json`/`_free` helpers.
///
/// Such functions cannot be generated correctly and must be skipped.
fn uses_ffi_enum_type(
    func_params: &[alef_core::ir::ParamDef],
    return_type: &TypeRef,
    ffi_enum_names: &HashSet<String>,
    opaque_names: &std::collections::HashSet<&str>,
) -> bool {
    let named_is_problem = |n: &str| is_ffi_enum_type(n, ffi_enum_names) && !opaque_names.contains(n);
    let return_uses = match return_type {
        TypeRef::Named(n) => named_is_problem(n),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if named_is_problem(n)),
        _ => false,
    };
    if return_uses {
        return true;
    }
    func_params.iter().any(|p| match &p.ty {
        TypeRef::Named(n) => named_is_problem(n),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if named_is_problem(n)),
        _ => false,
    })
}

/// Generate the complete Go binding file wrapping the C FFI layer.
#[allow(clippy::too_many_arguments)]
fn gen_go_file(
    api: &ApiSurface,
    ffi_prefix: &str,
    pkg_name: &str,
    ffi_lib_name: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    go_output_dir: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    streaming_methods: &HashSet<String>,
    ffi_exclude_functions: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(4096);

    // Go convention: generated file marker must appear before package declaration.
    // Blank line after header prevents revive from treating it as package doc.
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push('\n');

    // Compute relative path from Go output dir to project root.
    // go_output_dir is like "packages/go/", so we need "../../" to reach root.
    let depth = go_output_dir.trim_end_matches('/').matches('/').count() + 1;
    let to_root = "../".repeat(depth);

    // Package header and cgo directives.
    // The package comment must immediately precede the package declaration with no blank line.
    writeln!(
        out,
        "// Package {} provides Go bindings for the liter-llm library.",
        pkg_name
    )
    .ok();
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
            .map(|i| format!("\t{}", i))
            .collect::<Vec<_>>()
            .join("\n")
    )
    .ok();

    // Error helper functions
    writeln!(out, "{}\n", gen_last_error_helper(ffi_prefix)).ok();

    // Generate error types (sentinel errors + structured error type)
    for error in &api.errors {
        writeln!(
            out,
            "{}\n",
            alef_codegen::error_gen::gen_go_error_types(error, pkg_name)
        )
        .ok();
    }

    // When a visitor bridge is active, visitor.go defines NodeContext and VisitResult
    // with FFI-compatible fields. Skip them in binding.go to avoid redeclarations.
    let visitor_types: std::collections::HashSet<&str> = if !bridge_param_names.is_empty() {
        ["NodeContext", "VisitResult"].into_iter().collect()
    } else {
        std::collections::HashSet::new()
    };

    // Generate enum types and constants
    // Only unit enums map to `type X string` — data enums are generated as Go structs below.
    let unit_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();
    for enum_def in api.enums.iter().filter(|e| !visitor_types.contains(e.name.as_str())) {
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

    // Collect all enum type names (both unit and data enums from api.enums).
    // These types do NOT have _from_json/_to_json/_free helpers in the FFI header —
    // only non-opaque api.types have those helpers. Functions that use an enum type
    // as a parameter or return value (via TypeRef::Named) cannot be correctly generated
    // (unless the type also appears as an opaque type in api.types) and are excluded.
    let ffi_enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Generate struct types
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !visitor_types.contains(typ.name.as_str()))
    {
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
    // Skip functions excluded from FFI generation (their C symbols don't exist in the header)
    // and functions whose parameter or return types are enum types without FFI JSON helpers.
    for func in api.functions.iter().filter(|f| {
        !ffi_exclude_functions.contains(&f.name)
            && !uses_ffi_enum_type(&f.params, &f.return_type, &ffi_enum_names, &opaque_names)
    }) {
        writeln!(
            out,
            "{}\n",
            gen_function_wrapper(func, ffi_prefix, &opaque_names, bridge_param_names, bridge_type_aliases)
        )
        .ok();
    }

    // Generate struct methods.
    // Skip static methods that return Named types (e.g., Default() constructors) —
    // these are redundant with the generated New*() functional options constructors,
    // and the opaque handle conversion pipeline is not yet implemented.
    // Streaming adapter methods use a callback-based C signature that CGO can't call directly —
    // they are skipped here and must be implemented via a separate Go-native streaming API.
    // Also skip methods excluded from FFI or using enum types without FFI JSON helpers.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if method.is_static && matches!(method.return_type, TypeRef::Named(_)) {
                continue;
            }
            if streaming_methods.contains(&method.name) {
                continue;
            }
            if ffi_exclude_functions.contains(&method.name) {
                continue;
            }
            if uses_ffi_enum_type(&method.params, &method.return_type, &ffi_enum_names, &opaque_names) {
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
fn emit_type_doc(out: &mut String, type_name: &str, doc: &str, fallback: &str) {
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
fn gen_opaque_type(typ: &TypeDef, ffi_prefix: &str) -> String {
    let mut out = String::with_capacity(512);
    let type_snake = typ.name.to_snake_case();
    let go_name = go_type_name(&typ.name);

    emit_type_doc(&mut out, &go_name, &typ.doc, "is an opaque handle type.");
    writeln!(out, "type {} struct {{", go_name).ok();
    writeln!(out, "\tptr unsafe.Pointer").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Free method
    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), typ.name.to_pascal_case());
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

/// Returns true when `param` is a visitor bridge parameter that should be stripped from the
/// generated Go function signature and replaced with a nil argument to the C function.
fn is_bridge_param(
    param: &alef_core::ir::ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    if bridge_param_names.contains(param.name.as_str()) {
        return true;
    }
    let type_name = match &param.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                Some(n.as_str())
            } else {
                None
            }
        }
        _ => None,
    };
    type_name.is_some_and(|n| bridge_type_aliases.contains(n))
}

/// Generate a wrapper function for a free function.
fn gen_function_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    let func_go_name = to_go_name(&func.name);

    emit_type_doc(&mut out, &func_go_name, &func.doc, "calls the FFI function.");

    // A function that marshals parameters to JSON can fail even without a declared error_type.
    // Synthesize an error return in those cases so we never panic on marshal failure.
    // Exclude bridge params — they are not marshalled (they're passed as nil).
    let non_bridge_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();
    let marshals_params = params_require_marshal(&non_bridge_params, opaque_names);
    let can_return_error = func.error_type.is_some() || marshals_params;

    let return_type = if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            format!("({}, error)", go_optional_type(&func.return_type))
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        go_optional_type(&func.return_type).into_owned()
    };

    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);

    write!(out, "func {}(", func_go_name).ok();

    // All optional params (wherever they appear) are represented as pointer types in the Go
    // signature so callers can pass nil to omit them.  This is simpler and more correct than
    // the earlier variadic approach which broke when more than one trailing optional existed.
    // Bridge params (visitor handles) are stripped from the public signature — ConvertWithVisitor
    // provides the visitor-accepting variant separately.
    let mut param_strs: Vec<String> = Vec::new();
    for p in func.params.iter() {
        if is_bridge_param(p, bridge_param_names, bridge_type_aliases) {
            continue;
        }
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
        param_strs.push(format!("{} {}", go_param_name(&p.name), param_type));
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
        if is_bridge_param(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
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

    // Build the C call with converted parameters.
    // Bridge params that are sanitized (unknown type in IR) are omitted from the C call — the
    // FFI backend strips them from the generated C function signature entirely and handles the
    // visitor path via a separate {prefix}_convert_with_visitor function.
    // Non-sanitized bridge params pass nil (no visitor) in the plain Convert().
    // Bytes params expand to two C arguments: the pointer and the length.
    let c_params: Vec<String> = func
        .params
        .iter()
        .flat_map(|p| -> Vec<String> {
            if is_bridge_param(p, bridge_param_names, bridge_type_aliases) {
                // Sanitized bridge params have been removed from the C function signature;
                // do not emit a nil slot for them.
                if p.sanitized { vec![] } else { vec!["nil".to_string()] }
            } else {
                let c_name = go_param_name(&format!("c_{}", p.name));
                if matches!(p.ty, TypeRef::Bytes) {
                    vec![c_name.clone(), format!("{}Len", c_name)]
                } else {
                    vec![c_name]
                }
            }
        })
        .collect();

    let c_call = format!("{}({})", ffi_name, c_params.join(", "));

    // Handle result and error.
    // When can_return_error is true (either from declared error_type or synthesized for
    // marshal-requiring params), emit lastError() checks. For synthesized-error functions
    // that have no declared error_type, the FFI call itself never sets a last error, so
    // lastError() will return nil and the return value flows through normally.
    if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            writeln!(out, "\t{}", c_call).ok();
            if func.error_type.is_some() {
                writeln!(out, "\treturn lastError()").ok();
            } else {
                writeln!(out, "\treturn nil").ok();
            }
        } else {
            writeln!(out, "\tptr := {}", c_call).ok();
            if func.error_type.is_some() {
                writeln!(out, "\tif err := lastError(); err != nil {{").ok();
                // Free the pointer if non-nil even on error, to avoid leaks
                if matches!(
                    func.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
                ) {
                    writeln!(out, "\t\tif ptr != nil {{").ok();
                    writeln!(out, "\t\t\tC.{}_free_string(ptr)", ffi_prefix).ok();
                    writeln!(out, "\t\t}}").ok();
                }
                if let TypeRef::Named(name) = &func.return_type {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "\t\tif ptr != nil {{").ok();
                    writeln!(out, "\t\t\tC.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                    writeln!(out, "\t\t}}").ok();
                }
                writeln!(out, "\t\treturn nil, err").ok();
                writeln!(out, "\t}}").ok();
            }
            // Free the FFI-allocated string after unmarshaling
            if matches!(
                func.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
            ) {
                writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
            }
            // For non-opaque Named types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &func.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                }
            }
            writeln!(
                out,
                "\treturn {}, nil",
                go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
            )
            .ok();
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        writeln!(out, "\t{}", c_call).ok();
    } else {
        writeln!(out, "\tptr := {}", c_call).ok();
        // Add defer free for C string returns
        if matches!(
            func.return_type,
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
        ) {
            writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
        }
        // For non-opaque Named types, free the handle after JSON extraction.
        // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
        if let TypeRef::Named(name) = &func.return_type {
            if !opaque_names.contains(name.as_str()) {
                let type_snake = name.to_snake_case();
                writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
            }
        }
        writeln!(
            out,
            "\treturn {}",
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

    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a method.");

    // A non-opaque, non-static method marshals its receiver to JSON — that is fallible.
    // Also include params that require marshaling.
    let receiver_requires_marshal = !method.is_static && !typ.is_opaque;
    let method_marshals = receiver_requires_marshal || params_require_marshal(&method.params, opaque_names);
    let method_can_return_error = method.error_type.is_some() || method_marshals;

    let return_type = if method_can_return_error {
        if matches!(method.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            format!("({}, error)", go_optional_type(&method.return_type))
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        go_optional_type(&method.return_type).into_owned()
    };

    // Opaque types use "h" (for "handle") to match the receiver name in Free().
    // Non-opaque types use "r" (for "receiver").
    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);

    // Static methods become package-level functions (no receiver in Go)
    if method.is_static {
        write!(out, "func {}{}(", go_receiver_type, method_go_name).ok();
    } else {
        write!(
            out,
            "func ({} *{}) {}(",
            receiver_name, go_receiver_type, method_go_name
        )
        .ok();
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
            format!("{} {}", go_param_name(&p.name), param_type)
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

        // Bytes params expand to two C arguments: the pointer and the length.
        let c_params: Vec<String> = method
            .params
            .iter()
            .flat_map(|p| -> Vec<String> {
                let c_name = go_param_name(&format!("c_{}", p.name));
                if matches!(p.ty, TypeRef::Bytes) {
                    vec![c_name.clone(), format!("{}Len", c_name)]
                } else {
                    vec![c_name]
                }
            })
            .collect();

        let type_snake = typ.name.to_snake_case();
        let method_snake = method.name.to_snake_case();
        let c_call = if method.is_static {
            // Static methods don't pass a receiver
            if c_params.is_empty() {
                format!("C.{}_{}_{}()", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{}({})",
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
                format!("C.{}_{}_{}({})", ffi_prefix, type_snake, method_snake, c_receiver)
            } else {
                format!(
                    "C.{}_{}_{}({}, {})",
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
            let err_action = format!("return {err_prefix}fmt.Errorf(\"failed to marshal receiver: %w\", err)");
            writeln!(
                out,
                "\tjsonBytesRecv, err := json.Marshal({recv})\n\t\
                 if err != nil {{\n\t\t\
                 {err_action}\n\t\
                 }}\n\t\
                 tmpStrRecv := C.CString(string(jsonBytesRecv))\n\t\
                 cRecv := C.{ffi_prefix}_{type_snake}_from_json(tmpStrRecv)\n\t\
                 C.free(unsafe.Pointer(tmpStrRecv))\n\t\
                 defer C.{ffi_prefix}_{type_snake}_free(cRecv)",
                recv = receiver_name,
                err_action = err_action,
                ffi_prefix = ffi_prefix,
                type_snake = type_snake,
            )
            .ok();
            if c_params.is_empty() {
                format!("C.{}_{}_{}(cRecv)", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{}(cRecv, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        };

        // Detect builder pattern: opaque type method that returns the same opaque type.
        // The C function consumes (Box::from_raw) the input pointer and returns a new pointer.
        // Instead of creating a new Go struct, update r.ptr so the caller's handle stays valid.
        let is_builder_return =
            typ.is_opaque && matches!(&method.return_type, TypeRef::Named(n) if n.as_str() == typ.name.as_str());

        if method_can_return_error {
            if matches!(method.return_type, TypeRef::Unit) {
                writeln!(out, "\t{}", c_call).ok();
                // For non-opaque, non-static methods with Unit return, the C function may have
                // mutated cRecv in place (e.g. apply_update).  Write the updated state back to
                // the Go receiver so the mutation is visible to the caller.
                if !method.is_static && !typ.is_opaque {
                    writeln!(
                        out,
                        "\tjsonPtrUpdated := C.{ffi_prefix}_{type_snake}_to_json(cRecv)\n\t\
                         if jsonPtrUpdated != nil {{\n\t\t\
                         _ = json.Unmarshal([]byte(C.GoString(jsonPtrUpdated)), {recv})\n\t\t\
                         C.{ffi_prefix}_free_string(jsonPtrUpdated)\n\t\
                         }}",
                        ffi_prefix = ffi_prefix,
                        type_snake = type_snake,
                        recv = receiver_name,
                    )
                    .ok();
                }
                if method.error_type.is_some() {
                    writeln!(out, "\treturn lastError()").ok();
                } else {
                    writeln!(out, "\treturn nil").ok();
                }
            } else {
                writeln!(out, "\tptr := {}", c_call).ok();
                if method.error_type.is_some() {
                    writeln!(out, "\tif err := lastError(); err != nil {{").ok();
                    // Free the pointer if non-nil even on error, to avoid leaks
                    if matches!(
                        method.return_type,
                        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
                    ) {
                        writeln!(out, "\t\tif ptr != nil {{").ok();
                        writeln!(out, "\t\t\tC.{}_free_string(ptr)", ffi_prefix).ok();
                        writeln!(out, "\t\t}}").ok();
                    }
                    writeln!(out, "\t\treturn nil, err").ok();
                    writeln!(out, "\t}}").ok();
                }
                // Free the FFI-allocated string after unmarshaling
                if matches!(
                    method.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
                ) {
                    writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
                }
                // For non-opaque Named return types, free the handle after JSON extraction.
                // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
                if let TypeRef::Named(name) = &method.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                    }
                }
                if is_builder_return {
                    // Builder pattern: C consumed the old pointer and returned a new one.
                    // Update r.ptr in-place so the caller's handle remains valid.
                    writeln!(out, "\t{}.ptr = unsafe.Pointer(ptr)", receiver_name).ok();
                    writeln!(out, "\treturn {}, nil", receiver_name).ok();
                } else {
                    writeln!(
                        out,
                        "\treturn {}, nil",
                        go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names)
                    )
                    .ok();
                }
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "\t{}", c_call).ok();
        } else {
            writeln!(out, "\tptr := {}", c_call).ok();
            // Add defer free for C string returns
            if matches!(
                method.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes
            ) {
                writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
            }
            // For non-opaque Named return types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &method.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                }
            }
            if is_builder_return {
                // Builder pattern: C consumed the old pointer and returned a new one.
                // Update r.ptr in-place so the caller's handle remains valid.
                writeln!(out, "\t{}.ptr = unsafe.Pointer(ptr)", receiver_name).ok();
                writeln!(out, "\treturn {}", receiver_name).ok();
            } else {
                writeln!(
                    out,
                    "\treturn {}",
                    go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names)
                )
                .ok();
            }
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
    // Go param names must be lowerCamelCase (no underscores), and internal C-side
    // temporaries use the same stem with acronym uppercasing applied.
    let go_param = go_param_name(&param.name);
    let c_name = go_param_name(&format!("c_{}", param.name));
    let err_return_prefix = if returns_value_and_error { "nil, " } else { "" };

    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            if param.optional {
                // Optional string param (ty=String, optional=true): the Go variable holds *string.
                writeln!(
                    out,
                    "\tvar {c_name} *C.char\n\tif {param} != nil {{\n\t\t\
                     {c_name} = C.CString(*{param})\n\t\tdefer C.free(unsafe.Pointer({c_name}))\n\t\
                     }}",
                    c_name = c_name,
                    param = go_param,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "\t{} := C.CString({})\n\tdefer C.free(unsafe.Pointer({}))",
                    c_name, go_param, c_name
                )
                .ok();
            }
        }
        TypeRef::Path => {
            if param.optional {
                writeln!(
                    out,
                    "\tvar {c_name} *C.char\n\tif {param} != nil {{\n\t\t\
                     {c_name} = C.CString(*{param})\n\t\tdefer C.free(unsafe.Pointer({c_name}))\n\t\
                     }}",
                    c_name = c_name,
                    param = go_param,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "\t{} := C.CString({})\n\tdefer C.free(unsafe.Pointer({}))",
                    c_name, go_param, c_name
                )
                .ok();
            }
        }
        TypeRef::Bytes => {
            writeln!(out, "\t{} := (*C.uint8_t)(unsafe.Pointer(&{}[0]))", c_name, go_param).ok();
            writeln!(out, "\t{}Len := C.uintptr_t(len({}))", c_name, go_param).ok();
        }
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // Opaque types are pointer wrappers — cast the raw pointer to the C type.
                let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name.to_pascal_case());
                writeln!(
                    out,
                    "\t{c_name} := (*C.{c_type})(unsafe.Pointer({param}.ptr))",
                    param = go_param,
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
                    "\tjsonBytes{c_name}, err := json.Marshal({param})\n\t\
                     if err != nil {{\n\t\t\
                     {err_action}\n\t\
                     }}\n\t\
                     tmpStr{c_name} := C.CString(string(jsonBytes{c_name}))\n\t\
                     {c_name} := C.{ffi_prefix}_{type_snake}_from_json(tmpStr{c_name})\n\t\
                     C.free(unsafe.Pointer(tmpStr{c_name}))\n\t\
                     defer C.{ffi_prefix}_{type_snake}_free({c_name})",
                    c_name = c_name,
                    param = go_param,
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
                "\tjsonBytes{c_name}, err := json.Marshal({param})\n\t\
                 if err != nil {{\n\t\t\
                 {err_action}\n\t\
                 }}\n\t\
                 {c_name} := C.CString(string(jsonBytes{c_name}))\n\t\
                 defer C.free(unsafe.Pointer({c_name}))",
                c_name = c_name,
                param = go_param,
                err_action = err_action,
            )
            .ok();
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(
                        out,
                        "\tvar {} *C.char\n\tif {} != nil {{\n\t\t\
                         {} = C.CString(*{})\n\t\tdefer C.free(unsafe.Pointer({}))\n\t\
                         }}",
                        c_name, go_param, c_name, go_param, c_name
                    )
                    .ok();
                }
                TypeRef::Named(name) if opaque_names.contains(name.as_str()) => {
                    // Optional opaque type: cast the raw pointer to the C type or pass nil.
                    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name.to_pascal_case());
                    writeln!(
                        out,
                        "\tvar {c_name} *C.{c_type}\n\tif {param} != nil {{\n\t\t\
                         {c_name} = (*C.{c_type})(unsafe.Pointer({param}.ptr))\n\t\
                         }}",
                        c_name = c_name,
                        c_type = c_type,
                        param = go_param,
                    )
                    .ok();
                }
                TypeRef::Named(_) => {
                    writeln!(
                        out,
                        "\tvar {} *C.char\n\tif {} != nil {{\n\t\t\
                         jsonBytes, _ := json.Marshal({})\n\t\t\
                         {} = C.CString(string(jsonBytes))\n\t\t\
                         defer C.free(unsafe.Pointer({}))\n\t\
                         }}",
                        c_name, go_param, go_param, c_name, c_name
                    )
                    .ok();
                }
                _ => {
                    // For other optional types, just pass nil or default
                    writeln!(out, "\tvar {} *C.char", c_name).ok();
                }
            }
        }
        TypeRef::Primitive(prim) if !param.optional => {
            // Non-optional primitive: cast to the CGo type so the value can be passed directly
            // to C functions that expect C types (e.g., uintptr_t, uint32_t).
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            writeln!(
                out,
                "\t{c_name} := {cgo_ty}({go_ty}({param}))",
                c_name = c_name,
                cgo_ty = cgo_ty,
                go_ty = go_ty,
                param = go_param,
            )
            .ok();
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
                "\tvar {c_name} {cgo_ty} = {cgo_ty}({sentinel})\n\tif {param} != nil {{\n\t\t\
                 {c_name} = {cgo_ty}({go_ty}(*{param}))\n\t}}",
                c_name = c_name,
                cgo_ty = cgo_ty,
                go_ty = go_ty,
                sentinel = sentinel,
                param = go_param,
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
fn gen_config_options(typ: &TypeDef, enum_names: &std::collections::HashSet<&str>) -> String {
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
