//! WASM (wasm-bindgen) backend: orchestration and `Backend` trait implementation.

pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod types;

use crate::type_map::WasmMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_codegen::naming::to_node_name;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, TypeRef};
use alef_core::template_versions as tv;
use std::path::PathBuf;

use enums::gen_enum;
use errors::gen_error_converter;
use functions::{gen_env_shims, gen_function};
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
/// Parses cfg strings like `feature = "api"` and checks if the feature
/// is in the enabled features list.
fn is_gated_behind_disabled_feature(cfg: &Option<String>, enabled_features: &[String]) -> bool {
    let Some(cfg_str) = cfg else {
        return false;
    };

    // Parse simple feature gates like `feature = "api"`
    if let Some(start) = cfg_str.find("feature") {
        let rest = &cfg_str[start..];
        if let Some(eq_pos) = rest.find('=') {
            let after_eq = rest[eq_pos + 1..].trim();
            if let Some(start_quote) = after_eq.find('"') {
                let after_quote = &after_eq[start_quote + 1..];
                if let Some(end_quote) = after_quote.find('"') {
                    let feature_name = &after_quote[..end_quote];
                    return !enabled_features.iter().any(|f| f == feature_name);
                }
            }
        }
    }
    false
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
                        for field in &mut typ.fields {
                            if skip_list.iter().any(|s| s == &field.name) && field.cfg.is_none() {
                                field.cfg = Some("alef_excluded".to_string());
                                typ.has_stripped_cfg_fields = true;
                            }
                        }
                    }
                }
                cloned
            };
            &api_owned
        };

        // Note: custom modules and registrations handled below after builder creation

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::useless_conversion)");
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

        // Note: custom_modules for WASM are TypeScript-only re-exports
        // (used in generate_public_api), not Rust module declarations.

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !exclude_types.contains(&t.name))
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
        }

        // Trait bridge type aliases (e.g. `VisitorHandle`) are opaque — they map to
        // `Arc<core::VisitorHandle>` in the binding layer and must not attempt From/Into
        // conversion in generated From impls.  Include them so struct fields referencing
        // these types use Default::default() instead of val.visitor.map(Into::into).
        let bridge_type_aliases: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let mut opaque_names_vec: Vec<String> = opaque_types.iter().cloned().collect();
        opaque_names_vec.extend(bridge_type_aliases);
        let opaque_names_set: AHashSet<String> = opaque_names_vec.iter().cloned().collect();

        // Build adapter body map before type iteration so bodies are available for method generation.
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Wasm)?;

        // Emit adapter-generated standalone items (streaming iterators, callback bridges).
        for adapter in &config.adapters {
            match adapter.pattern {
                alef_core::config::AdapterPattern::Streaming => {
                    let key = format!("{}.__stream_struct__", adapter.item_type.as_deref().unwrap_or(""));
                    if let Some(struct_code) = adapter_bodies.get(&key) {
                        builder.add_item(struct_code);
                    }
                }
                alef_core::config::AdapterPattern::CallbackBridge => {
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
                ));
            } else {
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // so no separate Default impl is needed.
                builder.add_item(&gen_struct(typ, &mapper, &exclude_types, &prefix));
                builder.add_item(&gen_struct_methods(
                    typ,
                    &mapper,
                    &exclude_types,
                    &core_import,
                    &opaque_types,
                    &api.enums,
                    &prefix,
                ));
            }
        }

        for enum_def in &api.enums {
            if !exclude_types.contains(&enum_def.name) {
                builder.add_item(&gen_enum(enum_def, &prefix));
            }
        }

        for func in &api.functions {
            if !exclude_functions.contains(&func.name) {
                // Skip functions whose signature references excluded types
                let refs_excluded = func
                    .params
                    .iter()
                    .any(|p| field_references_excluded_type(&p.ty, &exclude_types))
                    || field_references_excluded_type(&func.return_type, &exclude_types);
                if refs_excluded {
                    continue;
                }
                let bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges);
                if let Some((param_idx, bridge_cfg)) = bridge_param {
                    builder.add_item(&crate::trait_bridge::gen_bridge_function(
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &prefix,
                    ));
                } else {
                    builder.add_item(&gen_function(func, &mapper, &core_import, &opaque_types, &prefix));
                }
            }
        }

        // Trait bridge wrappers — generate WASM bridge structs that delegate to JS objects
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                let bridge = crate::trait_bridge::gen_trait_bridge(
                    trait_type,
                    bridge_cfg,
                    &core_import,
                    &config.error_type_name(),
                    &config.error_constructor_expr(),
                    api,
                );
                for imp in &bridge.imports {
                    builder.add_import(imp);
                }
                builder.add_item(&bridge.code);
            }
        }

        let wasm_conv_config = alef_codegen::conversions::ConversionConfig {
            type_name_prefix: &prefix,
            map_uses_jsvalue: true,
            option_duration_on_defaults: true,
            exclude_types: &exclude_types,
            source_crate_remaps: &source_remaps_borrowed,
            // Treat bridge type aliases (e.g. VisitorHandle) as opaque so binding→core
            // From impls emit Default::default() instead of val.visitor.map(Into::into).
            opaque_types: if opaque_names_set.is_empty() {
                None
            } else {
                Some(&opaque_names_set)
            },
            ..Default::default()
        };
        let convertible = alef_codegen::conversions::convertible_types(api);
        let core_to_binding_convertible = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        // From/Into conversions using shared parameterized generators
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            let is_strict = alef_codegen::conversions::can_generate_conversion(typ, &convertible);
            let is_relaxed = alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding_convertible);
            if is_strict {
                // Both directions
                if input_types.contains(&typ.name) {
                    builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                        typ,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &wasm_conv_config,
                ));
            } else if is_relaxed {
                // Only core→binding (sanitized fields prevent binding→core)
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &wasm_conv_config,
                ));
            }
        }
        for e in &api.enums {
            if !exclude_types.contains(&e.name) {
                if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                        e,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
                if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                        e,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
            }
        }

        // Error converter functions (skip excluded errors)
        for error in &api.errors {
            if exclude_types.contains(&error.name) {
                continue;
            }
            builder.add_item(&gen_error_converter(error, &core_import));
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

        // Fix From<WasmConversionOptions/Update> to pass through visitor.
        // The replacements must be very careful to only apply to the forward direction impls.

        // For WasmConversionOptions -> ConversionOptions, the unique marker is:
        // - Has "strong_em_symbol.chars().next().unwrap_or" (forward only)
        // - Has "visitor: Default::default()" followed by "../Default::default()"
        let forward_conv_marker = "impl From<WasmConversionOptions> for html_to_markdown_rs::options::ConversionOptions";
        if content.contains(forward_conv_marker) {
            // Find first occurrence of this impl
            if let Some(pos) = content.find(forward_conv_marker) {
                // Find the matching closing brace + comment pattern
                let closing = "\n}\n\n#[allow(clippy::redundant_closure, clippy::useless_conversion)]\nimpl From<html_to_markdown_rs::options::ConversionOptions>";
                if let Some(end) = content[pos..].find(closing) {
                    let before_impl = &content[..pos];
                    let impl_content = &content[pos..pos + end];
                    let after_impl = &content[pos + end..];

                    // Replace visitor in this impl only
                    let visitor_old = "            visitor: Default::default(),\n            ..Default::default()";
                    let visitor_new = "            visitor: val.visitor.map(|v| (*v.inner).clone()),\n            ..Default::default()";
                    let modified_impl = impl_content.replace(visitor_old, visitor_new);
                    content = format!("{}{}{}", before_impl, modified_impl, after_impl);
                }
            }
        }

        // For WasmConversionOptionsUpdate -> ConversionOptionsUpdate
        let forward_update_marker = "impl From<WasmConversionOptionsUpdate> for html_to_markdown_rs::options::ConversionOptionsUpdate";
        if content.contains(forward_update_marker) {
            if let Some(pos) = content.find(forward_update_marker) {
                let closing_update = "\n}\n\n#[allow(clippy::redundant_closure, clippy::useless_conversion)]\nimpl From<html_to_markdown_rs::options::ConversionOptionsUpdate>";
                if let Some(end) = content[pos..].find(closing_update) {
                    let before_impl = &content[..pos];
                    let impl_content = &content[pos..pos + end];
                    let after_impl = &content[pos + end..];

                    let visitor_old = "            visitor: Default::default(),\n            ..Default::default()";
                    let visitor_new = "            visitor: val.visitor.map(|v| (*v.inner).clone()),\n            ..Default::default()";
                    let modified_impl = impl_content.replace(visitor_old, visitor_new);
                    content = format!("{}{}{}", before_impl, modified_impl, after_impl);
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

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let wasm_config = config.wasm.as_ref();
        let exclude_functions = wasm_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
        let exclude_types = wasm_config.map(|c| c.exclude_types.clone()).unwrap_or_default();
        let exclude_reexports = wasm_config.map(|c| c.exclude_reexports.clone()).unwrap_or_default();
        let prefix = config.wasm_type_prefix();

        // Collect all exported names from the API
        let mut exports = vec![];

        // Collect all types (exported with prefix from WASM module)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !exclude_types.contains(&typ.name) {
                exports.push(format!("{prefix}{}", typ.name));
            }
        }

        // Collect all enums (exported with prefix from WASM module)
        for enum_def in &api.enums {
            if !exclude_types.contains(&enum_def.name) {
                exports.push(format!("{prefix}{}", enum_def.name));
            }
        }

        // Collect all functions (exported from WASM module)
        for func in &api.functions {
            if !exclude_functions.contains(&func.name) {
                // Convert snake_case to camelCase for JavaScript naming
                let js_name = to_node_name(&func.name);
                exports.push(js_name);
            }
        }

        // Collect trait-bridge register/unregister/clear functions. They are
        // emitted as `#[wasm_bindgen(js_name = "registerOcrBackend")]` etc. in
        // the trait-bridge code path and are NOT part of `api.functions`, so
        // they need to be added to `index.ts` re-exports explicitly.
        for bridge in &config.trait_bridges {
            if let Some(register_fn) = bridge.register_fn.as_deref()
                && !exclude_functions.contains(&register_fn.to_string())
            {
                exports.push(to_node_name(register_fn));
            }
            if let Some(unregister_fn) = bridge.unregister_fn.as_deref()
                && !exclude_functions.contains(&unregister_fn.to_string())
            {
                exports.push(to_node_name(unregister_fn));
            }
            if let Some(clear_fn) = bridge.clear_fn.as_deref()
                && !exclude_functions.contains(&clear_fn.to_string())
            {
                exports.push(to_node_name(clear_fn));
            }
        }

        // Collect all error types (exported from WASM module), skipping excluded
        for error in &api.errors {
            if exclude_types.contains(&error.name) {
                continue;
            }
            exports.push(error.name.clone());
        }

        // Remove any exports that should be provided by custom modules instead
        exports.retain(|name| !exclude_reexports.contains(name));

        // Sort for consistent output
        exports.sort();

        // Generate the index.ts re-export file
        let header = hash::header(CommentStyle::DoubleSlash);
        let mut lines: Vec<String> = header.lines().map(str::to_string).collect();
        lines.push("".to_string());

        if !exports.is_empty() {
            lines.push("export {".to_string());
            for (i, name) in exports.iter().enumerate() {
                let comma = if i < exports.len() - 1 { "," } else { "" };
                lines.push(format!("  {}{}", name, comma));
            }
            lines.push("} from './wasm';".to_string());
        }

        // Append re-exports for custom modules (from [custom_modules] wasm = [...])
        let custom_mods = config.custom_modules.for_language(Language::Wasm);
        for module_name in custom_mods {
            lines.push(format!("export * from './{module_name}';"));
        }

        let content = lines.join("\n");

        // Output path: packages/wasm/src/index.ts
        let output_path = PathBuf::from("packages/wasm/src/index.ts");

        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: false,
        }])
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

    let header = hash::header(CommentStyle::Hash);

    // Layout follows cargo-sort canonical order: [package] -> [package.metadata.*]
    // -> [lib] -> [dependencies] (alphabetical). Otherwise cargo-sort rewrites the
    // file post-generate and breaks the alef hash header.
    let mut deps: Vec<(String, String)> = vec![
        (
            core_dep_key.clone(),
            format!(r#"{{ path = "../{core_crate_dir}"{features_clause} }}"#),
        ),
        ("futures-util".to_string(), format!(r#""{}""#, tv::cargo::FUTURES_UTIL)),
        ("js-sys".to_string(), format!(r#""{}""#, tv::cargo::JS_SYS)),
        ("wasm-bindgen".to_string(), format!(r#""{}""#, tv::cargo::WASM_BINDGEN)),
        (
            "wasm-bindgen-futures".to_string(),
            format!(r#""{}""#, tv::cargo::WASM_BINDGEN_FUTURES),
        ),
        (
            "serde-wasm-bindgen".to_string(),
            format!(r#""{}""#, tv::cargo::SERDE_WASM_BINDGEN),
        ),
        ("serde_json".to_string(), r#""1""#.to_string()),
    ];
    // Append parsed extra deps as (name, value) pairs.
    for line in extra_deps_section.lines() {
        let trimmed = line.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            deps.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
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
ignored = ["futures-util", "js-sys", "wasm-bindgen-futures", "serde_json"]

[package.metadata.wasm-pack.profile.release]
wasm-opt = false

[lib]
crate-type = ["cdylib"]

[dependencies]
{deps_block}

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = {{ version = "0.3", features = ["wasm_js"] }}
"#,
        header = header,
        pkg_prefix = pkg_prefix,
        version = version,
        license = license,
        description = description,
        repository = repository,
        keywords_toml = keywords_toml,
        deps_block = deps_block,
    )
}
#[cfg(test)]
mod tests {
    use super::WasmBackend;
    use alef_core::backend::Backend;
    use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
    use alef_core::ir::ApiSurface;

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
        };
        let config = make_config();
        let files = WasmBackend.generate_bindings(&api, &config).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].path.to_string_lossy().ends_with("lib.rs"));
        assert!(files[1].path.to_string_lossy().ends_with("Cargo.toml"));
    }
}
