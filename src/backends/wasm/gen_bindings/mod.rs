//! WASM (wasm-bindgen) backend: orchestration and `Backend` trait implementation.

pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod service_api;
pub mod types;

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use crate::core::template_versions as tv;
use ahash::AHashSet;
use std::path::PathBuf;

use enums::gen_enum;
use errors::{gen_error_converter, gen_error_methods};
use functions::{gen_env_shims, gen_function_with_emitted_dtos};
use types::{gen_opaque_struct, gen_opaque_struct_methods, gen_struct, gen_struct_methods};

pub struct WasmBackend;

/// Check if a TypeRef references a Named type that is in the exclude set.
/// Used to skip fields whose types were excluded from WASM generation,
/// preventing references to non-existent Js* wrapper types.
pub(super) fn field_references_excluded_type(ty: &TypeRef, exclude_types: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => exclude_types.iter().any(|e| e == name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_excluded_type(inner, exclude_types),
        TypeRef::Map(k, v) => {
            field_references_excluded_type(k, exclude_types) || field_references_excluded_type(v, exclude_types)
        }
        _ => false,
    }
}

/// Check if an item is gated behind a disabled feature.
///
/// Evaluates cfg condition strings against the enabled feature list.
/// Returns `true` when the cfg condition is *not* satisfied (i.e. the item
/// must be excluded from generation).  Handles:
/// - `feature = "name"`
/// - `any(feature = "a", feature = "b", ...)`
/// - `all(feature = "a", feature = "b", ...)`
/// - `not(<inner>)`
///
/// The IR encodes cfgs via `proc_macro2::TokenStream::to_string()`, which
/// inserts whitespace between tokens (e.g. `any (feature = "a" , ...)`); the
/// evaluator normalises that before parsing.
fn is_gated_behind_disabled_feature(cfg: &Option<String>, enabled_features: &[String]) -> bool {
    let Some(cfg_str) = cfg else {
        return false;
    };
    !cfg_condition_enabled(cfg_str, enabled_features)
}

fn cfg_condition_enabled(cfg_str: &str, enabled_features: &[String]) -> bool {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        return enabled_features.iter().any(|ef| ef == feature);
    }
    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .any(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .all(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return !cfg_condition_enabled(inner.trim(), enabled_features);
    }
    // Unknown pattern → treat as enabled (no exclusion). Preserves prior behaviour
    // for cfgs the WASM backend has never inspected (target_arch, target_os, ...).
    true
}

/// Extract every `feature = "X"` referenced by a cfg expression.
///
/// Recursively descends through `any(...)`, `all(...)`, and `not(...)` so that
/// the wasm Cargo.toml emitter can declare a passthrough Cargo feature for
/// every feature the generated source references. Without this, items emitted
/// behind `#[cfg(feature = "X")]` produce
/// `error: unexpected cfg condition value: X` when the binding crate's
/// `Cargo.toml` only declares an unrelated feature list (e.g. `wasm-target`).
///
/// Unknown cfg patterns (`target_arch`, `target_os`, ...) yield no features
/// — those are recognised by Cargo directly and don't need passthroughs.
fn collect_cfg_feature_names(cfg_str: &str, out: &mut std::collections::BTreeSet<String>) {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        out.insert(feature.to_string());
        return;
    }
    if let Some(inner) = cfg_str
        .strip_prefix("any(")
        .and_then(|s| s.strip_suffix(')'))
        .or_else(|| cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')))
    {
        for cond in parse_cfg_list(inner) {
            collect_cfg_feature_names(&cond, out);
        }
        return;
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        collect_cfg_feature_names(inner.trim(), out);
    }
}

/// Walk the full [`ApiSurface`] and return the set of feature names referenced
/// by any cfg attribute on a type, field, enum, or top-level function.
///
/// The set is sorted (via `BTreeSet`) so the resulting Cargo.toml is stable
/// across regenerations.
fn collect_cfg_features(api: &ApiSurface) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for typ in &api.types {
        if let Some(cfg) = &typ.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        for field in &typ.fields {
            if let Some(cfg) = &field.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
        }
    }
    for enum_def in &api.enums {
        if let Some(cfg) = &enum_def.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
    }
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
    }
    out
}

fn parse_cfg_list(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

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
        let wasm_config = config.wasm.as_ref();
        let mut exclude_functions = wasm_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
        let mut exclude_types = wasm_config.map(|c| c.exclude_types.clone()).unwrap_or_default();
        // Declared opaque types from `[workspace.opaque_types]` are external host-runtime
        // references. When their `rust_path` carries generic parameters (e.g. `Arc<Mutex<dyn T>>`),
        // the injected IR cannot model them, so wasm-bindgen can't wrap them — exclude those.
        // Simple newtype opaques (no generics in the path) DO wrap as `#[wasm_bindgen]` classes
        // fine; leave them in so consumers with simple external handle types get a
        // `WasmLanguage` wrapper struct emitted.
        exclude_types.extend(
            config
                .opaque_types
                .iter()
                .filter(|(_, path)| path.contains('<'))
                .map(|(name, _)| name.clone()),
        );
        let type_overrides = wasm_config.map(|c| c.type_overrides.clone()).unwrap_or_default();
        let env_shims = wasm_config.map(|c| c.env_shims.clone()).unwrap_or_default();
        let prefix = config.wasm_type_prefix();

        // Auto-exclude types/functions/enums gated behind disabled features
        let enabled_features = config.features_for_language(Language::Wasm).to_vec();
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

        let mapper = WasmMapper::new(type_overrides, prefix.clone());
        let core_import = config.core_import_for_language(Language::Wasm);

        // Build source-crate remaps from config: each `source_crate_remaps` entry
        // becomes `(original_crate_name_with_underscores, core_import)`. References
        // to `<original>::T` in IR rust_paths are rewritten to `<core_import>::T`.
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
        // Crates whose types should be auto-skipped: source crates that aren't
        // available as deps (excluded via `exclude_extra_dependencies`) and aren't
        // remapped to the override. Generated code referencing these would fail
        // to resolve because the crate isn't in the binding's Cargo deps.
        let dropped_crates: AHashSet<String> = wasm_config
            .map(|c| c.exclude_extra_dependencies.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|name| name.replace('-', "_"))
            .filter(|underscored| {
                // Keep if it's the core_import (already a dep via override) or remapped
                underscored != &core_import && !source_remap_pairs.iter().any(|(orig, _)| orig == underscored)
            })
            .collect();
        // Auto-exclude types whose source crate is in `dropped_crates`. The IR's
        // `rust_path` starts with the source crate name; if that name is in the
        // dropped set, alef cannot generate working bindings for the type
        // (the From impl would reference a crate not in the dep tree).
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
        // Errors mirror types — skip those whose source crate isn't in the dep tree.
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

        // Apply per-type field exclusions: any field listed in `[wasm].exclude_fields`
        // is treated as if it were `#[cfg]`-gated, so the binding struct omits it and
        // the From impl uses `..Default::default()` to fill it.
        let exclude_fields_map = wasm_config.map(|c| c.exclude_fields.clone()).unwrap_or_default();
        // Build a fresh ApiSurface clone with exclude_fields applied: each field in
        // the exclude list gets `cfg: Some("excluded_via_alef_toml")` so all
        // downstream cfg-skip code paths (binding struct gen, From impls, builder
        // patterns) treat it as native-only and emit ..Default::default() on wasm.
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

        // Note: custom modules and registrations handled below after builder creation

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables, unreachable_patterns, missing_docs)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::useless_conversion, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy, clippy::should_implement_trait, clippy::await_holding_refcell_ref, clippy::new_without_default)");
        builder.add_import("wasm_bindgen::prelude::*");

        // Hand-written Rust modules: declare `pub mod <name>;` at the top of lib.rs
        // and re-export with `pub use <name>::*;`. Source files for these modules
        // must exist in `crates/<name>-wasm/src/<name>.rs` and are not managed by alef.
        if let Some(modules) = wasm_config.map(|c| c.custom_rust_modules.as_slice()) {
            for module in modules {
                builder.add_item(&format!("pub mod {module};"));
                builder.add_item(&format!("pub use {module}::*;"));
            }
        }

        // js_sys items are always referenced with full paths (js_sys::Object, js_sys::Reflect, etc.)
        // so no explicit `use js_sys;` import is needed (clippy::single_component_path_imports).

        // Import traits needed for trait method dispatch
        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        // Note: HashMap is intentionally not imported here.
        // The WasmMapper always converts Map types to JsValue (wasm-bindgen cannot
        // pass HashMap<K, V> across the JS boundary), so HashMap is never referenced
        // in the generated WASM binding code.

        // Emit environment shims for C external scanner interop (e.g. iswspace, iswalnum).
        // Only shims whose names appear in env_shims are emitted.
        if !env_shims.is_empty() {
            builder.add_item(&gen_env_shims(&env_shims));
        }

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !exclude_types.contains(&t.name))
            .map(|t| t.name.clone())
            .collect();
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

        // Trait bridge type aliases (e.g. `VisitorHandle`) are opaque — they map to
        // `Arc<core::VisitorHandle>` in the binding layer and must not attempt From/Into
        // conversion in generated From impls.  Include them so struct fields referencing
        // these types use Default::default() instead of val.visitor.map(Into::into).
        let bridge_type_aliases: AHashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let mut opaque_names_vec: Vec<String> = opaque_types.iter().cloned().collect();
        opaque_names_vec.extend(bridge_type_aliases.iter().cloned());
        let opaque_names_set: AHashSet<String> = opaque_names_vec.iter().cloned().collect();

        // Build adapter body map before type iteration so bodies are available for method generation.
        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Wasm)?;

        // Map "OwnerType.method" -> streaming item type. The wasm backend needs to
        // override the IR-declared return type with the iterator struct name
        // for streaming adapters, since the generated body returns an iterator instance.
        // Exclude adapters that have "wasm" in skip_languages — those methods fall back
        // to the unimplemented-body path and must not receive an iterator return type.
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

        // Build the set of "OwnerType.method" keys for streaming adapters that are
        // explicitly skipped for WASM via skip_languages. The WASM backend must omit
        // these methods entirely — not emit unimplemented stubs — because their
        // parameter types (e.g. WasmCrawlStreamRequest) are only generated by the
        // streaming adapter body path and will be missing when the adapter is skipped.
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

        // Emit adapter-generated standalone items (streaming iterators, callback bridges).
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

        // Service-API glue lives in the generated `service.rs`; declare it so its
        // `#[wasm_bindgen]` entrypoints (e.g. `app_run`) are compiled and exported.
        // Only emit the module declaration when at least one service is NOT skipped
        // for the wasm backend via its `skip_languages` config entry.
        let has_wasm_services = api.services.iter().any(|svc| {
            !config
                .services
                .iter()
                .any(|sc| sc.owner_type == svc.name && sc.skip_languages.iter().any(|l| l == "wasm"))
        });
        if has_wasm_services {
            builder.add_item("pub mod service;");
        }

        // Collect tagged-data enum names once for use in struct generation and conversions.
        // These are enums with a serde tag *and* at least one data variant — they are emitted
        // as wasm-bindgen structs with `JsValue` storage for fields that are Vec<TaggedEnum>.
        let tagged_data_enum_names: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| !exclude_types.contains(&e.name) && enums::is_tagged_data_enum(e))
            .map(|e| e.name.clone())
            .collect();

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
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // so no separate Default impl is needed.
                builder.add_item(&gen_struct(
                    typ,
                    &mapper,
                    &exclude_types,
                    &prefix,
                    &tagged_data_enum_names,
                ));
                builder.add_item(&gen_struct_methods(
                    typ,
                    &mapper,
                    &exclude_types,
                    &core_import,
                    &opaque_types,
                    &api.enums,
                    &prefix,
                    &mutex_types,
                    &streaming_item_types,
                ));
            }
        }

        for enum_def in &api.enums {
            if !exclude_types.contains(&enum_def.name) {
                builder.add_item(&gen_enum(enum_def, &prefix));
            }
        }

        // Pre-compute all input DTOs needed across all functions to avoid duplicate emissions.
        // Collect all config-like types used as parameters across functions, generate each once,
        // then mark them as already-emitted so gen_function_with_emitted_dtos() skips them.
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
                        if let TypeRef::Named(name) = &p.ty {
                            if !opaque_types.contains(name.as_str())
                                && !emitted_input_dtos.contains(name)
                                && let Some(type_def) = api.types.iter().find(|t| t.name == *name)
                                && functions::should_have_input_dto(type_def)
                            {
                                let non_deserializable_type_names: std::collections::HashSet<String> = api
                                    .types
                                    .iter()
                                    .filter(|t| !t.has_serde || t.is_trait || t.is_opaque)
                                    .map(|t| t.name.clone())
                                    .collect();
                                let (dto_code, _dto_name) = functions::gen_input_dto_for_type_with_cfg(
                                    name,
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
        }
        if !input_dto_code.is_empty() {
            builder.add_item(&input_dto_code);
        }

        for func in &api.functions {
            if !exclude_functions.contains(&func.name) {
                // clear_fn functions are emitted inside the bridge module and glob-re-exported;
                // emitting them again here would produce duplicate wasm-bindgen symbol names.
                if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(
                    &func.name,
                    &config.trait_bridges,
                ) {
                    continue;
                }
                // Skip functions whose signature references excluded types
                let refs_excluded = func
                    .params
                    .iter()
                    .any(|p| field_references_excluded_type(&p.ty, &exclude_types))
                    || field_references_excluded_type(&func.return_type, &exclude_types);
                if refs_excluded {
                    continue;
                }
                let bridge_param = crate::backends::wasm::trait_bridge::find_bridge_param(func, &config.trait_bridges);
                let options_field_bridge = crate::backends::wasm::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
                    // Only use the options-field path when the bridge field actually survives
                    // into the binding struct.
                    .filter(|(_, bridge_cfg)| {
                        let Some(field_name) = bridge_cfg.resolved_options_field() else { return false; };
                        let Some(options_type) = bridge_cfg.options_type.as_deref() else { return false; };
                        api.types
                            .iter()
                            .filter(|t| t.name == options_type)
                            .flat_map(|t| t.fields.iter())
                            .any(|f| f.cfg.is_none() && f.name == field_name)
                    });
                if let Some((param_idx, bridge_cfg)) = bridge_param {
                    builder.add_item(&crate::backends::wasm::trait_bridge::gen_bridge_function(
                        api,
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &prefix,
                    ));
                } else if let Some((param_idx, bridge_cfg)) = options_field_bridge {
                    builder.add_item(&crate::backends::wasm::trait_bridge::gen_options_field_bridge_function(
                        api,
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &prefix,
                    ));
                } else {
                    builder.add_item(&gen_function_with_emitted_dtos(
                        func,
                        &mapper,
                        &core_import,
                        &opaque_types,
                        &prefix,
                        &mutex_types,
                        api,
                        &emitted_input_dtos,
                    ));
                }
            }
        }

        // Trait bridge wrappers — generate WASM bridge structs that delegate to JS objects
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
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

        // Trait-bridge fields whose binding-side wrapper holds `inner: Arc<core::T>`
        // (every OptionsField-style bridge in alef follows this convention). Used by
        // `binding_to_core` to emit `val.{f}.map(|v| (*v.inner).clone())` instead of
        // `Default::default()` so the JS visitor handle survives `.into()`.
        let trait_bridge_arc_wrapper_field_names: Vec<String> = config
            .trait_bridges
            .iter()
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
            // Treat bridge type aliases (e.g. VisitorHandle) as opaque so binding→core
            // From impls emit Default::default() instead of val.visitor.map(Into::into),
            // unless the field is in `trait_bridge_arc_wrapper_field_names` (then it gets
            // proper Arc-dereferencing forwarding).
            opaque_types: if opaque_names_set.is_empty() {
                None
            } else {
                Some(&opaque_names_set)
            },
            trait_bridge_arc_wrapper_field_names: &trait_bridge_arc_wrapper_field_names,
            // Vec<TaggedDataEnum> fields are stored as JsValue; conversions use serde_wasm_bindgen.
            tagged_data_enum_names: if tagged_data_enum_names.is_empty() {
                None
            } else {
                Some(&tagged_data_enum_names)
            },
            ..Default::default()
        };
        let convertible = crate::codegen::conversions::convertible_types(api);
        let core_to_binding_convertible = crate::codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = crate::codegen::conversions::input_type_names(api);
        // From/Into conversions using shared parameterized generators
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            let is_strict = crate::codegen::conversions::can_generate_conversion(typ, &convertible);
            let is_relaxed = crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding_convertible);
            if is_strict {
                // Both directions
                if input_types.contains(&typ.name) {
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
                // Only core→binding (sanitized fields prevent binding→core)
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
                if enums::is_tagged_data_enum(e) {
                    // Tagged data enums emit as a flat wasm-bindgen struct (see
                    // `gen_tagged_enum_as_struct`); their From/Into impls match on the
                    // string discriminator field instead of the destination enum variants
                    // produced by the generic enum-conversion templates.
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

        // Error converter functions (skip excluded errors).
        // When an error declares whitelisted introspection methods, also emit an opaque
        // WASM struct + impl block so JS/TS callers can inspect the error object.
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

        let mut content = builder.build();

        // Post-process generated code to fix bridge type builder methods.
        // Builder methods on has_default types with opaque bridge parameters
        // (e.g., visitor: WasmVisitorHandle) should not attempt to access .inner,
        // as there is no From impl from Arc<VisitorHandle> to the core visitor type.
        // Replace patterns like .visitor(visitor.as_ref().map(|v| &v.inner))
        // with .visitor(None) to skip setting the visitor on the core builder.
        for bridge in &config.trait_bridges {
            if let Some(field_name) = bridge.resolved_options_field() {
                let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
                let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
                let replacement = format!(".{}(None)", field_name);
                content = content.replace(&pattern, &replacement);
            }
        }

        // Fix From<Wasm*> impls to pass through the bridge field instead of Default::default().
        // Post-process every From<Wasm{options_type}{variant}> impl whose bridge is bound via
        // options_field, replacing `{field_name}: Default::default()` with the proper pass-through.
        for bridge in &config.trait_bridges {
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
                // Try multiple indentation patterns (tabs or spaces)
                // Most commonly 12 spaces (3 indent levels) in generated code
                let patterns = &[
                    ("            ", "\n            "), // 12 spaces
                    ("        ", "\n        "),         // 8 spaces
                    ("  ", "\n  "),                     // 2 spaces
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

        let output_dir = resolve_output_dir(config.output_paths.get("wasm"), &config.name, "crates/{name}-wasm/src/");

        let cargo_toml_path = PathBuf::from(&output_dir)
            .parent()
            .map(|p| p.join("Cargo.toml"))
            .unwrap_or_else(|| PathBuf::from("Cargo.toml"));

        Ok(vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join("lib.rs"),
                content,
                generated_header: false,
            },
            GeneratedFile {
                path: cargo_toml_path,
                content: gen_cargo_toml(api, config),
                generated_header: true,
            },
        ])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(service_api::gen_service_files(api, config))
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "wasm-pack",
            crate_suffix: "-wasm",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

/// Generate the `Cargo.toml` for the WASM binding crate.
///
/// This is emitted by [`WasmBackend::generate_bindings`] so that the file is
/// always regenerated on `alef generate` / `alef all` alongside `lib.rs`.
/// Emitting it here (rather than only in `alef-scaffold`) ensures that the
/// `js-sys` dependency required by trait-bridge and visitor-bridge generated
/// code is always present, even in projects whose `Cargo.toml` was created
/// before `js-sys` was added to the scaffold template.
fn gen_cargo_toml(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_crate_dir = config.core_crate_for_language(Language::Wasm);
    let crate_name = &config.name;
    // Package-name prefix for `<prefix>-wasm`. Preserves prior behaviour
    // (derived from sources) when no override is set; switches to the
    // umbrella crate name when an override redirects the core dep elsewhere
    // so the binding crate keeps its original published name.
    let pkg_prefix: String = if config
        .wasm
        .as_ref()
        .and_then(|c| c.core_crate_override.as_deref())
        .is_some()
    {
        crate_name.clone()
    } else {
        core_crate_dir.clone()
    };
    // Cargo dep KEY for the core dependency: the override when set, otherwise
    // the umbrella crate name. Must match `core_crate_dir` so
    // `path = "../{core_crate_dir}"` resolves to a crate whose Cargo.toml
    // `name` equals the dep key.
    let core_dep_key: String = config
        .wasm
        .as_ref()
        .and_then(|c| c.core_crate_override.clone())
        .unwrap_or_else(|| crate_name.clone());
    let version = &api.version;

    let scaffold = config.scaffold.as_ref();
    let license = scaffold.and_then(|s| s.license.as_deref()).unwrap_or("MIT");
    let description = scaffold
        .and_then(|s| s.description.as_deref())
        .unwrap_or(crate_name.as_str());
    let repository = scaffold.and_then(|s| s.repository.as_deref()).unwrap_or("");

    let keywords = scaffold.map(|s| s.keywords.as_slice()).unwrap_or(&[]);
    let keywords_toml = if keywords.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = keywords.iter().map(|k| format!("\"{k}\"")).collect();
        format!("keywords = [{}]\n", quoted.join(", "))
    };

    let features = config.features_for_language(Language::Wasm);
    let features_clause = if features.is_empty() {
        String::new()
    } else {
        // When the consumer pinned an explicit feature set for wasm, also
        // disable default features so "download" or similar host-only
        // defaults don't sneak in (mio/getrandom can't compile to
        // wasm32-unknown-unknown).
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", default-features = false, features = [{}]", quoted.join(", "))
    };

    let extra_deps = config.extra_deps_for_language(Language::Wasm);
    let mut extra_dep_lines: Vec<String> = extra_deps
        .iter()
        .map(|(name, value)| {
            if let Some(s) = value.as_str() {
                format!("{name} = \"{s}\"")
            } else {
                format!("{name} = {value}")
            }
        })
        .collect();
    extra_dep_lines.sort();
    let extra_deps_section = if extra_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_dep_lines.join("\n"))
    };

    // Collect every feature name referenced by a cfg attribute on a generated
    // item. Each becomes a passthrough Cargo feature on the binding crate so
    // rustc does not warn `unexpected cfg condition value` under `-D warnings`.
    //
    // Features are declared but NOT enabled by default. Items behind
    // `#[cfg(feature = X)]` on the binding crate intentionally evaluate false
    // when the feature isn't enabled — they're forwarded to the core dep via
    // the `dep features = [..]` clause, which makes the core types reachable,
    // but the binding's own mirror items (DTOs, From impls) remain hidden so
    // serde-Deserialize on trait-object handles like `VisitorHandle` does not
    // surface in the binding's deserialization surface.
    let _ = features;
    let cfg_features = collect_cfg_features(api);
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = cfg_features
            .iter()
            .map(|name| format!(r#"{name} = ["{core_dep_key}/{name}"]"#))
            .collect();
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    let header = hash::header(CommentStyle::Hash);

    // Layout follows cargo-sort canonical order: [package] -> [package.metadata.*]
    // -> [lib] -> [dependencies] (alphabetical). Otherwise cargo-sort rewrites the
    // file post-generate and breaks the alef hash header.
    let mut deps: Vec<(String, String)> = vec![
        (
            core_dep_key.clone(),
            format!(r#"{{ path = "../{core_crate_dir}"{features_clause} }}"#),
        ),
        ("futures".to_string(), format!(r#""{}""#, tv::cargo::FUTURES)),
        ("futures-util".to_string(), format!(r#""{}""#, tv::cargo::FUTURES_UTIL)),
        ("js-sys".to_string(), format!(r#""{}""#, tv::cargo::JS_SYS)),
        (
            "serde".to_string(),
            r#"{ version = "1", features = ["derive"] }"#.to_string(),
        ),
        (
            "serde-wasm-bindgen".to_string(),
            format!(r#""{}""#, tv::cargo::SERDE_WASM_BINDGEN),
        ),
        ("serde_json".to_string(), r#""1""#.to_string()),
        ("wasm-bindgen".to_string(), format!(r#""{}""#, tv::cargo::WASM_BINDGEN)),
        (
            "wasm-bindgen-futures".to_string(),
            format!(r#""{}""#, tv::cargo::WASM_BINDGEN_FUTURES),
        ),
    ];
    // Parse extra deps into (name, value) pairs.
    let mut extra_parsed: Vec<(String, String)> = Vec::new();
    for line in extra_deps_section.lines() {
        let trimmed = line.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            extra_parsed.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    // A dependency listed in `[crates.<lang>.extra_dependencies]` overrides the
    // built-in of the same name rather than emitting a second key. Without this,
    // re-declaring a built-in (e.g. `serde`) produces a duplicate key and cargo
    // rejects the manifest with "duplicate key in dependencies".
    let extra_names: AHashSet<&str> = extra_parsed.iter().map(|(name, _)| name.as_str()).collect();
    deps.retain(|(name, _)| !extra_names.contains(name.as_str()));
    deps.extend(extra_parsed);
    deps.sort_by(|a, b| a.0.cmp(&b.0));
    let deps_block = deps
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"{header}
[package]
name = "{pkg_prefix}-wasm"
version = "{version}"
edition = "2024"
license = "{license}"
description = "{description}"
repository = "{repository}"
{keywords_toml}
[package.metadata.cargo-machete]
ignored = [
    "futures",
    "futures-util",
    "js-sys",
    "wasm-bindgen-futures",
    "serde",
    "serde_json",
]

[package.metadata.wasm-pack.profile.release]
wasm-opt = false

[lib]
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_block}

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = {{ version = "0.4", features = ["wasm_js"] }}
getrandom_02 = {{ package = "getrandom", version = "0.2", features = ["js"] }}
getrandom_03 = {{ package = "getrandom", version = "0.3", features = ["wasm_js"] }}
"#,
        header = header,
        pkg_prefix = pkg_prefix,
        version = version,
        license = license,
        description = description,
        repository = repository,
        keywords_toml = keywords_toml,
        deps_block = deps_block,
        features_table = features_table,
    )
}
#[cfg(test)]
mod tests {
    use super::WasmBackend;
    use crate::core::backend::Backend;
    use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
    use crate::core::ir::ApiSurface;

    fn make_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn wasm_backend_name_is_wasm() {
        assert_eq!(WasmBackend.name(), "wasm");
    }

    #[test]
    fn generate_bindings_empty_api_produces_files() {
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = make_config();
        let files = WasmBackend.generate_bindings(&api, &config).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].path.to_string_lossy().ends_with("lib.rs"));
        assert!(files[1].path.to_string_lossy().ends_with("Cargo.toml"));
    }

    #[test]
    fn extra_dependency_overrides_builtin_without_duplicate_key() {
        // `serde` is a built-in wasm dependency. Re-declaring it via
        // `[crates.wasm.extra_dependencies]` must override the built-in rather
        // than emit a second `serde` key (which makes cargo reject the manifest).
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dependencies]
serde = { version = "1", features = ["derive", "rc"] }
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let cargo_toml = super::gen_cargo_toml(&api, &config);

        let serde_lines = cargo_toml
            .lines()
            .filter(|l| l.trim_start().starts_with("serde =") || l.trim_start().starts_with("serde="))
            .count();
        assert_eq!(serde_lines, 1, "expected exactly one `serde` key, got:\n{cargo_toml}");
        // The override (with the extra `rc` feature) wins over the built-in.
        assert!(
            cargo_toml.contains(r#"features = ["derive", "rc"]"#),
            "extra_dependencies override should win:\n{cargo_toml}"
        );
        // The manifest must parse as valid TOML (duplicate keys would fail here).
        toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    }

    #[test]
    fn collect_cfg_feature_names_extracts_every_feature_reference() {
        use std::collections::BTreeSet;
        let mut out = BTreeSet::new();
        super::collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
        super::collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
        super::collect_cfg_feature_names(
            r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
            &mut out,
        );
        // Unknown / non-feature cfg expressions yield nothing.
        super::collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
        let want: BTreeSet<String> = ["html", "layout-types", "pdf", "wasm-target", "xml"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(out, want);
    }

    #[test]
    fn cargo_toml_emits_passthrough_features_for_type_cfg_attrs() {
        // Without passthrough features in the binding Cargo.toml, cargo errors
        // with `unexpected cfg condition value: pdf` (etc.) for every cfg
        // attribute the wasm backend emits on generated items.
        use crate::core::ir::TypeDef;

        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "PdfThing".to_string(),
                rust_path: "test_lib::PdfThing".to_string(),
                cfg: Some(r#"feature = "pdf""#.to_string()),
                ..Default::default()
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
        let config = make_config();
        let cargo_toml = super::gen_cargo_toml(&api, &config);

        assert!(
            cargo_toml.contains(r#"pdf = ["test-lib/pdf"]"#),
            "expected `pdf = [\"test-lib/pdf\"]` in:\n{cargo_toml}"
        );
        assert_eq!(
            cargo_toml.matches("\n[features]\n").count(),
            1,
            "exactly one [features] block expected:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    }

    #[test]
    fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = make_config();
        let cargo_toml = super::gen_cargo_toml(&api, &config);
        assert!(
            !cargo_toml.contains("[features]"),
            "expected no [features] block:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    }

    #[test]
    fn cargo_toml_declares_explicit_features_as_passthrough_without_enabling_default() {
        // Features in `[crates.wasm.features]` are forwarded to the core crate
        // via the dep features clause AND must still be declared on the binding
        // crate when they appear in cfg attrs on generated items so rustc does
        // not warn `unexpected cfg condition value` under `-D warnings`. They
        // are declared as passthrough but NOT included in `default = [...]`:
        // binding-side `#[cfg(feature = X)]` items intentionally remain hidden
        // so trait-object handles like `VisitorHandle` do not leak into the
        // binding's serde-deserialize surface.
        use crate::core::ir::TypeDef;

        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
features = ["wasm-target"]
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "GatedType".to_string(),
                rust_path: "test_lib::GatedType".to_string(),
                cfg: Some(r#"any(feature = "wasm-target", feature = "extra")"#.to_string()),
                ..Default::default()
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
        let cargo_toml = super::gen_cargo_toml(&api, &config);
        assert!(
            cargo_toml.contains(r#"extra = ["test-lib/extra"]"#),
            "expected `extra` passthrough:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(r#"wasm-target = ["test-lib/wasm-target"]"#),
            "wasm-target must be declared as passthrough so rustc sees the feature:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("default = ["),
            "no default = [...] line — binding-side cfg items stay hidden:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    }

    #[test]
    fn cargo_toml_has_no_issues_docs_line_and_getrandom_deps_are_alphabetical() {
        // Regression: older alef emitted a `# Issues & docs: …` header line into
        // the wasm Cargo.toml, which cargo-sort unconditionally strips, causing
        // prek to oscillate between the two tools forever. The wasm Cargo.toml
        // must use the plain `hash::header` (no issues_url) so cargo-sort is
        // satisfied on every run.
        //
        // The [target.*.dependencies] block must list getrandom_02 before
        // getrandom_03 so cargo-sort's alphabetical pass is a no-op.
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = make_config();
        let cargo_toml = super::gen_cargo_toml(&api, &config);

        assert!(
            !cargo_toml.contains("Issues & docs:"),
            "Cargo.toml must not contain 'Issues & docs:' line — cargo-sort strips it and \
             alef re-emits it, causing prek to loop forever:\n{cargo_toml}"
        );

        // Verify getrandom_02 appears before getrandom_03 (alphabetical order).
        let pos_02 = cargo_toml
            .find("getrandom_02")
            .expect("getrandom_02 must be present in target deps");
        let pos_03 = cargo_toml
            .find("getrandom_03")
            .expect("getrandom_03 must be present in target deps");
        assert!(
            pos_02 < pos_03,
            "getrandom_02 must appear before getrandom_03 (alphabetical order for cargo-sort \
             compatibility); got getrandom_02 at {pos_02}, getrandom_03 at {pos_03}:\n{cargo_toml}"
        );

        // Valid TOML throughout.
        toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    }

    #[test]
    fn test_visitor_field_substitution_in_post_process() {
        // Test the substitution logic directly with a real multi-line string
        let mut content = "impl From<WasmConversionOptions> for sample_markdown_rs::options::ConversionOptions {\n    fn from(val: WasmConversionOptions) -> Self {\n        Self {\n            heading_style: val.heading_style.into(),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\nimpl From<WasmConversionOptionsUpdate> for sample_markdown_rs::options::ConversionOptionsUpdate {\n    fn from(val: WasmConversionOptionsUpdate) -> Self {\n        Self {\n            heading_style: val.heading_style.map(Into::into),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\n".to_string();

        // Apply the same post-process logic
        let field_name = "visitor";
        let patterns = &[
            ("            ", "\n            "), // 12 spaces
            ("        ", "\n        "),         // 8 spaces
            ("  ", "\n  "),                     // 2 spaces
        ];
        for (indent, newline_indent) in patterns {
            let old_pattern = format!("{indent}{field_name}: Default::default(),{newline_indent}..Default::default()");
            let new_pattern = format!(
                "{indent}{field_name}: val.{field_name}.map(|v| (*v.inner).clone()),{newline_indent}..Default::default()"
            );
            if content.contains(&old_pattern) {
                content = content.replace(&old_pattern, &new_pattern);
            }
        }

        // Verify both From impls were updated
        assert!(
            content.contains("visitor: val.visitor.map(|v| (*v.inner).clone()),"),
            "Visitor field not forwarded in From impl"
        );
        assert!(
            !content.contains("visitor: Default::default(),\n            ..Default::default()"),
            "Unreplaced visitor: Default::default() with 12 spaces still present"
        );
    }
}
