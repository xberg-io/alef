//! Magnus (Ruby) backend for alef: orchestrates struct, enum, and function code generation.

mod classes;
pub mod functions;
mod streaming;

use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

use crate::type_map::MagnusMapper;

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
/// Handles both kebab-case (`tree-sitter-language-pack`) and snake_case
/// (`tree_sitter_language_pack`) inputs, since both are valid Cargo crate
/// name styles. The previous implementation split only on `-`, producing
/// `Tree_sitter_language_pack` for snake_case crates — which Rubocop
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
        let exclude_types: std::collections::HashSet<&str> = config
            .ruby
            .as_ref()
            .map(|c| c.exclude_types.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

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
                .any(|f| matches!(&f.ty, alef_core::ir::TypeRef::Map(_, _)))
        }) || api
            .functions
            .iter()
            .any(|f| matches!(&f.return_type, alef_core::ir::TypeRef::Map(_, _)));
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
                    alef_core::ir::TypeRef::Primitive(alef_core::ir::PrimitiveType::U64)
                        | alef_core::ir::TypeRef::Duration
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
            } else {
                let generates_default =
                    typ.has_default && alef_codegen::generators::can_generate_default_impl(typ, &default_types);
                // For Magnus bindings, always emit explicit Default impl when struct has fields.
                // This ensures serde deserialization uses correct field defaults instead of all-false/zeros.
                let has_explicit_impl_default = !typ.fields.is_empty();
                builder.add_item(&classes::gen_struct(
                    typ,
                    &mapper,
                    &module_name,
                    api,
                    has_explicit_impl_default,
                ));
                if generates_default {
                    // Use Magnus-specific Default impl that delegates to core type's Default
                    // instead of field-level defaults. This preserves core semantics
                    // (e.g., SecurityLimits::default() returns proper limits, not 0).
                    builder.add_item(&classes::gen_magnus_default_impl(typ, &core_import));
                } else if has_explicit_impl_default {
                    // Generate explicit impl Default using field-level defaults
                    let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);
                    if let Some(impl_str) = classes::gen_struct_default_impl_explicit(typ, &map_fn) {
                        builder.add_item(&impl_str);
                    }
                }
                builder.add_item(&classes::gen_struct_methods(
                    typ,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    has_explicit_impl_default,
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
                let bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges);
                if let Some((param_idx, bridge_cfg)) = bridge_param {
                    builder.add_item(&crate::trait_bridge::gen_bridge_function(
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &default_types,
                        &core_import,
                    ));
                } else if let Some((options_param_idx, bridge_cfg)) =
                    crate::trait_bridge::find_options_field_binding(func, &config.trait_bridges)
                {
                    builder.add_item(&crate::trait_bridge::gen_options_field_bridge_function(
                        func,
                        options_param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                    ));
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

        // Trait bridge wrappers — generate Magnus bridge structs that delegate to Ruby objects.
        // Pass the host crate's canonical error type/constructor so generated `impl Plugin`
        // and `impl {Trait}` blocks match the trait signatures (e.g. `Result<T, KreuzbergError>`).
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
                let bridge_code = crate::trait_bridge::gen_trait_bridge(
                    trait_type,
                    bridge_cfg,
                    &core_import,
                    &config.error_type_name(),
                    &config.error_constructor_expr(),
                    api,
                );
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
        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque || exclude_types.contains(typ.name.as_str()) {
                continue;
            }
            let is_strict = alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core);
            let is_relaxed = alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding);
            if is_strict && input_types.contains(&typ.name) {
                // Use custom From impl generator that filters thread-unsafe fields (e.g., VisitorHandle)
                builder.add_item(&classes::gen_from_binding_to_core_filtered(typ, &core_import));
            }
            if is_relaxed {
                // Use custom From impl generator that filters thread-unsafe fields (e.g., VisitorHandle)
                builder.add_item(&classes::gen_from_core_to_binding_filtered(
                    typ,
                    &core_import,
                    &opaque_types,
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
                    if let alef_core::ir::TypeRef::Named(name) = &field.ty {
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
        let magnus_conv_config = alef_codegen::conversions::ConversionConfig {
            binding_enums_have_data: true,
            exclude_types: &absent_named_types,
            ..Default::default()
        };
        for e in &api.enums {
            if exclude_types.contains(e.name.as_str()) {
                continue;
            }
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                    e,
                    &core_import,
                    &magnus_conv_config,
                ));
            }
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                    e,
                    &core_import,
                    &magnus_conv_config,
                ));
            }
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_magnus_error_converter(
                error,
                &core_import,
            ));
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Ruby)?;

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

        let gem_name = config.ruby_gem_name();
        let content = crate::gen_stubs::gen_stubs(api, &gem_name);

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
            crate::template_env::render(
                "main_rb_wrapper.rb.jinja",
                minijinja::context! {
                    gem_name_snake => gem_name_snake,
                    module_name => module_name,
                },
            )
            .trim_end_matches('\n'),
        );
        content.push('\n');

        // Generate the native.rb file that requires the extension and re-exports its symbols
        let native_module_name = get_module_name(&api.crate_name);
        let mut native_content = hash::header(CommentStyle::Hash);
        native_content.push_str(
            crate::template_env::render(
                "native_rb_wrapper.rb.jinja",
                minijinja::context! {
                    ext_name => ext_name,
                    module_name => module_name,
                    native_module_name => native_module_name,
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
        let version = alef_core::version::to_rubygems_prerelease(&cargo_version);

        let mut version_content = hash::header(CommentStyle::Hash);
        version_content.push_str(
            crate::template_env::render(
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
fn sorbet_type_for_field(ty: &alef_core::ir::TypeRef, optional: bool) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};
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

/// Generate a Ruby class hierarchy for an internally-tagged enum.
///
/// Emits:
/// - A sealed abstract base class with `extend T::Sig` and per-variant predicate methods
///   returning `false` by default.
/// - A concrete subclass per variant with Sorbet-typed `attr_reader` fields, an
///   `initialize` with keyword args, an overridden predicate, and a `from_hash` factory.
///
/// This replaces the Hash `method_missing` monkey-patch that was previously emitted in
/// `native.rb`. It is a BREAKING change for callers that relied on the Hash interface.
fn gen_tagged_enum_ruby_classes(enum_def: &alef_core::ir::EnumDef, module_name: &str) -> String {
    use alef_codegen::doc_emission::emit_yard_doc;
    use alef_core::ir::TypeRef;
    let mut out = String::new();

    let class_name = &enum_def.name;
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();

    // --- Base class ---
    out.push_str(&format!("module {module_name}\n"));
    if !enum_def.doc.is_empty() {
        emit_yard_doc(&mut out, &enum_def.doc, "  ");
    } else {
        // Fallback when no doc is available
        out.push_str(&format!("  # Sealed base class for the {class_name} tagged enum.\n"));
        out.push_str("  # Do not instantiate directly — use the variant subclasses.\n");
    }
    out.push_str(&format!("  class {class_name}\n"));
    out.push_str("    extend T::Sig\n");
    out.push_str("    extend T::Helpers\n\n");

    for variant_name in &variant_names {
        let snake = classes::pascal_to_snake(variant_name);
        out.push_str("    sig { returns(T::Boolean) }\n");
        out.push_str(&format!("    def {snake}? = false\n\n"));
    }

    out.push_str("  end\n\n");

    // --- Per-variant subclasses ---
    for variant in &enum_def.variants {
        let variant_class = format!("{}{}", class_name, &variant.name);
        let snake = classes::pascal_to_snake(&variant.name);

        if !variant.doc.is_empty() {
            emit_yard_doc(&mut out, &variant.doc, "  ");
        } else {
            // Fallback when no doc is available
            out.push_str(&format!(
                "  # Variant {variant_class} of the {class_name} tagged enum.\n"
            ));
        }
        out.push_str(&format!("  class {variant_class} < {class_name}\n"));
        out.push_str("    extend T::Sig\n\n");

        // attr_reader declarations with Sorbet sigs and YARD @return
        for field in &variant.fields {
            let attr_name = if field.name == "_0" {
                "value"
            } else {
                field.name.as_str()
            };
            let sorbet_t = sorbet_type_for_field(&field.ty, field.optional);
            if !field.doc.is_empty() {
                emit_yard_doc(&mut out, &field.doc, "    ");
            } else {
                out.push_str(&format!("    # @return [{sorbet_t}]\n"));
            }
            out.push_str(&format!("    sig {{ returns({sorbet_t}) }}\n"));
            out.push_str(&format!("    attr_reader :{attr_name}\n\n"));
        }

        // initialize with keyword args
        let init_params: Vec<String> = variant
            .fields
            .iter()
            .map(|f| {
                if f.name == "_0" {
                    "value".to_string()
                } else {
                    f.name.clone()
                }
            })
            .collect();
        let init_sig_params: Vec<String> = variant
            .fields
            .iter()
            .map(|f| {
                let sorbet_t = sorbet_type_for_field(&f.ty, f.optional);
                let param_name = if f.name == "_0" { "value" } else { f.name.as_str() };
                format!("{param_name}: {sorbet_t}")
            })
            .collect();
        let init_assigns: Vec<String> = variant
            .fields
            .iter()
            .map(|f| {
                let param_name = if f.name == "_0" { "value" } else { f.name.as_str() };
                format!("@{param_name} = {param_name}")
            })
            .collect();

        // YARD @param / @return for initialize
        for f in &variant.fields {
            let param_name = if f.name == "_0" { "value" } else { f.name.as_str() };
            let sorbet_t = sorbet_type_for_field(&f.ty, f.optional);
            out.push_str(&format!("    # @param {param_name} [{sorbet_t}]\n"));
        }
        out.push_str("    # @return [void]\n");
        if init_sig_params.is_empty() {
            out.push_str("    sig { void }\n");
        } else {
            out.push_str(&format!("    sig {{ params({}).void }}\n", init_sig_params.join(", ")));
        }
        let kwarg_list = init_params
            .iter()
            .map(|p| format!("{p}:"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    def initialize({kwarg_list})\n"));
        out.push_str("      super()\n");
        for assign in &init_assigns {
            out.push_str(&format!("      {assign}\n"));
        }
        out.push_str("    end\n\n");

        // Predicate override
        out.push_str(&format!("    # @return [Boolean] true when this variant is {snake}\n"));
        out.push_str("    sig { returns(T::Boolean) }\n");
        out.push_str(&format!("    def {snake}? = true\n\n"));

        // Class-level `from_hash` factory
        out.push_str("    # @param hash [Hash] a Hash deserialized from the native extension\n");
        out.push_str("    # @return [self]\n");
        out.push_str("    sig { params(hash: T::Hash[T.untyped, T.untyped]).returns(T.attached_class) }\n");
        out.push_str("    def self.from_hash(hash)\n");
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
                // Try both symbol (Magnus default) and string key
                let key_string = if f.name == "_0" { "_0" } else { f.name.as_str() };
                let val_expr = match &f.ty {
                    TypeRef::Optional(_) => format!("hash[{key_sym}] || hash[\"{key_string}\"]"),
                    _ => format!("hash[{key_sym}] || hash[\"{key_string}\"]"),
                };
                format!("{param_name}: {val_expr}")
            })
            .collect();
        if field_args.is_empty() {
            out.push_str("      new\n");
        } else {
            out.push_str(&format!("      new({})\n", field_args.join(", ")));
        }
        out.push_str("    end\n\n");

        out.push_str("  end\n\n");
    }

    out.push_str("end\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::backend::Backend;
    use alef_core::config::new_config::NewAlefConfig;
    use alef_core::ir::*;

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
}
