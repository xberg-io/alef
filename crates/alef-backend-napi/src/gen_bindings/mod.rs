//! NAPI-RS (Node.js) backend: orchestration and `Backend` trait implementation.

pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod types;

use crate::type_map::NapiMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators::{self, AsyncPattern, RustBindingConfig};
use alef_codegen::naming::to_node_name;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use alef_core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::ir::{ApiSurface, TypeRef};
use std::path::PathBuf;

pub struct NapiBackend;

impl NapiBackend {
    fn binding_config<'a>(core_import: &'a str, prefix: &'a str, has_serde: bool) -> RustBindingConfig<'a> {
        RustBindingConfig {
            struct_attrs: &["napi"],
            field_attrs: &[],
            struct_derives: &["Clone"],
            method_block_attr: Some("napi"),
            constructor_attr: "#[napi(constructor)]",
            static_attr: None,
            function_attr: "#[napi]",
            enum_attrs: &["napi(string_enum)"],
            enum_derives: &["Clone"],
            needs_signature: false,
            signature_prefix: "",
            signature_suffix: "",
            core_import,
            async_pattern: AsyncPattern::NapiNativeAsync,
            has_serde,
            // NAPI napi(object) structs don't derive Serialize — disable serde bridge
            type_name_prefix: prefix,
            option_duration_on_defaults: true,
            opaque_type_names: &[],
            skip_impl_constructor: false,
            cast_uints_to_i32: false,
            cast_large_ints_to_f64: false,
        }
    }
}

impl Backend for NapiBackend {
    fn name(&self) -> &str {
        "napi"
    }

    fn language(&self) -> Language {
        Language::Node
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
        let prefix = config.node_type_prefix();
        let trait_type_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_trait)
            .map(|t| t.name.clone())
            .collect();
        let mapper = NapiMapper::with_traits(prefix.clone(), trait_type_names);
        let core_import = config.core_import_name();

        // Detect serde availability from the output crate's Cargo.toml
        let output_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");
        let has_serde = alef_core::config::detect_serde_available(&output_dir);
        let cfg = Self::binding_config(&core_import, &prefix, has_serde);

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
        // Cast lints fire heavily on the JS u32/i64/Number bridge — these are
        // intentional, deliberate at the FFI boundary. Pedantic/nursery noise
        // (must_use_candidate, use_self, missing_const_for_fn, etc.) is
        // suppressed for the same reasons documented in the pyo3 backend.
        builder.add_inner_attribute(
            "allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::default_trait_access, clippy::useless_conversion, clippy::unsafe_derive_deserialize, clippy::must_use_candidate, clippy::return_self_not_must_use, clippy::use_self, clippy::missing_const_for_fn, clippy::missing_errors_doc, clippy::needless_pass_by_value, clippy::doc_markdown, clippy::derive_partial_eq_without_eq, clippy::uninlined_format_args, clippy::redundant_clone, clippy::implicit_clone, clippy::redundant_closure_for_method_calls, clippy::wildcard_imports, clippy::option_if_let_else, clippy::too_many_lines)",
        );
        builder.add_import("napi::*");
        builder.add_import("napi_derive::napi");

        // Always import serde_json for type conversion in From/Into impls,
        // even if the binding crate doesn't explicitly list it as a dependency.
        // serde_json is needed for conversions of types with serde-serializable fields.
        builder.add_import("serde_json");

        // Import traits needed for trait method dispatch
        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        // Only import HashMap when Map-typed fields or returns are present
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

        // Note: custom_modules for Node are TypeScript-only re-exports
        // (used in generate_public_api), not Rust module declarations.

        // Check if any function or method is async
        let has_async =
            api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));

        if has_async {
            builder.add_item(&functions::gen_tokio_runtime());
        }

        // Check if we have opaque types and trait types (visitors)
        // Exclude trait types from opaque_types since they use JsVisitorRef instead of Object<'static>
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !t.is_trait)
            .map(|t| t.name.clone())
            .collect();
        let has_traits = api.types.iter().any(|t| t.is_trait);
        if !opaque_types.is_empty() || has_traits {
            builder.add_import("std::sync::Arc");
        }

        let exclude_types: ahash::AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();

        // Build adapter body map before type iteration so bodies are available for method generation.
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Node)?;

        // JsVisitorRef: a thin wrapper around napi::Object that implements Clone.
        // This newtype makes Object<'static> work with napi(object) field derivations,
        // which require Clone. Uses std::sync::Arc to make the handle cheaply cloneable.
        if has_traits {
            let js_visitor_ref_def = r#"
/// Wrapper for trait visitor types (napi::Object<'static>) that implements Clone.
///
/// Object is not Clone. This wrapper uses Arc<Object<'static>> internally for cheap cloning.
/// The .inner field is public for compatibility with generated code that needs to access
/// the underlying Object for trait dispatch.
pub struct JsVisitorRef {
    pub inner: std::sync::Arc<napi::bindgen_prelude::Object<'static>>,
}

impl Clone for JsVisitorRef {
    fn clone(&self) -> Self {
        JsVisitorRef {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

impl From<napi::bindgen_prelude::Object<'static>> for JsVisitorRef {
    fn from(visitor: napi::bindgen_prelude::Object<'static>) -> Self {
        JsVisitorRef {
            inner: std::sync::Arc::new(visitor),
        }
    }
}

impl From<JsVisitorRef> for napi::bindgen_prelude::Object<'static> {
    fn from(visitor_ref: JsVisitorRef) -> Self {
        // SAFETY: Arc::clone does not actually clone the Object — it just increments
        // the refcount. When we deref via * and clone, we get a new arc-ed reference
        // to the same Object data. This is safe because Object<'static> is a reference
        // type (internally just holding an env + handle pair).
        (*visitor_ref.inner).clone()
    }
}
"#;
            builder.add_item(js_visitor_ref_def);
        }

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

        // NAPI has some unique patterns: Js-prefixed names, Option-wrapped fields,
        // and custom constructor. Use shared generators for enums and functions,
        // but keep struct/method generation custom.
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
        {
            if typ.is_opaque {
                builder.add_item(&alef_codegen::generators::gen_opaque_struct_prefixed(
                    typ, &cfg, &prefix,
                ));
                builder.add_item(&types::gen_opaque_struct_methods(
                    typ,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &prefix,
                    &adapter_bodies,
                ));
            } else {
                // Non-opaque structs use #[napi(object)] — plain JS objects without methods.
                // napi(object) structs cannot have #[napi] impl blocks.
                // gen_struct adds Default to derives when typ.has_default is true.
                builder.add_item(&types::gen_struct(typ, &mapper, &prefix, has_serde, &opaque_types));
            }
        }

        // Collect struct names so tagged enum codegen knows which Named types have binding structs
        let struct_names: ahash::AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();

        for enum_def in &api.enums {
            builder.add_item(&enums::gen_enum(enum_def, &prefix, has_serde));
        }

        let exclude_functions: ahash::AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        for func in &api.functions {
            if exclude_functions.contains(&func.name) {
                continue;
            }
            let bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges);
            let options_field_bridge = crate::trait_bridge::find_options_field_binding(func, &config.trait_bridges);
            // Skip sanitized functions when there's no trait bridge that can replace the
            // sanitized parameter — such functions cannot be auto-delegated. Functions
            // whose only "sanitized" param is a configured trait_bridge param (e.g.
            // Option<VisitorHandle> in html-to-markdown) are emitted via gen_bridge_function.
            if func.sanitized && bridge_param.is_none() && options_field_bridge.is_none() {
                continue;
            }
            if let Some((param_idx, bridge_cfg)) = bridge_param {
                builder.add_item(&crate::trait_bridge::gen_bridge_function(
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &cfg,
                    &Default::default(),
                    &opaque_types,
                    &core_import,
                ));
            } else if let Some((param_idx, bridge_cfg)) = options_field_bridge {
                builder.add_item(&crate::trait_bridge::gen_options_field_bridge_function(
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &core_import,
                ));
            } else {
                builder.add_item(&functions::gen_function(func, &mapper, &cfg, &opaque_types, &prefix));
            }
        }

        // Trait bridge wrappers — generate NAPI bridge structs that delegate to JS objects
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

        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        let napi_conv_config = alef_codegen::conversions::ConversionConfig {
            type_name_prefix: &prefix,
            cast_large_ints_to_i64: true,
            cast_f32_to_f64: true,
            // optionalize_defaults: For types with has_default, conversion generators
            // make all fields Option<T> and apply defaults via FromNapiValue,
            // enabling JS users to pass partial objects and omit fields they want defaults for.
            optionalize_defaults: true,
            option_duration_on_defaults: true,
            include_cfg_metadata: true,
            // Pass opaque_types so the conversion generator can emit `Default::default()`
            // for opaque-type fields (e.g. visitor: Object<'static>) instead of trying to
            // convert them via Into — these fields are handled separately via bridge code.
            opaque_types: Some(&opaque_types),
            ..Default::default()
        };
        // From/Into conversions using shared parameterized generators
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &napi_conv_config,
                ));
            }
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &napi_conv_config,
                ));
            }
        }
        for e in &api.enums {
            let is_tagged_data_enum = e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty());
            if is_tagged_data_enum {
                // Tagged data enums use flattened struct — generate custom conversions
                builder.add_item(&methods::gen_tagged_enum_binding_to_core(
                    e,
                    &core_import,
                    &prefix,
                    &struct_names,
                ));
                builder.add_item(&methods::gen_tagged_enum_core_to_binding(
                    e,
                    &core_import,
                    &prefix,
                    &struct_names,
                ));
            } else {
                if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                        e,
                        &core_import,
                        &napi_conv_config,
                    ));
                }
                if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                        e,
                        &core_import,
                        &napi_conv_config,
                    ));
                }
            }
        }

        // Error types (variant name constants + converter functions)
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_napi_error_types(error));
            builder.add_item(&alef_codegen::error_gen::gen_napi_error_converter(error, &core_import));
        }

        let content = builder.build();

        let output_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.node_type_prefix();

        // Separate exports into functions (plain export) and types (export type)
        let mut type_exports = vec![];
        let mut function_exports = vec![];

        // Collect all types (exported with prefix from native module) - export type.
        // Include all plain structs and all trait types (opaque classes and visitor interfaces).
        for typ in api.types.iter() {
            // Export all types: plain structs, opaque trait classes, and visitor trait interfaces
            type_exports.push(format!("{prefix}{}", typ.name));
        }

        // Collect all enums as type exports.
        // With verbatimModuleSyntax enabled, re-exporting const enums as values causes
        // TS2748/TS1205; using `export type` avoids both errors.
        for enum_def in &api.enums {
            type_exports.push(format!("{prefix}{}", enum_def.name));
        }

        // NAPI errors are thrown as native JS Error objects, not exported as TS types.
        // Skip error types in the public API re-exports.

        // Collect all functions (exported from native module) - plain export
        for func in &api.functions {
            // Convert snake_case to camelCase for JavaScript naming
            let js_name = to_node_name(&func.name);
            function_exports.push(js_name);
        }

        // Sort for consistent output
        type_exports.sort();
        function_exports.sort();

        // Generate the index.ts re-export file using a single export block
        // with inline `type` annotations for verbatimModuleSyntax compatibility.
        let mut lines = vec![
            "// This file is auto-generated by alef. DO NOT EDIT.".to_string(),
            "".to_string(),
        ];

        // Separate value and type exports for verbatimModuleSyntax compatibility.
        // Value exports (functions) in one block, type exports (structs + enums) in another.
        if !function_exports.is_empty() {
            lines.push("export {".to_string());
            for name in &function_exports {
                lines.push(format!("  {name},"));
            }
            lines.push(format!("}} from '{}';", config.node_package_name()));
            lines.push("".to_string());
        }
        if !type_exports.is_empty() {
            lines.push("export type {".to_string());
            for name in &type_exports {
                lines.push(format!("  {name},"));
            }
            lines.push(format!("}} from '{}';", config.node_package_name()));
        }

        // Append re-exports for custom modules (from [custom_modules] node = [...])
        let custom_mods = config.custom_modules.for_language(Language::Node);
        for module_name in custom_mods {
            lines.push(format!("export * from './{module_name}';"));
        }

        let content = lines.join("\n");

        // Output path: packages/typescript/src/index.ts
        let output_path = PathBuf::from("packages/typescript/src/index.ts");

        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: false,
        }])
    }

    fn generate_type_stubs(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.node_type_prefix();
        let exclude_functions: ahash::AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let content = errors::gen_dts(api, &prefix, &exclude_functions, &config.trait_bridges);

        // `output_for("node")` points to the `src/` directory (e.g., `crates/{name}-node/src/`).
        // `index.d.ts` belongs at the crate root, one level up from `src/`.
        // When the configured path ends in `src/` or `src`, strip that suffix to get the crate root.
        // Falls back to `crates/{name}-node/` if no node output is configured.
        let src_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");
        let crate_root = {
            let p = PathBuf::from(&src_dir);
            match p.file_name().and_then(|n| n.to_str()) {
                Some("src") => p.parent().map(|parent| parent.to_path_buf()).unwrap_or(p),
                _ => p,
            }
        };

        Ok(vec![GeneratedFile {
            path: crate_root.join("index.d.ts"),
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "napi",
            crate_suffix: "-node",
            build_dep: BuildDependency::None,
            post_build: vec![PostBuildStep::PatchFile {
                path: "index.d.ts",
                find: "export declare const enum",
                replace: "export declare enum",
            }],
        })
    }
}

/// Generate a NAPI struct with Js-prefixed name and fields wrapped in Option only if optional.
#[cfg(test)]
mod tests {
    use super::NapiBackend;
    use alef_core::backend::Backend;
    use alef_core::config::Language;

    /// NapiBackend::name returns "napi".
    #[test]
    fn napi_backend_name_is_napi() {
        let b = NapiBackend;
        assert_eq!(b.name(), "napi");
    }

    /// NapiBackend::language returns Language::Node.
    #[test]
    fn napi_backend_language_is_node() {
        let b = NapiBackend;
        assert_eq!(b.language(), Language::Node);
    }
}
