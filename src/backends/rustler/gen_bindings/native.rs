use crate::backends::rustler::gen_bindings::functions::{
    gen_nif_async_function, gen_nif_async_method, gen_nif_function, gen_nif_method, regate_ungated_same_name_functions,
};
use crate::backends::rustler::gen_bindings::helpers::{collect_types_for_nif_derives, get_module_info};
use crate::backends::rustler::gen_bindings::rust_items::{
    gen_from_json_nif, gen_nif_init, patch_streaming_default_param, rustler_default_for_type,
};
use crate::backends::rustler::gen_bindings::types::{
    gen_enum, gen_opaque_resource, gen_rustler_config_impl, gen_rustler_flat_data_enum_from_core,
    gen_rustler_flat_data_enum_to_core, gen_struct,
};
use crate::backends::rustler::type_map::RustlerMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use std::path::PathBuf;

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
}

pub(super) fn generate_bindings(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let mapper = RustlerMapper;
    let core_import = config.core_import_name();
    // This binding's own configured feature set (already expanded through the core crate's
    // `[features]` graph), used to decide whether a FOREIGN-owned cfg-gated enum variant is
    // provably unreachable for this binding -- see
    // `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features`. ~keep
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Elixir);

    let elixir_config = config.elixir.as_ref();
    let exclude_functions: AHashSet<String> = elixir_config
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let binding_excluded_names: Vec<String> = api
        .types
        .iter()
        .filter(|t| t.binding_excluded)
        .map(|t| t.name.clone())
        .collect();
    let mut exclude_types: AHashSet<&str> = elixir_config
        .map(|c| c.exclude_types.iter().map(String::as_str).collect())
        .unwrap_or_default();
    exclude_types.extend(binding_excluded_names.iter().map(String::as_str));
    let cpu_bound_functions: AHashSet<String> = elixir_config
        .map(|c| c.cpu_bound_functions.iter().cloned().collect())
        .unwrap_or_default();

    let mut bridge_excluded_fields: std::collections::HashMap<String, AHashSet<String>> =
        std::collections::HashMap::new();
    for b in config
        .trait_bridges
        .iter()
        .filter(|b| b.bind_via == BridgeBinding::OptionsField)
        .filter(|b| crate::backends::rustler::trait_bridge::targets_rustler(b))
    {
        let field_name = b.resolved_options_field().unwrap_or("visitor").to_string();
        let trait_alias = b.type_alias.as_deref().unwrap_or(&b.trait_name);
        if let Some(opts_type) = b.options_type.as_deref() {
            bridge_excluded_fields
                .entry(opts_type.to_string())
                .or_default()
                .insert(field_name.clone());
        }
        for typ in api.types.iter() {
            if binding_fields(&typ.fields).any(|f| {
                if f.name != field_name {
                    return false;
                }
                let type_name = match &f.ty {
                    crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                type_name == Some(trait_alias)
            }) {
                bridge_excluded_fields
                    .entry(typ.name.clone())
                    .or_default()
                    .insert(field_name.clone());
            }
        }
    }

    let mut builder = RustFileBuilder::new().with_generated_header();
    builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
    builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
    if let Some(extra_attr) =
        crate::codegen::shared::format_extra_clippy_allows(&config.extra_clippy_allows, builder.inner_attributes_text())
    {
        builder.add_inner_attribute(&extra_attr);
    }
    for attribute in crate::codegen::shared::format_crate_attributes(&config.crate_attributes) {
        builder.add_inner_attribute(&attribute);
    }
    builder.add_import("rustler::ResourceArc");
    builder.add_import("rustler::Encoder");

    for trait_path in generators::collect_trait_imports(api) {
        builder.add_import(&trait_path);
    }

    let has_maps = api
        .types
        .iter()
        .any(|t| t.fields.iter().any(|f| matches!(&f.ty, TypeRef::Map(_, _))))
        || api
            .functions
            .iter()
            .any(|f| matches!(&f.return_type, TypeRef::Map(_, _)));
    if has_maps {
        builder.add_import("std::collections::HashMap");
    }

    let custom_mods = config.custom_modules.for_language(Language::Elixir);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    // codegen emits additional `#[rustler::nif]` functions that Rustler's
    if !api.services.is_empty() {
        builder.add_item("pub mod service;");
    }

    if !config.components.is_empty() {
        builder.add_item(&super::components::generate(config));
    }

    let (_module_name, module_prefix) = get_module_info(api, config);

    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    if !opaque_types.is_empty() {
        builder.add_import("std::sync::Arc");
    }
    let opaque_type_names_vec: Vec<String> = opaque_types.iter().cloned().collect();
    // Computed early (depends only on `api`) so `gen_struct`'s delegating-`Deserialize` decision
    // below can reuse the exact set that gates `gen_from_core_to_binding_cfg` further down (see
    // `core_to_binding`) -- guaranteeing the `From<core::Type>` a delegating `Deserialize` needs
    // is always emitted for any type this set names. ~keep
    let core_to_binding_for_deserialize = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);

    let mut types_to_emit = collect_types_for_nif_derives(api, &exclude_types);

    for typ in &api.types {
        if typ.is_opaque && !exclude_types.contains(typ.name.as_str()) {
            types_to_emit.insert(typ.name.clone());
        }
    }

    let empty_set: AHashSet<String> = AHashSet::new();
    // Sorted by name before emission: `api.types` reflects source-extraction order, which is not
    // guaranteed stable across separate `alef` invocations (e.g. under parallel crate/file
    // extraction). Emitting structs in that order directly into the single concatenated `lib.rs`
    // makes struct block order -- and therefore the file's bytes -- depend on that upstream
    // ordering. Sorting here decouples this backend's output from it. ~keep
    let mut types_for_struct_emission: Vec<&crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && types_to_emit.contains(&typ.name))
        .collect();
    types_for_struct_emission.sort_by(|a, b| a.name.cmp(&b.name));
    for typ in types_for_struct_emission {
        if typ.is_opaque {
            builder.add_item(&gen_opaque_resource(typ, &core_import, &opaque_types));
            if let Some(ctor) = config.client_constructors.get(&typ.name) {
                let ctor_body = crate::codegen::generators::gen_opaque_constructor(ctor, &typ.name, &core_import, "");
                let ctor_impl = format!("impl {} {{\n{}}}", typ.name, ctor_body);
                builder.add_item(&ctor_impl);
            }
        } else {
            let excl = bridge_excluded_fields.get(typ.name.as_str()).unwrap_or(&empty_set);
            builder.add_item(&gen_struct(
                typ,
                &mapper,
                &module_prefix,
                excl,
                &core_import,
                &opaque_type_names_vec,
                &core_to_binding_for_deserialize,
            ));
            if typ.has_default && !typ.fields.is_empty() {
                let config_impl = gen_rustler_config_impl(typ, &mapper, excl);
                builder.add_item(&config_impl);
            }
        }
    }

    // Same rationale as `types_for_struct_emission` above: sort by name so enum block order in
    // the concatenated `lib.rs` does not depend on `api.enums`' incoming order. ~keep
    let mut enums_for_emission: Vec<&crate::core::ir::EnumDef> = api.enums.iter().collect();
    enums_for_emission.sort_by(|a, b| a.name.cmp(&b.name));
    for enum_def in enums_for_emission {
        builder.add_item(&gen_enum(
            enum_def,
            &module_prefix,
            api,
            &core_import,
            Some(enabled_features.as_slice()),
        ));
    }

    let default_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Elixir)?;

    let streaming_method_keys: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
        .collect();

    for adapter in &config.adapters {
        match adapter.pattern {
            crate::core::config::AdapterPattern::Streaming => {
                let key = crate::adapters::stream_struct_key(adapter);
                if let Some(struct_code) = adapter_bodies.get(&key) {
                    let patched = patch_streaming_default_param(struct_code, adapter, &default_types, &core_import);
                    builder.add_item(&patched);
                }
            }
            crate::core::config::AdapterPattern::CallbackBridge => {
                let struct_key = format!("{}.__bridge_struct__", adapter.name);
                let impl_key = format!("{}.__bridge_impl__", adapter.name);
                if let Some(struct_code) = adapter_bodies.get(&struct_key) {
                    builder.add_item(struct_code);
                }
                if let Some(impl_code) = adapter_bodies.get(&impl_key) {
                    builder.add_item(impl_code);
                }
            }
            _ => {}
        }
    }

    let active_bridges: Vec<_> = config
        .trait_bridges
        .iter()
        .filter(|b| crate::backends::rustler::trait_bridge::targets_rustler(b))
        .cloned()
        .collect();

    if !active_bridges.is_empty() {
        builder.add_import("std::sync::atomic::{AtomicU64, Ordering}");
        builder.add_import("std::sync::Mutex");
        builder.add_import("std::collections::HashMap");
        builder.add_item("static TRAIT_REPLY_COUNTER: AtomicU64 = AtomicU64::new(1);");
        builder.add_item("type TraitReplyChannel = tokio::sync::oneshot::Sender<std::result::Result<String, String>>;");
        builder.add_item(
            "static TRAIT_REPLY_CHANNELS: std::sync::LazyLock<Mutex<HashMap<u64, TraitReplyChannel>>> = \
                 std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));",
        );
    }

    let types_by_name: ahash::AHashMap<&str, &crate::core::ir::TypeDef> =
        api.types.iter().map(|t| (t.name.as_str(), t)).collect();

    // `#[rustler::nif]` items whose cfgs OVERLAP — rustler auto-discovers both and aborts `on_load`
    let deduped_functions = crate::codegen::fn_dedup::dedup_same_name_functions(&api.functions);
    // whose enclosing cfg gate was dropped by extraction would emit a second `#[rustler::nif]`
    let mut regated_functions = regate_ungated_same_name_functions(&deduped_functions);
    // Same rationale as `types_for_struct_emission` above: sort by name (a stable sort, so
    // same-named cfg-variant groups keep the relative order `regate_ungated_same_name_functions`
    // depends on) so `#[rustler::nif]` block order in `lib.rs` does not depend on `api.functions`'
    // incoming order. ~keep
    regated_functions.sort_by(|a, b| a.name.cmp(&b.name));

    for func in regated_functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &active_bridges) {
            continue;
        }
        let bridge_param = crate::backends::rustler::trait_bridge::find_bridge_param(func, &active_bridges);
        let bridge_field =
            crate::codegen::generators::trait_bridge::find_bridge_field(func, &api.types, &active_bridges);
        if func.sanitized && bridge_param.is_none() && bridge_field.is_none() {
            continue;
        }
        if let Some((param_idx, bridge_cfg)) = bridge_param {
            let item = crate::backends::rustler::trait_bridge::gen_bridge_function(
                api,
                func,
                param_idx,
                bridge_cfg,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
        } else if let Some(ref bm) = bridge_field {
            let item = crate::backends::rustler::trait_bridge::gen_bridge_field_function(
                api,
                func,
                bm,
                bm.bridge,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
        } else if func.is_async {
            let item = gen_nif_async_function(
                func,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
                &types_by_name,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
        } else {
            let item = gen_nif_function(
                func,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
                &cpu_bound_functions,
                &types_by_name,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
        }
    }

    let has_trait_bridges = config
        .trait_bridges
        .iter()
        .any(crate::backends::rustler::trait_bridge::targets_rustler);

    for bridge_cfg in config
        .trait_bridges
        .iter()
        .filter(|b| crate::backends::rustler::trait_bridge::targets_rustler(b))
    {
        if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
            let bridge = crate::backends::rustler::trait_bridge::gen_trait_bridge(
                trait_type,
                bridge_cfg,
                &core_import,
                &config.error_type_name(),
                &config.error_constructor_expr(),
                api,
            )?;
            for imp in &bridge.imports {
                builder.add_import(imp);
            }
            let bridge_code = bridge.code.replace(
                "drop(tokio::task::spawn_blocking(move || {",
                "drop(std::thread::spawn(move || {",
            );
            builder.add_item(&bridge_code);
        }
    }

    if has_trait_bridges {
        let ctx = minijinja::context! {};
        builder.add_item(&crate::backends::rustler::template_env::render(
            "trait_support_nifs.rs.jinja",
            ctx,
        ));
    }

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
    {
        for method in typ
            .methods
            .iter()
            .filter(|m| !exclude_functions.contains(m.name.as_str()))
            .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
            .filter(|m| {
                !crate::codegen::conversions::field_references_excluded_type(
                    &m.return_type,
                    &exclude_types.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                )
            })
        {
            let core_path = crate::codegen::conversions::core_type_path(typ, &core_import);
            // The same gate the function loop above applies via `prepend_cfg`. A rustler method is
            // flattened to a standalone `#[rustler::nif] pub fn`, so it takes an item-level
            // `#[cfg]` directly, and the scaffolded Elixir crate declares every referenced cfg
            // feature as a default-on passthrough (`scaffold::languages::elixir`) so `rustc` can
            // resolve it. There is no registration list to keep in step: `rustler::init!` is
            // rendered with the module name alone (`rustler_init*.rs.jinja` never reads the
            // `nifs` context it is handed), and rustler discovers NIFs from the attribute — so a
            // method the gate compiles out is simply never registered. ~keep
            let item = if method.is_async {
                gen_nif_async_method(
                    &typ.name,
                    &core_path,
                    method,
                    &mapper,
                    typ.is_opaque,
                    &opaque_types,
                    &default_types,
                    &core_import,
                    &adapter_bodies,
                    &types_by_name,
                )
            } else {
                gen_nif_method(
                    &typ.name,
                    &core_path,
                    method,
                    &mapper,
                    typ.is_opaque,
                    &opaque_types,
                    &default_types,
                    &core_import,
                    &adapter_bodies,
                    &types_by_name,
                )
            };
            builder.add_item(&prepend_cfg(method.cfg.as_deref(), item));
        }
    }

    let binding_to_core = crate::codegen::conversions::convertible_types(api);
    let input_types = crate::codegen::conversions::input_type_names(api);
    let flat_data_enum_names_vec: Vec<String> = api
        .enums
        .iter()
        .filter(|e| {
            let has_data = e.variants.iter().any(|v| !v.fields.is_empty());
            has_data && e.variants.iter().filter(|v| !v.fields.is_empty()).all(|v| v.is_tuple)
        })
        .map(|e| e.name.clone())
        .collect();

    let bridge_conv_exclude_types: Vec<String> = config
        .trait_bridges
        .iter()
        .filter(|b| crate::backends::rustler::trait_bridge::targets_rustler(b))
        .filter(|b| b.bind_via == BridgeBinding::OptionsField)
        .map(|b| b.type_alias.as_deref().unwrap_or(&b.trait_name).to_string())
        .collect();

    // Sorted by name, same rationale as `types_for_struct_emission` above. ~keep
    let mut types_for_conversion_emission: Vec<&crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()) && types_to_emit.contains(&typ.name))
        .collect();
    types_for_conversion_emission.sort_by(|a, b| a.name.cmp(&b.name));
    for typ in types_for_conversion_emission {
        let rustler_struct_cfg = crate::codegen::conversions::ConversionConfig {
            map_as_string: false,
            exclude_types: &bridge_conv_exclude_types,
            from_binding_skip_types: &flat_data_enum_names_vec,
            ..Default::default()
        };
        let needs_binding_to_core =
            input_types.contains(&typ.name) || (!typ.is_opaque && typ.methods.iter().any(|m| m.receiver.is_some()));
        if needs_binding_to_core && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core) {
            builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                typ,
                &core_import,
                &rustler_struct_cfg,
            ));
        }
        if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding_for_deserialize) {
            builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                typ,
                &core_import,
                &opaque_types,
                &rustler_struct_cfg,
            ));
        }
    }
    // Sorted by name, same rationale as `types_for_struct_emission` above. ~keep
    let mut enums_for_conversion_emission: Vec<&crate::core::ir::EnumDef> = api.enums.iter().collect();
    enums_for_conversion_emission.sort_by(|a, b| a.name.cmp(&b.name));
    for e in enums_for_conversion_emission {
        let has_data = e.variants.iter().any(|v| !v.fields.is_empty());
        let is_flat_data = has_data && e.variants.iter().filter(|v| !v.fields.is_empty()).all(|v| v.is_tuple);

        if is_flat_data {
            if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&gen_rustler_flat_data_enum_from_core(
                    e,
                    &core_import,
                    Some(enabled_features.as_slice()),
                ));
            }
            if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&gen_rustler_flat_data_enum_to_core(e, &core_import));
            }
        } else {
            let rustler_conv_config = crate::codegen::conversions::ConversionConfig {
                binding_enums_have_data: has_data,
                configured_features: Some(enabled_features.as_slice()),
                // `types::gen_enum` (Rustler's own enum declaration, both the `NifUnitEnum` and
                // `NifTaggedEnum` branches) already drops a FOREIGN cfg-gated variant this
                // binding's own feature set proves unreachable -- via the same `declared_variants`
                // filter over `enum_variant_declaration` (alef #589d67e8a) -- so the binding->core
                // catch-all decision here must know the declaration can leave a gap-free match on
                // its own, mirroring `magnus_conv_config`. Leaving this unset (the `false`
                // default) makes `has_unresolved_foreign_cfg_variants` assume the declaration
                // keeps every variant unconditionally, which stopped being true the moment the
                // declaration started dropping them: the match is exhaustive without the
                // catch-all, so keeping it is `unreachable_patterns` under `-D warnings`. ~keep
                declaration_drops_unreachable_foreign_variants: true,
                ..Default::default()
            };
            if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(
                    e,
                    &core_import,
                    &rustler_conv_config,
                ));
            }
            if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(
                    e,
                    &core_import,
                    &rustler_conv_config,
                ));
            }
        }
    }

    // Sorted by name, same rationale as `types_for_struct_emission` above. ~keep
    let mut errors_for_emission: Vec<_> = api.errors.iter().collect();
    errors_for_emission.sort_by(|a, b| a.name.cmp(&b.name));

    for &error in &errors_for_emission {
        builder.add_item(&crate::codegen::error_gen::gen_rustler_error_converter(
            error,
            &core_import,
        ));
    }

    for &error in &errors_for_emission {
        for method in error.methods.iter().filter(|m| !m.sanitized) {
            let fn_name = format!("{}_{}", error.name.to_lowercase(), method.name);
            let return_type = mapper.map_type(&method.return_type);
            let default_val = rustler_default_for_type(&method.return_type);
            let shim = format!(
                "/// Introspection NIF: returns the `{method_name}` value carried by the error.\n\
                     /// Planned: extend to accept a structured error term once error passing is upgraded.\n\
                     #[allow(dead_code)]\n\
                     #[rustler::nif]\n\
                     fn {fn_name}(_msg: String) -> {return_type} {{\n    {default_val}\n}}\n",
                method_name = method.name,
                fn_name = fn_name,
                return_type = return_type,
                default_val = default_val,
            );
            builder.add_item(&shim);
        }
    }

    // Sorted by name, same rationale as `types_for_struct_emission` above. ~keep
    let mut types_for_from_json_nif_emission: Vec<&crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| {
            !t.is_trait
                && !t.is_opaque
                && !t.fields.is_empty()
                && t.has_serde
                && !exclude_types.contains(t.name.as_str())
                && types_to_emit.contains(&t.name)
        })
        .collect();
    types_for_from_json_nif_emission.sort_by(|a, b| a.name.cmp(&b.name));
    for typ in types_for_from_json_nif_emission {
        builder.add_item(&gen_from_json_nif(typ, &core_import));
    }

    builder.add_item(&gen_nif_init(api, config, &exclude_functions, &exclude_types));

    let content = builder.build();

    let output_dir = resolve_output_dir(
        config.output_paths.get("elixir"),
        &config.name,
        "packages/elixir/native/{name}_nif/src/",
    );

    // A `[crates.output] elixir = "..."` override can point `output_dir` (this `lib.rs`'s own
    // destination) somewhere `scaffold_elixir_cargo`'s `nif_name` segment never appears under, so
    // the manifest path is read from `scaffold::elixir_native_crate_dir` -- the same formula that
    // actually wrote it -- rather than derived from `output_dir`. Elixir has no analogue of
    // Ruby's gated registration statement (`#[rustler::nif]` auto-registers, so a definition
    // compiled out by its own `#[cfg]` simply has nothing left to register) -- an undeclared
    // feature here does not fail the build, it silently drops the NIF at runtime while the
    // generated `.ex` facade and docs keep advertising it. This warning is the only place that
    // gets surfaced. ~keep
    let native_manifest_dir = crate::scaffold::elixir_native_crate_dir(config);
    let manifest_path = crate::codegen::cfg::resolve_against_workspace_root(
        config,
        &PathBuf::from(native_manifest_dir).join("Cargo.toml"),
    );
    crate::codegen::cfg::warn_on_undeclared_binding_cfg_features(api, Language::Elixir, &manifest_path);

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("lib.rs"),
        content,
        generated_header: false,
    }])
}
