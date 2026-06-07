use super::constructors::gen_go_opaque_constructor;
use super::functions::{gen_adapter_wrapper, gen_convert_with_visitor_wrapper, gen_function_wrapper};
use super::methods::{gen_method_wrapper, gen_streaming_method_wrapper};
use super::types::{
    gen_config_options, gen_enum_type, gen_last_error_helper, gen_opaque_type, gen_opaque_type_free_only,
    gen_ptr_helper, gen_struct_type, gen_unmarshal_bytes_helper, is_passthrough_raw_message_enum, is_tuple_field,
};
use crate::core::config::{AdapterPattern, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use std::collections::HashSet;

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
pub(super) fn strip_trailing_whitespace(content: &str) -> String {
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
pub(super) fn format_go_code(code: &str) -> String {
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
pub(super) fn is_ffi_enum_type(name: &str, ffi_enum_names: &HashSet<String>) -> bool {
    ffi_enum_names.contains(name)
}

/// Returns true if a function references a DATA enum type (from `api.enums`) as a parameter type
/// or return type, for which the FFI header lacks `_from_json`/`_to_json`/`_free` helpers.
///
/// Unit-variant enums (in `ffi_param_enum_names`) can be marshaled to/from i32 and do NOT cause
/// skipping. Data enums without those helpers cannot be generated correctly and must be skipped.
fn uses_ffi_enum_type(
    func_params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    ffi_enum_names: &HashSet<String>,
    ffi_param_enum_names: &HashSet<String>,
    opaque_names: &std::collections::HashSet<&str>,
) -> bool {
    let named_is_problem = |n: &str| {
        // Only skip if it's an enum AND not a unit-variant param enum AND not opaque
        is_ffi_enum_type(n, ffi_enum_names) && !ffi_param_enum_names.contains(n) && !opaque_names.contains(n)
    };
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

/// Returns true if a type reference mentions any excluded type.
fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

/// Returns true if any parameter or return type mentions an excluded type.
fn signature_references_excluded_type(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

pub(super) fn find_options_bridge_function<'a>(
    api: &'a ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
) -> Option<&'a crate::core::ir::FunctionDef> {
    api.functions
        .iter()
        .find(|func| options_bridge_function_matches(func, bridge_cfg))
}

fn options_bridge_function_matches(func: &crate::core::ir::FunctionDef, bridge_cfg: &TraitBridgeConfig) -> bool {
    let Some(options_type) = bridge_cfg.options_type.as_deref() else {
        return false;
    };
    func.params
        .iter()
        .any(|param| type_ref_named_type(&param.ty) == Some(options_type))
}

fn type_ref_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => type_ref_named_type(inner),
        _ => None,
    }
}

/// Generate the complete Go binding file wrapping the C FFI layer.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_go_file(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    ffi_prefix: &str,
    pkg_name: &str,
    ffi_lib_name: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    go_output_dir: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    streaming_methods: &std::collections::HashMap<(String, String), String>,
    ffi_exclude_functions: &HashSet<String>,
    exclude_types: &HashSet<String>,
    value_only_types: &HashSet<String>,
    visitor_bridge_cfg: Option<&TraitBridgeConfig>,
) -> String {
    // Two-pass generation: accumulate body content first, then scan for actual usage
    // to determine which imports are truly needed (e.g., runtime.Pinner only appears
    // in bytes_to_c_pointer.jinja when actually emitted for FFI params).

    // Pass 1: Generate header, cgo, and placeholder for imports (to be filled in Pass 2)
    let mut header = String::with_capacity(2048);

    // Go convention: generated file marker must appear before package declaration.
    // Blank line after header prevents revive from treating it as package doc.
    header.push_str(&hash::header(CommentStyle::DoubleSlash));
    header.push('\n');

    // Compute relative path from Go output dir to project root.
    // go_output_dir is like "packages/go/", so we need "../../" to reach root.
    let depth = go_output_dir.trim_end_matches('/').matches('/').count() + 1;
    let to_root = "../".repeat(depth);

    // Package header and cgo directives.
    // The package comment must immediately precede the package declaration with no blank line.
    header.push_str(&crate::backends::go::template_env::render(
        "package_doc_and_declaration.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            crate_name => &config.name,
        },
    ));
    header.push_str(&crate::backends::go::template_env::render(
        "cgo_preamble_binding.jinja",
        minijinja::context! {
            to_root => &to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_lib_name => ffi_lib_name,
            ffi_header => ffi_header,
        },
    ));
    header.push('\n');

    // Pass 2: Generate the body content
    let mut body = String::with_capacity(8192);

    // Error helper functions
    body.push_str(&gen_last_error_helper(ffi_prefix));
    body.push_str("\n\n");

    // Bytes helper: emitted once per package, used by every method/function
    // returning `TypeRef::Bytes`. Defining it here (rather than inline at each
    // call site) avoids repeated declarations and keeps a single place to
    // adjust ownership semantics.
    body.push_str(&gen_unmarshal_bytes_helper());
    body.push_str("\n\n");

    // Pointer helper: emitted once per package, used by data DTOs to construct
    // pointers for optional fields without functional-options boilerplate.
    // Usage: &MyStruct{Field: Ptr("value")}
    body.push_str(&gen_ptr_helper());
    body.push_str("\n\n");

    // Note: trait bridge exports (//export trampolines) are emitted by trait_bridges.go
    // (generated when has_plugin_bridges is true). Do NOT emit them here to avoid duplication.

    // Generate error types: a single consolidated sentinel `var (...)` block
    // across all ErrorDefs (variant-name collisions are disambiguated by
    // qualifying with the parent error's base name, e.g.
    // `ErrGraphQLValidationError` vs `ErrSchemaValidationError`), followed by
    // the per-error structured error struct + Error() method.
    if !api.errors.is_empty() {
        body.push_str(&crate::codegen::error_gen::gen_go_sentinel_errors(&api.errors));
        body.push_str("\n\n");
        for error in &api.errors {
            body.push_str(&crate::codegen::error_gen::gen_go_error_struct(error, pkg_name));
            body.push_str("\n\n");
        }
    }

    // When a visitor bridge is active, visitor.go defines the bridge's associated types
    // (e.g. SyntaxContext, WalkDecision) with FFI-compatible fields. Skip them in binding.go
    // to avoid redeclarations.
    let bridge_associated_types = config.bridge_associated_types();
    let visitor_types: std::collections::HashSet<&str> = if !bridge_param_names.is_empty() {
        bridge_associated_types.iter().map(|s| s.as_str()).collect()
    } else {
        std::collections::HashSet::new()
    };

    // Generate enum types and constants
    // Both unit enums and newtype-tuple enums map to `type X string` in Go.
    // Unit enums: all variants have no fields.
    // Newtype-tuple enums: all data variants contain only positional tuple fields (which Go
    // cannot represent as struct fields and are therefore treated as raw string values).
    // Data enums with named fields become Go structs and must NOT be included here.
    let unit_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| {
            !exclude_types.contains(&e.name)
                && e.variants
                    .iter()
                    .all(|v| v.fields.is_empty() || v.fields.iter().all(is_tuple_field))
        })
        .filter(|e| !is_passthrough_raw_message_enum(e))
        .map(|e| e.name.as_str())
        .collect();
    let passthrough_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| is_passthrough_raw_message_enum(e))
        .filter(|e| !exclude_types.contains(&e.name))
        .map(|e| e.name.as_str())
        .collect();
    for enum_def in api
        .enums
        .iter()
        .filter(|e| !visitor_types.contains(e.name.as_str()) && !exclude_types.contains(&e.name))
    {
        body.push_str(&gen_enum_type(enum_def));
        body.push_str("\n\n");
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
        .filter(|t| !exclude_types.contains(&t.name))
        .map(|t| t.name.as_str())
        .collect();

    // Collect all enum type names (both unit and data enums from api.enums).
    // These types do NOT have _from_json/_to_json/_free helpers in the FFI header —
    // only non-opaque api.types have those helpers. Functions that use an enum type
    // as a parameter or return value (via TypeRef::Named) cannot be correctly generated
    // (unless the type also appears as an opaque type in api.types) and are excluded.
    let ffi_enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // All UNIT-VARIANT enum names — used for FFI param-type emission (i32 discriminant) and the
    // matching from_i32 body conversion. Only unit-variant enums can be round-tripped through a
    // bare i32 discriminant: data-bearing variants (tuple or struct) carry field data that cannot
    // be reconstructed from the discriminant alone. The is_copy flag is intentionally not checked
    // here — a non-Copy unit-variant enum (e.g. one missing the Copy derive) can still be passed
    // by value over the C boundary using the auto-generated from_i32_rs match helper.
    let ffi_param_enum_names: HashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty() && !v.is_tuple))
        .map(|e| e.name.clone())
        .collect();

    // Data enums (sealed interfaces): enums with named fields in at least one variant
    let data_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| {
            !exclude_types.contains(&e.name)
                && e.variants
                    .iter()
                    .any(|v| !v.fields.is_empty() && v.fields.iter().any(|f| !is_tuple_field(f)))
        })
        .map(|e| e.name.as_str())
        .collect();

    // Collect non-opaque struct type names — these are real struct types that should NOT fall back to *json.RawMessage
    let struct_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !exclude_types.contains(&t.name))
        .map(|t| t.name.as_str())
        .collect();

    // Generate struct types
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !visitor_types.contains(typ.name.as_str()) && !exclude_types.contains(&typ.name))
    {
        if typ.is_opaque {
            // If an error type has the same name as this opaque type, the structured error
            // struct was already emitted by gen_go_error_types. Skip the duplicate struct
            // definition but still emit the Free() method.
            if error_names.contains(typ.name.as_str()) {
                body.push_str(&gen_opaque_type_free_only(typ, ffi_prefix));
                body.push_str("\n\n");
            } else {
                body.push_str(&gen_opaque_type(typ, ffi_prefix));
                body.push_str("\n\n");
            }
            // Client constructor — emit New<TypeName> when configured.
            if let Some(ctor) = config.client_constructors.get(&typ.name) {
                body.push_str(&gen_go_opaque_constructor(typ, ffi_prefix, ctor));
                body.push_str("\n\n");
            }
        } else {
            body.push_str(&gen_struct_type(
                typ,
                &unit_enum_names,
                &passthrough_enum_names,
                &data_enum_names,
                &struct_names,
                &config.trait_bridges,
            ));
            body.push_str("\n\n");
            // Generate functional options pattern only if type is in the functional_options allowlist.
            // By default, pure data DTOs use idiomatic struct literals instead of functional options.
            // Skip "Update" types (e.g., ParseOptionsUpdate) — they are partial update
            // structs that share field names with the primary config type, producing duplicate
            // With* function declarations.
            let empty_functional_options = vec![];
            let functional_options = config
                .go
                .as_ref()
                .map(|g| &g.functional_options)
                .unwrap_or(&empty_functional_options);
            if !typ.name.ends_with("Update") && functional_options.contains(&typ.name) {
                body.push_str(&gen_config_options(
                    typ,
                    &unit_enum_names,
                    &passthrough_enum_names,
                    &data_enum_names,
                    &config.trait_bridges,
                ));
                body.push_str("\n\n");
            }
        }
    }

    // Generate free function wrappers.
    // Async functions are included — the underlying FFI uses block_on() for synchronous C calls.
    // Skip functions excluded from FFI generation (their C symbols don't exist in the header)
    // and functions whose parameter or return types are enum types without FFI JSON helpers.
    for func in api.functions.iter().filter(|f| {
        !ffi_exclude_functions.contains(&f.name)
            && !signature_references_excluded_type(&f.params, &f.return_type, exclude_types)
            && !uses_ffi_enum_type(
                &f.params,
                &f.return_type,
                &ffi_enum_names,
                &ffi_param_enum_names,
                &opaque_names,
            )
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
    }) {
        // For the configured options-field visitor function, wrap it with visitor-awareness
        // logic instead of generating the basic wrapper.
        if visitor_bridge_cfg.is_some_and(|bridge_cfg| options_bridge_function_matches(func, bridge_cfg)) {
            body.push_str(&gen_convert_with_visitor_wrapper(
                func,
                ffi_prefix,
                &opaque_names,
                value_only_types,
                visitor_bridge_cfg.expect("checked above"),
            ));
            body.push_str("\n\n");
        } else {
            body.push_str(&gen_function_wrapper(
                func,
                ffi_prefix,
                &opaque_names,
                bridge_param_names,
                bridge_type_aliases,
                value_only_types,
                &ffi_enum_names,
                &ffi_param_enum_names,
            ));
            body.push_str("\n\n");
        }
    }

    // Emit module-level wrapper functions for streaming adapters so tests/consumers
    // can call them as pkg.CrawlStream(engine, url) instead of engine.CrawlStream(url).
    // These wrap the instance methods emitted below.
    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        if adapter.owner_type.is_none() || adapter.item_type.is_none() {
            continue;
        }
        body.push_str(&gen_adapter_wrapper(adapter, pkg_name, &api.types));
        body.push_str("\n\n");
    }

    // Generate struct methods.
    // Skip static methods that return Named types (e.g., Default() constructors) —
    // these are redundant with the generated New*() functional options constructors,
    // and the opaque handle conversion pipeline is not yet implemented.
    // Streaming adapter methods use a callback-based C signature that CGO can't call directly —
    // they are skipped here and must be implemented via a separate Go-native streaming API.
    // Also skip methods excluded from FFI or using enum types without FFI JSON helpers.
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
    {
        // Types that are both opaque and error types are emitted as Go value
        // structs (Code/Message fields) by `gen_go_error_struct` — they have
        // no `ptr` field to dispatch through. Skip method emission here so we
        // do not generate `h.ptr` references that fail to compile against a
        // value-type struct.
        if typ.is_opaque && error_names.contains(typ.name.as_str()) {
            continue;
        }
        // Non-opaque types without serde derives cannot be bridged through the
        // JSON roundtrip path used by the Go binding (receiver marshal + return
        // unmarshal both depend on `{prefix}_{type}_{to,from}_json` helpers,
        // which the FFI backend only emits when `has_serde` is true). Emitting
        // their methods would produce calls to non-existent C symbols and fail
        // to link. Types in this shape (e.g. `NodeContext` with manual serde
        // impls) are still reachable via the visitor flow, which generates its
        // own struct representation in `visitor.go`.
        if !typ.is_opaque && !typ.has_serde {
            continue;
        }
        for method in &typ.methods {
            // Skip methods named "default" — these are Rust's Default::default() trait impl
            // and should not be emitted as free functions in Go (use struct literals instead).
            if method.name == "default" {
                continue;
            }
            // For opaque types, skip static methods that return Named types OTHER than `new`
            // constructors. The `new` constructor is special: it's emitted by gen_method_wrapper,
            // which properly handles FFI calls and opaque pointer wrapping. Other static methods
            // returning Named types (e.g., preset constructors on opaque types) are not yet
            // supported. For non-opaque DTO types, static preset constructors (e.g. All(),
            // Minimal()) are emitted as package-level free functions and must not be suppressed.
            if typ.is_opaque
                && method.is_static
                && method.name != "new"
                && matches!(method.return_type, TypeRef::Named(_))
            {
                continue;
            }
            if let Some(item_type) = streaming_methods.get(&(typ.name.clone(), method.name.clone())) {
                // Streaming method: drive the FFI iterator-handle exports and surface a typed
                // Go channel instead of calling the callback-based wrapper directly.
                body.push_str(&gen_streaming_method_wrapper(
                    typ,
                    method,
                    ffi_prefix,
                    item_type,
                    &data_enum_names,
                    &opaque_names,
                    value_only_types,
                    &ffi_enum_names,
                    &ffi_param_enum_names,
                ));
                body.push_str("\n\n");
                continue;
            }
            if ffi_exclude_functions.contains(&method.name) {
                continue;
            }
            if signature_references_excluded_type(&method.params, &method.return_type, exclude_types) {
                continue;
            }
            if uses_ffi_enum_type(
                &method.params,
                &method.return_type,
                &ffi_enum_names,
                &ffi_param_enum_names,
                &opaque_names,
            ) {
                continue;
            }
            body.push_str(&gen_method_wrapper(
                typ,
                method,
                ffi_prefix,
                &opaque_names,
                value_only_types,
                &ffi_enum_names,
                &ffi_param_enum_names,
            ));
            body.push_str("\n\n");
        }
    }

    // Pass 3: Determine imports based on actual content usage
    let has_opaque_types = api.types.iter().any(|t| t.is_opaque);
    let has_sync_functions = api.functions.iter().any(|f| !f.is_async);
    let has_non_static_methods = api.types.iter().any(|t| t.methods.iter().any(|m| !m.is_static));
    let needs_json_and_unsafe = has_sync_functions || has_non_static_methods;

    let mut imports = vec!["fmt"];
    if needs_json_and_unsafe {
        imports.insert(0, "encoding/json");
        // Check for runtime usage in non-comment code (e.g., runtime.Pinner)
        let has_runtime_usage = body.lines().any(|line| {
            if let Some(code_part) = line.split("//").next() {
                code_part.contains("runtime.")
            } else {
                false
            }
        });
        if has_runtime_usage {
            imports.push("runtime");
        }
        imports.push("unsafe");
    } else if has_opaque_types {
        // Opaque types need unsafe for pointer wrapping even without JSON serialization.
        imports.push("unsafe");
    }
    if !api.errors.is_empty() {
        imports.insert(1.min(imports.len()), "errors");
    }
    let imports_str = crate::backends::go::template_env::render(
        "imports_basic.jinja",
        minijinja::context! {
            imports => imports,
        },
    );

    // Assemble final output: header + imports + body
    let mut out = String::with_capacity(header.len() + imports_str.len() + body.len());
    out.push_str(&header);
    out.push_str(&imports_str);
    out.push_str(&body);

    out
}
