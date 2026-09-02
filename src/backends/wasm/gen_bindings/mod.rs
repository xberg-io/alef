//! WASM (wasm-bindgen) backend: orchestration and `Backend` trait implementation.

mod cfg;
mod components;
pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod service_api;
mod ts_union;
pub(crate) use ts_union::{docs_ts_type_for_untagged_enum, is_bigint_primitive};
pub mod types;

mod cargo;
mod helpers;
mod trait_bridge_docs;

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::{cfg::enabled_features_for_language, generators, shared};
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::{AHashMap, AHashSet};
use std::path::PathBuf;

use cargo::gen_cargo_toml;
use trait_bridge_docs::forward_trait_bridge_builder_fields;

pub(crate) use helpers::{WasmCallability, wasm_callability};
use helpers::{
    fix_dropped_payload_enum_option_fields, prepend_cfg, prepend_unknown_type_omission_marker,
    types_needing_self_delegation_reverse_impl, wasm_output_layout,
};
// Only `trait_bridge_tests` (via `super::function_is_exported`) calls this directly; production
// code reaches the same check through `function_is_callable`. ~keep
use cfg::{
    cfg_condition_enabled, collect_cfg_features, field_references_excluded_type, first_unknown_named_type,
    is_gated_behind_disabled_feature,
};
use enums::gen_enum;
use errors::{gen_error_converter, gen_error_methods};
use functions::{gen_env_shims, gen_function_with_emitted_dtos};
#[cfg(test)]
use helpers::function_is_exported;
use types::{
    filter_cfg_fields_for_features, gen_opaque_struct, gen_opaque_struct_methods, gen_struct, gen_struct_methods,
};

pub struct WasmBackend;

impl Backend for WasmBackend {
    fn name(&self) -> &str {
        "wasm"
    }

    fn language(&self) -> Language {
        Language::Wasm
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

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        crate::codegen::config_gen::validate_rust_default_functions(api)?;
        // Must run before dedup -- see `cfg::drop_cfg_disabled_functions`. ~keep
        let enabled_features = enabled_features_for_language(config, Language::Wasm);
        let sorted = crate::backends::ir_order::with_sorted_items(api);
        let api = &cfg::drop_cfg_disabled_functions(sorted, &enabled_features).with_deduped_functions();

        let wasm_config = config.wasm.as_ref();
        let mut exclude_functions = wasm_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
        let mut exclude_types = ts_union::wasm_exclude_types(config);
        let text_field_enum_names: AHashSet<String> = config.untagged_union_text_types.iter().cloned().collect();
        let mut type_overrides = wasm_config.map(|c| c.type_overrides.clone()).unwrap_or_default();
        for name in &text_field_enum_names {
            type_overrides
                .entry(name.clone())
                .or_insert_with(|| "String".to_string());
        }
        // See `enums::register_untagged_data_enum_overrides` for why. ~keep
        let untagged_data_enum_names = enums::register_untagged_data_enum_overrides(api, &mut type_overrides);
        let env_shims = wasm_config.map(|c| c.env_shims.clone()).unwrap_or_default();
        let prefix = config.wasm_type_prefix();

        for typ in &api.types {
            if is_gated_behind_disabled_feature(&typ.cfg, &enabled_features) {
                exclude_types.push(typ.name.clone());
            }
        }
        for enum_def in &api.enums {
            if is_gated_behind_disabled_feature(&enum_def.cfg, &enabled_features) {
                exclude_types.push(enum_def.name.clone());
            }
        }
        for func in &api.functions {
            if is_gated_behind_disabled_feature(&func.cfg, &enabled_features) {
                exclude_functions.push(func.name.clone());
            }
        }

        // Captured before the move: `known_type_names` below needs the override keys, and
        // `WasmMapper::new` takes the map by value.
        let override_type_names: Vec<String> = type_overrides.keys().cloned().collect();
        let mapper = WasmMapper::new(type_overrides, prefix.clone());
        let core_import = config.core_import_for_language(Language::Wasm);
        // See `enums::gen_enum`'s doc comment: wasm-bindgen cannot express a per-variant cfg
        // guard, so its declaration must be resolved definitively against this binding's own
        // configured feature set instead of deferring to the compiler. ~keep
        let configured_features_set: std::collections::HashSet<&str> =
            enabled_features.iter().map(String::as_str).collect();

        let source_remap_pairs: Vec<(String, String)> = wasm_config
            .map(|c| c.source_crate_remaps.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|orig| (orig.replace('-', "_"), core_import.clone()))
            .collect();
        let source_remaps_borrowed: Vec<(&str, &str)> = source_remap_pairs
            .iter()
            .map(|(o, n)| (o.as_str(), n.as_str()))
            .collect();
        let dropped_crates: AHashSet<String> = wasm_config
            .map(|c| c.exclude_extra_dependencies.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|name| name.replace('-', "_"))
            .filter(|underscored| {
                underscored != &core_import && !source_remap_pairs.iter().any(|(orig, _)| orig == underscored)
            })
            .collect();
        for typ in &api.types {
            let crate_seg = typ.rust_path.split("::").next().unwrap_or("").replace('-', "_");
            if dropped_crates.contains(&crate_seg) && !exclude_types.contains(&typ.name) {
                exclude_types.push(typ.name.clone());
            }
        }
        for enum_def in &api.enums {
            let crate_seg = enum_def.rust_path.split("::").next().unwrap_or("").replace('-', "_");
            if dropped_crates.contains(&crate_seg) && !exclude_types.contains(&enum_def.name) {
                exclude_types.push(enum_def.name.clone());
            }
        }
        for func in &api.functions {
            let crate_seg = func.rust_path.split("::").next().unwrap_or("").replace('-', "_");
            if dropped_crates.contains(&crate_seg) && !exclude_functions.contains(&func.name) {
                exclude_functions.push(func.name.clone());
            }
        }
        let dropped_error_names: Vec<String> = api
            .errors
            .iter()
            .filter(|e| {
                let crate_seg = e.rust_path.split("::").next().unwrap_or("").replace('-', "_");
                dropped_crates.contains(&crate_seg)
            })
            .map(|e| e.name.clone())
            .collect();
        for name in dropped_error_names {
            if !exclude_types.contains(&name) {
                exclude_types.push(name);
            }
        }

        // is treated as if it were `#[cfg]`-gated, so the binding struct omits it and
        let exclude_fields_map = wasm_config.map(|c| c.exclude_fields.clone()).unwrap_or_default();
        let api_owned;
        let api: &ApiSurface = if exclude_fields_map.is_empty() {
            api
        } else {
            api_owned = {
                let mut cloned = api.clone();
                for typ in &mut cloned.types {
                    if let Some(skip_list) = exclude_fields_map.get(&typ.name) {
                        let before = typ.fields.len();
                        typ.fields.retain(|field| !skip_list.iter().any(|s| s == &field.name));
                        if typ.fields.len() != before {
                            typ.has_stripped_cfg_fields = true;
                        }
                    }
                }
                cloned
            };
            &api_owned
        };
        let cfg_filtered_api = filter_cfg_fields_for_features(api, &enabled_features);
        let api = &cfg_filtered_api;

        // Detect fields that reference a type with no generated wasm binding: neither a
        // `TypeDef`/`EnumDef` present in the (already cfg-filtered) API surface nor an explicit
        // `type_overrides` entry. `WasmMapper::named` (see `type_map.rs`) maps every
        // `TypeRef::Named` unconditionally to `"{prefix}{name}"` with no existence check, so
        // left alone this would silently emit a reference to a `Wasm*` struct that is never
        // generated — a dangling-type compile failure the consumer only discovers by running
        // `wasm-pack build`, not by reading the generated source. Route such fields through the
        // same exclusion machinery as cfg-gated fields, but warn loudly and mark the omission in
        // the generated source instead of dropping it in silence.
        let mut known_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
        known_type_names.extend(api.enums.iter().map(|e| e.name.clone()));
        known_type_names.extend(override_type_names.iter().cloned());
        let mut unknown_type_omissions: AHashMap<String, Vec<(String, String)>> = AHashMap::default();
        for typ in api.types.iter().filter(|t| !t.is_opaque && !t.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            for field in shared::binding_fields(&typ.fields) {
                if field_references_excluded_type(&field.ty, &exclude_types) {
                    continue;
                }
                let Some(unknown_name) = first_unknown_named_type(&field.ty, &known_type_names) else {
                    continue;
                };
                let unknown_name = unknown_name.to_string();
                tracing::warn!(
                    struct_name = %typ.name,
                    field_name = %field.name,
                    referenced_type = %unknown_name,
                    "wasm backend: field references a type with no generated wasm binding; omitting field"
                );
                if !exclude_types.contains(&unknown_name) {
                    exclude_types.push(unknown_name.clone());
                }
                unknown_type_omissions
                    .entry(typ.name.clone())
                    .or_default()
                    .push((field.name.clone(), unknown_name));
            }
        }

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute(
            "allow(dead_code, unused_imports, unused_variables, unreachable_patterns, missing_docs)",
        );
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::redundant_field_names, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::useless_conversion, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy, clippy::should_implement_trait, clippy::await_holding_refcell_ref, clippy::new_without_default)");
        if let Some(extra_attr) = crate::codegen::shared::format_extra_clippy_allows(
            &config.extra_clippy_allows,
            builder.inner_attributes_text(),
        ) {
            builder.add_inner_attribute(&extra_attr);
        }
        for attribute in crate::codegen::shared::format_crate_attributes(&config.crate_attributes) {
            builder.add_inner_attribute(&attribute);
        }
        builder.add_import("wasm_bindgen::prelude::*");

        if let Some(modules) = wasm_config.map(|c| c.custom_rust_modules.as_slice()) {
            for module in modules {
                builder.add_item(&format!("pub mod {module};"));
                builder.add_item(&format!("pub use {module}::*;"));
            }
        }

        // so no explicit `use js_sys;` import is needed (clippy::single_component_path_imports).

        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        if !env_shims.is_empty() {
            builder.add_item(&gen_env_shims(&env_shims));
        }

        let opaque_types: AHashSet<String> = ts_union::wasm_opaque_type_names(api, &exclude_types);
        let mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !exclude_types.contains(&t.name) && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
            if !mutex_types.is_empty() {
                builder.add_import("std::sync::Mutex");
            }
        }

        let bridge_type_aliases: AHashSet<String> = trait_bridge_docs::active_bridges(config)
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let mut opaque_names_vec: Vec<String> = opaque_types.iter().cloned().collect();
        opaque_names_vec.extend(bridge_type_aliases.iter().cloned());
        let opaque_names_set: AHashSet<String> = opaque_names_vec.iter().cloned().collect();

        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Wasm)?;

        let streaming_item_types: ahash::AHashMap<String, String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter(|a| !a.skip_languages.iter().any(|l| l == "wasm"))
            .filter_map(|a| {
                let owner = a.owner_type.as_deref()?;
                let item = a.item_type.as_deref()?;
                Some((format!("{owner}.{}", a.name), item.to_string()))
            })
            .collect();

        let wasm_skipped_methods: AHashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter(|a| a.skip_languages.iter().any(|l| l == "wasm"))
            .filter_map(|a| {
                let owner = a.owner_type.as_deref()?;
                Some(format!("{owner}.{}", a.name))
            })
            .collect();

        for adapter in &config.adapters {
            match adapter.pattern {
                crate::core::config::AdapterPattern::Streaming => {
                    let key = crate::adapters::stream_struct_key(adapter);
                    if let Some(struct_code) = adapter_bodies.get(&key) {
                        builder.add_item(struct_code);
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

        // `#[wasm_bindgen]` entrypoints (e.g. `app_run`) are compiled and exported.
        let has_wasm_services = api.services.iter().any(|svc| {
            !config
                .services
                .iter()
                .any(|sc| sc.owner_type == svc.name && sc.skip_languages.iter().any(|l| l == "wasm"))
        });
        if has_wasm_services {
            builder.add_item("pub mod service;");
        }

        // Names whose fields are stored as `JsValue` and bridged through `serde_wasm_bindgen`,
        // for both `gen_struct` below and `wasm_conv_config`. ~keep
        //
        // `untagged_union_text_types` members are excluded even when they are untagged data
        // enums: that opt-in already pinned their `type_overrides` entry to `String`, which is
        // what the accessor/constructor emitter reads. Leaving them here too would store the
        // field as `JsValue` while the getter and setter still say `String` -- a struct that
        // does not compile. The text opt-in is the more specific signal, so it wins on both
        // sides. ~keep
        let jsvalue_bridged_enum_names: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                !exclude_types.contains(&e.name)
                    && !text_field_enum_names.contains(&e.name)
                    && (enums::is_tagged_data_enum(e) || enums::is_untagged_data_enum(e))
            })
            .map(|e| e.name.clone())
            .collect();

        let methods_enums: Vec<_> = api
            .enums
            .iter()
            .filter(|e| !text_field_enum_names.contains(&e.name) && !untagged_data_enum_names.contains(&e.name))
            .cloned()
            .collect();

        // Built up front (structs are emitted before enums below) so `gen_struct_methods` can
        // type an untagged-enum-typed field's getter/setter as the real structural union instead
        // of bare `JsValue`. `untagged_ts_plan.plans` is consumed later, in the enum loop, to
        // keep emission order (structs, then enums) unchanged. ~keep
        let mut untagged_ts_plan = ts_union::build_untagged_enum_ts_plan_for_api(
            api,
            &exclude_types,
            &opaque_types,
            &text_field_enum_names,
            &prefix,
        );
        if !untagged_ts_plan.custom_section.is_empty() {
            builder.add_item(&untagged_ts_plan.custom_section);
        }
        let untagged_ts_value_types = ts_union::value_type_names(&untagged_ts_plan);

        let core_to_binding_convertible_for_structs =
            crate::codegen::conversions::core_to_binding_convertible_types(api, &exclude_types);

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            if typ.is_opaque {
                builder.add_item(&gen_opaque_struct(typ, &core_import, &prefix));
                builder.add_item(&gen_opaque_struct_methods(
                    typ,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    &prefix,
                    &adapter_bodies,
                    &mutex_types,
                    &streaming_item_types,
                    &wasm_skipped_methods,
                    &config.trait_bridges,
                ));
                // Client constructor — emit a #[wasm_bindgen(constructor)] impl
                if let Some(ctor) = config.client_constructors.get(&typ.name) {
                    let struct_name = format!("{prefix}{}", typ.name);
                    let ctor_body = generators::gen_opaque_constructor(
                        ctor,
                        &typ.name,
                        &core_import,
                        "#[wasm_bindgen(constructor)]",
                    );
                    let ctor_impl = format!("#[wasm_bindgen]\nimpl {struct_name} {{\n{}}}", ctor_body);
                    builder.add_item(&ctor_impl);
                }
            } else {
                // A type that dropped a field is NOT core-to-binding convertible: the
                // delegating `Default` impl is `<core::T as Default>::default().into()`, which
                // needs a `From<core::T>` that can carry every field across. The omitted field has
                // no binding representation to carry it into, so the conversion cannot exist and
                // the struct must fall back to `#[derive(Default)]` on the fields that remain.
                let is_core_to_binding_convertible = core_to_binding_convertible_for_structs.contains(&typ.name)
                    && !unknown_type_omissions.contains_key(&typ.name);
                // gen_struct gates #[derive(Default)] and the delegating Default impl on
                let struct_code = gen_struct(
                    typ,
                    &mapper,
                    &exclude_types,
                    &core_import,
                    &prefix,
                    &jsvalue_bridged_enum_names,
                    &source_remaps_borrowed,
                    is_core_to_binding_convertible,
                );
                builder.add_item(&prepend_unknown_type_omission_marker(
                    unknown_type_omissions.get(&typ.name),
                    struct_code,
                ));
                builder.add_item(&gen_struct_methods(
                    typ,
                    &mapper,
                    &exclude_types,
                    &core_import,
                    &opaque_types,
                    &methods_enums,
                    &prefix,
                    &mutex_types,
                    &streaming_item_types,
                    &untagged_ts_value_types,
                ));
            }
        }

        for enum_def in &api.enums {
            if exclude_types.contains(&enum_def.name) {
                continue;
            }
            // No `Wasm{Enum}` type for an untagged data enum — see `register_untagged_data_enum_overrides`. ~keep
            if let Some(plan) = untagged_ts_plan.plans.remove(&enum_def.name) {
                builder.add_item(&plan.extern_type_declaration);
            } else if !enums::is_untagged_data_enum(enum_def) {
                builder.add_item(&gen_enum(enum_def, &prefix, &core_import, &configured_features_set));
            }
        }

        let mut emitted_input_dtos = AHashSet::new();
        let mut input_dto_code = String::new();

        for func in &api.functions {
            if !exclude_functions.contains(&func.name)
                && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(
                    &func.name,
                    &config.trait_bridges,
                )
            {
                let refs_excluded = func
                    .params
                    .iter()
                    .any(|p| field_references_excluded_type(&p.ty, &exclude_types))
                    || field_references_excluded_type(&func.return_type, &exclude_types);
                if !refs_excluded {
                    for p in &func.params {
                        if let TypeRef::Named(name) = &p.ty
                            && !opaque_types.contains(name.as_str())
                            && !emitted_input_dtos.contains(name.as_str())
                            && let Some(type_def) = api.types.iter().find(|t| t.name == name.as_str())
                            && functions::should_have_input_dto(type_def)
                        {
                            let non_deserializable_type_names: std::collections::HashSet<String> = api
                                .types
                                .iter()
                                .filter(|t| !t.has_serde || t.is_trait || t.is_opaque)
                                .map(|t| t.name.clone())
                                .collect();
                            let (dto_code, _dto_name) = functions::gen_input_dto_for_type_with_cfg(
                                name.as_str(),
                                &core_import,
                                type_def,
                                &exclude_types,
                                &enabled_features,
                                &non_deserializable_type_names,
                            );
                            if !dto_code.is_empty() {
                                input_dto_code.push_str(&dto_code);
                                input_dto_code.push_str("\n\n");
                                emitted_input_dtos.insert(name.clone());
                            }
                        }
                    }
                }
            }
        }
        if !input_dto_code.is_empty() {
            builder.add_item(&input_dto_code);
        }

        for func in &api.functions {
            if !exclude_functions.contains(&func.name) {
                if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(
                    &func.name,
                    &config.trait_bridges,
                ) {
                    continue;
                }
                let refs_excluded = func
                    .params
                    .iter()
                    .any(|p| field_references_excluded_type(&p.ty, &exclude_types))
                    || field_references_excluded_type(&func.return_type, &exclude_types);
                if refs_excluded {
                    continue;
                }
                let bridge_param = crate::backends::wasm::trait_bridge::find_bridge_param(func, &config.trait_bridges)
                    .filter(|(_, bridge_cfg)| trait_bridge_docs::targets_wasm(bridge_cfg));
                let options_field_bridge =
                    crate::backends::wasm::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
                        .filter(|(_, bridge_cfg)| trait_bridge_docs::targets_wasm(bridge_cfg))
                        .filter(|(_, bridge_cfg)| {
                            let Some(field_name) = bridge_cfg.resolved_options_field() else {
                                return false;
                            };
                            let Some(options_type) = bridge_cfg.options_type.as_deref() else {
                                return false;
                            };
                            api.types
                                .iter()
                                .filter(|t| t.name == options_type)
                                .flat_map(|t| t.fields.iter())
                                .any(|f| f.cfg.is_none() && f.name == field_name)
                        });
                if let Some((param_idx, bridge_cfg)) = bridge_param {
                    let item = crate::backends::wasm::trait_bridge::gen_bridge_function(
                        api,
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &prefix,
                    );
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
                } else if let Some((param_idx, bridge_cfg)) = options_field_bridge {
                    let item = crate::backends::wasm::trait_bridge::gen_options_field_bridge_function(
                        api,
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &prefix,
                    );
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
                } else {
                    let item = gen_function_with_emitted_dtos(
                        func,
                        &mapper,
                        &core_import,
                        &opaque_types,
                        &prefix,
                        &mutex_types,
                        api,
                        &emitted_input_dtos,
                    );
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
                }
            }
        }

        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = trait_bridge_docs::active_bridge_trait(bridge_cfg, api) {
                let bridge = crate::backends::wasm::trait_bridge::gen_trait_bridge(
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
                builder.add_item(&bridge.code);
            }
        }

        let trait_bridge_arc_wrapper_field_names: Vec<String> = trait_bridge_docs::active_bridges(config)
            .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
            .filter_map(|b| b.resolved_options_field().map(String::from))
            .collect();
        let wasm_conv_config = crate::codegen::conversions::ConversionConfig {
            type_name_prefix: &prefix,
            map_uses_jsvalue: true,
            option_duration_on_defaults: true,
            optionalize_defaults: false,
            exclude_types: &exclude_types,
            source_crate_remaps: &source_remaps_borrowed,
            opaque_types: if opaque_names_set.is_empty() {
                None
            } else {
                Some(&opaque_names_set)
            },
            trait_bridge_arc_wrapper_field_names: &trait_bridge_arc_wrapper_field_names,
            tagged_data_enum_names: if jsvalue_bridged_enum_names.is_empty() {
                None
            } else {
                Some(&jsvalue_bridged_enum_names)
            },
            text_field_enum_names: if text_field_enum_names.is_empty() {
                None
            } else {
                Some(&text_field_enum_names)
            },
            // See `enums::gen_enum`'s doc comment and `configured_features_set` above: this is
            // the same set already threaded into the enum declaration path, now also reaching the
            // conversion path so a foreign cfg-gated variant proven unreachable here suppresses
            // its now-provably-dead `_ => Default::default()` catch-all instead of leaving one
            // behind for `cargo clippy -- -D warnings` to trip on. ~keep
            configured_features: Some(enabled_features.as_slice()),
            ..Default::default()
        };
        let convertible = crate::codegen::conversions::convertible_types(api);
        let core_to_binding_convertible =
            crate::codegen::conversions::core_to_binding_convertible_types(api, &exclude_types);
        let input_types = crate::codegen::conversions::input_type_names(api);
        let self_delegating_types = types_needing_self_delegation_reverse_impl(api, &opaque_types);
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            let is_strict = crate::codegen::conversions::can_generate_conversion(typ, &convertible);
            let is_relaxed = crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding_convertible);
            if is_strict {
                if input_types.contains(&typ.name) || self_delegating_types.contains(&typ.name) {
                    builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                        typ,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
                builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &wasm_conv_config,
                ));
            } else if is_relaxed {
                builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &wasm_conv_config,
                ));
            }
        }
        for e in &api.enums {
            if !exclude_types.contains(&e.name) {
                if enums::is_untagged_data_enum(e) {
                    // No `Wasm{Enum}` type to write a `From` impl against; see the `gen_enum` skip above. ~keep
                } else if enums::is_tagged_data_enum(e) {
                    if input_types.contains(&e.name) {
                        builder.add_item(&enums::gen_tagged_enum_binding_to_core(e, &core_import, &prefix));
                    }
                    builder.add_item(&enums::gen_tagged_enum_core_to_binding(e, &core_import, &prefix));
                } else {
                    if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                        builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(
                            e,
                            &core_import,
                            &wasm_conv_config,
                        ));
                    }
                    if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                        builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(
                            e,
                            &core_import,
                            &wasm_conv_config,
                        ));
                    }
                }
            }
        }

        for error in &api.errors {
            if exclude_types.contains(&error.name) {
                continue;
            }
            builder.add_item(&gen_error_converter(error, &core_import, &source_remaps_borrowed));
            let methods_block = gen_error_methods(error, &core_import, &prefix);
            if !methods_block.is_empty() {
                builder.add_item(&methods_block);
            }
        }

        if let Some(component_api) = components::generate(config) {
            builder.add_item(&component_api);
        }

        let mut content = builder.build();
        content = fix_dropped_payload_enum_option_fields(content);

        content = forward_trait_bridge_builder_fields(content, config);

        for bridge in trait_bridge_docs::active_bridges(config) {
            if bridge.bind_via != crate::core::config::BridgeBinding::OptionsField {
                continue;
            }
            let (Some(options_type), Some(field_name)) =
                (bridge.options_type.as_deref(), bridge.resolved_options_field())
            else {
                continue;
            };
            for variant in ["", "Update"] {
                let binding_name = format!("Wasm{options_type}{variant}");
                let core_path = format!("{core_import}::options::{options_type}{variant}");
                let impl_header = format!("impl From<{binding_name}> for {core_path} {{");
                if !content.contains(&impl_header) {
                    continue;
                }
                let patterns = &[
                    ("            ", "\n            "),
                    ("        ", "\n        "),
                    ("  ", "\n  "),
                ];
                for (indent, newline_indent) in patterns {
                    let old_pattern =
                        format!("{indent}{field_name}: Default::default(),{newline_indent}..Default::default()");
                    let new_pattern = format!(
                        "{indent}{field_name}: val.{field_name}.map(|v| (*v.inner).clone()),{newline_indent}..Default::default()"
                    );
                    if content.contains(&old_pattern) {
                        content = content.replace(&old_pattern, &new_pattern);
                    }
                }
            }
        }

        // `gen_cargo_toml` emits no `[lib] path`, so cargo resolves the library at
        // `<crate root>/src/lib.rs`; both paths must therefore come from one derivation of the
        // output path, and taking the crate root to be its parent is right only when the
        // configured path is `src`-suffixed. ~keep
        let layout = wasm_output_layout(config);
        let cargo_toml_path = layout.root.join("Cargo.toml");

        Ok(vec![
            GeneratedFile {
                path: layout.src.join("lib.rs"),
                content,
                generated_header: false,
            },
            GeneratedFile {
                // The wasm crate builds its own manifest instead of going through
                // `scaffold::scaffold`, so it needs the version floor applied here or it
                // stays the one regenerated binding manifest that can still downgrade a
                // dependency the consumer has already moved forward. ~keep
                content: crate::scaffold::version_floor::floor_manifest(
                    &gen_cargo_toml(api, config),
                    config,
                    &cargo_toml_path,
                ),
                path: cargo_toml_path,
                generated_header: true,
            },
        ])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        Ok(service_api::gen_service_files(&sorted_api, config))
    }

    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<crate::core::backend::TraitBridgeRegistrationSurface> {
        trait_bridge_docs::registration_surface(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "wasm-pack",
            crate_suffix: "-wasm",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }

    fn build_config_with_config(&self, config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        let mut build_config = self.build_config()?;

        // `wasm-pack build --target nodejs --out-dir pkg/nodejs` (see `build_command_for`'s
        // "wasm-pack" arm) writes its own `package.json` derived from the wasm crate's
        // `Cargo.toml`, not from `config.wasm_package_name()`. Every `file:` dependency and
        // `require()`/`import` specifier the wasm e2e codegen emits uses
        // `wasm_package_name()` (see `ResolvedCrateConfig::wasm_crate_path`), so rewrite the
        // built artifact's declared name to match after every build — otherwise the
        // specifier names a package the directory does not declare. Verified against a
        // consumer whose e2e manifest depends on a scoped `@scope/<crate>-wasm` name resolved
        // by a `file:` path into `crates/<crate>-wasm/pkg/nodejs`, while that directory
        // declares the bare crate name `<crate>-wasm`.
        //
        // This must resolve the crate directory exactly as `build_command_for` does —
        // `[crates.output] wasm` first (minus a trailing `src`, which is where the *generated
        // sources* land, not the crate root), then the `package_dir` default formula. Deriving
        // it any other way lets the build write `pkg/nodejs` under one directory while this
        // step looks under another, and a missing file is only debug-logged, so the rewrite
        // would silently never fire. ~keep
        let wasm_crate_dir = config
            .explicit_output
            .wasm
            .as_deref()
            .map(|output| {
                if output.file_name().is_some_and(|name| name == "src") {
                    output.parent().unwrap_or(output)
                } else {
                    output
                }
            })
            .map_or_else(|| PathBuf::from(config.package_dir(Language::Wasm)), PathBuf::from);
        let package_json_path = wasm_crate_dir.join("pkg/nodejs/package.json");
        build_config.post_build.push(PostBuildStep::RewriteWasmPackageName {
            package_json_path,
            package_name: config.wasm_package_name(),
        });

        Some(build_config)
    }
}

#[cfg(test)]
mod cargo_feature_leak_tests;
#[cfg(test)]
mod cfg_variant_e2e_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trait_bridge_tests;
#[cfg(test)]
mod untagged_enum_tests;
