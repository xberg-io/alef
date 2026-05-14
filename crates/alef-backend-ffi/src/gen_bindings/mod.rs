mod functions;
mod helpers;
mod types;

use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::ir::ApiSurface;
use heck::ToPascalCase;
use std::path::PathBuf;

use alef_adapters::AdapterBodies;
use alef_core::config::AdapterPattern;

use functions::{gen_free_function, gen_method_wrapper, gen_streaming_method_wrapper};
use helpers::{
    gen_build_rs, gen_cbindgen_toml, gen_ffi_tokio_runtime, gen_free_bytes, gen_free_string, gen_last_error,
    gen_version,
};
use types::{
    gen_enum_free, gen_enum_from_i32, gen_enum_from_json, gen_enum_to_i32, gen_enum_to_json, gen_enum_to_string,
    gen_field_accessor, gen_type_free, gen_type_from_json, gen_type_to_json,
};

pub struct FfiBackend;

impl FfiBackend {}

impl Backend for FfiBackend {
    fn name(&self) -> &str {
        "ffi"
    }

    fn language(&self) -> Language {
        Language::Ffi
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.ffi_prefix();
        let header_name = config.ffi_header_name();

        let output_dir = config
            .output_for("ffi")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("crates/{}-ffi/src/", config.name));

        let parent_dir = PathBuf::from(&output_dir)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();

        let go_output_dir = if config.targets(Language::Go) {
            config.output_paths.get("go").map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };

        let files = vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join("lib.rs"),
                content: gen_lib_rs(api, &prefix, config),
                generated_header: false,
            },
            GeneratedFile {
                path: parent_dir.join("cbindgen.toml"),
                content: gen_cbindgen_toml(&prefix, api),
                generated_header: false,
            },
            GeneratedFile {
                path: parent_dir.join("build.rs"),
                content: gen_build_rs(&header_name, go_output_dir.as_deref()),
                generated_header: false,
            },
        ];

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-ffi",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// lib.rs generation
// ---------------------------------------------------------------------------

fn gen_lib_rs(api: &ApiSurface, prefix: &str, config: &ResolvedCrateConfig) -> String {
    let mut builder = RustFileBuilder::new().with_generated_header();
    builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables, unused_mut, noop_method_call)");
    // useless_conversion is suppressed because `From<X> for Y` impls (where X != Y) get
    // extracted as static methods on Y, then the FFI wrapper signature normalizes the param
    // to Self. The generated `Y::from(arg: Y)` resolves to the blanket `From<T> for T`
    // (identity) at runtime; the wrapper is preserved for ABI stability.
    builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::redundant_locals, dropping_references, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::useless_conversion, clippy::type_complexity, clippy::clone_on_copy)");

    // Imports
    builder.add_import("std::ffi::{c_char, CStr, CString}");
    builder.add_import("std::cell::RefCell");
    let core_import = config.core_import_name();

    // Build path map: short name -> full rust_path for all types and enums.
    // Normalize dashes to underscores since IR paths use Cargo package names (dashes)
    // but Rust source code requires crate names (underscores).
    let mut path_map = ahash::AHashMap::new();
    for t in api.types.iter().filter(|t| !t.is_trait) {
        path_map.insert(t.name.clone(), t.rust_path.replace('-', "_"));
    }
    for e in &api.enums {
        path_map.insert(e.name.clone(), e.rust_path.replace('-', "_"));
    }
    for err in &api.errors {
        path_map.insert(err.name.clone(), err.rust_path.replace('-', "_"));
    }

    // Copy-typed named types (structs and enums that derive Copy). For these, callers emit
    // auto-copy/deref instead of `.clone()` to avoid clippy::clone_on_copy.
    let enum_names: ahash::AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.is_copy)
        .map(|e| e.name.clone())
        .chain(
            api.types
                .iter()
                .filter(|t| !t.is_trait && t.is_copy)
                .map(|t| t.name.clone()),
        )
        .collect();
    // Clone-but-not-Copy named types (structs + data-bearing enums). Callers emit `.clone()`.
    let clone_names: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && t.is_clone && !t.is_copy)
        .map(|t| t.name.clone())
        .chain(api.enums.iter().filter(|e| !e.is_copy).map(|e| e.name.clone()))
        .collect();

    // Import traits needed for trait method dispatch
    for trait_path in generators::collect_trait_imports(api) {
        builder.add_import(&trait_path);
    }
    // FFI backend uses fully qualified paths (e.g. html_to_markdown_rs::ConversionOptions)
    // for all core type references, so no named or glob imports from the core crate are
    // needed. Trait imports (collected above) are sufficient for method dispatch.

    // Only import serde_json when types need from_json deserialization or
    // when Json/Vec/Map fields/returns require serialization
    let has_from_json_types = api
        .types
        .iter()
        .any(|t| !t.is_opaque && !t.fields.iter().any(|f| f.sanitized));
    let has_serde_fields = api.types.iter().any(|t| {
        t.fields.iter().any(|f| {
            matches!(f.ty, alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
                || matches!(&f.ty, alef_core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)))
        })
    });
    let has_serde_returns = api.types.iter().any(|t| {
        t.methods.iter().any(|m| {
            matches!(m.return_type, alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
                || matches!(&m.return_type, alef_core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)))
        })
    }) || api.functions.iter().any(|f| {
        matches!(f.return_type, alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
            || matches!(&f.return_type, alef_core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)))
    });
    if has_from_json_types || has_serde_fields || has_serde_returns {
        builder.add_import("serde_json");
    }

    // Custom module declarations
    let custom_mods = config.custom_modules.for_language(Language::Ffi);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    // Thread-local last_error infrastructure
    builder.add_item(&gen_last_error(prefix));

    // free_string helper
    builder.add_item(&gen_free_string(prefix));

    // free_bytes helper — companion for functions returning Result<Vec<u8>> via out-params
    builder.add_item(&gen_free_bytes(prefix));

    // version helper
    builder.add_item(&gen_version(prefix));

    // Build adapter body map before the method loop so streaming adapters can
    // substitute in their callback-based bodies instead of the normal wrapper.
    let adapter_bodies: AdapterBodies = alef_adapters::build_adapter_bodies(config, Language::Ffi).unwrap_or_default();

    // Emit the stream callback type alias once if any streaming adapters exist.
    let has_streaming_adapters = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    if has_streaming_adapters {
        builder.add_item(
            "/// Callback invoked for each streamed chunk.\n\
             /// `chunk_json` is a JSON-encoded chunk; `user_data` is forwarded from the caller.\n\
             pub type LiterLlmStreamCallback =\n    \
             unsafe extern \"C\" fn(chunk_json: *const std::ffi::c_char, user_data: *mut std::ffi::c_void);",
        );

        // Also emit iterator-handle functions for each streaming adapter.
        // These provide a pull-based alternative to the callback-based wrappers so that
        // language bindings without native async (Go, Ruby, Java, C#, Elixir, C) can drive
        // the stream in a simple while loop without holding a C function pointer.
        for adapter in config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        {
            let Some(owner_type) = adapter.owner_type.as_deref() else {
                continue;
            };
            let Some(item_type) = adapter.item_type.as_deref() else {
                continue;
            };
            let Some(request_type) = adapter.request_type.as_deref() else {
                continue;
            };
            builder.add_item(&helpers::gen_stream_handle_functions(
                prefix,
                owner_type,
                &adapter.name,
                &adapter.core_path,
                item_type,
                request_type,
                &core_import,
            ));
        }
    }

    // Collect the set of type names excluded via [ffi] exclude_types.
    let ffi_exclude_types: ahash::AHashSet<&str> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_types.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    // Struct opaque-handle functions (from_json + free + field accessors + methods)
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !ffi_exclude_types.contains(typ.name.as_str()))
    {
        // Generate from_json/to_json for types that derive serde Serialize/Deserialize.
        // Opaque types and types without serde derives are skipped.
        // Note: sanitized fields do NOT block from_json/to_json generation because these
        // functions use serde for the full core type (bypassing field-level type mapping).
        if !typ.is_opaque && typ.has_serde {
            // Skip auto-from_json when the type defines its own `from_json`/`from_str`
            // method — the method wrapper produces the same FFI export name and would
            // collide.
            let has_from_json_method = typ.methods.iter().any(|m| m.name == "from_json");
            if !has_from_json_method {
                builder.add_item(&gen_type_from_json(typ, prefix, &core_import));
            }
            // Generate to_json for types that support serialization. Skip Update types
            // (partial update structs typically derive Deserialize only) and skip when
            // the type already exposes a `to_json` method (would collide on FFI name).
            //
            // We used to skip "value-only" types (all primitive fields) under the assumption
            // that FFI callers would reconstruct them from field accessors. That assumption
            // breaks down for bindings (Java FFM, C# P/Invoke) that don't have per-binding
            // value-only reconstruction codegen and rely on the JSON path uniformly. Emitting
            // to_json for value-only types unblocks `get_embedding_preset` etc. across all
            // bindings; the FFI surface gains a few extra `_to_json` exports.
            let has_to_json_method = typ.methods.iter().any(|m| m.name == "to_json");
            if !typ.name.ends_with("Update") && !has_to_json_method {
                builder.add_item(&gen_type_to_json(typ, prefix, &core_import));
            }
        }
        builder.add_item(&gen_type_free(typ, prefix, &core_import));

        // Field accessors — skip sanitized fields (binding type differs from core)
        for field in &typ.fields {
            if !field.sanitized {
                builder.add_item(&gen_field_accessor(
                    typ,
                    field,
                    prefix,
                    &core_import,
                    &path_map,
                    &enum_names,
                    &clone_names,
                ));
            }
        }

        // Method wrappers — streaming adapters get a dedicated callback-based wrapper.
        for method in &typ.methods {
            let streaming_adapter = config.adapters.iter().find(|a| {
                matches!(a.pattern, AdapterPattern::Streaming)
                    && a.owner_type.as_deref() == Some(typ.name.as_str())
                    && a.name == method.name
            });
            if let Some(adapter) = streaming_adapter {
                let adapter_key = format!("{}.{}", typ.name, adapter.name);
                if let Some(body) = adapter_bodies.get(&adapter_key) {
                    builder.add_item(&gen_streaming_method_wrapper(typ, method, prefix, &core_import, body));
                    continue;
                }
            }
            builder.add_item(&gen_method_wrapper(
                typ,
                method,
                prefix,
                &core_import,
                &path_map,
                &enum_names,
            ));
        }
    }

    // Enum functions (from_i32 + to_i32) — only for simple unit-variant enums
    for enum_def in &api.enums {
        if alef_codegen::conversions::can_generate_enum_conversion(enum_def) {
            builder.add_item(&gen_enum_from_i32(enum_def, prefix, &core_import));
            builder.add_item(&gen_enum_to_i32(enum_def, prefix, &core_import));
        }
    }

    // Enum pointer lifecycle helpers (_free, _to_json, _from_json) for enums that:
    //   a) are returned as heap-allocated *mut T by any non-excluded function or struct method, OR
    //   b) are accepted as parameters by any non-excluded function
    // These are required by Panama FFM callers (Java) that receive enum pointers and must
    // free them, serialize them, or pass them as JSON-encoded parameters.
    {
        let ffi_exclude_set: ahash::AHashSet<&str> = config
            .ffi
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        // Collect enum names returned as Named pointers by non-excluded free functions
        let mut enum_pointer_return: ahash::AHashSet<String> = ahash::AHashSet::new();
        for func in &api.functions {
            if ffi_exclude_set.contains(func.name.as_str()) {
                continue;
            }
            let return_named = match &func.return_type {
                alef_core::ir::TypeRef::Named(n) => Some(n.clone()),
                alef_core::ir::TypeRef::Optional(inner) => {
                    if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(name) = return_named {
                if api.enums.iter().any(|e| e.name == name) {
                    enum_pointer_return.insert(name);
                }
            }
        }
        // Also check struct field accessors and method returns that yield enum pointers.
        // Field accessors (gen_field_accessor) emit `*mut Enum` returns whenever a struct
        // field's type is a Named enum, so any such field implies a heap-allocated enum
        // pointer that callers must free / stringify — match the function path above.
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for method in &typ.methods {
                let return_named = match &method.return_type {
                    alef_core::ir::TypeRef::Named(n) => Some(n.clone()),
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = return_named {
                    if api.enums.iter().any(|e| e.name == name) {
                        enum_pointer_return.insert(name);
                    }
                }
            }
            for field in &typ.fields {
                let field_named = match &field.ty {
                    alef_core::ir::TypeRef::Named(n) => Some(n.clone()),
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = field_named {
                    if api.enums.iter().any(|e| e.name == name) {
                        enum_pointer_return.insert(name);
                    }
                }
            }
        }

        // Collect enum names used as parameters in non-excluded free functions
        let mut enum_pointer_param: ahash::AHashSet<String> = ahash::AHashSet::new();
        for func in &api.functions {
            if ffi_exclude_set.contains(func.name.as_str()) {
                continue;
            }
            for param in &func.params {
                let param_named = match &param.ty {
                    alef_core::ir::TypeRef::Named(n) => Some(n.clone()),
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = param_named {
                    if api.enums.iter().any(|e| e.name == name) {
                        enum_pointer_param.insert(name);
                    }
                }
            }
        }

        let mut emitted_enum_free: ahash::AHashSet<String> = ahash::AHashSet::new();
        for enum_def in &api.enums {
            let needs_free = enum_pointer_return.contains(&enum_def.name);
            // needs_from_json also implies needs_free: `_from_json` returns *mut T (heap-allocated)
            // so the caller must call `_free` on the returned pointer after use.
            let needs_from_json = enum_pointer_param.contains(&enum_def.name);
            let has_serde = enum_def.has_serde;

            // Generate _free (and _to_json / _to_string for serialization) for return-type enums.
            if needs_free && emitted_enum_free.insert(enum_def.name.clone()) {
                builder.add_item(&gen_enum_free(enum_def, prefix, &core_import));
                if has_serde {
                    builder.add_item(&gen_enum_to_json(enum_def, prefix, &core_import));
                    // _to_string yields the bare unit-variant string (no JSON quotes).
                    // Required by C callers that compare enum values to fixture strings.
                    if alef_codegen::conversions::can_generate_enum_conversion(enum_def) {
                        builder.add_item(&gen_enum_to_string(enum_def, prefix, &core_import));
                    }
                }
            }
            if needs_from_json && has_serde {
                let from_json_key = format!("{}_from_json", enum_def.name);
                if emitted_enum_free.insert(from_json_key) {
                    builder.add_item(&gen_enum_from_json(enum_def, prefix, &core_import));
                }
                // `_from_json` returns *mut T — generate _free if not already emitted.
                // This allows callers (Java FFM) to free the heap pointer returned by _from_json.
                if emitted_enum_free.insert(enum_def.name.clone()) {
                    builder.add_item(&gen_enum_free(enum_def, prefix, &core_import));
                }
            }
        }
    }

    // Emit tokio runtime helper if any function or method is async
    let has_async_functions =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    if has_async_functions {
        builder.add_item(&gen_ffi_tokio_runtime());
    }

    let visitor_callbacks_enabled = config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks);

    // Detect whether any options_field bridge is configured.  When true, visitor callbacks
    // are handled via gen_bridge_field (VTable + options setter) rather than the legacy
    // gen_visitor path (VisitorCallbacks struct + convert_with_visitor).
    let has_options_field_bridge = config
        .trait_bridges
        .iter()
        .any(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField);

    let ffi_exclude_functions: ahash::AHashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    // Free functions (async functions are wrapped with block_on via the runtime helper)
    for func in &api.functions {
        if ffi_exclude_functions.contains(&func.name) {
            continue;
        }
        // For the legacy FunctionParam visitor path: skip the sanitized convert stub;
        // gen_convert_no_visitor emits the real implementation below.
        // For the OptionsField path: skip the IR-generated convert entirely;
        // gen_convert_with_options_field_bridge emits the definitive implementation that
        // passes the embedded visitor through options.  The IR version is a duplicate
        // because the visitor lives in ConversionOptions (not a function parameter), so
        // the IR correctly generates a 2-arg convert — but we still want the one with
        // the full options-clone-with-visitor semantics only.
        if visitor_callbacks_enabled && func.sanitized && func.name == "convert" {
            continue;
        }
        if has_options_field_bridge && func.name == "convert" {
            continue;
        }
        builder.add_item(&gen_free_function(func, prefix, &core_import, &path_map, &enum_names));
    }

    // Visitor/callback FFI support.
    // - OptionsField bridge: VTable + options setter + correct convert implementation.
    // - FunctionParam bridge (legacy): VisitorCallbacks struct + convert_with_visitor.
    //
    // When both flags are active simultaneously (e.g. html-to-markdown with
    // `visitor_callbacks = true` and an `[[trait_bridges]]` entry using
    // `bind_via = "options_field"`), we emit BOTH:
    //   1. The OptionsField vtable / options-setter / {prefix}_convert  (used by Go, C)
    //   2. The visitor-callbacks symbols ({prefix}_visitor_create/free/convert_with_visitor) (used by Java)
    // The two sets of symbols use different function names and do not conflict.
    if has_options_field_bridge {
        // Build a type_paths map for delegation method signature generation.
        let type_paths: std::collections::HashMap<String, String> =
            path_map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let trait_map: ahash::AHashMap<&str, &alef_core::ir::TypeDef> = api
            .types
            .iter()
            .filter(|t| t.is_trait)
            .map(|t| (t.name.as_str(), t))
            .collect();

        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.bind_via != alef_core::config::BridgeBinding::OptionsField {
                continue;
            }
            let Some(trait_def) = trait_map.get(bridge_cfg.trait_name.as_str()) else {
                continue;
            };
            let Some(options_type_name) = bridge_cfg.options_type.as_deref() else {
                continue;
            };
            let Some(field_name) = bridge_cfg.resolved_options_field() else {
                continue;
            };

            builder.add_item(&crate::gen_bridge_field::gen_options_set_bridge(
                prefix,
                &core_import,
                trait_def,
                &bridge_cfg.trait_name,
                field_name,
                options_type_name,
                &type_paths,
            ));
        }

        // Emit the correct {prefix}_convert that passes options (with embedded visitor) to core.
        builder.add_item(&crate::gen_bridge_field::gen_convert_with_options_field_bridge(
            prefix,
            &core_import,
        ));

        // When visitor_callbacks is also enabled, additionally emit the
        // {prefix}_visitor_create / {prefix}_visitor_free / {prefix}_options_set_visitor_handle
        // symbols.  These are needed by Java's Panama FFM binding which uses the
        // callbacks-struct pattern rather than the vtable/options-field pattern.
        // NOTE: gen_visitor_bindings does NOT emit {prefix}_convert (only
        // {prefix}_options_set_visitor_handle), so there is no symbol collision with the
        // OptionsField convert generated above.
        if visitor_callbacks_enabled {
            // Use the first OptionsField bridge's trait to drive callback spec generation.
            let visitor_trait_def = config
                .trait_bridges
                .iter()
                .filter(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField)
                .find_map(|b| trait_map.get(b.trait_name.as_str()).copied());
            if let Some(vtd) = visitor_trait_def {
                builder.add_item(&crate::gen_visitor::gen_visitor_bindings(
                    prefix,
                    &core_import,
                    true,
                    vtd,
                ));
            } else {
                eprintln!(
                    "[alef] gen_visitor_bindings(ffi): visitor_callbacks=true but no OptionsField trait found in IR, skipping visitor callbacks"
                );
            }
        }
    } else if visitor_callbacks_enabled {
        // Legacy FunctionParam path: emit the real {prefix}_convert (no-visitor) and then
        // the visitor bindings with {prefix}_options_set_visitor_handle.
        // Use the first is_trait type in the IR to drive callback spec generation.
        let visitor_trait_def = api.types.iter().find(|t| t.is_trait);
        if let Some(vtd) = visitor_trait_def {
            builder.add_item(&crate::gen_visitor::gen_convert_no_visitor(prefix, &core_import));
            builder.add_item(&crate::gen_visitor::gen_visitor_bindings(
                prefix,
                &core_import,
                false,
                vtd,
            ));
        } else {
            eprintln!(
                "[alef] gen_visitor_bindings(ffi): visitor_callbacks=true but no trait type found in IR, skipping visitor callbacks"
            );
        }
    }

    // Plugin bridge support — vtable + user_data pattern for each [[trait_bridges]] entry.
    // Only emitted when the entry applies to traits present in the extracted IR.
    if !config.trait_bridges.is_empty() {
        // Bridge code uses c_void and Arc; add them here so the builder deduplicates
        // against the c_char/CStr/CString already added above.
        builder.add_import("std::ffi::c_void");
        builder.add_import("std::sync::Arc");

        // Emit the shared FFI error helper once for all trait bridges
        builder.add_item(&crate::trait_bridge::gen_ffi_set_out_error_helper());

        let trait_map: ahash::AHashMap<&str, &alef_core::ir::TypeDef> = api
            .types
            .iter()
            .filter(|t| t.is_trait)
            .map(|t| (t.name.as_str(), t))
            .collect();

        let error_type_name = config.error_type_name();
        let error_constructor = config.error_constructor_expr();
        let plugin_error_constructor = config.ffi_plugin_error_constructor();
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_def) = trait_map.get(bridge_cfg.trait_name.as_str()) {
                let bridge_code = crate::trait_bridge::gen_trait_bridge(
                    trait_def,
                    bridge_cfg,
                    prefix,
                    &core_import,
                    &error_type_name,
                    &error_constructor,
                    plugin_error_constructor.as_deref(),
                    api,
                );
                builder.add_item(&bridge_code);

                // For options-field bridges, emit exported C constructor/destructor so that
                // non-Rust callers (Go, Java, C#) can create and free bridge handles entirely
                // through the C ABI.  The exported function signature also forces cbindgen to
                // emit the full vtable struct definition in the generated C header, which
                // callers must fill in before invoking `bridge_new`.
                if bridge_cfg.bind_via == alef_core::config::BridgeBinding::OptionsField {
                    let pascal_prefix = prefix.to_pascal_case();
                    builder.add_item(&crate::trait_bridge::gen_bridge_new_free(
                        prefix,
                        &pascal_prefix,
                        &bridge_cfg.trait_name,
                    ));
                }
            }
        }
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::NewAlefConfig;
    use alef_core::ir::*;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn visitor_config_htm() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"
visitor_callbacks = true
"#,
        )
    }

    fn visitor_config_ml() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"
visitor_callbacks = true
"#,
        )
    }

    fn sample_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "Config".to_string(),
                rust_path: "my_lib::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    FieldDef {
                        name: "timeout".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U64),
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
                        serde_rename: None,
                        serde_flatten: false,
                    },
                    FieldDef {
                        name: "name".to_string(),
                        ty: TypeRef::String,
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
                        serde_rename: None,
                        serde_flatten: false,
                    },
                    FieldDef {
                        name: "verbose".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Bool),
                        optional: true,
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
                        serde_rename: None,
                        serde_flatten: false,
                    },
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: "Configuration struct.".to_string(),
                cfg: None,
            }],
            functions: vec![FunctionDef {
                name: "extract".to_string(),
                rust_path: "my_lib::extract".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                return_type: TypeRef::Named("ExtractionResult".to_string()),
                is_async: false,
                error_type: Some("MyError".to_string()),
                doc: "Extract content from a file.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![EnumDef {
                name: "OutputFormat".to_string(),
                rust_path: "my_lib::OutputFormat".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Text".to_string(),
                        fields: vec![],
                        is_tuple: false,
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                    EnumVariant {
                        name: "Html".to_string(),
                        fields: vec![],
                        is_tuple: false,
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                ],
                doc: "Output format.".to_string(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        }
    }

    /// Like `sample_api()` but includes an `HtmlVisitor` trait with representative methods.
    ///
    /// Use this for tests that exercise visitor callback generation.  The methods cover each
    /// `ParamKind` variant: Str, OptStr, Bool, U32, Usize, CellSlice, and no-params.
    fn visitor_api() -> ApiSurface {
        let mut api = sample_api();
        api.types.push(TypeDef {
            name: "HtmlVisitor".to_string(),
            rust_path: "my_lib::visitor::HtmlVisitor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![
                MethodDef {
                    name: "visit_text".to_string(),
                    params: vec![
                        ParamDef {
                            name: "ctx".to_string(),
                            ty: TypeRef::Named("NodeContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "text".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit text nodes.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "visit_element_start".to_string(),
                    params: vec![ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                    }],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Called before entering any element.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "visit_link".to_string(),
                    params: vec![
                        ParamDef {
                            name: "ctx".to_string(),
                            ty: TypeRef::Named("NodeContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "href".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "title".to_string(),
                            ty: TypeRef::String,
                            optional: true,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit anchor links.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "visit_heading".to_string(),
                    params: vec![
                        ParamDef {
                            name: "ctx".to_string(),
                            ty: TypeRef::Named("NodeContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "level".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::U32),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "text".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit heading elements.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "visit_blockquote".to_string(),
                    params: vec![
                        ParamDef {
                            name: "ctx".to_string(),
                            ty: TypeRef::Named("NodeContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "content".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "depth".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::Usize),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit blockquote elements.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "visit_list_item".to_string(),
                    params: vec![
                        ParamDef {
                            name: "ctx".to_string(),
                            ty: TypeRef::Named("NodeContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "ordered".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::Bool),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "text".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit list items.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "visit_table_row".to_string(),
                    params: vec![
                        ParamDef {
                            name: "ctx".to_string(),
                            ty: TypeRef::Named("NodeContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "cells".to_string(),
                            ty: TypeRef::Vec(Box::new(TypeRef::String)),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                        ParamDef {
                            name: "is_header".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::Bool),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit table rows.".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
            ],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "HTML visitor trait.".to_string(),
            cfg: None,
        });
        api
    }

    fn sample_config() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn test_generates_lib_rs() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        assert!(files.iter().any(|f| f.path.ends_with("lib.rs")));

        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(lib.content.contains("extern \"C\""));
        assert!(lib.content.contains("my_lib_last_error_code"));
        assert!(lib.content.contains("my_lib_config_from_json"));
        assert!(lib.content.contains("my_lib_config_free"));
        assert!(lib.content.contains("my_lib_config_timeout"));
        assert!(lib.content.contains("my_lib_config_name"));
        assert!(lib.content.contains("my_lib_free_string"));
        assert!(lib.content.contains("my_lib_version"));
        assert!(lib.content.contains("my_lib_extract"));
        assert!(lib.content.contains("my_lib_output_format_from_i32"));
        assert!(lib.content.contains("my_lib_output_format_from_str"));
    }

    /// Build an `ApiSurface` whose only function returns the unit-variant enum
    /// `Color` (`has_serde: true`) by pointer. Used to exercise emission of
    /// `_to_string` accessors alongside `_free` / `_to_json`.
    fn enum_return_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "current_color".to_string(),
                rust_path: "my_lib::current_color".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Named("Color".to_string()),
                is_async: false,
                error_type: None,
                doc: "Currently selected color.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![EnumDef {
                name: "Color".to_string(),
                rust_path: "my_lib::Color".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Red".to_string(),
                        fields: vec![],
                        is_tuple: false,
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                    EnumVariant {
                        name: "Green".to_string(),
                        fields: vec![],
                        is_tuple: false,
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                ],
                doc: "Colors.".to_string(),
                cfg: None,
                is_copy: false,
                has_serde: true,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_emits_enum_to_string_for_pointer_return_enum() {
        let api = enum_return_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Sanity: pointer-return enum lifecycle helpers are emitted.
        assert!(
            lib.content.contains("my_lib_color_free"),
            "expected my_lib_color_free in emitted lib.rs"
        );
        assert!(
            lib.content.contains("my_lib_color_to_json"),
            "expected my_lib_color_to_json in emitted lib.rs"
        );

        // The new accessor: takes *const Color, returns *mut c_char.
        assert!(
            lib.content
                .contains("pub unsafe extern \"C\" fn my_lib_color_to_string("),
            "expected pub unsafe extern \"C\" fn my_lib_color_to_string in emitted lib.rs"
        );
        assert!(
            lib.content.contains("ptr: *const my_lib::Color)"),
            "to_string should accept *const Color"
        );
        assert!(
            lib.content.contains("-> *mut c_char"),
            "to_string should return *mut c_char"
        );
        // Body should extract the unit-variant name via serde, not via JSON-with-quotes.
        assert!(
            lib.content.contains("serde_json::to_value(val)"),
            "to_string should use serde_json::to_value"
        );
        assert!(
            lib.content.contains(".as_str()"),
            "to_string should call .as_str() to strip JSON quotes"
        );
    }

    #[test]
    fn test_omits_enum_to_string_when_enum_not_returned() {
        // The default sample_api() uses `OutputFormat` only as a non-return enum
        // (no function returns it, no struct field has it), so neither _free nor
        // _to_string should be emitted.
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(
            !lib.content.contains("my_lib_output_format_to_string"),
            "expected NO my_lib_output_format_to_string when enum is not returned by pointer"
        );
        assert!(
            !lib.content.contains("my_lib_output_format_free"),
            "expected NO my_lib_output_format_free when enum is not returned by pointer"
        );
    }

    #[test]
    fn test_generates_cbindgen_toml() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
        assert!(cbindgen.content.contains("MY_LIB_H"));
        assert!(cbindgen.content.contains("language = \"C\""));
    }

    #[test]
    fn test_generates_build_rs() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
        assert!(build.content.contains("cbindgen::generate"));
        assert!(build.content.contains("my_lib.h"));
    }

    #[test]
    fn test_custom_prefix() {
        let api = sample_api();
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"
header_name = "mylib.h"
"#,
        );
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(lib.content.contains("ml_last_error_code"));
        assert!(lib.content.contains("ml_config_from_json"));

        let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
        assert!(build.content.contains("mylib.h"));
    }

    #[test]
    fn test_visitor_callbacks_disabled_by_default() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // When visitor_callbacks is not enabled, no visitor code should be generated
        assert!(!lib.content.contains("VisitorCallbacks"));
        assert!(!lib.content.contains("visit_text"));
        assert!(!lib.content.contains("_visitor_create"));
        assert!(!lib.content.contains("_visitor_free"));
        assert!(!lib.content.contains("_convert_with_visitor"));
    }

    #[test]
    fn test_visitor_callbacks_enabled() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Callback struct should be generated
        assert!(lib.content.contains("struct HtmVisitorCallbacks"));
        assert!(lib.content.contains("pub struct HtmNodeContext"));

        // Visit-result codes should be defined
        assert!(lib.content.contains("HTM_VISIT_CONTINUE"));
        assert!(lib.content.contains("HTM_VISIT_SKIP"));
        assert!(lib.content.contains("HTM_VISIT_PRESERVE_HTML"));
        assert!(lib.content.contains("HTM_VISIT_CUSTOM"));
        assert!(lib.content.contains("HTM_VISIT_ERROR"));

        // NodeContext fields
        assert!(lib.content.contains("node_type: i32"));
        assert!(lib.content.contains("tag_name: *const std::ffi::c_char"));
        assert!(lib.content.contains("depth: usize"));
        assert!(lib.content.contains("index_in_parent: usize"));
        assert!(lib.content.contains("parent_tag: *const std::ffi::c_char"));
        assert!(lib.content.contains("is_inline: i32"));
    }

    #[test]
    fn test_visitor_callbacks_visitor_handle_struct() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Visitor handle struct should exist
        assert!(lib.content.contains("pub struct HtmVisitor"));
        assert!(lib.content.contains("callbacks: HtmVisitorCallbacks"));
        assert!(lib.content.contains("_tag_scratch"));
    }

    #[test]
    fn test_visitor_callbacks_callback_fields() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Key visitor callback fields generated from the IR trait methods in visitor_api()
        assert!(lib.content.contains("visit_text"));
        assert!(lib.content.contains("visit_element_start"));
        assert!(lib.content.contains("visit_link"));
        assert!(lib.content.contains("visit_heading"));
        assert!(lib.content.contains("visit_blockquote"));
        assert!(lib.content.contains("visit_list_item"));
        assert!(lib.content.contains("visit_table_row"));
    }

    #[test]
    fn test_visitor_callbacks_ffi_functions() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Public FFI entry points for visitor management
        assert!(lib.content.contains("htm_visitor_create"));
        assert!(lib.content.contains("htm_visitor_free"));
        assert!(lib.content.contains("htm_convert_with_visitor"));

        // Functions should be extern "C"
        assert!(lib.content.contains("extern \"C\" fn htm_visitor_create"));
        assert!(lib.content.contains("extern \"C\" fn htm_visitor_free"));
        assert!(lib.content.contains("extern \"C\" fn htm_convert_with_visitor"));
    }

    #[test]
    fn test_visitor_callbacks_callback_signatures() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Callback type signatures should be extern "C" function pointers
        assert!(lib.content.contains("extern \"C\" fn("));
        assert!(lib.content.contains("*const HtmNodeContext"));
        assert!(lib.content.contains("user_data: *mut std::ffi::c_void"));
        assert!(lib.content.contains("out_custom: *mut *mut std::ffi::c_char"));
        assert!(lib.content.contains("out_len: *mut usize"));

        // Return type should be i32
        assert!(lib.content.contains(") -> i32"));
    }

    #[test]
    fn test_visitor_callbacks_custom_prefix() {
        let api = visitor_api();
        let config = visitor_config_ml();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Custom prefix should be used throughout (struct/function names and constants)
        assert!(lib.content.contains("MlVisitorCallbacks"));
        assert!(lib.content.contains("MlNodeContext"));
        assert!(lib.content.contains("ml_visitor_create"));
        assert!(lib.content.contains("ml_visitor_free"));
        assert!(lib.content.contains("ml_convert_with_visitor"));
        // Visit result constants use HTM_ prefix (hardcoded in gen_visitor)
        assert!(lib.content.contains("HTM_VISIT_CONTINUE"));
    }

    #[test]
    fn test_visitor_callbacks_visitor_ref_wrapper() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // VisitorRef wrapper for forwarding trait methods
        assert!(lib.content.contains("struct VisitorRef"));
        assert!(lib.content.contains("impl std::fmt::Debug for VisitorRef"));
        // VisitorRef should implement HtmlVisitor trait (core_import is my_lib for this test)
        assert!(lib.content.contains("impl my_lib::visitor::HtmlVisitor for VisitorRef"));
    }

    #[test]
    fn test_visitor_callbacks_safety_comments() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Should document safety invariants for unsafe blocks
        assert!(lib.content.contains("// SAFETY:"));
        assert!(lib.content.contains("unsafe"));
        assert!(lib.content.contains("unsafe extern \"C\" fn"));
    }

    #[test]
    fn test_visitor_callbacks_decode_visit_result() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper function to decode visit result codes back to Rust enum
        assert!(lib.content.contains("decode_visit_result"));
        assert!(lib.content.contains("VisitResult::Skip"));
        assert!(lib.content.contains("VisitResult::PreserveHtml"));
        assert!(lib.content.contains("VisitResult::Custom"));
        assert!(lib.content.contains("VisitResult::Error"));
    }

    #[test]
    fn test_visitor_callbacks_call_with_ctx() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper function for building and passing NodeContext to C callbacks
        assert!(lib.content.contains("call_with_ctx"));
        assert!(lib.content.contains("HtmNodeContext"));
        assert!(lib.content.contains("tag_cstring"));
        assert!(lib.content.contains("parent_cstring"));
    }

    #[test]
    fn test_visitor_callbacks_opt_str_to_c() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper to convert Option<&str> to C pointer (null or valid CString)
        assert!(lib.content.contains("opt_str_to_c"));
    }

    #[test]
    fn test_visitor_callbacks_repr_c() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // FFI-crossing types must use #[repr(C)]
        assert!(lib.content.contains("#[repr(C)]"));
    }

    #[test]
    fn test_visitor_callbacks_send_impl() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // VisitorCallbacks should be Send (safe to share across thread boundaries)
        assert!(lib.content.contains("unsafe impl Send for HtmVisitorCallbacks"));
    }

    /// Regression test: Option<Option<Primitive>> (update-struct pattern) must generate
    /// a getter that returns the primitive type — not *mut c_char — and collapses both
    /// None cases to the primitive's zero sentinel.
    #[test]
    fn test_option_option_primitive_getter_returns_primitive_type() {
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "ConfigUpdate".to_string(),
                rust_path: "my_lib::ConfigUpdate".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "max_depth".to_string(),
                    // field.ty = Optional(Primitive(Usize)), field.optional = true
                    // represents Rust type Option<Option<usize>>
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
                    optional: true,
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
                    serde_rename: None,
                    serde_flatten: false,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Return type must be `usize`, not `*mut std::ffi::c_char`
        assert!(
            lib.content.contains("-> usize"),
            "expected `-> usize` in getter but got:\n{}",
            lib.content
        );
        assert!(
            !lib.content.contains("-> *mut std::ffi::c_char"),
            "getter must not return *mut c_char for Option<Option<usize>>"
        );

        // Both None arms must return 0, not a pointer
        assert!(
            lib.content.contains("None => 0"),
            "expected `None => 0` sentinel in generated getter"
        );

        // The inner Some(inner_val) branch must dereference the usize
        assert!(
            lib.content.contains("*inner_val"),
            "expected `*inner_val` deref for inner primitive in generated getter"
        );
    }

    /// Build a minimal `ApiSurface` with one struct that has a Named field,
    /// controlling `is_clone` on the field's referenced type.
    fn api_with_named_field(field_type: &str, is_clone: bool) -> ApiSurface {
        // The struct that holds the Named field
        let holder = TypeDef {
            name: "Holder".to_string(),
            rust_path: "my_lib::Holder".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "inner".to_string(),
                ty: TypeRef::Named(field_type.to_string()),
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
                serde_rename: None,
                serde_flatten: false,
            }],
            methods: vec![],
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
            doc: String::new(),
            cfg: None,
        };
        // The type referenced by the Named field
        let named_type = TypeDef {
            name: field_type.to_string(),
            rust_path: format!("my_lib::{field_type}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
        };
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![holder, named_type],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        }
    }

    /// Non-Clone opaque Named-type fields must not emit `.clone()` in the
    /// generated field accessor — the accessor should use a raw pointer cast instead.
    #[test]
    fn test_named_field_non_clone_no_clone_call() {
        let api = api_with_named_field("LanguageRegistry", false);
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The field accessor for `inner` (a non-Clone opaque type) must not call .clone()
        assert!(
            !lib.content.contains(".clone()"),
            "non-Clone opaque Named field must not emit .clone() in accessor:\n{}",
            lib.content
        );
    }

    /// Clone-capable Named-type fields must still emit `.clone()` in the accessor.
    #[test]
    fn test_named_field_clone_capable_emits_clone() {
        let api = api_with_named_field("ConversionOptions", true);
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The field accessor for `inner` (a Clone type) must clone the value
        assert!(
            lib.content.contains(".clone()"),
            "Clone-capable Named field must emit .clone() in accessor:\n{}",
            lib.content
        );
    }

    /// When both `visitor_callbacks = true` AND a `[[crates.trait_bridges]]` entry with
    /// `bind_via = "options_field"` are configured, the generated lib.rs must include BOTH:
    ///   - The OptionsField vtable / bridge-new / options-setter / {prefix}_convert symbols
    ///   - The visitor-callbacks symbols: {prefix}_visitor_create, {prefix}_visitor_free,
    ///     {prefix}_convert_with_visitor
    ///
    /// This is the configuration used by html-to-markdown where Go/C use the OptionsField
    /// path and Java uses the callbacks-struct path.
    #[test]
    fn test_both_options_field_and_visitor_callbacks_emit_both_symbol_sets() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
bind_via = "options_field"
options_type = "ConversionOptions"
"#,
        );
        let api = visitor_api();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // OptionsField symbols must be present
        assert!(
            lib.content.contains("htm_options_set_visitor"),
            "must include htm_options_set_visitor from OptionsField path"
        );

        // Visitor-callbacks symbols must ALSO be present
        assert!(
            lib.content.contains("htm_visitor_create"),
            "must include htm_visitor_create from visitor_callbacks path"
        );
        assert!(
            lib.content.contains("htm_visitor_free"),
            "must include htm_visitor_free from visitor_callbacks path"
        );
        assert!(
            lib.content.contains("htm_convert_with_visitor"),
            "must include htm_convert_with_visitor from visitor_callbacks path"
        );

        // No duplicate htm_convert — only the OptionsField version is emitted
        let convert_count = lib.content.matches("fn htm_convert(").count();
        assert_eq!(
            convert_count, 1,
            "htm_convert must appear exactly once (no duplicate from visitor path)"
        );

        // htm_convert_with_visitor must embed the visitor in options, not pass it as a
        // 3rd argument to convert — because the OptionsField `convert` only takes 2 args.
        assert!(
            lib.content.contains("opts.visitor = visitor_handle"),
            "convert_with_visitor must embed visitor in options for OptionsField path"
        );
        assert!(
            !lib.content.contains("convert(&html_str, options_rs, visitor_handle"),
            "convert_with_visitor must NOT pass visitor as 3rd arg in OptionsField path"
        );
    }

    /// Fix 1 regression test: `type_ref_to_rust_type` must use the configured `core_import`
    /// for `TypeRef::Named` variants, not a hard-coded `"kreuzberg"` prefix.
    ///
    /// When a crate uses `core_import = "my_custom_lib"`, generated Vec/Map turbofish type
    /// annotations that reference Named types must use `my_custom_lib::TypeName`, not
    /// `kreuzberg::TypeName`.
    #[test]
    fn test_core_import_parameterization_uses_configured_import_not_hardcoded_kreuzberg() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-custom-lib"
sources = ["src/lib.rs"]
core_import = "my_custom_lib"
"#,
        );
        let api = sample_api();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The generated code must not contain the old hard-coded kreuzberg prefix
        // in any type annotation position.  (It may legitimately appear in doc comments
        // or string literals, but never as a Rust path qualifier in generated code.)
        assert!(
            !lib.content.contains("kreuzberg::"),
            "generated code must not hard-code 'kreuzberg::' when core_import is 'my_custom_lib'; got:\n{}",
            &lib.content[..lib.content.len().min(2000)]
        );
        // The configured import must appear as a qualifier for core types
        assert!(
            lib.content.contains("my_custom_lib::"),
            "generated code must use the configured core_import 'my_custom_lib::' as a type qualifier"
        );
    }

    /// Fix 2 regression test: functions returning `Result<Vec<u8>>` must use the out-param
    /// convention (i32 return + out_ptr/out_len/out_cap parameters) and the module must
    /// include a companion `{prefix}_free_bytes` function.
    #[test]
    fn test_bytes_result_return_uses_out_params_and_emits_free_bytes() {
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "render_page".to_string(),
                rust_path: "my_lib::render_page".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "page_index".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                return_type: TypeRef::Bytes,
                is_async: false,
                error_type: Some("MyError".to_string()),
                doc: "Render a page to PNG bytes.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The function must use out-params, not return *mut u8 directly
        assert!(
            lib.content.contains("out_ptr: *mut *mut u8"),
            "Result<Vec<u8>> function must have out_ptr out-param"
        );
        assert!(
            lib.content.contains("out_len: *mut usize"),
            "Result<Vec<u8>> function must have out_len out-param"
        );
        assert!(
            lib.content.contains("out_cap: *mut usize"),
            "Result<Vec<u8>> function must have out_cap out-param"
        );
        // The function must return i32, not *mut u8
        assert!(
            lib.content.contains("fn my_lib_render_page("),
            "function must be emitted with the correct FFI name"
        );
        // Vec::into_raw_parts must be used to decompose the result
        assert!(
            lib.content.contains("into_raw_parts()"),
            "Result<Vec<u8>> success arm must use Vec::into_raw_parts()"
        );
        // The module must include a free_bytes companion
        assert!(
            lib.content.contains("fn my_lib_free_bytes("),
            "module must include my_lib_free_bytes companion function"
        );
        assert!(
            lib.content.contains("Vec::from_raw_parts(ptr, len, cap)"),
            "free_bytes must reconstruct and drop the Vec via Vec::from_raw_parts"
        );
    }

    /// Verify that a `Streaming` adapter causes codegen to emit the three iterator-handle
    /// functions (`_start`, `_next`, `_free`) plus the opaque handle struct.
    #[test]
    fn test_streaming_adapter_emits_iterator_handle_functions() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatChunk"
error_type = "MyError"
request_type = "my_lib::ChatRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatRequest"
"#,
        );
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "DefaultClient".to_string(),
                rust_path: "my_lib::DefaultClient".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "chat_stream".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: true,
                    is_static: false,
                    error_type: Some("MyError".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    receiver: Some(ReceiverKind::Ref),
                    trait_source: None,
                    has_default_impl: false,
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Opaque handle struct must be present
        assert!(
            lib.content.contains("MlDefaultClientChatStreamStreamHandle"),
            "handle struct must be emitted: got\n{}",
            &lib.content[..lib.content.len().min(3000)]
        );

        // All three exported functions must be present
        assert!(
            lib.content.contains("fn ml_default_client_chat_stream_start("),
            "_start function must be emitted"
        );
        assert!(
            lib.content.contains("fn ml_default_client_chat_stream_next("),
            "_next function must be emitted"
        );
        assert!(
            lib.content.contains("fn ml_default_client_chat_stream_free("),
            "_free function must be emitted"
        );

        // Functions must be #[unsafe(no_mangle)] extern "C"
        assert!(
            lib.content.contains("#[unsafe(no_mangle)]"),
            "functions must be marked #[unsafe(no_mangle)]"
        );
        assert!(
            lib.content
                .contains("pub unsafe extern \"C\" fn ml_default_client_chat_stream_start"),
            "_start must be pub unsafe extern C"
        );
        assert!(
            lib.content
                .contains("pub unsafe extern \"C\" fn ml_default_client_chat_stream_next"),
            "_next must be pub unsafe extern C"
        );
        assert!(
            lib.content
                .contains("pub unsafe extern \"C\" fn ml_default_client_chat_stream_free"),
            "_free must be pub unsafe extern C"
        );

        // _next must return a pointer to the item type
        assert!(
            lib.content.contains("-> *mut my_lib::ChatChunk"),
            "_next must return *mut my_lib::ChatChunk"
        );

        // _free must be null-safe
        assert!(
            lib.content.contains("if !handle.is_null()"),
            "_free must check for null before dropping"
        );

        // SAFETY comments must be present
        assert!(
            lib.content.contains("// SAFETY:"),
            "generated code must include SAFETY comments on unsafe blocks"
        );

        // Error protocol: _next sets last_error on stream errors
        assert!(
            lib.content.contains("set_last_error"),
            "_next must call set_last_error on error"
        );
    }
}
