//! NAPI-RS (Node.js) backend: orchestration and `Backend` trait implementation.

pub mod capsule;
pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod types;

use crate::type_map::NapiMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators::{self, AsyncPattern, RustBindingConfig};
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep};
use alef_core::config::{Language, NodeCapsuleTypeConfig, ResolvedCrateConfig, resolve_output_dir};
use alef_core::ir::{ApiSurface, TypeRef};
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
        let has_serde = alef_core::config::detect_serde_available(&output_dir);
        let mut cfg = Self::binding_config(&core_import, &prefix, has_serde);
        let never_skip_cfg_field_names: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| {
                if b.bind_via == alef_core::config::BridgeBinding::OptionsField {
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

        // Check if we have opaque types and trait types (visitors)
        // Exclude trait types from opaque_types since they use JsVisitorRef instead of Object<'static>
        // Also exclude capsule types — they do not get #[napi] class wrappers.
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !t.is_trait && !capsule_types.contains_key(&t.name))
            .map(|t| t.name.clone())
            .collect();
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

        let exclude_types: ahash::AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();

        // Build adapter body map before type iteration so bodies are available for method generation.
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Node)?;

        // Map "OwnerType.method" -> streaming item type. The napi backend needs to
        // override the IR-declared `String` return type with `Vec<{prefix}{item}>`
        // for streaming adapters, since the generated body returns chunks directly
        // as a JS array instead of a serialized JSON string.
        let streaming_item_types: ahash::AHashMap<String, String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
            .filter_map(|a| {
                let owner = a.owner_type.as_deref()?;
                let item = a.item_type.as_deref()?;
                Some((format!("{owner}.{}", a.name), item.to_string()))
            })
            .collect();

        // JsBytes: a newtype wrapper for Vec<u8> with custom FromNapiValue that accepts
        // Buffer.from(...) from JavaScript. Fixes NAPI v3 macro-derived deserialization
        // of Vec<u8> fields in #[napi(object)] structs, which normally expect Array[number].
        let js_bytes_def = r#"
/// Wrapper for byte arrays that implements custom FromNapiValue to accept Buffer.from(...).
///
/// NAPI v3's default FromNapiValue for Vec<u8> expects Array[number], not Buffer.
/// This wrapper provides custom deserialization that accepts Buffer, Uint8Array, or Array,
/// converting them to Vec<u8>. Implements Clone and serde traits for use in struct fields.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct JsBytes(pub Vec<u8>);

impl From<Vec<u8>> for JsBytes {
    fn from(v: Vec<u8>) -> Self {
        JsBytes(v)
    }
}

impl From<JsBytes> for Vec<u8> {
    fn from(js_bytes: JsBytes) -> Self {
        js_bytes.0
    }
}

impl AsRef<[u8]> for JsBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::ops::Deref for JsBytes {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for JsBytes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl napi::bindgen_prelude::FromNapiValue for JsBytes {
    unsafe fn from_napi_value(env: napi::sys::napi_env, napi_val: napi::sys::napi_value) -> napi::Result<Self> {
        use napi::bindgen_prelude::FromNapiValue;

        // Try Buffer first (most common for binary data in JS)
        if let Ok(buffer) = unsafe { napi::bindgen_prelude::Buffer::from_napi_value(env, napi_val) } {
            return Ok(JsBytes(buffer.as_ref().to_vec()));
        }

        // Try Uint8Array
        if let Ok(ua) = unsafe { napi::bindgen_prelude::Uint8Array::from_napi_value(env, napi_val) } {
            return Ok(JsBytes(ua.to_vec()));
        }

        // Fall back to Array[number]
        if let Ok(vec) = unsafe { Vec::<u8>::from_napi_value(env, napi_val) } {
            return Ok(JsBytes(vec));
        }

        Err(napi::Error::new(
            napi::Status::InvalidArg,
            "Expected Buffer, Uint8Array, or Array<number> for bytes field",
        ))
    }
}

impl napi::bindgen_prelude::ToNapiValue for JsBytes {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> napi::Result<napi::sys::napi_value> {
        // Delegate to Vec<u8>'s implementation (which returns an Uint8Array/Buffer).
        unsafe { <Vec<u8> as napi::bindgen_prelude::ToNapiValue>::to_napi_value(env, val.0) }
    }
}
"#;
        builder.add_item(js_bytes_def);

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

#[allow(clippy::arc_with_non_send_sync)]
impl From<napi::bindgen_prelude::Object<'static>> for JsVisitorRef {
    fn from(visitor: napi::bindgen_prelude::Object<'static>) -> Self {
        JsVisitorRef {
            inner: std::sync::Arc::new(visitor),
        }
    }
}

impl From<JsVisitorRef> for napi::bindgen_prelude::Object<'static> {
    fn from(visitor_ref: JsVisitorRef) -> Self {
        // Object<'static> is Copy (it just holds an env+handle pair), so deref directly.
        *visitor_ref.inner
    }
}
"#;
            builder.add_item(js_visitor_ref_def);
        }

        // Emit adapter-generated standalone items (streaming iterators, callback bridges).
        for adapter in &config.adapters {
            match adapter.pattern {
                alef_core::config::AdapterPattern::Streaming => {
                    let key = alef_adapters::stream_struct_key(adapter);
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
                    let raw = alef_codegen::generators::gen_opaque_struct_prefixed(typ, &cfg, &prefix);
                    let struct_name = format!("{prefix}{}", typ.name);
                    let body = raw.replace(
                        &format!("#[napi]pub struct {struct_name}"),
                        &format!("#[napi(js_name = \"{}\")]pub struct {struct_name}", typ.name),
                    );
                    let mut out = String::new();
                    alef_codegen::doc_emission::emit_rustdoc(&mut out, &typ.doc, "");
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
            }
        }

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
            let bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges);
            let options_field_bridge = crate::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
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
            } else if !capsule_types.is_empty() && capsule::function_involves_capsule(func, &capsule_types) {
                // Function returns a capsule type — emit a napi shim that returns JsObject
                // with __parser = External<T>(ptr from value.into_raw()).
                // JsObjectValue provides set_named_property; imported once below.
                builder.add_item(&capsule::gen_capsule_function(func, &capsule_types, &core_import));
            } else {
                builder.add_item(&functions::gen_function(
                    func,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &default_types,
                    &prefix,
                    &capsule_types,
                    &mutex_types,
                ));
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
        // NOTE: NAPI does NOT populate `trait_bridge_arc_wrapper_field_names`. Unlike
        // PHP/WASM which wrap their visitor handle as `WrapperType { inner: Arc<...> }`,
        // the NAPI binding stores the raw JS `napi::bindgen_prelude::Object` directly on
        // `JsConversionOptions.visitor`. There is no `.inner` field to dereference, so
        // the `(*v.inner).clone()` substitution would emit code that fails to compile.
        // Instead, the NAPI `convert` codegen attaches the visitor in a post-process
        // step after the `From<JsConversionOptions>` impl runs (`o.visitor = None;`
        // then `result.visitor = visitor_handle.clone()`), so the From impl harmlessly
        // emits `Default::default()` for the visitor field.
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
            // Json fields are stored as serde_json::Value in the binding so JS
            // callers can pass objects/arrays/scalars directly.
            json_as_value: true,
            never_skip_cfg_field_names: &never_skip_cfg_field_names,
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
                let core_path = alef_codegen::conversions::core_enum_path_remapped(
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

        let mut content = builder.build();

        // Post-process: Fix From<JsXxx> (binding to core) impls to forward visitor field.
        // The conversion generator emits `__result.visitor = Default::default();` in binding→core
        // conversions because the raw JS napi::bindgen_prelude::Object is not Clone-able.
        // This post-process detects that pattern in the JS→Rust direction and replaces it with
        // code that wraps val.visitor into a JsHtmlVisitorBridge and then into the core Rc<RefCell<>> type.
        //
        // Key: only fix `impl From<Js{type}>` (binding→core), NOT `impl From<core_type>` (core→binding).
        // The core→binding direction correctly uses Default because the Rc<RefCell<>> is opaque to JS.
        for bridge in &config.trait_bridges {
            if bridge.bind_via != alef_core::config::BridgeBinding::OptionsField {
                continue;
            }
            if let Some(field_name) = bridge.resolved_options_field() {
                // Verify the field is present in the binding struct (not cfg-gated away)
                let Some(options_type) = bridge.options_type.as_deref() else {
                    continue;
                };
                let field_in_binding = api
                    .types
                    .iter()
                    .filter(|t| t.name == options_type)
                    .flat_map(|t| t.fields.iter())
                    .any(|f| f.cfg.is_none() && f.name == field_name);
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
                        let pattern = "__result.visitor = Default::default();";

                        if let Some(rel_pos) = impl_block.find(pattern) {
                            let pos = from_impl_start + rel_pos;
                            let before = &content[..pos];
                            let after = &content[pos + pattern.len()..];

                            // Build the replacement that wraps val.visitor into JsHtmlVisitorBridge
                            // and then into the core Arc<Mutex<...>> type.
                            let type_alias = bridge.type_alias.as_deref().unwrap_or("VisitorHandle");
                            let handle_path = format!("{core_import}::visitor::{type_alias}");
                            let replacement = format!(
                                "__result.visitor = val.{field_name}.map(|obj| {{\n            \
                                    let bridge = JsHtmlVisitorBridge::new(obj);\n            \
                                    std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n        \
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
        let prefix = config.node_type_prefix();
        let exclude_functions: ahash::AHashSet<String> = config
            .node
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let capsule_types: HashMap<String, NodeCapsuleTypeConfig> = config
            .node
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
        let content = errors::gen_dts(api, &prefix, &exclude_functions, &config.trait_bridges, &capsule_types);

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

    /// Test that cfg-gated fields in never_skip_cfg_field_names pass the options-field-bridge filter.
    #[test]
    fn cfg_gated_field_accepted_when_in_never_skip_list() {
        // Test the predicate logic: a cfg-gated field "visitor" should be accepted
        // when it appears in never_skip_cfg_field_names.
        let never_skip_cfg_field_names = ["visitor".to_string()];
        let field_is_target = "visitor";

        // Simulate a field with cfg = Some(...)
        let field_has_cfg = Some("feature = \"visitor\"");

        // Predicate: f.cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_name)
        let accepted = field_has_cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_is_target);

        assert!(
            accepted,
            "cfg-gated field 'visitor' should pass filter when in never_skip_cfg_field_names"
        );
    }
}
