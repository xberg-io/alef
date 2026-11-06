//! Emits the Rust-side swift-bridge crate for Phase 2C.
//!
//! Three files are generated into `packages/swift/rust/`:
//! - `Cargo.toml` — declares the `cdylib`/`staticlib` crate with swift-bridge dependencies
//! - `src/lib.rs` — `#[swift_bridge::bridge] mod ffi { … }` plus wrapper newtypes and shims
//! - `build.rs` — calls `swift_bridge_build::parse_bridges` to run codegen at build time

mod artifacts;
pub(crate) mod bridge_error;
#[cfg(test)]
mod bridge_error_skipped_fields_tests;
mod callback_files;
pub(crate) mod cargo;
#[cfg(test)]
mod cargo_excluded_features_tests;
#[cfg(test)]
mod cargo_sort_order_tests;
#[cfg(test)]
mod cfg_variant_e2e_tests;
mod components;
mod constructor_history;
pub(crate) mod default_construction;
mod deferred_noop;
pub(crate) mod enums;
pub(crate) mod extern_block;
pub(crate) mod feature_gate;
mod json_bridge;
#[cfg(test)]
mod owned_inputs_tests;
#[cfg(test)]
mod owned_values_tests;
pub(crate) mod owned_vec;
#[cfg(test)]
mod owned_vec_tests;
pub(crate) mod plugin_inbound;
#[cfg(test)]
mod raw_string_constructor_tests;
pub(crate) mod service_app_wrappers;
pub(crate) mod shims;
#[cfg(test)]
mod tagged_enum_wiring_tests;
pub(crate) mod text_projection;
pub mod trait_bridge;
pub(crate) mod type_bridge;
pub(crate) mod wrappers;

use crate::backends::swift::gen_bindings::swift_package_root;
use crate::codegen::generators::type_paths::build_type_path_lookup;
use crate::core::backend::GeneratedFile;
use crate::core::config::extras::Language;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, TypeDef};
use crate::core::template_versions;
use heck::AsSnakeCase;
use std::collections::HashSet;

pub use artifacts::emit;

fn emit_lib_rs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    crate_name: &str,
    exclude_functions: &HashSet<String>,
    exclude_types: &HashSet<String>,
    exclude_fields: &HashSet<String>,
    configured_features: &HashSet<&str>,
) -> anyhow::Result<(String, owned_vec::OwnedVecAccessors)> {
    let source_crate = crate_name.replace('-', "_");

    // This binding's own configured feature set (already expanded through the core crate's
    // `[features]` graph), used to decide whether a FOREIGN-owned cfg-gated enum variant is
    // provably unreachable for this binding -- see
    // `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features`. Distinct from
    // `configured_features` above: that set is widened with every HOST-owned cfg feature name
    // `collect_cfg_features` finds (`feature_gate::effective_swift_codegen_features`), to decide
    // which host-owned cfg-gated methods this bridge crate can expose unconditionally -- the
    // wrong question for a FOREIGN variant's reachability, which needs the real configured
    // feature list instead. ~keep
    let configured_enum_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Swift);

    let type_paths = build_type_path_lookup(api);

    let mut out = String::new();
    out.push_str(crate::core::hash::SELF_MARKING_HEADER_LINE);
    out.push_str("\n\n");
    out.push_str("#![allow(unused_variables, unreachable_code, unreachable_patterns, missing_docs)]\n");
    out.push_str("#![allow(\n");
    out.push_str("    clippy::collapsible_if,\n");
    out.push_str("    clippy::collapsible_match,\n");
    out.push_str("    clippy::needless_borrows_for_generic_args,\n");
    out.push_str("    clippy::field_reassign_with_default,\n");
    out.push_str("    clippy::too_many_arguments,\n");
    out.push_str("    clippy::clone_on_copy,\n");
    out.push_str("    clippy::unnecessary_cast,\n");
    out.push_str("    clippy::manual_flatten,\n");
    out.push_str("    clippy::match_single_binding,\n");
    out.push_str("    clippy::redundant_closure,\n");
    out.push_str("    clippy::useless_conversion,\n");
    out.push_str("    clippy::inherent_to_string,\n");
    out.push_str("    clippy::new_without_default,\n");
    out.push_str(")]\n");
    if let Some(extra_attr) = crate::codegen::shared::format_extra_clippy_allows(&config.extra_clippy_allows, &out) {
        out.push_str(&format!("#![{extra_attr}]\n"));
    }
    for attribute in crate::codegen::shared::format_crate_attributes(&config.crate_attributes) {
        out.push_str(&format!("#![{attribute}]\n"));
    }
    out.push('\n');

    out.push_str(shims::ALEF_TOKIO_RUNTIME_DEFINITION);
    out.push('\n');

    out.push_str(&crate::backends::swift::template_env::render(
        "ffi_keep_alive_shim.rs.jinja",
        crate::alef_context! {
            ffi_import => config.ffi_crate_package_name().replace('-', "_"),
            ffi_version_fn => format!("{}_version", config.ffi_prefix()),
        },
    ));
    out.push('\n');

    let visible_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait)
        .filter(|t| feature_gate::cfg_satisfied(t.cfg.as_deref(), configured_features))
        .collect();

    // `&mut T` DTO parameters can only hand the caller their update back through a
    // returned value, never through the opaque handle mutation an opaque type gets — see
    // `mut_writeback`. ~keep
    let opaque_type_names_ahash: ahash::AHashSet<String> = visible_types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let visible_enums: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .filter(|e| feature_gate::cfg_satisfied(e.cfg.as_deref(), configured_features))
        .collect();

    let enum_names: HashSet<&str> = visible_enums.iter().map(|e| e.name.as_str()).collect();

    let unit_enum_names: HashSet<&str> = visible_enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    let tagged_enum_names: HashSet<&str> = visible_enums
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();
    let defaultable_names: HashSet<&str> = visible_types
        .iter()
        .filter(|ty| ty.has_default)
        .map(|ty| ty.name.as_str())
        .chain(
            visible_enums
                .iter()
                .filter(|en| en.has_default)
                .map(|en| en.name.as_str()),
        )
        .collect();
    let reconstructible_enum_names: HashSet<&str> = visible_enums
        .iter()
        .filter(|en| enums::can_reconstruct_legacy_variant(en, &defaultable_names))
        .map(|en| en.name.as_str())
        .collect();
    let enum_kinds = default_construction::EnumKinds::with_reconstructible(
        &enum_names,
        &unit_enum_names,
        &reconstructible_enum_names,
    );

    let visible_type_names: HashSet<&str> = visible_types
        .iter()
        .map(|t| t.name.as_str())
        .chain(enum_names.iter().copied())
        .collect();

    let no_serde_names: HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.has_serde)
        .map(|t| t.name.as_str())
        .collect();
    let first_class_owned =
        crate::backends::swift::gen_bindings::dto::compute_first_class_dto_names(api, exclude_types);
    let first_class_names: HashSet<&str> = first_class_owned.iter().map(|s| s.as_str()).collect();
    let no_serde_enum_names: HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| !e.has_serde)
        .map(|e| e.name.as_str())
        .collect();

    let handle_returned_types: HashSet<String> = type_bridge::compute_handle_returned_types(api);

    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .filter(|f| feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features))
        .filter(|f| !config.trait_bridge_manages_function(&f.name))
        .filter(|f| {
            shims::is_bridgeable_fn(
                f,
                &unit_enum_names,
                &type_paths,
                &no_serde_names,
                &no_serde_enum_names,
                &visible_type_names,
                &handle_returned_types,
            )
        })
        .collect();

    let active_bridges: Vec<(&TraitBridgeConfig, &TypeDef)> = config
        .trait_bridges_for(Language::Swift)
        .filter(|b| !b.exclude_languages.iter().any(|l| l == "swift"))
        .filter_map(|b| {
            api.types
                .iter()
                .find(|t| t.is_trait && t.name == b.trait_name)
                .map(|t| (b, t))
        })
        .collect();

    let result_type_enums: std::collections::HashSet<String> = active_bridges
        .iter()
        .filter_map(|(bridge_cfg, _)| bridge_cfg.result_type.as_deref().map(|s| s.to_string()))
        .collect();

    let enum_names_owned: std::collections::HashSet<String> = enum_names.iter().map(|s| s.to_string()).collect();
    let mut extern_blocks: Vec<String> = Vec::new();
    let mut owned_vec_accessors = owned_vec::OwnedVecAccessors {
        inputs: super::owned_vector_inputs::collect(api, exclude_types, exclude_functions, |name| {
            first_class_names.contains(name) || !visible_type_names.contains(name)
        }),
        ..Default::default()
    };
    // `BTreeSet` so the paired `extern "Rust"` declaration (`extern_block::
    // emit_extern_block_for_functions`) and no-op shim (`deferred_noop::emit_shims`) are
    // emitted in the same deterministic order across runs. ~keep
    let mut deferred_empty_handle_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for ty in &visible_types {
        let is_handle_returned = handle_returned_types.contains(&ty.name);
        let parent_first_class = first_class_names.contains(ty.name.as_str());
        let would_be_empty_type_block = ty.fields.is_empty()
            && !extern_block::has_constructor_extern(
                ty,
                exclude_fields,
                configured_features,
                enum_kinds,
                parent_first_class,
            )
            && ty
                .methods
                .iter()
                .all(|m| m.binding_excluded || m.sanitized || m.is_static);
        if is_handle_returned && would_be_empty_type_block {
            deferred_empty_handle_types.insert(ty.name.clone());
            continue;
        }

        // Wrap each extern block in `#[cfg(feature = "...")]` when the type carries a
        // cfg condition.  swift-bridge 0.1.59 cannot parse `#[cfg(...)]` *inside* an
        // extern block, but `#[cfg(...)] extern "Rust" { ... }` at the module item
        // `#[cfg(feature = "...")]`-gated in the shims — causing "no type named 'T' in
        let cfg_open = ty
            .cfg
            .as_deref()
            .map(|c| format!("    #[cfg({c})]\n"))
            .unwrap_or_default();
        let raw_block = extern_block::emit_extern_block_for_type(
            ty,
            exclude_fields,
            &type_paths,
            &no_serde_names,
            &first_class_names,
            enum_kinds,
            &config.untagged_union_text_types.iter().map(String::as_str).collect(),
            &text_projection::serde_text_types(api, &config.untagged_union_text_types),
            configured_features,
        );
        extern_blocks.push(format!("{cfg_open}{raw_block}"));
        owned_vec_accessors.types.push((
            ty.name.clone(),
            owned_vec::owned_vec_getters(
                ty,
                &owned_vec::OwnedVecGetterFacts {
                    exclude_fields,
                    type_paths: &type_paths,
                    no_serde_names: &no_serde_names,
                    first_class_names: &first_class_names,
                    enum_kinds,
                    text_types: &config.untagged_union_text_types.iter().map(String::as_str).collect(),
                    configured_features,
                },
            ),
        ));
        if ty.is_opaque && !ty.methods.iter().all(|m| m.sanitized) && !ty.methods.is_empty() {
            let has_ctor_override = config
                .swift
                .as_ref()
                .is_some_and(|c| c.client_constructor_body.contains_key(&ty.name));
            if has_ctor_override && let Some(ctor_block) = extern_block::emit_extern_block_for_type_constructor(ty) {
                extern_blocks.push(format!("{cfg_open}{ctor_block}"));
            }
            if let Some(method_block) = extern_block::emit_extern_block_for_type_methods(
                ty,
                &handle_returned_types,
                &enum_names,
                &unit_enum_names,
            ) {
                extern_blocks.push(format!("{cfg_open}{method_block}"));
            }
            for method in ty.methods.iter().filter(|m| !m.sanitized && !m.is_static) {
                let bridge_fn = extern_block::type_method_bridge_fn(
                    ty,
                    method,
                    &handle_returned_types,
                    &enum_names,
                    &unit_enum_names,
                );
                owned_vec_accessors.add_function(&bridge_fn)?;
            }
        }
        if let Some(dto_method_block) =
            extern_block::emit_extern_block_for_first_class_dto_methods(ty, &handle_returned_types, &enum_names)
        {
            extern_blocks.push(format!("{cfg_open}{dto_method_block}"));
        }
    }
    for en in &visible_enums {
        // Wrap cfg-gated enums in `#[cfg(...)]` for the same reason as types above.
        if !result_type_enums.contains(&en.name) {
            let raw_block = extern_block::emit_extern_block_for_enum(en);
            if let Some(cfg) = en.cfg.as_deref() {
                extern_blocks.push(format!("    #[cfg({cfg})]\n{raw_block}"));
            } else {
                extern_blocks.push(raw_block);
            }
        }
    }
    let swift_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> = config
        .swift
        .as_ref()
        .map(|c| c.capsule_types.clone())
        .unwrap_or_default();

    for f in &visible_functions {
        let bridge_fn = extern_block::function_bridge_fn(
            f,
            &handle_returned_types,
            &enum_names_owned,
            &unit_enum_names,
            &swift_capsule_types,
            &opaque_type_names_ahash,
        )?;
        owned_vec_accessors.add_function(&bridge_fn)?;
    }

    if !visible_functions.is_empty() {
        // cfg-gated functions are emitted into per-cfg `#[cfg(...)] extern "Rust" { }`
        // proc macro runs when the feature is off.  The shim body is also `#[cfg(...)]`-
        let non_cfg_fns: Vec<FunctionDef> = visible_functions
            .iter()
            .filter(|f| f.cfg.is_none())
            .map(|f| (*f).clone())
            .collect();
        if !non_cfg_fns.is_empty() {
            extern_blocks.push(extern_block::emit_extern_block_for_functions(
                &non_cfg_fns,
                &handle_returned_types,
                &enum_names_owned,
                &unit_enum_names,
                &deferred_empty_handle_types,
                &swift_capsule_types,
                &opaque_type_names_ahash,
            )?);
        }
        // `#[cfg(...)] extern "Rust" { }` block per distinct condition.
        let mut cfg_groups: std::collections::BTreeMap<String, Vec<FunctionDef>> = std::collections::BTreeMap::new();
        for f in visible_functions.iter().filter(|f| f.cfg.is_some()) {
            cfg_groups.entry(f.cfg.clone().unwrap()).or_default().push((*f).clone());
        }
        for (cfg_cond, fns) in &cfg_groups {
            let empty_set = std::collections::BTreeSet::new();
            let block = extern_block::emit_extern_block_for_functions(
                fns,
                &handle_returned_types,
                &enum_names_owned,
                &unit_enum_names,
                &empty_set,
                &swift_capsule_types,
                &opaque_type_names_ahash,
            )?;
            extern_blocks.push(format!("    #[cfg({cfg_cond})]\n{block}"));
        }
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        for method in trait_def.methods.iter().filter(|m| !m.has_default_impl) {
            let bridge_fn = trait_bridge::trait_method_bridge_fn(trait_def, method, &visible_type_names);
            owned_vec_accessors.add_function(&bridge_fn)?;
        }
        extern_blocks.push(trait_bridge::emit_extern_block_for_trait_bridge(
            trait_def,
            &visible_type_names,
        ));
    }
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::FunctionParam {
            continue;
        }
        let reg_block = plugin_inbound::emit_extern_block_for_inbound_registration(trait_def, bridge_cfg);
        if !reg_block.is_empty() {
            extern_blocks.push(reg_block);
        }
    }
    for (bridge_cfg, trait_def) in &active_bridges {
        extern_blocks.push(plugin_inbound::emit_extern_block_for_inbound(trait_def, bridge_cfg));
    }
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        let (factory_extern, _factory_body) =
            plugin_inbound::emit_options_field_factory(trait_def, bridge_cfg, api, &source_crate);
        if !factory_extern.is_empty() {
            extern_blocks.push(factory_extern);
        }
        let (helper_extern, _helper_body) =
            plugin_inbound::emit_options_field_options_helper(bridge_cfg, api, &source_crate);
        if !helper_extern.is_empty() {
            extern_blocks.push(helper_extern);
        }
    }

    let declared_owner_types: std::collections::HashSet<String> = visible_types
        .iter()
        .filter(|ty| ty.is_opaque)
        .map(|ty| ty.name.clone())
        .collect();
    if let Some(streaming_block) =
        extern_block::emit_extern_block_for_streaming_adapters(&config.adapters, &declared_owner_types)
    {
        extern_blocks.push(streaming_block);
    }

    let service_api_blocks = super::gen_bindings::service_api::generate_rust_extern_blocks(api).unwrap_or_default();
    extern_blocks.extend(service_api_blocks);

    let service_api_callback_funcs: Vec<String> =
        super::gen_bindings::service_api::generate_rust_callback_c_functions(api).unwrap_or_default();

    let signature_serde_types: Vec<&TypeDef> =
        json_bridge::collect_signature_serde_types(&visible_types, &visible_functions, &[]);

    let signature_serde_names: std::collections::HashSet<&str> =
        signature_serde_types.iter().map(|t| t.name.as_str()).collect();
    let streaming_item_types: Vec<&TypeDef> = {
        let streaming_item_names: std::collections::HashSet<&str> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.item_type.as_deref())
            .collect();
        visible_types
            .iter()
            .copied()
            .filter(|ty| streaming_item_names.contains(ty.name.as_str()))
            .filter(|ty| ty.has_serde && !ty.is_opaque && !ty.is_trait)
            .filter(|ty| !signature_serde_names.contains(ty.name.as_str()))
            .collect()
    };

    let streaming_item_names: std::collections::HashSet<&str> =
        streaming_item_types.iter().map(|t| t.name.as_str()).collect();
    let json_fallback_types: Vec<&TypeDef> = visible_types
        .iter()
        .copied()
        .filter(|ty| ty.has_serde && !ty.is_opaque && !ty.is_trait)
        .filter(|ty| !signature_serde_names.contains(ty.name.as_str()))
        .filter(|ty| !streaming_item_names.contains(ty.name.as_str()))
        .collect();

    let json_fallback_enums: Vec<&EnumDef> = visible_enums.iter().copied().filter(|e| e.has_serde).collect();

    out.push_str("#[swift_bridge::bridge]\nmod ffi {\n");
    if let Some(component_externs) = components::extern_block(config) {
        out.push_str(&component_externs);
    }
    out.push_str(&bridge_error::extern_block());
    for block in &extern_blocks {
        out.push_str(block);
    }
    json_bridge::emit_type_from_json_extern_block(&mut out, &signature_serde_types);
    json_bridge::emit_type_from_json_extern_block(&mut out, &streaming_item_types);
    json_bridge::emit_type_from_json_extern_block(&mut out, &json_fallback_types);
    let json_fallback_enums_filtered: Vec<&EnumDef> = json_fallback_enums
        .iter()
        .filter(|en| !result_type_enums.contains(&en.name))
        .copied()
        .collect();
    json_bridge::emit_enum_from_json_extern_block(&mut out, &json_fallback_enums_filtered);

    // cfg-gated types are split into per-type `#[cfg(...)] extern "Rust" { }` blocks
    let vec_accessible_enums: Vec<&EnumDef> = visible_enums
        .iter()
        .filter(|en| !result_type_enums.contains(&en.name))
        .copied()
        .collect();
    // Skip types marked `#[swift_bridge(already_declared)]` (streaming-adapter
    let already_declared =
        crate::backends::swift::gen_bindings::opaque_handles::collect_already_declared_owner_types(config);
    let non_cfg_types: Vec<&TypeDef> = visible_types
        .iter()
        .copied()
        .filter(|t| t.cfg.is_none() && !already_declared.contains(&t.name))
        .collect();
    let non_cfg_enums: Vec<&EnumDef> = vec_accessible_enums
        .iter()
        .copied()
        .filter(|e| e.cfg.is_none())
        .collect();
    let vec_accessors_block = extern_block::emit_extern_block_for_vec_accessors(&non_cfg_types, &non_cfg_enums);
    if !vec_accessors_block.is_empty() {
        out.push_str(&vec_accessors_block);
    }
    // cfg-gated types each get their own `#[cfg(...)] extern "Rust" { }` block.
    for ty in visible_types
        .iter()
        .filter(|t| t.cfg.is_some() && !already_declared.contains(&t.name))
    {
        let cfg = ty.cfg.as_deref().unwrap();
        let block = extern_block::emit_extern_block_for_vec_accessors(&[ty], &[]);
        if !block.is_empty() {
            out.push_str(&format!("    #[cfg({cfg})]\n{block}"));
        }
    }
    for en in vec_accessible_enums.iter().filter(|e| e.cfg.is_some()) {
        let cfg = en.cfg.as_deref().unwrap();
        let block = extern_block::emit_extern_block_for_vec_accessors(&[], &[en]);
        if !block.is_empty() {
            out.push_str(&format!("    #[cfg({cfg})]\n{block}"));
        }
    }

    out.push_str(&super::owned_vector_inputs::externs(&owned_vec_accessors.inputs));
    out.push_str("}\n\n");
    out.push_str(&super::owned_vector_inputs::implementations(
        &owned_vec_accessors.inputs,
    ));
    out.push_str(&bridge_error::definition(api, &source_crate));
    out.push('\n');
    if let Some(component_implementation) = components::implementation(config) {
        out.push_str(&component_implementation);
        out.push_str("\n\n");
    }

    let vec_impl_types: Vec<&TypeDef> = visible_types
        .iter()
        .copied()
        .filter(|t| !already_declared.contains(&t.name))
        .collect();
    let phantom_impl = extern_block::emit_phantom_vec_impl(&vec_impl_types, &vec_accessible_enums);
    if !phantom_impl.is_empty() {
        out.push_str(&phantom_impl);
    }

    out.push_str(&service_app_wrappers::emit_service_app_wrappers(api, &source_crate));
    out.push('\n');

    // `#[unsafe(no_mangle)]` attribute. swift-bridge only parses `src/lib.rs`,
    if !service_api_callback_funcs.is_empty() {
        out.push_str("mod extern_callbacks;\n\n");
    }

    let text_types: HashSet<&str> = config.untagged_union_text_types.iter().map(String::as_str).collect();
    for ty in &visible_types {
        out.push_str(&wrappers::emit_type_wrapper(
            ty,
            &source_crate,
            &type_paths,
            &enum_names,
            &unit_enum_names,
            &reconstructible_enum_names,
            &no_serde_names,
            &first_class_names,
            exclude_fields,
            configured_features,
            &text_types,
            &text_projection::serde_text_types(api, &config.untagged_union_text_types),
        ));
        out.push('\n');
        if ty.is_opaque && !ty.methods.iter().all(|m| m.sanitized) && !ty.methods.is_empty() {
            let custom_body = config
                .swift
                .as_ref()
                .and_then(|c| c.client_constructor_body.get(&ty.name))
                .map(String::as_str);
            if custom_body.is_some() {
                out.push_str(&wrappers::emit_type_constructor_shim(
                    ty,
                    &source_crate,
                    &type_paths,
                    custom_body,
                ));
                out.push('\n');
            }
            out.push_str(&wrappers::emit_type_method_shims(
                ty,
                &source_crate,
                &type_paths,
                &handle_returned_types,
                &enum_names,
                &unit_enum_names,
            ));
            out.push('\n');
        }

        let dto_wrappers =
            wrappers::emit_first_class_dto_method_wrappers(ty, &source_crate, &type_paths, &unit_enum_names);
        if !dto_wrappers.is_empty() {
            out.push_str(&dto_wrappers);
        }
    }

    let mut noop_def_types = deferred_empty_handle_types.clone();
    for ty in &visible_types {
        if extern_block::type_needs_own_block_noop(ty) {
            noop_def_types.insert(ty.name.clone());
        }
    }
    out.push_str(&deferred_noop::emit_shims(&noop_def_types, &visible_types));

    for en in &visible_enums {
        out.push_str(&enums::emit_enum_wrapper_with_defaults(
            en,
            &source_crate,
            &type_paths,
            Some(configured_enum_features.as_slice()),
            &defaultable_names,
        ));
        out.push('\n');
    }
    let function_shim_context = shims::FunctionShimContext {
        source_crate: &source_crate,
        type_paths: &type_paths,
        unit_enum_names: &unit_enum_names,
        tagged_enum_names: &tagged_enum_names,
        no_serde_names: &no_serde_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &swift_capsule_types,
        opaque_types: &opaque_type_names_ahash,
    };
    for f in &visible_functions {
        out.push_str(&shims::emit_function_shim(f, &function_shim_context)?);
        out.push('\n');
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        // `enum_names` (ALL enums), not `unit_enum_names`: the trampoline uses this set purely to
        // pick `{t}::from(..)` over the tuple-struct call `{t}(..)` for a `Named` return, and only
        // a struct wrapper is a tuple struct. Handing it `unit_enum_names` made a data-carrying
        // enum return emit a tuple-struct call against an enum type -- `E0423`. ~keep
        out.push_str(&trait_bridge::emit_trait_bridge_wrapper(
            trait_def,
            &source_crate,
            &enum_names,
            &visible_type_names,
            &type_paths,
        ));
        out.push('\n');
    }

    if !active_bridges.is_empty() {
        out.push_str(&plugin_inbound::emit_plugin_error_helper(
            &source_crate,
            &config.error_type_name(),
            &config.error_constructor_expr(),
        ));
    }
    for (bridge_cfg, trait_def) in &active_bridges {
        out.push_str(&plugin_inbound::emit_inbound_wrapper(
            trait_def,
            bridge_cfg,
            api,
            &source_crate,
            &type_paths,
            &config.error_type_name(),
            &config.error_constructor_expr(),
        ));
        out.push('\n');
    }
    let mut options_field_from_emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        let from_impls = plugin_inbound::emit_options_field_from_impls(
            bridge_cfg,
            api,
            &source_crate,
            &mut options_field_from_emitted,
        );
        if !from_impls.is_empty() {
            out.push_str(&from_impls);
            out.push('\n');
        }
        let (_factory_extern, factory_body) =
            plugin_inbound::emit_options_field_factory(trait_def, bridge_cfg, api, &source_crate);
        if !factory_body.is_empty() {
            out.push_str(&factory_body);
            out.push('\n');
        }
        let (_helper_extern, helper_body) =
            plugin_inbound::emit_options_field_options_helper(bridge_cfg, api, &source_crate);
        if !helper_body.is_empty() {
            out.push_str(&helper_body);
            out.push('\n');
        }
    }

    let streaming_shims = wrappers::emit_streaming_adapter_shims(&config.adapters, &source_crate)?;
    if !streaming_shims.is_empty() {
        out.push_str(&streaming_shims);
    }

    for ty in &signature_serde_types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path_base =
            crate::codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        let source_path = if ty.has_lifetime_params {
            format!("{source_path_base}<'static>")
        } else {
            source_path_base
        };
        json_bridge::emit_from_json_shim(&mut out, &type_snake, type_name, &source_path, type_name);
    }

    for ty in &streaming_item_types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path_base =
            crate::codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        let source_path = if ty.has_lifetime_params {
            format!("{source_path_base}<'static>")
        } else {
            source_path_base
        };
        json_bridge::emit_from_json_shim(&mut out, &type_snake, type_name, &source_path, type_name);
    }

    for ty in &json_fallback_types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path_base =
            crate::codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        let source_path = if ty.has_lifetime_params {
            format!("{source_path_base}<'static>")
        } else {
            source_path_base
        };
        json_bridge::emit_from_json_shim(&mut out, &type_snake, type_name, &source_path, type_name);
    }

    for en in &json_fallback_enums_filtered {
        let enum_snake = AsSnakeCase(en.name.as_str()).to_string();
        let enum_name = &en.name;
        let source_path =
            crate::codegen::generators::type_paths::resolve_type_path(enum_name, &source_crate, &type_paths);
        let map_expr = format!("{enum_name}::from");
        json_bridge::emit_from_json_shim(&mut out, &enum_snake, enum_name, &source_path, &map_expr);
    }

    Ok((out, owned_vec_accessors))
}

#[cfg(test)]
mod structured_text_tests;
