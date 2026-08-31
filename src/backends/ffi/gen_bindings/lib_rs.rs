use crate::adapters::AdapterBodies;
use crate::backends::ffi::gen_bindings::functions::{
    gen_free_function, gen_free_function_len_companion, gen_free_function_result_presence_wrapper,
    gen_method_result_presence_wrapper, gen_method_wrapper, gen_streaming_method_wrapper, is_owned_default_constructor,
    returns_bytes_out_params, returns_c_char, should_skip_method_wrapper,
};
use crate::backends::ffi::gen_bindings::helpers;
use crate::backends::ffi::gen_bindings::helpers::{
    gen_ffi_tokio_runtime, gen_free_bytes, gen_free_string, gen_last_error, gen_version,
};
use crate::backends::ffi::gen_bindings::lib_setup::{
    build_lib_setup_context, crosses_borrowed_handle_boundary, function_param_bridge_for_visitor_callbacks,
    has_trait_bridge_param, options_field_bridge_for_function,
};
use crate::backends::ffi::gen_bindings::types::{
    gen_enum_free, gen_enum_from_i32, gen_enum_from_i32_rs_helper, gen_enum_from_json, gen_enum_to_i32,
    gen_enum_to_json, gen_enum_to_string, gen_field_accessor, gen_field_presence_accessor,
    gen_opaque_static_constructor, gen_type_free, gen_type_from_json, gen_type_new, gen_type_to_json,
    is_static_constructor,
};
use crate::backends::ffi::type_map::optional_leaf_needs_presence_signal;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use heck::ToPascalCase;

/// Extracts the `Named` (or `Optional<Named>`) type name a return/param/field position holds,
/// if any. Shared by the enum-pointer-return and enum-pointer-param scans below. ~keep
fn named_type_ref(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(n) => Some(n.clone()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a field carrying a full `#[serde(skip)]` can still be written through some other FFI
/// entry point, despite the type's only generic constructor (`_from_json`) never reaching it.
///
/// The one such entry point alef currently generates is a
/// `[[crates.trait_bridges]] bind_via = "options_field"` setter (see
/// `gen_bridge_field::gen_options_set_bridge`, wired up below via
/// `TraitBridgeConfig::resolved_options_field`): it writes `options.<field>` directly, bypassing
/// serde entirely. A bridge only counts when both its configured `options_type` AND its
/// resolved field name (`options_field`, falling back to `param_name` when the two differ --
/// see `TraitBridgeConfig::resolved_options_field`) match this exact `(type, field)` pair; a
/// bridge aimed at a different options type, or a different field on the same type, proves
/// nothing about reachability here.
fn serde_skip_field_reachable_via_options_bridge(
    typ_name: &str,
    field_name: &str,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
) -> bool {
    trait_bridges.iter().any(|bridge| {
        bridge.bind_via == crate::core::config::BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(typ_name)
            && bridge.resolved_options_field() == Some(field_name)
    })
}

/// A field's FFI getter must not be emitted when the field is structurally guaranteed to always
/// read back the same Rust-side default regardless of what a caller supplies: a full
/// `#[serde(skip)]` on a field of a non-opaque, serde-derived type is invisible to
/// `Deserialize`, and `_from_json` -- gated on exactly `!typ.is_opaque && typ.has_serde`
/// elsewhere in this file -- is the ONLY generic FFI constructor such a type gets. Emitting a
/// getter there documents an "owned handle the caller must free" contract
/// (`field_accessor_ownership_lines`) that no caller-supplied JSON payload can ever satisfy.
///
/// The one known escape hatch is a field reachable through an `options_field` trait-bridge
/// setter (`{prefix}_options_set_{field}`), which writes the field directly and bypasses serde
/// -- see `serde_skip_field_reachable_via_options_bridge`. Opaque types are excluded from this
/// check entirely: their constructors are arbitrary Rust methods alef does not model from IR
/// alone, so it cannot prove such a field is unreachable there the way it can for a type whose
/// only construction path IS the generic `_from_json` this backend itself emits.
fn field_getter_is_reachable(
    typ: &crate::core::ir::TypeDef,
    field: &crate::core::ir::FieldDef,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
) -> bool {
    if !field.serde_skip || typ.is_opaque || !typ.has_serde {
        return true;
    }
    serde_skip_field_reachable_via_options_bridge(&typ.name, &field.name, trait_bridges)
}

pub(super) fn gen_lib_rs(api: &ApiSurface, prefix: &str, config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let mut builder = RustFileBuilder::new().with_generated_header();
    builder.add_inner_attribute("allow(dead_code, unused_imports, unused_mut, noop_method_call)");
    builder.add_inner_attribute("allow(unsafe_op_in_unsafe_fn, unsafe_attr_outside_unsafe)");
    builder.add_inner_attribute("allow(clippy::let_unit_value, clippy::needless_borrow, clippy::redundant_locals, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::type_complexity, clippy::clone_on_copy, clippy::collapsible_if)");
    builder.add_inner_attribute(
        "allow(clippy::missing_safety_doc, clippy::doc_lazy_continuation, clippy::doc_overindented_list_items)",
    );
    for attribute in crate::codegen::shared::format_crate_attributes(&config.crate_attributes) {
        builder.add_inner_attribute(&attribute);
    }
    if let Some(extra_attribute) =
        crate::codegen::shared::format_extra_clippy_allows(&config.extra_clippy_allows, builder.inner_attributes_text())
    {
        builder.add_inner_attribute(&extra_attribute);
    }

    builder.add_import("std::ffi::{c_char, CStr, CString}");
    builder.add_import("std::cell::RefCell");
    let core_import = config.core_import_name();
    // This binding's own configured feature set, used to decide whether a FOREIGN-owned
    // cfg-gated enum variant is provably unreachable for this binding -- see
    // `codegen::conversions::enum_variant_declaration`, the same authority napi's `gen_enum`
    // consults for its wrapper declaration. ~keep
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Ffi);
    let configured_features_set: std::collections::HashSet<&str> =
        enabled_features.iter().map(String::as_str).collect();

    let lib_setup = build_lib_setup_context(api, config);
    let path_map = &lib_setup.path_map;
    let enum_names = &lib_setup.enum_names;
    let ffi_param_enums = &lib_setup.ffi_param_enums;
    let clone_names = &lib_setup.clone_names;
    let serde_names = &lib_setup.serde_names;

    let empty_fields_c_types = std::collections::HashMap::new();
    let fields_c_types = lib_setup.fields_c_types.unwrap_or(&empty_fields_c_types);

    for trait_path in generators::collect_trait_imports(api) {
        builder.add_import(&trait_path);
    }

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

    let custom_mods = config.custom_modules.for_language(Language::Ffi);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    if !api.services.is_empty() {
        builder.add_item("pub mod service;");
    }

    builder.add_item(&gen_last_error(api, prefix, &core_import));

    builder.add_item(&crate::backends::ffi::template_env::render(
        "handle_registry.rs.jinja",
        minijinja::context! {},
    ));

    builder.add_item(&gen_free_string(prefix));

    let has_bytes_results = api
        .functions
        .iter()
        .any(|function| returns_bytes_out_params(&function.return_type))
        || api.types.iter().any(|typ| {
            typ.methods
                .iter()
                .any(|method| returns_bytes_out_params(&method.return_type))
        });
    if has_bytes_results {
        builder.add_item(&gen_free_bytes(prefix));
    }

    builder.add_item(&gen_version(prefix));

    let adapter_bodies: AdapterBodies =
        crate::adapters::build_adapter_bodies(config, Language::Ffi).unwrap_or_default();

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

    for enum_def in &api.enums {
        if ffi_param_enums.contains(&enum_def.name)
            && crate::codegen::conversions::can_generate_enum_conversion(enum_def)
        {
            builder.add_item(&gen_enum_from_i32_rs_helper(enum_def, &core_import, &api.crate_name));
        }
    }

    let ffi_capsule_types: std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig> =
        config.ffi.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();

    let mut ffi_exclude_types: ahash::AHashSet<&str> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_types.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    ffi_exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()));
    let capsule_used_as_opaque: ahash::AHashSet<&str> = api
        .types
        .iter()
        .flat_map(|t| t.methods.iter())
        .filter_map(|m| match &m.return_type {
            crate::core::ir::TypeRef::Named(name) if ffi_capsule_types.contains_key(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    ffi_exclude_types.extend(
        ffi_capsule_types
            .keys()
            .map(|s| s.as_str())
            .filter(|name| !capsule_used_as_opaque.contains(name)),
    );
    let exclude_generic_opaques: ahash::AHashSet<&str> = config
        .opaque_types
        .iter()
        .filter(|(_, path)| path.contains('<'))
        .map(|(name, _)| name.as_str())
        .collect();
    ffi_exclude_types.extend(exclude_generic_opaques);
    let borrowed_handle_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|typ| typ.has_lifetime_params)
        .map(|typ| typ.name.clone())
        .collect();

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !ffi_exclude_types.contains(typ.name.as_str()))
    {
        if !typ.is_opaque && typ.has_serde {
            builder.add_item(&gen_type_from_json(typ, prefix, &core_import));
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

        let mut emitted_field_names: ahash::AHashSet<&str> = ahash::AHashSet::default();
        for field in &typ.fields {
            if !field.sanitized
                && !field.binding_excluded
                && !crosses_borrowed_handle_boundary(&field.ty, &borrowed_handle_types)
                && field_getter_is_reachable(typ, field, &config.trait_bridges)
            {
                emitted_field_names.insert(field.name.as_str());
                builder.add_item(&gen_field_accessor(
                    typ,
                    field,
                    prefix,
                    &core_import,
                    path_map,
                    enum_names,
                    clone_names,
                    fields_c_types,
                )?);
                // A `has_<field>` companion is required whenever the field's own getter has no
                // null representation to signal `None` -- see
                // `optional_leaf_needs_presence_signal` for exactly which shapes collapse.
                if field.optional && optional_leaf_needs_presence_signal(&field.ty) {
                    builder.add_item(&gen_field_presence_accessor(typ, field, prefix, &core_import));
                }
            }
        }

        if typ.is_opaque {
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

        let ffi_exclude_methods: ahash::AHashSet<String> = config.exclude.methods.iter().cloned().collect();

        for method in &typ.methods {
            let method_key = format!("{}.{}", typ.name, method.name);
            if ffi_exclude_methods.contains(&method_key) {
                continue;
            }
            let returns_owned_self = is_owned_default_constructor(method, typ)
                || (!method.returns_ref
                    && matches!(&method.return_type, crate::core::ir::TypeRef::Named(name) if name == &typ.name));
            if (!returns_owned_self && crosses_borrowed_handle_boundary(&method.return_type, &borrowed_handle_types))
                || method
                    .params
                    .iter()
                    .any(|parameter| crosses_borrowed_handle_boundary(&parameter.ty, &borrowed_handle_types))
            {
                continue;
            }

            if should_skip_method_wrapper(method, typ, path_map, &emitted_field_names) {
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
            if let Some(presence) =
                gen_method_result_presence_wrapper(typ, method, prefix, &core_import, path_map, ffi_param_enums)
            {
                builder.add_item(&presence);
            }
        }
    }

    for enum_def in &api.enums {
        if crate::codegen::conversions::can_generate_enum_conversion(enum_def) {
            // `&api.crate_name`, not `&core_import`: matches `gen_enum_from_i32_rs_helper`'s own
            // `host_crate_name` argument a few lines above, so the two agree about which variants
            // are host- vs foreign-owned for the identical enum.
            builder.add_item(&gen_enum_from_i32(
                enum_def,
                prefix,
                &api.crate_name,
                Some(&configured_features_set),
            ));
            builder.add_item(&gen_enum_to_i32(
                enum_def,
                prefix,
                &api.crate_name,
                Some(&configured_features_set),
            ));
        }
    }

    {
        let ffi_exclude_set: ahash::AHashSet<&str> = config
            .ffi
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        let mut enum_pointer_return: ahash::AHashSet<String> = ahash::AHashSet::new();
        for func in &api.functions {
            if ffi_exclude_set.contains(func.name.as_str()) {
                continue;
            }
            if let Some(name) = named_type_ref(&func.return_type)
                && api.enums.iter().any(|e| e.name == name)
            {
                enum_pointer_return.insert(name);
            }
        }
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for method in &typ.methods {
                if let Some(name) = named_type_ref(&method.return_type)
                    && api.enums.iter().any(|e| e.name == name)
                {
                    enum_pointer_return.insert(name);
                }
            }
            for field in &typ.fields {
                if field.binding_excluded {
                    continue;
                }
                if let Some(name) = named_type_ref(&field.ty)
                    && api.enums.iter().any(|e| e.name == name)
                {
                    enum_pointer_return.insert(name);
                }
            }
        }

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

        // `enum_pointer_param` must walk every parameter position an enum can occupy, not just
        // free functions: `enum_pointer_return` above already does that (functions, methods,
        // fields), but this set previously covered only `api.functions[].params`. A
        // data-carrying enum reached solely through a `TypeDef::methods[].params` position (no
        // free function ever consumed it) got no `_from_json`, even though the C# backend
        // declares the DllImport for any `has_serde` parameter-position enum regardless of
        // whether a free function or a method carries it.
        //
        // Fieldless `is_copy` enums must stay excluded from this set: they cross the C ABI as a
        // scalar `int32_t` (`ffi::type_map::scalar_c_abi_named_types`), and the FFI deliberately
        // gives them `from_i32`/`from_str` instead, never a handle-returning `from_json`. Without
        // this filter, broadening the scan to method params would resurrect the symbol the C#
        // fix in scalar_c_abi_named_types's callers (task #48) had to work around: an
        // `EntryPointNotFoundException`-shaped export that no caller expects. ~keep
        let scalar_named_enums = crate::backends::ffi::type_map::scalar_c_abi_named_types(api);
        let mut enum_pointer_param: ahash::AHashSet<String> = ahash::AHashSet::new();
        for func in &api.functions {
            if ffi_exclude_set.contains(func.name.as_str()) {
                continue;
            }
            for param in &func.params {
                if let Some(name) = named_type_ref(&param.ty)
                    && api.enums.iter().any(|e| e.name == name)
                    && !scalar_named_enums.contains(&name)
                {
                    enum_pointer_param.insert(name);
                }
            }
        }
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for method in &typ.methods {
                for param in &method.params {
                    if let Some(name) = named_type_ref(&param.ty)
                        && api.enums.iter().any(|e| e.name == name)
                        && !scalar_named_enums.contains(&name)
                    {
                        enum_pointer_param.insert(name);
                    }
                }
            }
        }

        let mut emitted_enum_free: ahash::AHashSet<String> = ahash::AHashSet::new();
        for enum_def in &api.enums {
            let needs_free = enum_pointer_return.contains(&enum_def.name);
            let needs_from_json = enum_pointer_param.contains(&enum_def.name);
            let has_serde = enum_def.has_serde;

            if needs_free && emitted_enum_free.insert(enum_def.name.clone()) {
                builder.add_item(&gen_enum_free(enum_def, prefix, &core_import));
                if has_serde {
                    builder.add_item(&gen_enum_to_json(enum_def, prefix, &core_import));
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
                if emitted_enum_free.insert(enum_def.name.clone()) {
                    builder.add_item(&gen_enum_free(enum_def, prefix, &core_import));
                }
            }
        }
    }

    let has_async_functions =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    if has_async_functions {
        builder.add_item(&gen_ffi_tokio_runtime());
    }

    let visitor_callbacks_enabled = config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks);

    let has_options_field_bridge = config
        .trait_bridges
        .iter()
        .any(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField);

    let ffi_exclude_functions: ahash::AHashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    // `#[cfg(feature = "X")]` plus a stub fallback under
    // `#[cfg(all(feature = "X-presets", not(feature = "X")))]`) are collapsed locally for the
    let ffi_functions = crate::backends::ffi::gen_bindings::functions::dedup_same_name_functions(&api.functions);
    for func in &ffi_functions {
        if ffi_exclude_functions.contains(&func.name) {
            continue;
        }
        let returns_owned_borrowed_handle = !func.returns_ref
            && matches!(&func.return_type, crate::core::ir::TypeRef::Named(name) if borrowed_handle_types.contains(name));
        if (!returns_owned_borrowed_handle
            && crosses_borrowed_handle_boundary(&func.return_type, &borrowed_handle_types))
            || func
                .params
                .iter()
                .any(|parameter| crosses_borrowed_handle_boundary(&parameter.ty, &borrowed_handle_types))
        {
            continue;
        }
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        if visitor_callbacks_enabled && func.sanitized && has_trait_bridge_param(func, &config.trait_bridges) {
            continue;
        }
        if has_options_field_bridge
            && let Some((options_param, options_type_name)) =
                options_field_bridge_for_function(func, &config.trait_bridges)
            && let Some(wrapper) = crate::backends::ffi::gen_bridge_field::gen_function_with_options_field_bridge(
                prefix,
                &core_import,
                func,
                options_param,
                options_type_name,
            )
        {
            builder.add_item(&wrapper);
            continue;
        }
        let capsule_cfg = if matches!(func.return_type, crate::core::ir::TypeRef::Named(_)) {
            super::capsule::capsule_return_name(func, &ffi_capsule_types).and_then(|name| ffi_capsule_types.get(name))
        } else {
            None
        };
        builder.add_item(&gen_free_function(
            func,
            prefix,
            &core_import,
            path_map,
            ffi_param_enums,
            serde_names,
            capsule_cfg,
            returns_owned_borrowed_handle,
        ));
        if returns_c_char(&func.return_type) {
            builder.add_item(&gen_free_function_len_companion(
                func,
                prefix,
                &core_import,
                path_map,
                ffi_param_enums,
            ));
        }
        if let Some(presence) =
            gen_free_function_result_presence_wrapper(func, prefix, &core_import, path_map, ffi_param_enums)
        {
            builder.add_item(&presence);
        }
    }

    if has_options_field_bridge {
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

        if visitor_callbacks_enabled {
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
                tracing::warn!(
                    "gen_visitor_bindings(ffi): visitor_callbacks=true but no OptionsField trait found in IR, skipping visitor callbacks"
                );
            }
        }
    } else if visitor_callbacks_enabled {
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
                tracing::warn!(
                    "gen_visitor_bindings(ffi): visitor_callbacks=true but configured trait `{}` is not present in IR, skipping visitor callbacks",
                    bridge_cfg.trait_name
                );
            }
        } else {
            tracing::warn!(
                "gen_visitor_bindings(ffi): visitor_callbacks=true but no FunctionParam trait bridge matched a public function, skipping visitor callbacks"
            );
        }
    }

    for error in &api.errors {
        let methods_code = crate::codegen::error_gen::gen_ffi_error_methods(error, &core_import, prefix);
        if !methods_code.is_empty() {
            builder.add_item(&methods_code);
        }
    }

    if !config.trait_bridges.is_empty() {
        builder.add_import("std::ffi::c_void");
        builder.add_import("std::sync::Arc");

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
            if visitor_callbacks_enabled
                && bridge_cfg.bind_via == crate::core::config::BridgeBinding::OptionsField
                && bridge_cfg.register_fn.is_none()
            {
                continue;
            }
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

    // This is the authoritative opaque-handle ABI artifact: `handle_registry.rs.jinja` defines
    // the representation every other backend hand-copies. Stamped here, inside the backend, so
    // the marker is part of the file body the pipeline's `finalize_hashes` pass later hashes —
    // stamping after `inject_hash_line` would make every stamped file read as stale. ~keep
    Ok(crate::core::hash::inject_stamp_line(
        &builder.build(),
        crate::core::hash::HANDLE_ABI_STAMP_KEY,
        crate::core::template_versions::abi::HANDLE_ABI_VERSION,
    ))
}
