//! Magnus (Ruby) backend for alef: orchestrates struct, enum, and function code generation.

mod classes;
pub mod functions;
pub mod service_api;
mod streaming;

use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use std::path::PathBuf;

use crate::backends::magnus::type_map::MagnusMapper;

/// Names that conflict with magnus imports or generated code.
/// `Error` conflicts with `magnus::Error`.
const MAGNUS_RESERVED_ENUM_NAMES: &[&str] = &["Error"];
const MAGNUS_RESERVED_FN_NAMES: &[&str] = &[];

pub(super) fn is_reserved_enum(name: &str) -> bool {
    MAGNUS_RESERVED_ENUM_NAMES.contains(&name)
}

pub(super) fn is_reserved_fn(name: &str) -> bool {
    MAGNUS_RESERVED_FN_NAMES.contains(&name)
}

pub struct MagnusBackend;

/// Convert crate name to PascalCase module name.
///
/// Handles both kebab-case (`parser-language-pack`) and snake_case
/// (`parser_language_pack`) inputs, since both are valid Cargo crate
/// name styles. The previous implementation split only on `-`, producing
/// `Sample_language_pack` for snake_case crates — which Rubocop
/// rejects as `Naming/ClassAndModuleCamelCase`.
fn get_module_name(crate_name: &str) -> String {
    use heck::ToUpperCamelCase;
    crate_name.to_upper_camel_case()
}

impl Backend for MagnusBackend {
    fn name(&self) -> &str {
        "magnus"
    }

    fn language(&self) -> Language {
        Language::Ruby
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
        let mapper = MagnusMapper;
        let core_import = config.core_import_name();

        // Per-language exclusion lists from [ruby] section of alef.toml
        let exclude_functions: std::collections::HashSet<&str> = config
            .ruby
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        // Service-owner types and handler-contract traits are marked binding_excluded
        // by the service extraction pass: they are emitted by the service-API codegen,
        // not the generic struct/trait codegen, so skip them in the generic loop too.
        let binding_excluded_names: Vec<String> = api
            .types
            .iter()
            .filter(|t| t.binding_excluded)
            .map(|t| t.name.clone())
            .collect();
        let mut exclude_types: std::collections::HashSet<&str> = config
            .ruby
            .as_ref()
            .map(|c| c.exclude_types.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        exclude_types.extend(binding_excluded_names.iter().map(|s| s.as_str()));

        let mut builder = RustFileBuilder::new().with_generated_header();
        // Match the inner-attribute set every other backend uses so that bookkeeping
        // helpers (cached names, intermediate bindings, conditional imports) introduced
        // by the codegen don't fire warnings in legitimately-functional generated code.
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute(
            "allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, \
             clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, \
             clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, \
             clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, \
             clippy::type_complexity, clippy::useless_conversion, clippy::clone_on_copy)",
        );
        builder.add_import(
            "magnus::{function, method, prelude::*, Error, Ruby, IntoValueFromNative, try_convert::TryConvertOwned}",
        );

        // Import traits needed for trait method dispatch
        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        // Only import HashMap when Map-typed fields or returns are present
        let has_maps = api.types.iter().any(|t| {
            t.fields
                .iter()
                .any(|f| matches!(&f.ty, crate::core::ir::TypeRef::Map(_, _)))
        }) || api
            .functions
            .iter()
            .any(|f| matches!(&f.return_type, crate::core::ir::TypeRef::Map(_, _)));
        if has_maps {
            builder.add_import("std::collections::HashMap");
        }

        // Compute module name early so it can be used for class paths in #[magnus::wrap]
        let module_name = get_module_name(&api.crate_name);

        // Collect streaming adapters: for each, we generate a custom iterator
        // wrapper struct + an instance method on the owning opaque type that
        // drives the Rust core stream natively (yielding to Ruby blocks /
        // returning an Enumerator). The default async-stub emission is bypassed
        // for these methods.
        let streaming_adapters: Vec<streaming::StreamingAdapter<'_>> = config
            .adapters
            .iter()
            .filter(|a| !a.skip_languages.iter().any(|l| l == "ruby"))
            .filter_map(|a| streaming::StreamingAdapter::from_config(a, &module_name, &core_import))
            .collect();
        let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.to_string()).collect();
        let mut streaming_methods_by_owner: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for adapter in &streaming_adapters {
            streaming_methods_by_owner
                .entry(adapter.owner_type.to_string())
                .or_default()
                .push(adapter.name.to_string());
        }
        let mut streaming_method_registrations: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for adapter in &streaming_adapters {
            streaming_method_registrations
                .entry(adapter.owner_type.to_string())
                .or_default()
                .push(streaming::gen_streaming_method_registration(adapter));
        }
        let mut streaming_iterator_registrations: Vec<String> = Vec::new();
        for adapter in &streaming_adapters {
            streaming_iterator_registrations.extend(streaming::gen_iterator_registration(adapter));
        }

        // Add imports needed by the streaming generators.
        if !streaming_adapters.is_empty() {
            builder.add_import("futures::StreamExt as _");
        }

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::Ruby);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        // Service-API glue lives in the generated `service.rs`; declare it so its
        // `#[magnus::function]` entrypoints are compiled and can be registered in
        // the module init.
        if !api.services.is_empty() {
            builder.add_item("pub mod service;");
        }

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        let mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
        }
        if !mutex_types.is_empty() {
            builder.add_import("std::sync::Mutex");
        }

        // Check if any data enum exists (needs json_to_ruby helper)
        let has_data_enum = api
            .enums
            .iter()
            .any(|e| e.variants.iter().any(|v| !v.fields.is_empty()));
        if has_data_enum {
            // Add json_to_ruby helper for converting serde_json::Value to Magnus values
            builder.add_item(
                "fn json_to_ruby(handle: &Ruby, val: serde_json::Value) -> magnus::Value {\n\
                 \x20   use magnus::IntoValue;\n\
                 \x20   match val {\n\
                 \x20       serde_json::Value::Null => handle.qnil().into_value_with(handle),\n\
                 \x20       serde_json::Value::Bool(b) => b.into_value_with(handle),\n\
                 \x20       serde_json::Value::Number(n) => {\n\
                 \x20           if let Some(i) = n.as_i64() { i.into_value_with(handle) }\n\
                 \x20           else if let Some(f) = n.as_f64() { f.into_value_with(handle) }\n\
                 \x20           else { handle.qnil().into_value_with(handle) }\n\
                 \x20       }\n\
                 \x20       serde_json::Value::String(s) => s.into_value_with(handle),\n\
                 \x20       serde_json::Value::Array(arr) => {\n\
                 \x20           let ruby_arr = handle.ary_new_capa(arr.len());\n\
                 \x20           for item in arr { let _ = ruby_arr.push(json_to_ruby(handle, item)); }\n\
                 \x20           ruby_arr.into_value_with(handle)\n\
                 \x20       }\n\
                 \x20       serde_json::Value::Object(map) => {\n\
                 \x20           let hash = handle.hash_new();\n\
                 \x20           for (k, v) in map {\n\
                 \x20               let key = handle.to_symbol(&k);\n\
                 \x20               let val = json_to_ruby(handle, v);\n\
                 \x20               let _ = hash.aset(key, val);\n\
                 \x20           }\n\
                 \x20           hash.into_value_with(handle)\n\
                 \x20       }\n\
                 \x20   }\n\
                 }",
            );
        }

        // Build set of types with has_default for validation
        let default_types: std::collections::HashSet<&str> = api
            .types
            .iter()
            .filter(|t| t.has_default)
            .map(|t| t.name.as_str())
            .collect();

        // Emit a single module-level `default_timeout` helper when any struct field
        // references it (per the `request_timeout` / `timeout` u64 default pattern in
        // struct_def.rs.jinja). Skipping this when no struct uses it avoids dead-code
        // warnings; emitting it here (rather than per-struct) avoids duplicate
        // definitions when multiple structs share the helper.
        let needs_default_timeout = api.types.iter().filter(|t| !t.is_trait && !t.is_opaque).any(|t| {
            t.fields.iter().any(|f| {
                let is_timeout_named = f.name == "request_timeout" || f.name == "timeout";
                let is_u64_or_duration = matches!(
                    f.ty,
                    crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::U64)
                        | crate::core::ir::TypeRef::Duration
                );
                is_timeout_named && is_u64_or_duration
            })
        });
        if needs_default_timeout {
            builder.add_item("fn default_timeout() -> u64 {\n    30000\n}");
        }

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(typ.name.as_str()) {
                continue;
            }
            if typ.is_opaque {
                builder.add_item(&classes::gen_opaque_struct(typ, &core_import, &module_name));
                builder.add_item(&classes::gen_opaque_struct_methods(
                    typ,
                    &mapper,
                    &opaque_types,
                    &mutex_types,
                    &core_import,
                    &streaming_method_names,
                ));
                // Append streaming methods in a separate impl block so they sit
                // alongside the auto-generated ones for this owner type.
                let owner_streaming: Vec<&streaming::StreamingAdapter<'_>> =
                    streaming_adapters.iter().filter(|a| a.owner_type == typ.name).collect();
                if !owner_streaming.is_empty() {
                    let mut impl_block = format!("impl {} {{\n", typ.name);
                    for adapter in &owner_streaming {
                        impl_block.push_str(&streaming::gen_streaming_method_body(adapter));
                    }
                    impl_block.push_str("}\n");
                    builder.add_item(&impl_block);
                }
                // Client constructor
                if let Some(ctor) = config.client_constructors.get(&typ.name) {
                    let ctor_body =
                        crate::codegen::generators::gen_opaque_constructor(ctor, &typ.name, &core_import, "");
                    let ctor_impl = format!("impl {} {{\n{}}}", typ.name, ctor_body);
                    builder.add_item(&ctor_impl);
                }
                // Variant-wrapper constructor — when the type is the target of one or more
                // registration-variant `wrapper_call`s (and therefore variant bodies emit
                // `WrapperType(args...)` constructor syntax), emit a static `new` on the
                // Magnus opaque struct. Magnus registers it as `define_singleton_method("new")`
                // in `ruby_init` so `WrapperType.new(args)` works from Ruby.
                //
                // `client_constructors` takes priority — if a constructor is already
                // configured explicitly, skip this auto-generated one to avoid duplicate `new`.
                else if typ.is_variant_wrapper && !config.client_constructors.contains_key(&typ.name) {
                    if let Some(ctor) = magnus_variant_wrapper_constructor(typ, &mapper, &core_import) {
                        builder.add_item(&ctor);
                    }
                }
            } else {
                let generates_default =
                    typ.has_default && crate::codegen::generators::can_generate_default_impl(typ, &default_types);
                // For Magnus bindings, always emit explicit Default impl when struct has fields.
                // This ensures serde deserialization uses correct field defaults instead of all-false/zeros.
                let has_explicit_impl_default = !typ.fields.is_empty();
                builder.add_item(&classes::gen_struct(
                    typ,
                    &mapper,
                    &module_name,
                    api,
                    has_explicit_impl_default,
                    &config.trait_bridges,
                ));
                if generates_default {
                    // Use Magnus-specific Default impl that delegates to core type's Default
                    // instead of field-level defaults. This preserves core semantics
                    // (e.g., SecurityLimits::default() returns proper limits, not 0).
                    builder.add_item(&classes::gen_magnus_default_impl(typ, &core_import));
                } else if has_explicit_impl_default {
                    // Generate explicit impl Default using field-level defaults
                    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
                    if let Some(impl_str) =
                        classes::gen_struct_default_impl_explicit(typ, &map_fn, &config.trait_bridges)
                    {
                        builder.add_item(&impl_str);
                    }
                }
                builder.add_item(&classes::gen_struct_methods(
                    typ,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    has_explicit_impl_default,
                    &config.trait_bridges,
                ));
            }
        }

        for enum_def in &api.enums {
            if !is_reserved_enum(&enum_def.name) && !exclude_types.contains(enum_def.name.as_str()) {
                builder.add_item(&classes::gen_enum(enum_def));
            }
        }

        for func in &api.functions {
            if !is_reserved_fn(&func.name) && !exclude_functions.contains(func.name.as_str()) {
                if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(
                    &func.name,
                    &config.trait_bridges,
                ) {
                    continue;
                }
                let bridge_param =
                    crate::backends::magnus::trait_bridge::find_bridge_param(func, &config.trait_bridges);
                if let Some((param_idx, bridge_cfg)) = bridge_param {
                    builder.add_item(&crate::backends::magnus::trait_bridge::gen_bridge_function(
                        api,
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &default_types,
                        &core_import,
                    ));
                } else if let Some((options_param_idx, bridge_cfg)) =
                    crate::backends::magnus::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
                {
                    builder.add_item(
                        &crate::backends::magnus::trait_bridge::gen_options_field_bridge_function(
                            api,
                            func,
                            options_param_idx,
                            bridge_cfg,
                            &mapper,
                            &opaque_types,
                            &core_import,
                        ),
                    );
                } else {
                    builder.add_item(&functions::gen_function(
                        func,
                        &mapper,
                        &opaque_types,
                        &mutex_types,
                        &core_import,
                        api,
                    ));
                    if func.is_async {
                        builder.add_item(&functions::gen_async_function(
                            func,
                            &mapper,
                            &opaque_types,
                            &mutex_types,
                            &core_import,
                            api,
                        ));
                    }
                }
            }
        }

        // Generate module-level wrapper functions for streaming adapters that have an owner type.
        // These allow calling `SampleCrawler.crawl_stream(engine, request)` at module level,
        // mirroring the pattern of non-streaming functions like `crawl`.
        for adapter in &streaming_adapters {
            builder.add_item(&streaming::gen_streaming_module_function(adapter));
        }

        // Trait bridge wrappers — generate Magnus bridge structs that delegate to Ruby objects.
        // Pass the host crate's canonical error type/constructor so generated `impl Plugin`
        // and `impl {Trait}` blocks match the trait signatures (e.g. `Result<T, SampleCrateError>`).
        // Check if any trait has async methods and add async_trait import if needed.
        if !config.trait_bridges.is_empty() {
            let needs_async_trait = config.trait_bridges.iter().any(|bridge_cfg| {
                api.types
                    .iter()
                    .find(|t| t.is_trait && t.name == bridge_cfg.trait_name)
                    .is_some_and(|trait_type| trait_type.methods.iter().any(|m| m.is_async))
            });
            if needs_async_trait {
                builder.add_import("async_trait::async_trait");
            }
        }

        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                let bridge_code = crate::backends::magnus::trait_bridge::gen_trait_bridge(
                    trait_type,
                    bridge_cfg,
                    &core_import,
                    &config.error_type_name(),
                    &config.error_constructor_expr(),
                    api,
                )?;
                let bridge_debug_count = bridge_code.matches("impl std::fmt::Debug").count();
                if bridge_debug_count != 1 {
                    eprintln!(
                        "[ALEF BUG] gen_trait_bridge returned {} Debug impls (expected 1) for {}",
                        bridge_debug_count, bridge_cfg.trait_name
                    );
                } else {
                    eprintln!(
                        "[ALEF OK] gen_trait_bridge returned {} Debug impl for {}",
                        bridge_debug_count, bridge_cfg.trait_name
                    );
                }
                builder.add_item(&bridge_code);
            }
        }

        // Magnus binding types are separate structs from core types and need From impls
        // for delegation. Generate both directions where possible.
        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = crate::codegen::conversions::input_type_names(api);
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque || exclude_types.contains(typ.name.as_str()) {
                continue;
            }
            let is_strict = crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core);
            let is_relaxed = crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding);
            if is_strict && input_types.contains(&typ.name) {
                // Use custom From impl generator that filters thread-unsafe fields (e.g., VisitorHandle)
                builder.add_item(&classes::gen_from_binding_to_core_filtered(
                    typ,
                    &core_import,
                    &config.trait_bridges,
                ));
            }
            if is_relaxed {
                // Use custom From impl generator that filters thread-unsafe fields (e.g., VisitorHandle)
                builder.add_item(&classes::gen_from_core_to_binding_filtered(
                    typ,
                    &core_import,
                    &opaque_types,
                    &config.trait_bridges,
                ));
            }
        }
        // Build the set of Named types referenced in enum variant fields that have no binding
        // struct. These are globally excluded types (e.g. PdfMetadata, ImageMetadata) that
        // the Magnus binding represents as String. The conversion config uses this list to
        // emit serde_json deserialization instead of .into() for those fields.
        let known_type_names: std::collections::HashSet<&str> = api
            .types
            .iter()
            .map(|t| t.name.as_str())
            .chain(api.enums.iter().map(|e| e.name.as_str()))
            .collect();
        let mut absent_named_types: Vec<String> = Vec::new();
        for e in &api.enums {
            for variant in &e.variants {
                for field in &variant.fields {
                    if let crate::core::ir::TypeRef::Named(name) = &field.ty {
                        if !known_type_names.contains(name.as_str()) && !absent_named_types.contains(name) {
                            absent_named_types.push(name.clone());
                        }
                    }
                }
            }
        }

        // Magnus generates data enums with fields (binding_enums_have_data = true).
        // Vec<Named> fields are kept as Vec<BindingType> in the enum struct (field_type_for_serde
        // recurses through Vec), so conversions use .into_iter().map(Into::into).collect() —
        // NOT serde_json round-trip. Only non-Named Vec types (Vec<String>, Vec<u8>, etc.) are
        // kept as-is. vec_named_to_string is intentionally false here.
        let magnus_conv_config = crate::codegen::conversions::ConversionConfig {
            binding_enums_have_data: true,
            // Magnus is the one backend whose enum body emits tuple-form `Variant(T)` for
            // `serde_untagged && variant.is_tuple` (see crates/alef-backend-magnus/templates/
            // enum_magnus.rs.jinja since commit a715f378). Conversion match arms must
            // destructure / construct in the same shape.
            binding_tuple_form_for_untagged_variants: true,
            exclude_types: &absent_named_types,
            ..Default::default()
        };
        for e in &api.enums {
            if exclude_types.contains(e.name.as_str()) {
                continue;
            }
            if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(
                    e,
                    &core_import,
                    &magnus_conv_config,
                ));
            }
            if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(
                    e,
                    &core_import,
                    &magnus_conv_config,
                ));
            }
        }

        // Error converter functions + optional introspection method structs
        for error in &api.errors {
            builder.add_item(&crate::codegen::error_gen::gen_magnus_error_converter(
                error,
                &core_import,
            ));
            // Emit Magnus-wrapped struct for errors with whitelisted introspection methods.
            let methods_struct = crate::codegen::error_gen::gen_magnus_error_methods_struct(error, &core_import);
            if !methods_struct.is_empty() {
                builder.add_item(&methods_struct);
            }
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Ruby)?;

        // Emit streaming iterator wrapper structs (e.g. ChatStreamIterator) plus
        // their inherent `next_chunk` / `each` methods. These are appended after
        // all opaque types so the iterator type names cannot shadow user types.
        for adapter in &streaming_adapters {
            builder.add_item(&streaming::gen_iterator_struct(adapter));
        }

        builder.add_item(&functions::gen_module_init(
            &module_name,
            api,
            config,
            &exclude_functions,
            &exclude_types,
            &streaming_methods_by_owner,
            &streaming_iterator_registrations,
            &streaming_method_registrations,
            &streaming_adapters,
        ));

        let content = builder.build();

        let output_dir = resolve_output_dir(
            config.output_paths.get("ruby"),
            &config.name.replace('-', "_"),
            "packages/ruby/ext/{name}_rb/native/src/",
        );

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
        let stubs_config = match config.ruby.as_ref().and_then(|c| c.stubs.as_ref()) {
            Some(s) => s,
            None => return Ok(vec![]),
        };

        let core_import = config.core_import_name();
        let gem_name = config.ruby_gem_name();
        let emit_docstrings = stubs_config.emit_docstrings;

        // Build streaming method names for RBS generation
        let streaming_adapters: Vec<_> = config
            .adapters
            .iter()
            .filter(|a| !a.skip_languages.iter().any(|l| l == "ruby"))
            .filter_map(|a| streaming::StreamingAdapter::from_config(a, &get_module_name(&gem_name), &core_import))
            .collect();
        let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.to_string()).collect();

        let content = crate::backends::magnus::gen_stubs::gen_stubs(
            api,
            &gem_name,
            emit_docstrings,
            &streaming_method_names,
            &config.trait_bridges,
        );

        let stubs_path = resolve_output_dir(
            Some(&stubs_config.output),
            &config.name,
            stubs_config.output.to_string_lossy().as_ref(),
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&stubs_path).join("types.rbs"),
            content,
            generated_header: true,
        }])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let gem_name = config.ruby_gem_name();
        let gem_name_snake = gem_name.replace('-', "_");
        let module_name = get_module_name(&gem_name);
        let ext_name = format!("{}_rb", config.core_crate_dir().replace('-', "_"));

        // Generate the main Ruby wrapper module file
        let mut content = hash::header(CommentStyle::Hash);
        content.push_str(
            crate::backends::magnus::template_env::render(
                "main_rb_wrapper.rb.jinja",
                minijinja::context! {
                    gem_name_snake => gem_name_snake,
                    module_name => module_name,
                    has_services => !api.services.is_empty(),
                },
            )
            .trim_end_matches('\n'),
        );
        content.push('\n');

        // Build explicit re-export lists: filter out excluded types and Update/Builder types.
        // Also skip binding-excluded types (service owners / handler-contract traits) — they
        // are exported by the service-API codegen, not the generic struct re-export list.
        let binding_excluded_names: Vec<String> = api
            .types
            .iter()
            .filter(|t| t.binding_excluded)
            .map(|t| t.name.clone())
            .collect();
        let mut exclude_types: std::collections::HashSet<&str> = config
            .ruby
            .as_ref()
            .map(|c| c.exclude_types.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        exclude_types.extend(binding_excluded_names.iter().map(|s| s.as_str()));

        let exclude_functions: std::collections::HashSet<&str> = config
            .ruby
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        // Collect public types: include all non-opaque, non-trait struct types,
        // except those in exclude_types and those ending with "Update" or "Builder".
        // NOTE: Enums are NOT included because Magnus backend does not register them as
        // module-level constants (only structs are registered in gen_module_init). Attempting
        // to const_get an enum raises NameError.
        let mut public_types = Vec::new();
        for typ in &api.types {
            if !typ.is_trait && !typ.is_opaque && !exclude_types.contains(typ.name.as_str()) {
                // Filter out partial-update structs (*Update) and builder types (*Builder)
                if !typ.name.ends_with("Update") && !typ.name.ends_with("Builder") {
                    public_types.push(typ.name.clone());
                }
            }
        }
        public_types.sort();

        // Collect public functions: include all non-excluded functions.
        let mut public_functions = Vec::new();
        for func in &api.functions {
            if !is_reserved_fn(&func.name) && !exclude_functions.contains(func.name.as_str()) {
                public_functions.push(func.name.clone());
            }
        }
        public_functions.sort();

        // Generate the native.rb file that requires the extension and re-exports its symbols
        let native_module_name = get_module_name(&api.crate_name);
        let mut native_content = hash::header(CommentStyle::Hash);
        native_content.push_str(
            crate::backends::magnus::template_env::render(
                "native_rb_wrapper.rb.jinja",
                minijinja::context! {
                    ext_name => ext_name,
                    module_name => module_name,
                    native_module_name => native_module_name,
                    public_types => public_types,
                    public_functions => public_functions,
                },
            )
            .trim_end_matches('\n'),
        );

        // Emit class-based tagged enum hierarchies for all internally-tagged data enums.
        // Each tagged sum type gets a base class with predicate methods and per-variant
        // subclasses with typed attribute readers and Sorbet sig blocks.
        for enum_def in &api.enums {
            if enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty()) {
                native_content.push('\n');
                native_content.push_str(&gen_tagged_enum_ruby_classes(enum_def, &module_name));
            }
        }

        native_content.push('\n');

        // Generate the version file. RubyGems rejects cargo's dash-form prerelease
        // syntax (e.g. `Gem::Version.new("1.8.0-rc.2")` raises), so write the
        // canonical `.pre.` form here. `alef verify` performs the same conversion
        // when comparing on-disk content.
        let cargo_version = if api.version.is_empty() {
            "0.0.0".to_string()
        } else {
            api.version.clone()
        };
        let version = crate::core::version::to_rubygems_prerelease(&cargo_version);

        let mut version_content = hash::header(CommentStyle::Hash);
        version_content.push_str(
            crate::backends::magnus::template_env::render(
                "version_rb_wrapper.rb.jinja",
                minijinja::context! {
                    module_name => module_name,
                    version => version,
                },
            )
            .trim_end_matches('\n'),
        );
        version_content.push('\n');

        let output_dir = resolve_output_dir(config.output_paths.get("ruby_lib"), &config.name, "packages/ruby/lib/");

        Ok(vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join(format!("{gem_name_snake}.rb")),
                content,
                generated_header: false,
            },
            GeneratedFile {
                path: PathBuf::from(&output_dir).join(format!("{gem_name_snake}/native.rb")),
                content: native_content,
                generated_header: false,
            },
            GeneratedFile {
                path: PathBuf::from(&output_dir).join(format!("{gem_name_snake}/version.rb")),
                content: version_content,
                generated_header: false,
            },
        ])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-rb",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

/// Map a field TypeRef to a Sorbet type string for use in `sig` blocks emitted in `.rb` files.
fn sorbet_type_for_field(ty: &crate::core::ir::TypeRef, optional: bool) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let base = match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "T::Boolean".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_string(),
            _ => "Integer".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Bytes => "String".to_string(),
        TypeRef::Vec(inner) => format!("T::Array[{}]", sorbet_type_for_field(inner, false)),
        TypeRef::Map(k, v) => format!(
            "T::Hash[{}, {}]",
            sorbet_type_for_field(k, false),
            sorbet_type_for_field(v, false)
        ),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Optional(inner) => return format!("T.nilable({})", sorbet_type_for_field(inner, false)),
        TypeRef::Duration => "Integer".to_string(),
        TypeRef::Json | TypeRef::Unit => "T.untyped".to_string(),
    };
    if optional { format!("T.nilable({base})") } else { base }
}

/// Generate a Ruby marker module and Data.define variants for an internally-tagged enum.
///
/// Emits:
/// - A marker module with `interface!` and `abstract!` Sorbet annotations, plus a
///   dispatcher `from_hash(hash)` that routes to the appropriate variant constructor
///   based on the discriminator field.
/// - A `Data.define(...)` per variant that includes the marker module, with typed
///   attribute accessors, variant predicate methods, and a per-variant `from_hash` factory.
///
/// This is the Ruby 3.2+ idiomatic pattern for sealed sum types using Data classes
/// mixed into marker modules. Each variant instance `is_a?(MarkerModule)` returns true.
fn gen_tagged_enum_ruby_classes(enum_def: &crate::core::ir::EnumDef, module_name: &str) -> String {
    use crate::codegen::doc_emission::emit_yard_doc;
    let mut out = String::new();

    let class_name = &enum_def.name;
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("kind");

    // --- Marker module ---
    let mut doc_comment = String::new();
    if !enum_def.doc.is_empty() {
        emit_yard_doc(&mut doc_comment, &enum_def.doc, "  ");
    } else {
        doc_comment.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_marker_doc.rb.jinja",
            minijinja::context! {
                class_name => class_name,
            },
        ));
    }
    // `interface!` already declares the module abstract; calling `abstract!` again
    // raises `T::Private::Abstract::Declare: already declared as abstract`.
    let mut dispatch_arms = String::new();
    for variant in &enum_def.variants {
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref().or(Some("snake_case")),
        );
        let variant_const = format!("{}{}", class_name, &variant.name);
        dispatch_arms.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_dispatch_arm.rb.jinja",
            minijinja::context! {
                wire_name => wire_name,
                variant_const => variant_const,
            },
        ));
    }
    out.push_str(&crate::backends::magnus::template_env::render(
        "tagged_enum_marker_module.rb.jinja",
        minijinja::context! {
            module_name => module_name,
            class_name => class_name,
            doc_comment => doc_comment,
            tag_field => tag_field,
            dispatch_arms => dispatch_arms,
        },
    ));

    // --- Per-variant Data.define classes ---
    for variant in &enum_def.variants {
        let variant_class = format!("{}{}", class_name, &variant.name);
        let field_names: Vec<&str> = variant.fields.iter().map(|f| f.name.as_str()).collect();

        let mut doc_comment = String::new();
        if !variant.doc.is_empty() {
            emit_yard_doc(&mut doc_comment, &variant.doc, "  ");
        } else {
            doc_comment.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_variant_doc.rb.jinja",
                minijinja::context! {
                    variant_class => &variant_class,
                    class_name => class_name,
                },
            ));
        }

        // Data.define(...) declaration — Ruby requires symbol arguments
        let symbol_args = if field_names.is_empty() {
            String::new()
        } else {
            variant
                .fields
                .iter()
                .map(|f| {
                    let attr_name = if f.name == "_0" { "value" } else { f.name.as_str() };
                    format!(":{attr_name}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        // Sorbet sigs for the Data attributes — wrap auto-generated accessors via super
        let mut field_accessors = String::new();
        for field in &variant.fields {
            let attr_name = if field.name == "_0" {
                "value"
            } else {
                field.name.as_str()
            };
            let sorbet_t = sorbet_type_for_field(&field.ty, field.optional);
            let mut doc_comment = String::new();
            if !field.doc.is_empty() {
                emit_yard_doc(&mut doc_comment, &field.doc, "    ");
            }
            // Wrap the Data-auto-generated accessor so the sig has a method to attach to.
            // `# rubocop:disable Lint/UselessMethodDefinition` keeps `rubocop -a` from
            // stripping the def (which would leave the sig orphaned and break Sorbet).
            field_accessors.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_field_accessor.rb.jinja",
                minijinja::context! {
                    doc_comment => doc_comment,
                    sorbet_t => sorbet_t,
                    attr_name => attr_name,
                },
            ));
        }

        // Variant predicate methods (return true for this variant, false for others)
        let mut predicate_methods = String::new();
        for variant_name in &variant_names {
            let v_snake = crate::codegen::naming::pascal_to_snake(variant_name);
            let returns_true = *variant_name == variant.name;
            predicate_methods.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_predicate_method.rb.jinja",
                minijinja::context! {
                    predicate_name => v_snake,
                    returns_true => returns_true,
                },
            ));
        }

        // Class-level `from_hash` factory — per-variant
        let field_args: Vec<String> = variant
            .fields
            .iter()
            .map(|f| {
                let key_sym = if f.name == "_0" {
                    ":_0".to_string()
                } else {
                    format!(":{}", f.name)
                };
                let param_name = if f.name == "_0" {
                    "value".to_string()
                } else {
                    f.name.clone()
                };
                let key_string = if f.name == "_0" { "_0" } else { f.name.as_str() };
                let val_expr = format!("hash[{key_sym}] || hash[\"{key_string}\"]");
                format!("{param_name}: {val_expr}")
            })
            .collect();
        let from_hash_call = if field_args.is_empty() {
            "new".to_string()
        } else {
            format!("new({})", field_args.join(", "))
        };

        // YARD documents constants only when the preceding comment uses `##`.
        // emit_yard_doc / tagged_enum_variant_doc.rb.jinja both emit single-`#`
        // form; promote to double for the constant-assignment site.
        let doc_comment = doc_comment.replace("  # ", "  ## ");

        out.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_variant_class.rb.jinja",
            minijinja::context! {
                doc_comment => doc_comment,
                symbol_args => symbol_args,
                variant_class => variant_class,
                class_name => class_name,
                field_accessors => field_accessors,
                predicate_methods => predicate_methods,
                from_hash_call => from_hash_call,
            },
        ));
    }

    // The variant loop leaves a trailing blank line; drop it so the enclosing
    // module body doesn't end with an empty line (Layout/EmptyLinesAroundModuleBody).
    if out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str("end\n");
    out
}

/// For a variant-wrapper opaque type (one whose `is_variant_wrapper` flag is set
/// by the extractor), emit a static `pub fn new(...)` on the Magnus binding struct
/// so that `define_singleton_method("new", function!(TypeName::new, N))` in
/// `ruby_init` resolves to a real Rust function.
///
/// The generated `new` creates a core instance via `CoreType::new(args)` and wraps
/// it in `Arc` — matching the opaque struct layout produced by `gen_opaque_struct`.
///
/// Returns `None` when the wrapper has no `new` method in the IR (or its receiver
/// is not `None`), in which case the variant body would not compile either but we
/// silently skip rather than panic so the rest of the surface can still be generated.
fn magnus_variant_wrapper_constructor(
    typ: &crate::core::ir::TypeDef,
    mapper: &MagnusMapper,
    core_import: &str,
) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);
    // Wrap binding-side newtype arguments (TypeRef::Named) with `.into()` so
    // they coerce to the core type before being passed to `<core>::new`.
    // Skip `String` and primitive types where `.into()` would produce
    // ambiguous type inference against multi-impl targets like
    // `impl Into<String>`. Binding-side scalars are already core-shaped, so
    // a bare pass-through is correct for them.
    let needs_into = |t: &crate::core::ir::TypeRef| -> bool {
        matches!(
            t,
            crate::core::ir::TypeRef::Named(_)
                | crate::core::ir::TypeRef::Optional(_)
                | crate::core::ir::TypeRef::Vec(_)
                | crate::core::ir::TypeRef::Map(_, _)
        )
    };
    let call_args = ctor
        .params
        .iter()
        .map(|p| {
            if needs_into(&p.ty) {
                format!("{}.into()", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let body = if call_args.is_empty() {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new()) }}")
    } else {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new({call_args})) }}")
    };
    let fn_sig = if sig_params.is_empty() {
        "pub fn new() -> Self".to_string()
    } else {
        format!("pub fn new({sig_params}) -> Self")
    };
    Some(format!(
        "impl {name} {{\n    {fn_sig} {{\n        {body}\n    }}\n}}\n",
        name = typ.name,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backend::Backend;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::ir::*;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn make_config() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
        )
    }

    fn make_api_surface() -> ApiSurface {
        ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "Config".to_string(),
                rust_path: "test_lib::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "timeout".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            }],
            functions: vec![FunctionDef {
                name: "process".to_string(),
                rust_path: "test_lib::process".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    #[test]
    fn generates_magnus_module_init() {
        let backend = MagnusBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        assert!(content.contains("#[magnus::init]"), "must emit #[magnus::init]");
    }

    #[test]
    fn generates_struct_with_magnus_wrap() {
        let backend = MagnusBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        let content = &files[0].content;
        assert!(
            content.contains("magnus::wrap"),
            "structs must have magnus::wrap attribute"
        );
        assert!(content.contains("struct Config"), "Config struct must be generated");
    }

    #[test]
    fn generate_public_api_emits_gem_files() {
        let backend = MagnusBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        assert_eq!(files.len(), 3, "must generate main rb file + native.rb + version file");
        let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
        assert!(
            paths.iter().any(|p| p.ends_with("test_lib.rb")),
            "must have main gem file"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("native.rb")),
            "must have native.rb file"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("version.rb")),
            "must have version file"
        );
    }

    #[test]
    fn output_path_defaults_to_packages_ruby() {
        let backend = MagnusBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        assert!(
            files[0].path.to_string_lossy().contains("lib.rs"),
            "output must be lib.rs"
        );
    }

    #[test]
    fn test_explicit_re_export_list_filters_internal_types() {
        // Verify that generate_public_api includes only struct types in the re-export list,
        // filtering out enums (which are not registered on the native module).
        let backend = MagnusBackend;

        // Create a custom config where module_name != native_module_name
        // (so the template emits the re-export block).
        // api.crate_name "my_lib" → native_module_name "MyLib"
        // gem_name "my_gem" → module_name "MyGem" (different!)
        let cfg_str = r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "my_lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "my_gem"
"#;
        let cfg: NewAlefConfig = toml::from_str(cfg_str).unwrap();
        let config = cfg.resolve().unwrap().remove(0);

        let mut api = make_api_surface();
        api.crate_name = "my_lib".to_string();
        // Add an enum to the API surface
        api.enums.push(EnumDef {
            name: "Status".to_string(),
            rust_path: "sample_markdown::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: String::new(),
            serde_tag: None,
            serde_rename_all: None,
            serde_untagged: false,
            is_copy: false,
            has_serde: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            cfg: None,
        });

        let files = backend.generate_public_api(&api, &config).unwrap();
        let native_file = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("native.rb"))
            .expect("native.rb must exist");

        // Verify that the enum (Status) is NOT in the re-export list via const_get
        assert!(
            !native_file.content.contains("const_get(:Status)"),
            "enum types must not be in re-export list"
        );
        // Verify that the struct type (Config) IS in the re-export list via const_get
        assert!(
            native_file.content.contains("const_get(:Config)"),
            "struct types must be in re-export list via const_get"
        );
    }
}
