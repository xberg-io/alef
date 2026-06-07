mod functions;
mod helpers;
mod service_api;
mod types;

use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use heck::ToPascalCase;
use std::path::PathBuf;

use crate::adapters::AdapterBodies;
use crate::core::config::AdapterPattern;

use functions::{
    gen_free_function, gen_free_function_len_companion, gen_method_wrapper, gen_streaming_method_wrapper,
    returns_c_char, should_skip_method_wrapper,
};
use helpers::{
    gen_build_rs, gen_cbindgen_toml, gen_ffi_tokio_runtime, gen_free_bytes, gen_free_string, gen_last_error,
    gen_version,
};
use types::{
    gen_enum_free, gen_enum_from_i32, gen_enum_from_i32_rs_helper, gen_enum_from_json, gen_enum_to_i32,
    gen_enum_to_json, gen_enum_to_string, gen_field_accessor, gen_opaque_static_constructor, gen_type_free,
    gen_type_from_json, gen_type_new, gen_type_to_json, is_static_constructor,
};

pub struct FfiBackend;

impl FfiBackend {}

fn named_type_ref(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => named_type_ref(inner),
        _ => None,
    }
}

fn has_trait_bridge_param(func: &FunctionDef, trait_bridges: &[TraitBridgeConfig]) -> bool {
    func.params.iter().any(|param| {
        let param_type = named_type_ref(&param.ty);
        trait_bridges.iter().any(|bridge| {
            bridge.bind_via != BridgeBinding::OptionsField
                && (bridge.param_name.as_deref() == Some(param.name.as_str())
                    || bridge.type_alias.as_deref() == param_type)
        })
    })
}

fn options_field_bridge_for_function<'a>(
    func: &'a FunctionDef,
    trait_bridges: &'a [TraitBridgeConfig],
) -> Option<(&'a ParamDef, &'a str)> {
    trait_bridges
        .iter()
        .filter(|bridge| bridge.bind_via == BridgeBinding::OptionsField)
        .find_map(|bridge| {
            let options_type = bridge.options_type.as_deref()?;
            let options_param = func
                .params
                .iter()
                .find(|param| named_type_ref(&param.ty) == Some(options_type))?;
            Some((options_param, options_type))
        })
}

fn function_param_bridge_for_visitor_callbacks<'a>(
    api: &'a ApiSurface,
    trait_bridges: &'a [TraitBridgeConfig],
) -> Option<(&'a TraitBridgeConfig, &'a FunctionDef)> {
    trait_bridges
        .iter()
        .filter(|bridge| bridge.bind_via != BridgeBinding::OptionsField)
        .find_map(|bridge| {
            api.functions
                .iter()
                .find(|func| has_trait_bridge_param(func, std::slice::from_ref(bridge)))
                .map(|func| (bridge, func))
        })
}

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
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.ffi_prefix();
        let header_name = config.ffi_header_name();
        let lib_name = config.ffi_lib_name();

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
                content: gen_build_rs(&header_name, &format!("lib{lib_name}"), go_output_dir.as_deref()),
                generated_header: false,
            },
        ];

        Ok(files)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
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
    // Unsafe extern "C" functions generated here do not have `# Safety` sections in their
    // rustdoc because the safety contract is documented at the C header level (cbindgen output).
    // Doc list indentation reflects the source format and is intentional in generated code.
    builder.add_inner_attribute(
        "allow(clippy::missing_safety_doc, clippy::doc_lazy_continuation, clippy::doc_overindented_list_items)",
    );

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
    // All UNIT-VARIANT enum names — used for FFI param-type emission (i32 discriminant) and the
    // matching from_i32 body conversion. Only unit-variant enums can be round-tripped through a
    // bare i32 discriminant: data-bearing variants (tuple or struct) carry field data that cannot
    // be reconstructed from the discriminant alone. The is_copy flag is intentionally not checked
    // here — a non-Copy unit-variant enum (e.g. one missing the Copy derive) can still be passed
    // by value over the C boundary using the auto-generated from_i32_rs match helper.
    let ffi_param_enums: ahash::AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty() && !v.is_tuple))
        .map(|e| e.name.clone())
        .collect();
    // Clone-but-not-Copy named types (structs + data-bearing enums). Callers emit `.clone()`.
    let clone_names: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && t.is_clone && !t.is_copy)
        .map(|t| t.name.clone())
        .chain(api.enums.iter().filter(|e| !e.is_copy).map(|e| e.name.clone()))
        .collect();
    // Named types that derive serde::Serialize. Required so the JSON return path for
    // Vec<Named> and Map<K, Named> is only emitted when serialization is actually available.
    // Types without has_serde get a stubbed (unimplemented) body instead.
    let serde_names: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_serde)
        .map(|t| t.name.clone())
        .chain(api.enums.iter().map(|e| e.name.clone())) // enums are always representable
        .collect();

    // Extract fields_c_types from e2e config if present.
    // This allows field accessors to override their return types when e2e explicitly
    // maps a field to an opaque handle type (e.g. "markdown_result.citations" = "CitationResult").
    let empty_fields_c_types = std::collections::HashMap::new();
    let fields_c_types = config
        .e2e
        .as_ref()
        .map(|e2e| &e2e.fields_c_types)
        .unwrap_or(&empty_fields_c_types);

    // Import traits needed for trait method dispatch
    for trait_path in generators::collect_trait_imports(api) {
        builder.add_import(&trait_path);
    }
    // FFI backend uses fully qualified paths (e.g. sample_crate::ParseOptions)
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
            matches!(f.ty, crate::core::ir::TypeRef::Json | crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _))
                || matches!(&f.ty, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Json | crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)))
        })
    });
    let has_serde_returns = api.types.iter().any(|t| {
        t.methods.iter().any(|m| {
            matches!(m.return_type, crate::core::ir::TypeRef::Json | crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _))
                || matches!(&m.return_type, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Json | crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)))
        })
    }) || api.functions.iter().any(|f| {
        matches!(f.return_type, crate::core::ir::TypeRef::Json | crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _))
            || matches!(&f.return_type, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Json | crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)))
    });
    if has_from_json_types || has_serde_fields || has_serde_returns {
        builder.add_import("serde_json");
    }

    // Custom module declarations
    let custom_mods = config.custom_modules.for_language(Language::Ffi);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    // Service API module (when services are present)
    if !api.services.is_empty() {
        builder.add_item("pub mod service;");
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
    let adapter_bodies: AdapterBodies =
        crate::adapters::build_adapter_bodies(config, Language::Ffi).unwrap_or_default();

    // Emit the stream callback type alias once if any streaming adapters exist.
    let has_streaming_adapters = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    if has_streaming_adapters {
        builder.add_item(&format!(
            "/// Callback invoked for each streamed chunk.\n\
             /// `chunk_json` is a JSON-encoded chunk; `user_data` is forwarded from the caller.\n\
             pub type {}StreamCallback =\n    \
             unsafe extern \"C\" fn(chunk_json: *const std::ffi::c_char, user_data: *mut std::ffi::c_void);",
            prefix.to_pascal_case()
        ));

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

    // Private Rust helpers: for every enum that may be passed as an `i32` discriminant param,
    // emit a `fn {enum_snake}_from_i32_rs(v: i32) -> Option<{qualified}>` helper. These are
    // used by constructor and method/function bodies so they don't reference a non-existent
    // `{core_import}::{enum_snake}_from_i32` Rust function (that would be the exported C ABI
    // helper, not a Rust-level conversion function).
    for enum_def in &api.enums {
        if ffi_param_enums.contains(&enum_def.name)
            && crate::codegen::conversions::can_generate_enum_conversion(enum_def)
        {
            builder.add_item(&gen_enum_from_i32_rs_helper(enum_def, &core_import));
        }
    }

    // Collect the set of type names excluded via [ffi] exclude_types, plus service-owner and
    // handler-contract types flagged `binding_excluded` by the service extraction pass. Those are
    // emitted through the service-API path (service.rs); also wrapping them as plain opaques here
    // would collide on the `_new`/`_free` C symbols. Configured opaque handle types that remain
    // present in the IR still need their lifecycle symbols for downstream FFI consumers.
    let mut ffi_exclude_types: ahash::AHashSet<&str> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_types.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    ffi_exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()));
    // Exclude workspace-declared opaque types whose `rust_path` carries generic parameters
    // (e.g. `Arc<Mutex<dyn Trait>>`), as the C ABI cannot represent them. Simple newtypes
    // (no generics) are left in so the binding layer emits free functions for them.
    let exclude_generic_opaques: ahash::AHashSet<&str> = config
        .opaque_types
        .iter()
        .filter(|(_, path)| path.contains('<'))
        .map(|(name, _)| name.as_str())
        .collect();
    ffi_exclude_types.extend(exclude_generic_opaques);

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

        // Client constructor — emit #[no_mangle] extern "C" fn {prefix}_{snake}_new(...)
        if let Some(ctor) = config.client_constructors.get(&typ.name) {
            let source_path = if core_import.is_empty() {
                typ.name.clone()
            } else {
                format!("{}::{}", core_import, typ.name)
            };
            let params_str = ctor
                .params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.ty))
                .collect::<Vec<_>>()
                .join(", ");
            let body = ctor
                .body
                .replace("{type_name}", &typ.name)
                .replace("{source_path}", &source_path);
            let err_ty = ctor.error_type.as_deref().unwrap_or("String");
            builder.add_item(&gen_type_new(typ, prefix, &core_import, &params_str, &body, err_ty));
        }

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
                    fields_c_types,
                ));
            }
        }

        // Opaque static constructors — emit opaque struct + extern "C" fn for static `new` methods
        if typ.is_opaque {
            // The static constructor returns `*mut {qualified}` to match the legacy
            // `_free()` signature — no separate `{TypeName}Opaque` wrapper is emitted.
            for method in &typ.methods {
                if is_static_constructor(method, &typ.name) {
                    builder.add_item(&gen_opaque_static_constructor(
                        typ,
                        method,
                        prefix,
                        &core_import,
                        &path_map,
                        &ffi_param_enums,
                    ));
                }
            }
        }

        // Build exclude set for FFI method emission. Defense-in-depth against API surface
        // inconsistencies: ensures methods listed in config.exclude.methods are never emitted,
        // even if they appear in typ.methods (which should not happen post-extraction, but
        // prevents header/impl desynchronization if an excluded method somehow persists).
        let ffi_exclude_methods: ahash::AHashSet<String> = config.exclude.methods.iter().cloned().collect();

        // Method wrappers — streaming adapters get a dedicated callback-based wrapper.
        for method in &typ.methods {
            // Check if this method is excluded by config.exclude.methods.
            let method_key = format!("{}.{}", typ.name, method.name);
            if ffi_exclude_methods.contains(&method_key) {
                continue;
            }

            // Skip methods with generic type parameters or builder-style returns.
            // These are handled via the service-API registration path instead.
            if should_skip_method_wrapper(method, typ, &path_map) {
                continue;
            }

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
                &ffi_param_enums,
                &serde_names,
            ));
        }
    }

    // Enum functions (from_i32 + to_i32) — only for simple unit-variant enums
    for enum_def in &api.enums {
        if crate::codegen::conversions::can_generate_enum_conversion(enum_def) {
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
                crate::core::ir::TypeRef::Named(n) => Some(n.clone()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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
                    crate::core::ir::TypeRef::Named(n) => Some(n.clone()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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
                    crate::core::ir::TypeRef::Named(n) => Some(n.clone()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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

        // Collect enum names used as streaming-adapter `item_type`. The streaming `_next`
        // helper (emitted via `gen_stream_handle_functions`) returns `*mut item_type`, so
        // when the item is an enum the FFI surface must also expose `_to_json` and `_free`
        // for that enum — otherwise downstream language bindings (Go, Ruby, Java, C#, …)
        // that drive the stream by repeatedly calling `_next` + `_to_json` + `_free` have
        // no way to consume the returned pointer.
        for adapter in &config.adapters {
            if !matches!(adapter.pattern, AdapterPattern::Streaming) {
                continue;
            }
            let Some(item_type) = adapter.item_type.as_deref() else {
                continue;
            };
            if api.enums.iter().any(|e| e.name == item_type) {
                enum_pointer_return.insert(item_type.to_string());
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
                    crate::core::ir::TypeRef::Named(n) => Some(n.clone()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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
                    if crate::codegen::conversions::can_generate_enum_conversion(enum_def) {
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
        .any(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField);

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
        // clear_fn functions are emitted by the trait bridge layer; emitting them here
        // too would produce duplicate C symbols in the generated FFI header.
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        // For legacy FunctionParam visitor bridges, skip sanitized bridge stubs; the
        // visitor-specific support emits those ABI entrypoints separately.
        if visitor_callbacks_enabled && func.sanitized && has_trait_bridge_param(func, &config.trait_bridges) {
            continue;
        }
        if has_options_field_bridge {
            if let Some((options_param, options_type_name)) =
                options_field_bridge_for_function(func, &config.trait_bridges)
            {
                if let Some(wrapper) = crate::backends::ffi::gen_bridge_field::gen_function_with_options_field_bridge(
                    prefix,
                    &core_import,
                    func,
                    options_param,
                    options_type_name,
                ) {
                    builder.add_item(&wrapper);
                    continue;
                }
            }
        }
        builder.add_item(&gen_free_function(
            func,
            prefix,
            &core_import,
            &path_map,
            &ffi_param_enums,
            &serde_names,
        ));
        // Emit a _len() companion for every function whose return type maps to *mut c_char
        // so that Zig and Java FFM Panama consumers get byte length without a NUL-scan.
        if returns_c_char(&func.return_type) {
            builder.add_item(&gen_free_function_len_companion(
                func,
                prefix,
                &core_import,
                &path_map,
                &ffi_param_enums,
            ));
        }
    }

    // Visitor/callback FFI support.
    // - OptionsField bridge: VTable + options setter + correct convert implementation.
    // - FunctionParam bridge (legacy): VisitorCallbacks struct + convert_with_visitor.
    //
    // When both flags are active simultaneously (for example, mixed callback modes with
    // `visitor_callbacks = true` and an `[[trait_bridges]]` entry using
    // `bind_via = "options_field"`), we emit BOTH:
    //   1. The OptionsField vtable / options-setter / {prefix}_convert  (used by Go, C)
    //   2. The visitor-callbacks symbols ({prefix}_visitor_create/free/convert_with_visitor) (used by Java)
    // The two sets of symbols use different function names and do not conflict.
    if has_options_field_bridge {
        // Build a type_paths map for delegation method signature generation.
        let type_paths: std::collections::HashMap<String, String> =
            path_map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let trait_map: ahash::AHashMap<&str, &crate::core::ir::TypeDef> = api
            .types
            .iter()
            .filter(|t| t.is_trait)
            .map(|t| (t.name.as_str(), t))
            .collect();

        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.bind_via != crate::core::config::BridgeBinding::OptionsField {
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

            builder.add_item(&crate::backends::ffi::gen_bridge_field::gen_options_set_bridge(
                prefix,
                &core_import,
                trait_def,
                &bridge_cfg.trait_name,
                field_name,
                options_type_name,
                &type_paths,
            ));
        }

        // When visitor_callbacks is also enabled, additionally emit the callback lifecycle
        // symbols ({prefix}_visitor_create/free) needed by Java's Panama FFM binding.
        // The legacy {prefix}_options_set_visitor_handle setter and {prefix}_*_with_visitor
        // wrapper stay isolated to the explicit FunctionParam path below.
        if visitor_callbacks_enabled {
            // Use the first OptionsField bridge's trait to drive callback spec generation.
            let visitor_trait_def = config
                .trait_bridges
                .iter()
                .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
                .find_map(|b| {
                    trait_map
                        .get(b.trait_name.as_str())
                        .copied()
                        .map(|trait_def| (trait_def, b))
                });
            if let Some((vtd, bridge_cfg)) = visitor_trait_def {
                builder.add_item(&crate::backends::ffi::gen_visitor::gen_visitor_bindings_with_api(
                    prefix,
                    &core_import,
                    true,
                    vtd,
                    Some(bridge_cfg),
                    None,
                    Some(api),
                    false,
                ));
            } else {
                eprintln!(
                    "[alef] gen_visitor_bindings(ffi): visitor_callbacks=true but no OptionsField trait found in IR, skipping visitor callbacks"
                );
            }
        }
    } else if visitor_callbacks_enabled {
        // FunctionParam path: emit a no-visitor wrapper and a visitor wrapper only when
        // config identifies the bridge parameter and the matching public function.
        let configured_bridge = function_param_bridge_for_visitor_callbacks(api, &config.trait_bridges);
        if let Some((bridge_cfg, visitor_function)) = configured_bridge {
            let visitor_trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name);
            if let Some(vtd) = visitor_trait_def {
                builder.add_item(&crate::backends::ffi::gen_visitor::gen_convert_no_visitor(
                    prefix,
                    &core_import,
                    Some(bridge_cfg),
                    Some(visitor_function),
                ));
                builder.add_item(&crate::backends::ffi::gen_visitor::gen_visitor_bindings_with_api(
                    prefix,
                    &core_import,
                    false,
                    vtd,
                    Some(bridge_cfg),
                    Some(visitor_function),
                    Some(api),
                    true,
                ));
            } else {
                eprintln!(
                    "[alef] gen_visitor_bindings(ffi): visitor_callbacks=true but configured trait `{}` is not present in IR, skipping visitor callbacks",
                    bridge_cfg.trait_name
                );
            }
        } else {
            eprintln!(
                "[alef] gen_visitor_bindings(ffi): visitor_callbacks=true but no FunctionParam trait bridge matched a public function, skipping visitor callbacks"
            );
        }
    }

    // Error introspection helpers — `extern "C"` wrappers for whitelisted methods
    // (status_code, is_transient, error_type) declared in ErrorDef.methods.
    for error in &api.errors {
        let methods_code = crate::codegen::error_gen::gen_ffi_error_methods(error, &core_import, prefix);
        if !methods_code.is_empty() {
            builder.add_item(&methods_code);
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
        builder.add_item(&crate::backends::ffi::trait_bridge::gen_ffi_set_out_error_helper());

        let trait_map: ahash::AHashMap<&str, &crate::core::ir::TypeDef> = api
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
                let bridge_code = crate::backends::ffi::trait_bridge::gen_trait_bridge(
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
                if bridge_cfg.bind_via == crate::core::config::BridgeBinding::OptionsField {
                    let pascal_prefix = prefix.to_pascal_case();
                    builder.add_item(&crate::backends::ffi::trait_bridge::gen_bridge_new_free(
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
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::*;

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

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
context_type = "NodeContext"
result_type = "VisitResult"
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

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
context_type = "NodeContext"
result_type = "VisitResult"
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        original_type: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        original_type: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        original_type: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![EnumDef {
                name: "OutputFormat".to_string(),
                rust_path: "my_lib::OutputFormat".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Text".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        is_tuple: false,
                        originally_had_data_fields: false,
                    },
                    EnumVariant {
                        name: "Html".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        is_tuple: false,
                        originally_had_data_fields: false,
                    },
                ],
                doc: "Output format.".to_string(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// Like `sample_api()` but includes a `SyntaxWalker` trait with representative methods.
    ///
    /// Use this for tests that exercise visitor callback generation.  The methods cover each
    /// `ParamKind` variant: Str, OptStr, Bool, U32, Usize, CellSlice, and no-params.
    fn visitor_api() -> ApiSurface {
        let mut api = sample_api();
        api.types.push(TypeDef {
            name: "NodeContext".to_string(),
            rust_path: "my_lib::visitor::NodeContext".to_string(),
            fields: vec![
                FieldDef {
                    name: "node_type".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "tag_name".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "depth".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Usize),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "index_in_parent".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Usize),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "parent_tag".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "is_inline".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        });
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit text nodes.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    }],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Called before entering any element.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit anchor links.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit heading elements.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit blockquote elements.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit list items.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("VisitResult".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Visit table rows.".to_string(),
                    receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        });
        api.types.push(TypeDef {
            name: "RenderSettings".to_string(),
            rust_path: "my_lib::RenderSettings".to_string(),
            fields: vec![],
            is_clone: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "RenderedDocument".to_string(),
            rust_path: "my_lib::RenderedDocument".to_string(),
            fields: vec![],
            is_clone: true,
            is_return_type: true,
            ..TypeDef::default()
        });
        api.enums.push(EnumDef {
            name: "VisitResult".to_string(),
            rust_path: "my_lib::visitor::VisitResult".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Continue".to_string(),
                    fields: vec![],
                    is_default: true,
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Skip".to_string(),
                    fields: vec![],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "PreserveHtml".to_string(),
                    fields: vec![],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    fields: vec![visitor_result_string_field("output")],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Error".to_string(),
                    fields: vec![visitor_result_string_field("message")],
                    ..EnumVariant::default()
                },
            ],
            has_serde: true,
            ..EnumDef::default()
        });
        api.functions.push(FunctionDef {
            name: "render_document".to_string(),
            rust_path: "my_lib::render_document".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    is_ref: false,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "settings".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "visitor".to_string(),
                    ty: TypeRef::Named("VisitorHandle".to_string()),
                    optional: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("RenderedDocument".to_string()),
            is_async: false,
            error_type: Some("RenderError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: true,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });
        api
    }

    fn visitor_result_string_field(name: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
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
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![EnumDef {
                name: "Color".to_string(),
                rust_path: "my_lib::Color".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Red".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        is_tuple: false,
                        originally_had_data_fields: false,
                    },
                    EnumVariant {
                        name: "Green".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        is_tuple: false,
                        originally_had_data_fields: false,
                    },
                ],
                doc: "Colors.".to_string(),
                cfg: None,
                is_copy: false,
                has_serde: true,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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
        assert!(cbindgen.content.contains("style = \"both\""));
    }

    // -----------------------------------------------------------------------
    // Doxygen comment emission on extern "C" fn, opaque typedefs, and enums.
    //
    // These tests assert the structural shape of the generated Rust source
    // (`pub unsafe extern "C" fn` declarations carry `\param`, `\return`,
    // `\note` markers; opaque-handle `typedef` lines in cbindgen.toml carry
    // a `/** ... */` block). cbindgen forwards these into the final `.h` file.
    // -----------------------------------------------------------------------

    fn doxygen_sample_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "Handle".to_string(),
                rust_path: "my_lib::Handle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
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
                doc: "An opaque handle that wraps the underlying resource.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![FunctionDef {
                name: "lookup".to_string(),
                rust_path: "my_lib::lookup".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "name".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                is_async: false,
                error_type: Some("MyError".to_string()),
                doc: "Look up the registry index for a name.\n\n\
                      # Arguments\n\n\
                      * `name` - The unique key to search.\n\n\
                      # Returns\n\n\
                      A non-zero index when found; zero on lookup miss.\n\n\
                      # Errors\n\n\
                      Returns the last-error code when the registry is poisoned."
                    .to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![EnumDef {
                name: "Severity".to_string(),
                rust_path: "my_lib::Severity".to_string(),
                original_rust_path: String::new(),
                variants: vec![EnumVariant {
                    name: "Warn".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                }],
                doc: "Diagnostic severity level.".to_string(),
                cfg: None,
                is_copy: true,
                has_serde: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    #[test]
    fn test_extern_fn_emits_doxygen_param_return_note_markers() {
        let api = doxygen_sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The generated extern fn carries Doxygen markers derived from the
        // upstream rustdoc sections.
        assert!(
            lib.content.contains("/// \\param name The unique key to search."),
            "expected \\param marker for `name`, got:\n{}",
            lib.content
        );
        assert!(
            lib.content
                .contains("/// \\return A non-zero index when found; zero on lookup miss."),
            "expected \\return marker, got:\n{}",
            lib.content
        );
        assert!(
            lib.content
                .contains("/// \\note Returns the last-error code when the registry is poisoned."),
            "expected \\note marker for # Errors, got:\n{}",
            lib.content
        );
        // The universal FFI safety clause is now expressed as a Doxygen note
        // (the previous hard-coded `/// # Safety` lines have been removed
        // from the templates).
        assert!(
            lib.content.contains("/// \\note SAFETY:"),
            "expected \\note SAFETY: marker derived from synthetic safety clause, got:\n{}",
            lib.content
        );
    }

    #[test]
    fn test_opaque_typedef_carries_doxygen_block_in_cbindgen_toml() {
        let api = doxygen_sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
        toml::from_str::<toml::Value>(&cbindgen.content).expect("cbindgen.toml must be valid TOML");

        // Doxygen block precedes the typedef in `forward_decls`. The doc text
        // is lifted from `TypeDef.doc` and rendered as `/** * ... */`.
        assert!(
            cbindgen.content.contains("/**"),
            "expected /** doxygen opener, got:\n{}",
            cbindgen.content
        );
        assert!(
            cbindgen
                .content
                .contains("* An opaque handle that wraps the underlying resource."),
            "expected typedef doc body, got:\n{}",
            cbindgen.content
        );
        assert!(
            cbindgen.content.contains("typedef struct MY_LIBHandle MY_LIBHandle;"),
            "expected prefixed typedef, got:\n{}",
            cbindgen.content
        );
    }

    #[test]
    fn test_cbindgen_toml_escapes_doxygen_backslashes() {
        let mut api = doxygen_sample_api();
        api.types[0].doc = r##"Has an example.

# Example

```rust
let value = "triple """ quote";
```"##
            .to_string();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
        let parsed = toml::from_str::<toml::Value>(&cbindgen.content).expect("cbindgen.toml must parse");
        let after_includes = parsed
            .get("after_includes")
            .and_then(toml::Value::as_str)
            .expect("after_includes must be a string");

        assert!(
            after_includes.contains("\\code") && after_includes.contains("\\endcode"),
            "Doxygen markers must survive TOML parsing: {after_includes}"
        );
        assert!(
            after_includes.contains("triple \"\"\" quote"),
            "triple quotes must round-trip through TOML parsing: {after_includes}"
        );
    }

    #[test]
    fn test_enum_opaque_typedef_carries_doxygen_block() {
        let api = doxygen_sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

        // The `Severity` enum is included as an opaque forward declaration
        // (enums travel across FFI as `*mut EnumName`). Its rustdoc must
        // surface as a Doxygen block above the typedef.
        assert!(
            cbindgen.content.contains("* Diagnostic severity level."),
            "expected enum typedef doc body, got:\n{}",
            cbindgen.content
        );
        assert!(
            cbindgen
                .content
                .contains("typedef struct MY_LIBSeverity MY_LIBSeverity;"),
            "expected prefixed enum typedef, got:\n{}",
            cbindgen.content
        );
    }

    /// Every error type whose accessor functions are emitted must also have a
    /// forward `typedef struct` in the cbindgen.toml `after_includes` block.
    /// Without it cbindgen produces an "unknown type name" compile error because
    /// the accessor signature references `*const ErrorType` but no opaque struct
    /// is declared in the header.
    #[test]
    fn test_error_type_with_methods_gets_opaque_typedef_in_cbindgen_toml() {
        let mut api = sample_api();
        // Add an error type with a whitelisted method — this is what triggers
        // gen_ffi_error_methods to emit `*const GraphQLError` in the accessor.
        api.errors.push(ErrorDef {
            name: "GraphQLError".to_string(),
            rust_path: "my_lib::GraphQLError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: "GraphQL execution error.".to_string(),
            methods: vec![MethodDef {
                name: "status_code".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "HTTP status code for the error.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            binding_excluded: false,
            binding_exclusion_reason: None,
        });

        let config = sample_config();
        let backend = FfiBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();

        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

        // The accessor function references *const MY_LIBGraphQLError — the typedef must exist.
        assert!(
            cbindgen
                .content
                .contains("typedef struct MY_LIBGraphQLError MY_LIBGraphQLError;"),
            "expected opaque typedef for error type with methods, got:\n{}",
            cbindgen.content
        );

        // Also verify the accessor itself is emitted in lib.rs.
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(
            lib.content.contains("my_lib_graph_q_l_error_status_code"),
            "expected accessor fn for error type, got:\n{}",
            lib.content
        );
    }

    /// Error types without any whitelisted methods must NOT produce a spurious
    /// typedef — the accessor function is not emitted so there is nothing to
    /// declare.
    #[test]
    fn test_error_type_without_methods_does_not_get_typedef_in_cbindgen_toml() {
        let mut api = sample_api();
        api.errors.push(ErrorDef {
            name: "SilentError".to_string(),
            rust_path: "my_lib::SilentError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        });

        let config = sample_config();
        let backend = FfiBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();
        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

        assert!(
            !cbindgen.content.contains("SilentError"),
            "error type with no methods must not appear in cbindgen.toml, got:\n{}",
            cbindgen.content
        );
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
    fn test_visitor_callbacks_without_matching_bridge_do_not_emit_fallback_conversion_api() {
        let api = visitor_api();
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"
visitor_callbacks = true
"#,
        );
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(!lib.content.contains("VisitorCallbacks"));
        assert!(!lib.content.contains("doc_convert"));
        assert!(!lib.content.contains("DocOptions"));
        assert!(!lib.content.contains("DocResult"));
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
        assert!(lib.content.contains("pub struct HtmContext"));

        // Visit-result codes should be defined
        assert!(lib.content.contains("HTM_VISIT_CONTINUE"));
        assert!(lib.content.contains("HTM_VISIT_SKIP"));
        assert!(lib.content.contains("HTM_VISIT_PRESERVE_HTML"));
        assert!(lib.content.contains("HTM_VISIT_CUSTOM"));
        assert!(lib.content.contains("HTM_VISIT_ERROR"));

        // SyntaxContext fields
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
        assert!(lib.content.contains("htm_render_document_with_visitor"));

        // Functions should be extern "C"
        assert!(lib.content.contains("extern \"C\" fn htm_visitor_create"));
        assert!(lib.content.contains("extern \"C\" fn htm_visitor_free"));
        assert!(lib.content.contains("extern \"C\" fn htm_render_document_with_visitor"));
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
        assert!(lib.content.contains("*const HtmContext"));
        assert!(lib.content.contains("user_data: *mut std::ffi::c_void"));
        assert!(lib.content.contains("out_custom: *mut *mut std::ffi::c_char"));
        assert!(lib.content.contains("out_len: *mut usize"));

        // Return type should be i32
        assert!(lib.content.contains(") -> i32"));
    }

    #[test]
    fn test_visitor_callbacks_generate_param_setup_blocks() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(lib.content.contains("let text_cs = match std::ffi::CString::new(text)"));
        assert!(
            lib.content
                .contains("let (title_ptr, _title_cs) = opt_str_to_c(title);")
        );
        assert!(lib.content.contains("let ordered_i = i32::from(ordered);"));
        assert!(
            lib.content
                .contains("let cells_cstrings: Vec<std::ffi::CString> = cells")
        );
        assert!(lib.content.contains("let cell_count = cells_ptrs.len();"));
        assert!(
            lib.content
                .contains("cb(c_ctx, user_data, cells_ptrs.as_ptr(), cell_count, is_header_i")
        );
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
        assert!(lib.content.contains("MlContext"));
        assert!(lib.content.contains("ml_visitor_create"));
        assert!(lib.content.contains("ml_visitor_free"));
        assert!(lib.content.contains("ml_render_document_with_visitor"));
        assert!(lib.content.contains("ML_VISIT_CONTINUE"));
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
        // VisitorRef should implement SyntaxWalker trait (core_import is my_lib for this test)
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
        assert!(lib.content.contains("VisitorResult::Skip"));
        assert!(lib.content.contains("VisitorResult::PreserveHtml"));
        assert!(lib.content.contains("VisitorResult::Custom"));
        assert!(lib.content.contains("VisitorResult::Error"));
    }

    #[test]
    fn test_legacy_visitor_callbacks_use_configured_context_and_result_metadata() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "prs"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "SyntaxVisitor"
type_alias = "SyntaxVisitorHandle"
param_name = "visitor"
options_type = "ParseOptions"
context_type = "ParseContext"
result_type = "WalkOutcome"
"#,
        );
        let mut api = sample_api();
        api.types.push(TypeDef {
            name: "SyntaxVisitor".to_string(),
            rust_path: "my_lib::syntax::SyntaxVisitor".to_string(),
            methods: vec![MethodDef {
                name: "visit_token".to_string(),
                params: vec![
                    ParamDef {
                        name: "context".to_string(),
                        ty: TypeRef::Named("ParseContext".to_string()),
                        is_ref: true,
                        ..ParamDef::default()
                    },
                    ParamDef {
                        name: "token".to_string(),
                        ty: TypeRef::String,
                        is_ref: true,
                        ..ParamDef::default()
                    },
                ],
                return_type: TypeRef::Named("WalkOutcome".to_string()),
                receiver: Some(ReceiverKind::RefMut),
                doc: "Visit parser tokens.".to_string(),
                is_async: false,
                is_static: false,
                error_type: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_trait: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "ParseContext".to_string(),
            rust_path: "my_lib::syntax::ParseContext".to_string(),
            fields: vec![
                FieldDef {
                    name: "rule_name".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "byte_offset".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Usize),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "source_path".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "is_recovery".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "my_lib::ParseOptions".to_string(),
            is_clone: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "ParseTree".to_string(),
            rust_path: "my_lib::ParseTree".to_string(),
            is_clone: true,
            is_return_type: true,
            ..TypeDef::default()
        });
        api.enums.push(EnumDef {
            name: "WalkOutcome".to_string(),
            rust_path: "my_lib::syntax::WalkOutcome".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Proceed".to_string(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "StopHere".to_string(),
                    is_default: true,
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "ReplaceWith".to_string(),
                    fields: vec![visitor_result_string_field("replacement")],
                    ..EnumVariant::default()
                },
            ],
            has_serde: true,
            ..EnumDef::default()
        });
        api.functions.push(FunctionDef {
            name: "parse".to_string(),
            rust_path: "my_lib::parse".to_string(),
            params: vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string()))),
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "visitor".to_string(),
                    ty: TypeRef::Named("SyntaxVisitorHandle".to_string()),
                    optional: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("ParseTree".to_string()),
            error_type: Some("ParseError".to_string()),
            sanitized: true,
            ..FunctionDef::default()
        });
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(lib.content.contains("pub struct PrsContext"));
        assert!(lib.content.contains("pub rule_name: *const std::ffi::c_char"));
        assert!(lib.content.contains("pub byte_offset: usize"));
        assert!(lib.content.contains("pub source_path: *const std::ffi::c_char"));
        assert!(lib.content.contains("pub is_recovery: i32"));
        assert!(lib.content.contains("PRS_VISIT_STOP_HERE"));
        assert!(lib.content.contains("my_lib::syntax::WalkOutcome::StopHere"));
        assert!(lib.content.contains("VisitorResult::ReplaceWith(msg)"));
        assert!(lib.content.contains("context: &my_lib::syntax::ParseContext"));
        assert!(!lib.content.contains("my_lib::visitor::VisitResult"));
        assert!(!lib.content.contains("my_lib::visitor::NodeContext"));
    }

    #[test]
    fn test_visitor_callbacks_call_with_ctx() {
        let api = visitor_api();
        let config = visitor_config_htm();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper function for building and passing SyntaxContext to C callbacks
        assert!(lib.content.contains("call_with_ctx"));
        assert!(lib.content.contains("HtmContext"));
        assert!(lib.content.contains("tag_cstring"));
        assert!(lib.content.contains("parent_tag_cstring"));
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
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    original_type: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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
                core_wrapper: crate::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                original_type: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![holder, named_type],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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

    #[test]
    fn test_options_field_visitor_callbacks_use_configured_renderer_setter() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "syn"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "SyntaxWalker"
type_alias = "SyntaxWalkerHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "ParseOptions"
options_field = "renderer"
context_type = "SyntaxContext"
result_type = "WalkOutcome"
"#,
        );
        let mut api = sample_api();
        api.types.push(TypeDef {
            name: "SyntaxWalker".to_string(),
            rust_path: "my_lib::syntax::SyntaxWalker".to_string(),
            methods: vec![MethodDef {
                name: "visit_token".to_string(),
                params: vec![ParamDef {
                    name: "context".to_string(),
                    ty: TypeRef::Named("SyntaxContext".to_string()),
                    is_ref: true,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("WalkOutcome".to_string()),
                receiver: Some(ReceiverKind::RefMut),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_trait: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "SyntaxContext".to_string(),
            rust_path: "my_lib::syntax::SyntaxContext".to_string(),
            fields: vec![FieldDef {
                name: "rule_name".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "my_lib::ParseOptions".to_string(),
            is_clone: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "ParseResult".to_string(),
            rust_path: "my_lib::ParseResult".to_string(),
            is_clone: true,
            is_return_type: true,
            ..TypeDef::default()
        });
        api.enums.push(EnumDef {
            name: "WalkOutcome".to_string(),
            rust_path: "my_lib::syntax::WalkOutcome".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Continue".to_string(),
                    is_default: true,
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Stop".to_string(),
                    ..EnumVariant::default()
                },
            ],
            has_serde: true,
            ..EnumDef::default()
        });
        api.functions.push(FunctionDef {
            name: "parse".to_string(),
            rust_path: "my_lib::parse".to_string(),
            params: vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string()))),
                    optional: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("ParseResult".to_string()),
            error_type: Some("ParseError".to_string()),
            ..FunctionDef::default()
        });
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(
            lib.content.contains("syn_options_set_renderer"),
            "options-field setter must derive from configured renderer field"
        );
        assert!(
            !lib.content.contains("syn_options_set_visitor_handle"),
            "options-field mode must not emit the legacy visitor_handle setter"
        );
        assert!(
            lib.content.contains("pub struct SynVisitorCallbacks"),
            "Java callback lifecycle support should remain available"
        );
        assert!(
            lib.content.contains("syn_visitor_create") && lib.content.contains("syn_visitor_free"),
            "visitor create/free symbols should remain available"
        );
        let convert_count = lib.content.matches("fn syn_parse(").count();
        assert_eq!(convert_count, 1, "syn_parse must appear exactly once");
        assert!(
            !lib.content.contains("syn_parse_with_visitor"),
            "options-field mode must not emit the legacy with_visitor wrapper"
        );
    }

    #[test]
    fn test_options_field_bridge_generates_non_convert_function_from_ir() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "RenderHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "RenderSettings"
options_field = "renderer"
"#,
        );
        let mut api = visitor_api();
        api.types.push(TypeDef {
            name: "RenderSettings".to_string(),
            rust_path: "my_lib::RenderSettings".to_string(),
            fields: vec![],
            is_clone: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "RenderedDocument".to_string(),
            rust_path: "my_lib::RenderedDocument".to_string(),
            fields: vec![],
            is_clone: true,
            ..TypeDef::default()
        });
        api.functions.push(FunctionDef {
            name: "render_document".to_string(),
            rust_path: "my_lib::render_document".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "settings".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                    optional: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("RenderedDocument".to_string()),
            is_async: false,
            error_type: Some("RenderError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(
            lib.content.contains("fn doc_render_document("),
            "must generate IR-derived symbol"
        );
        assert!(
            lib.content.contains("settings: *const my_lib::RenderSettings"),
            "must use configured options type"
        );
        assert!(
            lib.content.contains(") -> *mut my_lib::RenderedDocument"),
            "must use actual return type"
        );
        assert!(
            lib.content
                .contains("match my_lib::render_document(source_rs, settings_rs)"),
            "must call actual core function with actual parameters"
        );
        assert!(
            !lib.content.contains("my_lib::convert("),
            "must not hardcode conversion call"
        );
        assert!(
            !lib.content.contains("ConversionOptions") && !lib.content.contains("ConversionResult"),
            "must not leak conversion-shaped type names in generic wrapper"
        );
    }

    #[test]
    fn test_legacy_visitor_callbacks_use_configured_function_signature() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "RenderHandle"
param_name = "renderer"
context_type = "NodeContext"
result_type = "VisitResult"
"#,
        );
        let mut api = visitor_api();
        api.types.push(TypeDef {
            name: "RenderSettings".to_string(),
            rust_path: "my_lib::RenderSettings".to_string(),
            fields: vec![],
            is_clone: true,
            ..TypeDef::default()
        });
        api.types.push(TypeDef {
            name: "RenderedDocument".to_string(),
            rust_path: "my_lib::RenderedDocument".to_string(),
            fields: vec![],
            is_clone: true,
            is_return_type: true,
            ..TypeDef::default()
        });
        api.functions.push(FunctionDef {
            name: "render_document".to_string(),
            rust_path: "my_lib::render_document".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    is_ref: false,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "settings".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "renderer".to_string(),
                    ty: TypeRef::Named("RenderHandle".to_string()),
                    optional: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("RenderedDocument".to_string()),
            is_async: false,
            error_type: Some("RenderError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: true,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(lib.content.contains("fn doc_render_document("));
        assert!(lib.content.contains("fn doc_render_document_with_visitor("));
        assert!(lib.content.contains("settings: *const my_lib::RenderSettings"));
        assert!(lib.content.contains(") -> *mut my_lib::RenderedDocument"));
        assert!(
            lib.content
                .contains("match my_lib::render_document(source_rs, settings_rs, None)")
        );
        assert!(
            lib.content
                .contains("match my_lib::render_document(source_rs, settings_rs, visitor_handle)")
        );
        assert!(!lib.content.contains("my_lib::convert("));
        assert!(
            !lib.content.contains("ConversionOptions") && !lib.content.contains("ConversionResult"),
            "legacy visitor callback path must not assume conversion-shaped names"
        );
    }

    /// Fix 1 regression test: `type_ref_to_rust_type` must use the configured `core_import`
    /// for `TypeRef::Named` variants, not a hard-coded `"sample_core"` prefix.
    ///
    /// When a crate uses `core_import = "my_custom_lib"`, generated Vec/Map turbofish type
    /// annotations that reference Named types must use `my_custom_lib::TypeName`, not
    /// `sample_core::TypeName`.
    #[test]
    fn test_core_import_parameterization_uses_configured_import_not_hardcoded_sample_crate() {
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

        // The generated code must not contain the old hard-coded sample_core prefix
        // in any type annotation position.  (It may legitimately appear in doc comments
        // or string literals, but never as a Rust path qualifier in generated code.)
        assert!(
            !lib.content.contains("sample_crate::"),
            "generated code must not hard-code 'sample_crate::' when core_import is 'my_custom_lib'; got:\n{}",
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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

    #[test]
    fn test_client_constructors_emits_type_new_function() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[workspace.client_constructors.DefaultClient]
body = "my_lib::DefaultClient::new(api_key)"
error_type = "String"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"
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
                methods: vec![],
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let backend = FfiBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(
            lib.content.contains("fn ml_default_client_new("),
            "should emit _new function: got\n{}",
            &lib.content[..lib.content.len().min(2000)]
        );
        assert!(
            lib.content.contains("api_key: *const std::ffi::c_char"),
            "should include typed param in signature"
        );
        assert!(
            lib.content.contains("-> *mut my_lib::DefaultClient"),
            "should return *mut TypeName"
        );
        assert!(
            lib.content.contains("clear_last_error"),
            "should call clear_last_error at function entry"
        );
        assert!(
            lib.content.contains("set_last_error"),
            "should call set_last_error on Err path"
        );
        assert!(
            lib.content.contains("Box::into_raw(Box::new(val))"),
            "should box the value on Ok path"
        );
    }

    /// Build an `ApiSurface` with a free function whose `metadata` param is
    /// `Option<&AHashMap<Cow<'static, str>, serde_json::Value>>` — the shape that
    /// `sample_core::text::quality::calculate_quality_score` uses. The IR records
    /// `map_is_ahash=true` and `map_key_is_cow=true` on the param.
    fn ahashmap_cow_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "calculate_quality_score".to_string(),
                rust_path: "my_lib::calculate_quality_score".to_string(),
                original_rust_path: String::new(),
                params: vec![
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
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "metadata".to_string(),
                        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
                        optional: true,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: true,
                        map_key_is_cow: true,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Primitive(PrimitiveType::F64),
                is_async: false,
                error_type: None,
                doc: "Calculate quality score for text.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// The FFI wrapper for a function with `Option<&AHashMap<Cow<'static, str>, Value>>` must:
    /// 1. Deserialize using `ahash::AHashMap<std::borrow::Cow<'static, str>, ...>` turbofish
    /// 2. Pass `.as_ref()` to the core function (not `.as_deref()`, which fails for HashMap)
    #[test]
    fn test_optional_ahashmap_cow_key_uses_as_ref_not_as_deref() {
        let api = ahashmap_cow_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The deserialization turbofish must target AHashMap with Cow key, not HashMap<String, _>
        assert!(
            lib.content.contains("ahash::AHashMap<std::borrow::Cow<'static, str>,"),
            "should deserialize into AHashMap<Cow<'static, str>, ...>, got:\n{}",
            if lib.content.len() > 3000 {
                &lib.content[lib.content.len() - 3000..]
            } else {
                &lib.content
            }
        );

        // The call must use .as_ref() not .as_deref() — HashMap doesn't impl Deref
        assert!(
            lib.content.contains("metadata_rs.as_ref()"),
            "should pass metadata_rs.as_ref() (not .as_deref()), got:\n{}",
            if lib.content.len() > 3000 {
                &lib.content[lib.content.len() - 3000..]
            } else {
                &lib.content
            }
        );
        assert!(
            !lib.content.contains("metadata_rs.as_deref()"),
            "must NOT use .as_deref() on HashMap — HashMap does not impl Deref"
        );
    }

    /// Regression guard: `Option<Vec<String>>` with `is_ref=true` must still use
    /// `.as_deref()` since `Vec<T>: Deref<Target=[T]>`.
    #[test]
    fn test_optional_vec_still_uses_as_deref() {
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "process_items".to_string(),
                rust_path: "my_lib::process_items".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "items".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        assert!(
            lib.content.contains("items_rs.as_deref()"),
            "Optional Vec<String> with is_ref=true should still use .as_deref()"
        );
    }

    /// Regression test for the sample_crate issue tracker.
    /// Struct fields typed `Option<Bytes>` / `Option<Vec<u8>>` (e.g. EmailAttachment.data)
    /// must emit the same (ptr, out_len: *mut usize) contract as non-optional Bytes fields.
    /// Previously the needs_len_out predicate only matched `Bytes && !optional`.
    #[test]
    fn test_optional_bytes_field_accessor_emits_out_len_and_length_writes() {
        let field = FieldDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
            optional: true,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: crate::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            original_type: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let typ = TypeDef {
            name: "EmailAttachment".to_string(),
            rust_path: "my_lib::EmailAttachment".to_string(),
            original_rust_path: String::new(),
            fields: vec![field.clone()],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };

        let code = gen_field_accessor(
            &typ,
            &field,
            "kr",
            "my_lib",
            &ahash::AHashMap::<String, String>::new(),
            &ahash::AHashSet::<String>::new(),
            &ahash::AHashSet::<String>::new(),
            &::std::collections::HashMap::<String, String>::new(),
        );

        // The header must include the out_len companion (the reported contract violation).
        assert!(
            code.contains("out_len: *mut usize"),
            "optional Bytes field accessor must declare out_len param (issue #118), got:\n{code}"
        );

        // Body must write real length on Some path.
        assert!(
            code.contains("*out_len"),
            "optional Bytes field must write length to out_len (Some path writes real len, None writes 0), got:\n{code}"
        );

        // None arm must write 0, not just any *out_len write.
        assert!(
            code.contains("*out_len = 0"),
            "optional Bytes None arm must write 0 to out_len, got:\n{code}"
        );

        // Both arms must null-check out_len before dereferencing it.
        assert!(
            code.contains("!out_len.is_null()"),
            "optional Bytes field must null-check out_len before writing, got:\n{code}"
        );
    }

    /// Verify that methods with generic type parameters are skipped from C FFI wrapper generation.
    /// Generic methods (like `App::route<H: Handler>(...)`) cannot be wrapped as C functions
    /// because generic type parameters have no C FFI representation. These methods are handled
    /// through the service-API registration path instead.
    #[test]
    fn test_skips_method_with_generic_type_parameter() {
        // Create an API with a type that has a method with a generic parameter
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "App".to_string(),
                rust_path: "my_lib::App".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "route".to_string(),
                    params: vec![
                        ParamDef {
                            name: "builder".to_string(),
                            ty: TypeRef::Named("RouteBuilder".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                        ParamDef {
                            name: "handler".to_string(),
                            // This is a generic type parameter H that won't be in path_map
                            ty: TypeRef::Named("H".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("App".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Register a handler.".to_string(),
                    receiver: Some(ReceiverKind::Owned),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: true,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                }],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "App service.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The method with generic parameter H should NOT be wrapped as a C function
        assert!(
            !lib.content.contains("my_lib_app_route"),
            "method with generic type parameter H should NOT be wrapped as C function"
        );
    }

    /// Verify that methods returning a reference to the receiver (builder-style methods)
    /// are skipped from C FFI wrapper generation. Methods returning `&mut Self` or `&Self`
    /// cannot be represented as owned C handles, so they must be accessed through the
    /// service-API registration path instead.
    #[test]
    fn test_skips_method_with_receiver_reference_return() {
        // Create an API with a type that has a builder-style method
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "Builder".to_string(),
                rust_path: "my_lib::Builder".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "with_option".to_string(),
                    params: vec![ParamDef {
                        name: "value".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    }],
                    // This method returns &mut Self (a reference to the receiver)
                    return_type: TypeRef::Named("Builder".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Set an option (builder style).".to_string(),
                    receiver: Some(ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: true, // Marks that it returns a reference
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                }],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Builder type.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The builder-style method returning &mut Self should NOT be wrapped as a C function
        assert!(
            !lib.content.contains("my_lib_builder_with_option"),
            "builder-style method returning &mut Self should NOT be wrapped as C function"
        );
    }

    // -----------------------------------------------------------------------
    // Tests for opaque static constructor emission (Part B)
    // -----------------------------------------------------------------------

    /// Build an ApiSurface with an opaque type that has a static `new` constructor.
    fn opaque_with_constructor_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![
                // Enum used as parameter in the constructor
                TypeDef {
                    name: "Method".to_string(),
                    rust_path: "my_lib::Method".to_string(),
                    original_rust_path: String::new(),
                    fields: vec![],
                    methods: vec![],
                    is_opaque: false,
                    is_clone: true,
                    is_copy: false,
                    is_trait: false,
                    has_default: false,
                    has_stripped_cfg_fields: false,
                    is_return_type: false,
                    serde_rename_all: None,
                    has_serde: false,
                    super_traits: vec![],
                    doc: "HTTP method enum.".to_string(),
                    cfg: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_variant_wrapper: false,
                    has_lifetime_params: false,
                },
                // Opaque RouteBuilder with static new constructor
                TypeDef {
                    name: "RouteBuilder".to_string(),
                    rust_path: "my_lib::RouteBuilder".to_string(),
                    original_rust_path: String::new(),
                    fields: vec![],
                    methods: vec![MethodDef {
                        name: "new".to_string(),
                        params: vec![
                            ParamDef {
                                name: "method".to_string(),
                                ty: TypeRef::Named("Method".to_string()),
                                optional: false,
                                default: None,
                                sanitized: false,
                                typed_default: None,
                                is_ref: false,
                                is_mut: false,
                                newtype_wrapper: None,
                                original_type: None,
                                map_is_ahash: false,
                                map_key_is_cow: false,
                                vec_inner_is_ref: false,
                                map_is_btree: false,
                                core_wrapper: crate::core::ir::CoreWrapper::None,
                            },
                            ParamDef {
                                name: "path".to_string(),
                                ty: TypeRef::String,
                                optional: false,
                                default: None,
                                sanitized: false,
                                typed_default: None,
                                is_ref: false,
                                is_mut: false,
                                newtype_wrapper: None,
                                original_type: None,
                                map_is_ahash: false,
                                map_key_is_cow: false,
                                vec_inner_is_ref: false,
                                map_is_btree: false,
                                core_wrapper: crate::core::ir::CoreWrapper::None,
                            },
                        ],
                        return_type: TypeRef::Named("RouteBuilder".to_string()),
                        is_async: false,
                        is_static: true,
                        error_type: None,
                        doc: "Create a new route builder.".to_string(),
                        receiver: None,
                        sanitized: false,
                        trait_source: None,
                        returns_ref: false,
                        returns_cow: false,
                        return_newtype_wrapper: None,
                        has_default_impl: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
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
                    doc: "Opaque route builder.".to_string(),
                    cfg: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_variant_wrapper: false,
                    has_lifetime_params: false,
                },
            ],
            functions: vec![],
            enums: vec![EnumDef {
                name: "Method".to_string(),
                rust_path: "my_lib::Method".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Get".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        is_tuple: false,
                        originally_had_data_fields: false,
                    },
                    EnumVariant {
                        name: "Post".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        is_tuple: false,
                        originally_had_data_fields: false,
                    },
                ],
                doc: "HTTP method.".to_string(),
                cfg: None,
                is_copy: true,
                has_serde: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    #[test]
    fn test_emits_opaque_static_constructor_as_c_symbol() {
        let api = opaque_with_constructor_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Check that the extern "C" fn symbol is emitted
        assert!(
            lib.content
                .contains("pub unsafe extern \"C\" fn my_lib_route_builder_new("),
            "expected opaque constructor symbol my_lib_route_builder_new, got:\n{}",
            lib.content
        );
    }

    #[test]
    fn test_opaque_constructor_signature_has_enum_by_value_as_i32() {
        let api = opaque_with_constructor_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Check that enum parameter is passed as i32, not *const
        assert!(
            lib.content.contains("method: i32"),
            "expected enum parameter 'method: i32', got:\n{}",
            lib.content
        );
        // Verify it's NOT emitted as a pointer
        assert!(
            !lib.content.contains("method: *const my_lib::Method"),
            "enum parameter should not be passed as pointer"
        );
    }

    #[test]
    fn test_opaque_constructor_marshals_enum_from_i32() {
        let api = opaque_with_constructor_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Check that the constructor body reconstructs the enum using from_i32
        assert!(
            lib.content.contains("method_from_i32"),
            "constructor should use method_from_i32 to reconstruct enum from discriminant"
        );
    }

    #[test]
    fn test_opaque_constructor_returns_mut_opaque_pointer() {
        let api = opaque_with_constructor_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Constructor returns *mut to the inner type (matching the legacy `_free()`
        // signature) — no separate wrapper struct is emitted. The fixture's crate
        // name varies; just check for a `-> *mut <something>RouteBuilder {` body.
        let has_mut_return = lib.content.lines().any(|line| {
            line.contains("-> *mut") && line.contains("RouteBuilder") && !line.contains("RouteBuilderOpaque")
        });
        assert!(
            has_mut_return,
            "constructor should return *mut <core>::RouteBuilder (not a wrapper); got:\n{}",
            lib.content
        );
    }

    #[test]
    fn test_opaque_constructor_only_for_opaque_types() {
        let api = opaque_with_constructor_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // The Method type is NOT opaque, so its constructor should NOT be emitted
        // (if it had one). Only RouteBuilder's constructor should be in the output.
        // Should have RouteBuilder's _new, but not Method's
        assert!(
            lib.content.contains("my_lib_route_builder_new"),
            "RouteBuilder (opaque) should have _new constructor"
        );
    }
}
