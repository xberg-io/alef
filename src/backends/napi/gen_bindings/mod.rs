//! NAPI-RS (Node.js) backend: orchestration and `Backend` trait implementation.

pub mod capsule;
mod config_opaque;
mod constructors;
pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod service_api;
mod support;
mod type_stubs;
pub mod types;

#[cfg(test)]
mod tests;

use crate::backends::napi::type_map::NapiMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators::{self, AsyncPattern, RustBindingConfig};
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use crate::core::config::{Language, NodeCapsuleTypeConfig, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use constructors::{napi_default_constructor, napi_variant_wrapper_constructor};
use std::collections::HashMap;
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
            named_non_opaque_params_by_ref: false,
            lossy_skip_types: &[],
            serializable_opaque_type_names: &[],
            never_skip_cfg_field_names: &[],
            // NAPI bypasses the shared `gen_struct` and uses its own builder in
            // `src/backends/napi/gen_bindings/types.rs`, so this flag is a
            // no-op for NAPI — the napi-specific `From<JsT> for core::T` impl
            // (in `binding_to_core.rs`) uses `unwrap_or_default()` to fill
            // missing optional fields, which still emits primitive defaults.
            // Keep the flag off until the napi gen_struct learns the same
            // suppress-derive + delegate-impl pattern.
            emit_delegating_default_impl: false,
            skip_methods_when_not_delegatable: false,
            source_crate_remaps: &[],
            emit_delegating_default_for_types: None,
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
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Collapse same-named cfg-variant functions into one canonical entry. The napi `#[napi]`
        // wrapper delegates to the core crate (which resolves the cfg) and emits no `#[cfg]` gate,
        // so two same-named entries would otherwise produce duplicate `#[napi]` fn definitions.
        // Matches the FFI/pyo3 backends; see codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

        let prefix = config.node_type_prefix();
        let trait_type_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_trait)
            .map(|t| t.name.clone())
            .collect();
        let capsule_type_names_for_mapper: AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.capsule_types.keys().cloned().collect())
            .unwrap_or_default();
        let mapper =
            NapiMapper::with_traits_and_capsules(prefix.clone(), trait_type_names, capsule_type_names_for_mapper);
        let core_import = config.core_import_name();

        // Detect serde availability from the output crate's Cargo.toml
        let output_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");
        let has_serde = crate::core::config::detect_serde_available(&output_dir);
        let mut cfg = Self::binding_config(&core_import, &prefix, has_serde);
        let never_skip_cfg_field_names: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| {
                if b.bind_via == crate::core::config::BridgeBinding::OptionsField {
                    b.resolved_options_field().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
        cfg.never_skip_cfg_field_names = &never_skip_cfg_field_names;

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(unsafe_code)");
        // The napi-rs crate is entirely generated glue — rustdoc coverage is not meaningful here.
        builder.add_inner_attribute("allow(missing_docs)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy, clippy::should_implement_trait)");
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

        // Check if any function or method is async
        let has_async =
            api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));

        if has_async {
            builder.add_item(&functions::gen_tokio_runtime());
        }

        // Extract capsule_types from NodeConfig. Types listed here skip #[napi] opaque-class
        // emission; functions returning them produce a JsObject with __parser External<T>.
        let capsule_types: HashMap<String, NodeCapsuleTypeConfig> = config
            .node
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();

        // When capsule types are present, generated shims call set_named_property which
        // requires the JsObjectValue trait to be in scope.
        if !capsule_types.is_empty() {
            builder.add_import("napi::bindgen_prelude::JsObjectValue");
            // Emit the FFI declarations for napi_create_external and napi_type_tag_object,
            // and any per-capsule type tag constants. Done once per crate.
            builder.add_item(&capsule::gen_ffi_declarations());
            let constants = capsule::gen_type_tag_constants(&capsule_types);
            if !constants.is_empty() {
                builder.add_item(&constants);
            }
        }

        let opaque_types = config_opaque::collect_opaque_types(api, config, &capsule_types);
        let mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        let has_traits = api.types.iter().any(|t| t.is_trait);
        if !opaque_types.is_empty() || has_traits {
            builder.add_import("std::sync::Arc");
        }
        if !mutex_types.is_empty() {
            builder.add_import("std::sync::Mutex");
        }

        let mut exclude_types: ahash::AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();
        config_opaque::exclude_capsule_opaque_types(&mut exclude_types, config, &capsule_types);

        // Build adapter body map before type iteration so bodies are available for method generation.
        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Node)?;

        // Map "OwnerType.method" -> streaming item type. The napi backend needs to
        // override the IR-declared `String` return type with `Vec<{prefix}{item}>`
        // for streaming adapters, since the generated body returns chunks directly
        // as a JS array instead of a serialized JSON string.
        let streaming_item_types: ahash::AHashMap<String, String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter_map(|a| {
                let owner = a.owner_type.as_deref()?;
                let item = a.item_type.as_deref()?;
                Some((format!("{owner}.{}", a.name), item.to_string()))
            })
            .collect();

        builder.add_item(support::js_bytes_def());

        if has_traits {
            builder.add_item(support::js_visitor_ref_def());
        }

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

        // NAPI has some unique patterns: Js-prefixed names, Option-wrapped fields,
        // and custom constructor. Use shared generators for enums and functions,
        // but keep struct/method generation custom.
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
        {
            // Capsule types bypass #[napi] class emission entirely — they are exposed
            // as raw External<T> pointers in JsObject wrappers from functions that return them.
            if capsule_types.contains_key(&typ.name) {
                continue;
            }
            if typ.is_opaque {
                // gen_opaque_struct_prefixed emits `#[napi]` from cfg.struct_attrs.
                // Replace with `#[napi(js_name = "Foo")]` so NAPI-RS exports the
                // unprefixed name while the Rust struct stays JsFoo internally.
                // Prepend `///` rustdoc so napi-derive forwards it to JSDoc on
                // the corresponding `export declare class` in index.d.ts.
                let opaque_struct_code = {
                    let raw = crate::codegen::generators::gen_opaque_struct_prefixed(typ, &cfg, &prefix);
                    let struct_name = format!("{prefix}{}", typ.name);
                    let body = raw.replace(
                        &format!("#[napi]pub struct {struct_name}"),
                        &format!("#[napi(js_name = \"{}\")]pub struct {struct_name}", typ.name),
                    );
                    let mut out = String::new();
                    let sanitized_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
                        &typ.doc,
                        crate::codegen::doc_emission::DocTarget::TsDoc,
                    );
                    crate::codegen::doc_emission::emit_rustdoc(&mut out, &sanitized_doc, "");
                    out.push_str(&body);
                    out
                };
                builder.add_item(&opaque_struct_code);
                let capsule_type_names: AHashSet<String> = capsule_types.keys().cloned().collect();
                builder.add_item(&types::gen_opaque_struct_methods(
                    typ,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &prefix,
                    &adapter_bodies,
                    &streaming_item_types,
                    &capsule_type_names,
                    &mutex_types,
                    &capsule_types,
                ));
                // Client constructor — emit a #[napi(constructor)] impl
                if let Some(ctor) = config.client_constructors.get(&typ.name) {
                    let struct_name = format!("{prefix}{}", typ.name);
                    let ctor_body = crate::codegen::generators::gen_opaque_constructor(
                        ctor,
                        &typ.name,
                        &core_import,
                        "#[napi(constructor)]",
                    );
                    let ctor_impl = format!("#[napi]\nimpl {struct_name} {{\n{}}}", ctor_body);
                    builder.add_item(&ctor_impl);
                }
                // Constructors for opaque types: emit #[napi(constructor)] for default-constructors
                // or variant-wrapper constructors (but not both).
                // Variant-wrapper takes priority — when the type is referenced as the wrapper
                // of one or more registration variants, and variant bodies emit
                // `new WrapperType(args)` JS constructor-syntax, opt the wrapper's static
                // `new` into a `#[napi(constructor)]` so napi-rs exposes it as `new Class()`.
                //
                // `client_constructors` takes priority over both. Both variant-wrapper and
                // default constructors use a distinct Rust fn name (`new_constructor`) to avoid
                // a duplicate-`fn new` conflict with the static `#[napi]` method already emitted
                // by `gen_opaque_struct_methods`. This allows safe emission even when a static
                // `new()` method exists in typ.methods.
                else if !config.client_constructors.contains_key(&typ.name) {
                    if typ.is_variant_wrapper {
                        if let Some(ctor) = napi_variant_wrapper_constructor(typ, &mapper, &core_import, &prefix) {
                            builder.add_item(&ctor);
                        }
                    } else if typ.has_default {
                        if let Some(ctor) = napi_default_constructor(typ, &mapper, &core_import, &prefix) {
                            builder.add_item(&ctor);
                        }
                    }
                }
                // Emit impl Default if type has has_default=true and a parameterless new() method.
                // This satisfies clippy's new_without_default lint.
                // Only emit if Default trait isn't already emitted elsewhere (e.g., derived on struct).
                if typ.has_default
                    && typ
                        .methods
                        .iter()
                        .any(|m| m.name == "new" && m.receiver.is_none() && m.params.is_empty())
                {
                    let struct_name = format!("{prefix}{}", typ.name);
                    let default_impl = format!(
                        "impl Default for {struct_name} {{\n    fn default() -> Self {{\n        Self::new()\n    }}\n}}\n"
                    );
                    builder.add_item(&default_impl);
                }
            } else {
                // Non-opaque structs use #[napi(object)] — plain JS objects without methods.
                // napi(object) structs cannot have #[napi] impl blocks.
                // gen_struct adds Default to derives when typ.has_default is true.
                builder.add_item(&types::gen_struct(
                    typ,
                    &mapper,
                    &prefix,
                    has_serde,
                    &opaque_types,
                    &never_skip_cfg_field_names,
                ));
                // Emit impl methods as standalone #[napi] free functions.
                // #[napi(object)] structs cannot have impl blocks, so each method becomes a
                // module-level function whose JS name encodes the type as a namespace prefix
                // (e.g. `processConfigAll`, `processConfigWithChunking`). TypeScript callers
                // group them via a value-namespace declaration in their own code.
                let dto_fns = types::gen_dto_method_fns(typ, &mapper, &cfg, &opaque_types, &prefix, &mutex_types, api);
                if !dto_fns.is_empty() {
                    builder.add_item(&dto_fns);
                }
            }
        }
        config_opaque::emit_wrappers(&mut builder, api, config, &capsule_types, &prefix);

        // Collect struct names so tagged enum codegen knows which Named types have binding structs
        let struct_names: ahash::AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();

        // Collect Named types that have a Default impl. These are eligible to be
        // promoted to Option<T> in binding signatures so JS callers may pass
        // `undefined` to fall back to a default-constructed instance.
        let default_types: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_default)
            .map(|t| t.name.clone())
            .collect();

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
            if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
                continue;
            }
            let bridge_param = crate::backends::napi::trait_bridge::find_bridge_param(func, &config.trait_bridges);
            let options_field_bridge = crate::backends::napi::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
                // Only use the options-field path when the bridge field actually survives
                // into the binding struct. If the core field is `#[cfg(...)]`-gated, the
                // struct generator strips it and the generated bridge code would reference
                // a missing field, producing `E0609 no field` at compile time.
                // Exception: fields listed in never_skip_cfg_field_names are cfg-gated but
                // preserved by the struct generator, so they are valid for bridge codegen.
                .filter(|(_, bridge_cfg)| {
                    let Some(field_name) = bridge_cfg.resolved_options_field() else { return false; };
                    let Some(options_type) = bridge_cfg.options_type.as_deref() else { return false; };
                    api.types
                        .iter()
                        .filter(|t| t.name == options_type)
                        .flat_map(|t| t.fields.iter())
                        .any(|f| f.name == field_name && (f.cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_name)))
                });
            // Skip sanitized functions when there's no trait bridge that can replace the
            // sanitized parameter — such functions cannot be auto-delegated. Functions
            // whose only "sanitized" param is a configured trait_bridge param are emitted
            // via gen_bridge_function.
            if func.sanitized && bridge_param.is_none() && options_field_bridge.is_none() {
                continue;
            }
            // Prefer options_field binding over function-param binding for trait bridges.
            // When both exist, options_field provides the correct API surface (visitor
            // embedded in options object, not a separate parameter).
            if let Some((param_idx, bridge_cfg)) = options_field_bridge {
                let item = crate::backends::napi::trait_bridge::gen_options_field_bridge_function(
                    api,
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &core_import,
                );
                let item = support::prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            } else if let Some((param_idx, bridge_cfg)) = bridge_param {
                let item = crate::backends::napi::trait_bridge::gen_bridge_function(
                    api,
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &cfg,
                    &Default::default(),
                    &opaque_types,
                    &core_import,
                );
                let item = support::prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
                let item = crate::backends::napi::trait_bridge::gen_options_field_bridge_function(
                    api,
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &core_import,
                );
                let item = support::prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            } else if !capsule_types.is_empty() && capsule::function_involves_capsule(func, &capsule_types) {
                // Function returns a capsule type — emit a napi shim that returns JsObject
                // with __parser = External<T>(ptr from value.into_raw()).
                // JsObjectValue provides set_named_property; imported once below.
                let item = capsule::gen_capsule_function(func, &capsule_types, &core_import);
                let item = support::prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            } else {
                let item = functions::gen_function(
                    func,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &default_types,
                    &prefix,
                    &capsule_types,
                    &mutex_types,
                );
                let item = support::prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            }
        }

        // Emit module-level wrapper functions for adapters (streaming methods).
        for adapter in &config.adapters {
            builder.add_item(&functions::gen_adapter_wrapper(adapter, &core_import, &api.types));
        }

        // Service-API glue lives in the generated `service.rs`; declare it so its
        // `#[napi]` entrypoints (e.g. `app_run`) are compiled and exported.
        if !api.services.is_empty() {
            builder.add_item("pub mod service;");
        }

        // Trait bridge wrappers — generate NAPI bridge structs that delegate to JS objects
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                let bridge = crate::backends::napi::trait_bridge::gen_trait_bridge(
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

        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
        let input_types = crate::codegen::conversions::input_type_names(api);
        // NOTE: NAPI does NOT populate `trait_bridge_arc_wrapper_field_names`. Unlike
        // PHP/WASM which wrap their visitor handle as `WrapperType { inner: Arc<...> }`,
        // the NAPI binding stores the raw JS `napi::bindgen_prelude::Object` directly on
        // `JsConversionOptions.visitor`. There is no `.inner` field to dereference, so
        // the `(*v.inner).clone()` substitution would emit code that fails to compile.
        // Instead, the NAPI `convert` codegen attaches the visitor in a post-process
        // step after the `From<JsConversionOptions>` impl runs (`o.visitor = None;`
        // then `result.visitor = visitor_handle.clone()`), so the From impl harmlessly
        // emits `Default::default()` for the visitor field.
        let napi_conv_config = crate::codegen::conversions::ConversionConfig {
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
            // Json fields are stored as serde_json::Value in the binding so JS
            // callers can pass objects/arrays/scalars directly.
            json_as_value: true,
            never_skip_cfg_field_names: &never_skip_cfg_field_names,
            ..Default::default()
        };
        // From/Into conversions using shared parameterized generators.
        // Exclude Builder/Update DTOs — their struct definitions are filtered out of
        // emission upstream, so emitting `From<JsXxxUpdate> for core::XxxUpdate` would
        // reference an undefined `JsXxxUpdate` type.
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait)
            .filter(|typ| !typ.name.ends_with("Builder") && !typ.name.ends_with("Update"))
        {
            if input_types.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &napi_conv_config,
                ));
            }
            if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &napi_conv_config,
                ));
            }
        }
        let mut emitted_enum_binding_to_core: AHashSet<String> = AHashSet::new();
        for e in &api.enums {
            let has_data_variants = e.variants.iter().any(|v| !v.fields.is_empty());
            let is_tagged_data_enum = e.serde_tag.is_some() && has_data_variants;
            let is_untagged_data_enum = e.serde_untagged && has_data_variants;
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
            } else if is_untagged_data_enum {
                // Untagged data enums are wrapped around serde_json::Value — bridge via serde.
                let binding_name = format!("{prefix}{}", e.name);
                let core_path = crate::codegen::conversions::core_enum_path_remapped(
                    e,
                    &core_import,
                    napi_conv_config.source_crate_remaps,
                );
                builder.add_item(&format!(
                    "impl From<{binding_name}> for {core_path} {{\n    \
                         fn from(val: {binding_name}) -> Self {{\n        \
                             serde_json::from_value(val.0).unwrap_or_default()\n    \
                         }}\n\
                     }}\n"
                ));
                builder.add_item(&format!(
                    "impl From<{core_path}> for {binding_name} {{\n    \
                         fn from(val: {core_path}) -> Self {{\n        \
                             Self(serde_json::to_value(val).unwrap_or_default())\n    \
                         }}\n\
                     }}\n"
                ));
            } else {
                // Emit binding-to-core for plain data enums that are input types.
                // Even though NAPI string enums collapse to unit variants in the binding,
                // losing data-variant payloads, the binding-to-core conversion can still be
                // generated using `Default::default()` to fill lossy fields. This is needed
                // when structs containing these enums generate `From<binding>` impls that
                // call `.into()` on the enum fields (e.g., CustomProviderConfig.auth_header).
                //
                // Note: Enums NOT in input_types will be handled by the second enum loop
                // (lines 738+) which processes enums in fields of types with From impls.
                if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(
                        e,
                        &core_import,
                        &napi_conv_config,
                    ));
                    emitted_enum_binding_to_core.insert(e.name.clone());
                }
                // Always emit core-to-binding (unit variants 1:1, data variants discard data with `..`).
                // This ensures both directions are available for all plain enums in the API surface,
                // whether or not they appear in input types.
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(
                        e,
                        &core_import,
                        &napi_conv_config,
                    ));
                }
            }
        }

        // From impls for tagged data enums lowered to flat NAPI classes.
        // Track types whose `From<binding> for core` impl has already been emitted by
        // the main loop above (or by a prior variant in this loop) to avoid duplicate
        // impls when the same DTO appears both as a top-level input type and as a
        // variant payload of a tagged enum (e.g. `CrawlPageResult` used directly and
        // inside `CrawlEvent::Page { result: Box<CrawlPageResult> }`).
        // The main loop above emits a `From<binding> for core` impl for any type
        // that is `input_types.contains(&typ.name)`. Pre-seed the dedup set with those.
        let mut emitted_binding_to_core: AHashSet<String> = api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && input_types.contains(&typ.name))
            .filter(|typ| !typ.name.ends_with("Builder") && !typ.name.ends_with("Update"))
            .filter(|typ| crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core))
            .map(|typ| typ.name.clone())
            .collect();
        for enum_def in api.enums.iter() {
            let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
            let is_tagged_data_enum = enum_def.serde_tag.is_some() && has_data_variants;
            if !is_tagged_data_enum {
                continue;
            }
            // Also generate From impls for variant data types (e.g., BibtexMetadata from Metadata::Bibtex).
            // These are needed when tagged enum binding→core conversion calls `.into()` on variant fields.
            for variant in &enum_def.variants {
                for field in &variant.fields {
                    if let TypeRef::Named(type_name) = &field.ty {
                        if let Some(typ) = api.types.iter().find(|t| &t.name == type_name) {
                            if emitted_binding_to_core.contains(&typ.name) {
                                continue;
                            }
                            if crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core) {
                                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                                    typ,
                                    &core_import,
                                    &napi_conv_config,
                                ));
                                emitted_binding_to_core.insert(typ.name.clone());
                            }
                        }
                    }
                }
            }
        }

        // Emit From impls for all remaining DTO types that are convertible but haven't been
        // emitted yet. This handles nested types that appear as fields in output structures
        // but are not direct input types or enum variant payloads (e.g., DbfFieldInfo inside
        // Vec<DbfFieldInfo> in DbfMetadata, which itself is inside FormatMetadata::Dbf).
        for typ in api
            .types
            .iter()
            .filter(|t| !t.is_trait)
            .filter(|t| !t.name.ends_with("Builder") && !t.name.ends_with("Update"))
        {
            if !emitted_binding_to_core.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &napi_conv_config,
                ));
                emitted_binding_to_core.insert(typ.name.clone());
            }
        }

        // Emit From impls for enums that appear in fields of types we've already emitted
        // From impls for. These are needed because binding-to-core conversion code calls
        // `.into()` on enum fields when converting struct fields (e.g., CustomProviderConfig.auth_header).
        //
        // Handles both unit-variant enums (direct 1:1 mapping) and plain data enums
        // (collapses to unit variants in binding, uses Default::default() for lossy fields).
        // Skips only tagged/untagged data enums which have their own special conversion logic.
        for typ in api
            .types
            .iter()
            .filter(|t| !t.is_trait && emitted_binding_to_core.contains(&t.name))
        {
            for field in &typ.fields {
                // Recursively extract enum names from field types (e.g., Option<TextDirection> → TextDirection)
                fn collect_enum_names(ty: &TypeRef, enums: &mut AHashSet<String>) {
                    match ty {
                        TypeRef::Named(name) => {
                            enums.insert(name.clone());
                        }
                        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_enum_names(inner, enums),
                        TypeRef::Map(_k, v) => collect_enum_names(v, enums),
                        _ => {}
                    }
                }
                let mut field_enums = AHashSet::new();
                collect_enum_names(&field.ty, &mut field_enums);
                for enum_name in field_enums {
                    if let Some(enum_def) = api.enums.iter().find(|e| e.name == enum_name) {
                        // Only skip tagged/untagged data enums (they use custom conversion logic).
                        // Plain data enums (e.g., AuthHeaderFormat) still get binding-to-core impls.
                        let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
                        if enum_def.serde_tag.is_some() && has_data_variants {
                            continue;
                        }
                        if enum_def.serde_untagged && has_data_variants {
                            continue;
                        }
                        // Emit From impl for binding-to-core if not already emitted (handles both
                        // unit-variant and plain data enums; data enums use Default::default() for lossy fields)
                        if !emitted_enum_binding_to_core.contains(&enum_def.name)
                            && crate::codegen::conversions::can_generate_enum_conversion(enum_def)
                        {
                            builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(
                                enum_def,
                                &core_import,
                                &napi_conv_config,
                            ));
                            emitted_enum_binding_to_core.insert(enum_def.name.clone());
                        }
                    }
                }
            }
        }

        // Error types (variant name constants + converter functions)
        for error in &api.errors {
            builder.add_item(&crate::codegen::error_gen::gen_napi_error_types(error));
            builder.add_item(&crate::codegen::error_gen::gen_napi_error_converter(
                error,
                &core_import,
            ));
            // Emit #[napi] class for errors with introspection methods.
            let class_code = crate::codegen::error_gen::gen_napi_error_class(error, &core_import);
            if !class_code.is_empty() {
                builder.add_item(&class_code);
            }
        }

        let mut content = builder.build();

        // Post-process: Fix From<JsXxx> (binding to core) impls to forward visitor field.
        // The conversion generator emits `__result.visitor = Default::default();` in binding→core
        // conversions because the raw JS napi::bindgen_prelude::Object is not Clone-able.
        // This post-process detects that pattern in the JS→Rust direction and replaces it with
        // code that wraps the configured field into the generated bridge and then into the core handle type.
        //
        // Key: only fix `impl From<Js{type}>` (binding→core), NOT `impl From<core_type>` (core→binding).
        // The core→binding direction correctly uses Default because the Rc<RefCell<>> is opaque to JS.
        for bridge in &config.trait_bridges {
            if bridge.bind_via != crate::core::config::BridgeBinding::OptionsField {
                continue;
            }
            if let Some(field_name) = bridge.resolved_options_field() {
                // Verify the field is present in the binding struct. The NAPI backend keeps
                // cfg-gated fields in the binding (decorated with `#[cfg(...)]`) when their
                // names are in `never_skip_cfg_field_names` — every trait-bridge OptionsField
                // is registered there above (line 114-125), so accept cfg-gated fields too.
                let Some(options_type) = bridge.options_type.as_deref() else {
                    continue;
                };
                let field_in_binding = api
                    .types
                    .iter()
                    .filter(|t| t.name == options_type)
                    .flat_map(|t| t.fields.iter())
                    .any(|f| {
                        f.name == field_name
                            && (f.cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_name))
                    });
                if !field_in_binding {
                    continue;
                }

                // Find the binding→core conversion impl: `impl From<Js{options_type}> for core...`
                let prefix = config.node_type_prefix();
                let js_type_name = format!("{prefix}{options_type}");
                let impl_marker = format!("impl From<{js_type_name}> for {core_import}");

                // Search forward from the impl marker to find its closing brace and visitor wipe.
                // We only fix the impl that converts FROM the JS binding type.
                if let Some(impl_start) = content.find(&impl_marker) {
                    // Find the matching closing brace for this impl block
                    let from_impl_start = impl_start;
                    let impl_body = &content[from_impl_start..];

                    // Find the next `}` that closes this impl — being careful to count braces
                    let mut brace_depth = 0;
                    let mut impl_end = 0;
                    let mut found_fn_from = false;
                    for (i, ch) in impl_body.char_indices() {
                        if ch == '{' {
                            brace_depth += 1;
                            // Once we see the opening brace of `fn from(...) {`, mark it
                            if impl_body[..i].contains("fn from") {
                                found_fn_from = true;
                            }
                        } else if ch == '}' {
                            brace_depth -= 1;
                            if brace_depth == 0 && found_fn_from {
                                impl_end = i;
                                break;
                            }
                        }
                    }

                    if impl_end > 0 {
                        let impl_block = &impl_body[..impl_end];
                        let pattern = format!("__result.{field_name} = Default::default();");

                        if let Some(rel_pos) = impl_block.find(&pattern) {
                            let pos = from_impl_start + rel_pos;
                            let before = &content[..pos];
                            let after = &content[pos + pattern.len()..];

                            // Build the replacement that wraps the configured field into the configured bridge.
                            let handle_path =
                                crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge, &core_import);
                            let struct_name =
                                crate::codegen::generators::trait_bridge::bridge_wrapper_name("Js", bridge);
                            let replacement = format!(
                                "__result.{field_name} = val.{field_name}.and_then(|obj| {{\n            \
                                    {struct_name}::new(obj).ok().map(|bridge| {{\n                \
                                        std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n            \
                                    }})\n        \
                                }});"
                            );

                            content = format!("{}{}{}", before, replacement, after);
                        }
                    }
                }
            }
        }

        let output_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_type_stubs(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        type_stubs::generate(api, config)
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
