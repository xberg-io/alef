use crate::adapters::AdapterBodies;
use crate::backends::ffi::gen_bindings::functions::{
    gen_free_function, gen_free_function_len_companion, gen_method_wrapper, gen_streaming_method_wrapper,
    returns_c_char, should_skip_method_wrapper,
};
use crate::backends::ffi::gen_bindings::helpers;
use crate::backends::ffi::gen_bindings::helpers::{
    gen_ffi_tokio_runtime, gen_free_bytes, gen_free_string, gen_last_error, gen_version,
};
use crate::backends::ffi::gen_bindings::lib_setup::{
    build_lib_setup_context, function_param_bridge_for_visitor_callbacks, has_trait_bridge_param,
    options_field_bridge_for_function,
};
use crate::backends::ffi::gen_bindings::types::{
    gen_enum_free, gen_enum_from_i32, gen_enum_from_i32_rs_helper, gen_enum_from_json, gen_enum_to_i32,
    gen_enum_to_json, gen_enum_to_string, gen_field_accessor, gen_opaque_static_constructor, gen_type_free,
    gen_type_from_json, gen_type_new, gen_type_to_json, is_static_constructor,
};
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;

pub(super) fn gen_lib_rs(api: &ApiSurface, prefix: &str, config: &ResolvedCrateConfig) -> String {
    let mut builder = RustFileBuilder::new().with_generated_header();
    builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables, unused_mut, noop_method_call)");
    // The FFI crate is entirely generated glue — rustdoc coverage is not meaningful here.
    builder.add_inner_attribute("allow(missing_docs)");
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

    let lib_setup = build_lib_setup_context(api, config);
    let path_map = &lib_setup.path_map;
    let enum_names = &lib_setup.enum_names;
    let ffi_param_enums = &lib_setup.ffi_param_enums;
    let clone_names = &lib_setup.clone_names;
    let serde_names = &lib_setup.serde_names;

    // Extract fields_c_types from e2e config if present.
    // This allows field accessors to override their return types when e2e explicitly
    // maps a field to an opaque handle type (e.g. "markdown_result.citations" = "CitationResult").
    let empty_fields_c_types = std::collections::HashMap::new();
    let fields_c_types = lib_setup.fields_c_types.unwrap_or(&empty_fields_c_types);

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
                    path_map,
                    enum_names,
                    clone_names,
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
                        path_map,
                        ffi_param_enums,
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
            if should_skip_method_wrapper(method, typ, path_map) {
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
                path_map,
                ffi_param_enums,
                serde_names,
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
            path_map,
            ffi_param_enums,
            serde_names,
        ));
        // Emit a _len() companion for every function whose return type maps to *mut c_char
        // so that Zig and Java FFM Panama consumers get byte length without a NUL-scan.
        if returns_c_char(&func.return_type) {
            builder.add_item(&gen_free_function_len_companion(
                func,
                prefix,
                &core_import,
                path_map,
                ffi_param_enums,
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
                field_name,
                options_type_name,
                &type_paths,
                visitor_callbacks_enabled,
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
