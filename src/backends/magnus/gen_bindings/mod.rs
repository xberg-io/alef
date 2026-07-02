//! Magnus (Ruby) backend for alef: orchestrates struct, enum, and function code generation.

mod classes;
pub mod functions;
mod method_result_wrap;
pub mod service_api;
mod streaming;
mod tagged_enums;

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
use tagged_enums::{gen_tagged_enum_ruby_classes, magnus_variant_wrapper_constructor};

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

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
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
        // The Magnus Rust glue emits one top-level free function per `api.functions` entry,
        // cfg-gating each variant via `prepend_cfg`. That keeps disjoint cfg-variants from
        // colliding ONLY while every same-named entry carries a cfg. When the surface contains
        // same-named entries that are NOT all cfg-gated (e.g. a re-exported real impl under
        // `#[cfg(feature = "X")]`, its `#[cfg(not(...))]` stub, AND an unconditional stub from a
        // separate `#[cfg(not(...))]` parent module whose gate did not propagate into the IR),
        // emitting them verbatim yields two simultaneously-active `fn <name>` definitions and a
        // duplicate-definition error (E0428). Collapse same-named entries to a single canonical
        // wrapper (cfg = OR of the group, dropping to unconditional when any member is ungated)
        // that delegates to the core crate, which resolves the cfg itself. See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

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
        if let Some(extra_attr) = crate::codegen::shared::format_extra_clippy_allows(&config.extra_clippy_allows) {
            builder.add_inner_attribute(&extra_attr);
        }
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
                let generates_default = typ.has_default;
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
                // Per-variant singleton constructors (`Shape.circle(radius)`); empty for enums with
                // no qualifying struct variant. Skipped for tagged data enums — they have a Ruby
                // `module` representation that collides with a Rust factory class, so the
                // constructors are not registered (see `module_init`); emitting the unused
                // `_factory_*` methods would also trip `-D warnings` (dead_code).
                if enum_def.serde_tag.is_none() {
                    let constructors = classes::gen_data_enum_variant_constructors(enum_def);
                    if !constructors.is_empty() {
                        builder.add_item(&constructors);
                    }
                }
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
                    let item = crate::backends::magnus::trait_bridge::gen_bridge_function(
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
                } else if let Some((options_param_idx, bridge_cfg)) =
                    crate::backends::magnus::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
                {
                    let item = crate::backends::magnus::trait_bridge::gen_options_field_bridge_function(
                        api,
                        func,
                        options_param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                    );
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
                } else {
                    let item = functions::gen_function(func, &mapper, &opaque_types, &mutex_types, &core_import, api);
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
                    if func.is_async {
                        let item = functions::gen_async_function(
                            func,
                            &mapper,
                            &opaque_types,
                            &mutex_types,
                            &core_import,
                            api,
                        );
                        let item = prepend_cfg(func.cfg.as_deref(), item);
                        builder.add_item(&item);
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
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
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
        // The RBS signature file is a single Ruby surface; same-named cfg-variant functions must
        // collapse to one `def self.<fn>` to avoid duplicate signature declarations. The Magnus
        // Rust glue (generate_bindings) deduplicates the same way. See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

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
        // The Ruby facade module is a single surface; same-named cfg-variant functions must
        // collapse to one method to avoid redefinition. The Magnus Rust glue (generate_bindings)
        // deduplicates the same way. See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

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

#[cfg(test)]
mod tests;
