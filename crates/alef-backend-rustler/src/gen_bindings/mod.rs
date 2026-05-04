mod functions;
mod helpers;
mod types;

use crate::type_map::RustlerMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::ir::ApiSurface;
use alef_core::ir::TypeRef;
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

use functions::{gen_nif_async_function, gen_nif_async_method, gen_nif_function, gen_nif_method};
use helpers::{
    elixir_return_typespec, elixir_safe_param_name, elixir_typespec, gen_elixir_enum_module, gen_elixir_struct_module,
    gen_native_ex, get_module_info,
};
use types::{gen_enum, gen_opaque_resource, gen_rustler_config_impl, gen_struct};

pub struct RustlerBackend;

impl Backend for RustlerBackend {
    fn name(&self) -> &str {
        "rustler"
    }

    fn language(&self) -> Language {
        Language::Elixir
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
        let mapper = RustlerMapper;
        let core_import = config.core_import_name();

        let elixir_config = config.elixir.as_ref();
        let exclude_functions: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let cpu_bound_functions: AHashSet<String> = elixir_config
            .map(|c| c.cpu_bound_functions.iter().cloned().collect())
            .unwrap_or_default();

        // For options_field bridges, the bridge field (e.g. "visitor") is handled at the
        // Elixir layer via Map.pop — it must not appear as a typed struct field in the NIF
        // bindings because VisitorHandle (Rc<RefCell<dyn Trait>>) cannot implement
        // Rustler's Encoder/Decoder or Send+Sync traits.
        // Build a map: type_name -> set of field names to exclude.
        // We also cover update structs (e.g. ConversionOptionsUpdate) by scanning all IR types
        // for the same field name with a type matching the bridge trait alias.
        let mut bridge_excluded_fields: std::collections::HashMap<String, AHashSet<String>> =
            std::collections::HashMap::new();
        for b in config
            .trait_bridges
            .iter()
            .filter(|b| b.bind_via == BridgeBinding::OptionsField)
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"))
        {
            let field_name = b.resolved_options_field().unwrap_or("visitor").to_string();
            let trait_alias = b.type_alias.as_deref().unwrap_or(&b.trait_name);
            if let Some(opts_type) = b.options_type.as_deref() {
                bridge_excluded_fields
                    .entry(opts_type.to_string())
                    .or_default()
                    .insert(field_name.clone());
            }
            // Also exclude from any other IR type that has this field with the trait alias type.
            for typ in api.types.iter() {
                if typ.fields.iter().any(|f| {
                    if f.name != field_name {
                        return false;
                    }
                    let type_name = match &f.ty {
                        alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                        alef_core::ir::TypeRef::Optional(inner) => {
                            if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                                Some(n.as_str())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    type_name == Some(trait_alias)
                }) {
                    bridge_excluded_fields
                        .entry(typ.name.clone())
                        .or_default()
                        .insert(field_name.clone());
                }
            }
        }

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
        builder.add_import("rustler::ResourceArc");
        builder.add_import("rustler::Encoder");

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

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::Elixir);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        let (_module_name, module_prefix) = get_module_info(api, config);

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
        }

        let empty_set: AHashSet<String> = AHashSet::new();
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            if typ.is_opaque {
                builder.add_item(&gen_opaque_resource(typ, &core_import, &opaque_types));
            } else {
                // gen_struct adds Default to derives when typ.has_default is true,
                // so no separate Default impl is needed.
                let excl = bridge_excluded_fields.get(typ.name.as_str()).unwrap_or(&empty_set);
                builder.add_item(&gen_struct(typ, &mapper, &module_prefix, excl));
                // Generate config constructor if type has Default
                if typ.has_default && !typ.fields.is_empty() {
                    let config_impl = gen_rustler_config_impl(typ, &mapper, excl);
                    builder.add_item(&config_impl);
                }
            }
        }

        for enum_def in &api.enums {
            builder.add_item(&gen_enum(enum_def));
        }

        // Types with has_default=true accept JSON strings at the NIF boundary so
        // partial maps can be passed without every field being required.
        let default_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_default && !t.is_opaque)
            .map(|t| t.name.clone())
            .collect();

        // Build adapter body map before method iteration so bodies are available for NIF generation.
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Elixir)?;

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

        let active_bridges: Vec<_> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"))
            .cloned()
            .collect();

        // Add globals for trait call dispatch if there are active bridges
        if !active_bridges.is_empty() {
            builder.add_import("std::sync::atomic::{AtomicU64, Ordering}");
            builder.add_import("std::sync::Mutex");
            builder.add_import("std::collections::HashMap");
            builder.add_item("static TRAIT_REPLY_COUNTER: AtomicU64 = AtomicU64::new(1);");
            builder.add_item(
                "static TRAIT_REPLY_CHANNELS: std::sync::LazyLock<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<std::result::Result<String, String>>>>> = \
                 std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));",
            );
        }

        for func in api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
        {
            let bridge_param = crate::trait_bridge::find_bridge_param(func, &active_bridges);
            let bridge_field =
                alef_codegen::generators::trait_bridge::find_bridge_field(func, &api.types, &active_bridges);
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
            } else if let Some(ref bm) = bridge_field {
                builder.add_item(&crate::trait_bridge::gen_bridge_field_function(
                    func,
                    bm,
                    bm.bridge,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                ));
            } else if func.is_async {
                builder.add_item(&gen_nif_async_function(
                    func,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                ));
            } else {
                builder.add_item(&gen_nif_function(
                    func,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                    &cpu_bound_functions,
                ));
            }
        }

        // Trait bridge wrappers — generate Rustler bridge structs that delegate to Elixir terms
        for bridge_cfg in config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"))
        {
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

        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            for method in typ
                .methods
                .iter()
                .filter(|m| !exclude_functions.contains(m.name.as_str()))
                .filter(|m| {
                    // Skip methods whose return type references an excluded type.
                    // E.g. ConversionOptions::builder() returns ConversionOptionsBuilder which
                    // is excluded because it holds !Send + !Sync core types.
                    !alef_codegen::conversions::field_references_excluded_type(
                        &m.return_type,
                        &exclude_types.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    )
                })
            {
                if method.is_async {
                    builder.add_item(&gen_nif_async_method(
                        &typ.name,
                        method,
                        &mapper,
                        typ.is_opaque,
                        &opaque_types,
                        &core_import,
                        &adapter_bodies,
                    ));
                } else {
                    builder.add_item(&gen_nif_method(
                        &typ.name,
                        method,
                        &mapper,
                        typ.is_opaque,
                        &opaque_types,
                        &core_import,
                        &adapter_bodies,
                    ));
                }
            }
        }

        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);

        // Collect bridge type aliases so they can be passed as `exclude_types` in ConversionConfig.
        // This ensures From impls skip fields (e.g. `visitor: Option<VisitorHandle>`) that were
        // excluded from the binding struct because they reference !Send + !Sync core types.
        let bridge_conv_exclude_types: Vec<String> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"))
            .filter(|b| b.bind_via == BridgeBinding::OptionsField)
            .map(|b| b.type_alias.as_deref().unwrap_or(&b.trait_name).to_string())
            .collect();

        // From/Into conversions
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            let rustler_struct_cfg = alef_codegen::conversions::ConversionConfig {
                map_as_string: true,
                exclude_types: &bridge_conv_exclude_types,
                ..Default::default()
            };
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &rustler_struct_cfg,
                ));
            }
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &rustler_struct_cfg,
                ));
            }
        }
        for e in &api.enums {
            // Data enums (any variant has fields) are generated as NifTaggedEnum with real fields.
            // Set binding_enums_have_data so the From impls destructure fields instead of
            // fabricating Default::default() for every field (which would silently corrupt data).
            let has_data = e.variants.iter().any(|v| !v.fields.is_empty());
            let rustler_conv_config = alef_codegen::conversions::ConversionConfig {
                binding_enums_have_data: has_data,
                ..Default::default()
            };
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                    e,
                    &core_import,
                    &rustler_conv_config,
                ));
            }
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                    e,
                    &core_import,
                    &rustler_conv_config,
                ));
            }
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_rustler_error_converter(
                error,
                &core_import,
            ));
        }

        builder.add_item(&gen_nif_init(api, config, &exclude_functions, &exclude_types));

        let content = builder.build();

        let output_dir = resolve_output_dir(
            config.output_paths.get("elixir"),
            &config.name,
            "packages/elixir/native/{name}_nif/src/",
        );

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
        let app_name = config.elixir_app_name();
        let app_module = app_name.to_pascal_case();
        let native_mod = format!("{app_module}.Native");
        let crate_name = config.name.replace('-', "_");

        let elixir_config = config.elixir.as_ref();
        let exclude_functions: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();

        // Types whose NIF params are JSON strings (has_default = true, non-opaque).
        let default_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_default && !t.is_opaque)
            .map(|t| t.name.clone())
            .collect();

        // Build enum defaults map: enum name -> first variant snake_case (for struct field defaults)
        let enum_defaults: std::collections::HashMap<String, String> = api
            .enums
            .iter()
            .filter_map(|e| {
                // Use the variant marked is_default, or fall back to first variant
                let default_variant = e
                    .variants
                    .iter()
                    .find(|v| v.is_default)
                    .or_else(|| e.variants.first())?;
                Some((e.name.clone(), default_variant.name.to_snake_case()))
            })
            .collect();

        let mut files: Vec<GeneratedFile> = Vec::new();

        // Elixir .ex files belong in the Elixir lib/ directory, not the Rust native/src/ dir.
        // If config.output_paths["elixir"] points at a native/ path (e.g. packages/elixir/native/.../src/),
        // derive the lib/ sibling by stripping everything from "/native/" onwards.
        let output_dir = if let Some(elixir_output) = config.output_paths.get("elixir") {
            let s = elixir_output.to_string_lossy();
            if let Some(idx) = s.find("/native/") {
                format!("{}/lib/", &s[..idx])
            } else {
                s.into_owned()
            }
        } else {
            "packages/elixir/lib/".to_owned()
        };

        // ── 1. native.ex – NIF stub module ───────────────────────────────────
        let native_content = gen_native_ex(
            api,
            &app_name,
            &app_module,
            &crate_name,
            config,
            &exclude_functions,
            &exclude_types,
        );
        files.push(GeneratedFile {
            path: PathBuf::from(&output_dir)
                .join(app_name.to_snake_case())
                .join("native.ex"),
            content: native_content,
            generated_header: false,
        });

        // ── 2. Struct modules for non-opaque types with fields ────────────────
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            if typ.is_opaque || typ.fields.is_empty() {
                continue;
            }
            let struct_content = gen_elixir_struct_module(typ, &app_module, &enum_defaults, &opaque_types);
            let file_name = format!("{}.ex", typ.name.to_snake_case());
            files.push(GeneratedFile {
                path: PathBuf::from(&output_dir)
                    .join(app_name.to_snake_case())
                    .join(file_name),
                content: struct_content,
                generated_header: false,
            });
        }

        // ── 3. Enum modules ───────────────────────────────────────────────────
        for enum_def in &api.enums {
            let enum_content = gen_elixir_enum_module(enum_def, &app_module);
            let file_name = format!("{}.ex", enum_def.name.to_snake_case());
            files.push(GeneratedFile {
                path: PathBuf::from(&output_dir)
                    .join(app_name.to_snake_case())
                    .join(file_name),
                content: enum_content,
                generated_header: false,
            });
        }

        // ── 4. Main wrapper module ────────────────────────────────────────────
        let mut content = alef_core::hash::header(alef_core::hash::CommentStyle::Hash);
        content.push_str(&format!("defmodule {app_module} do\n"));
        content.push_str(&format!("  @moduledoc \"High-level API for {app_name}.\"\n\n"));

        // Wrapper functions for top-level API functions
        for func in &api.functions {
            let nif_fn_name = if func.is_async {
                let s = func.name.to_snake_case();
                if s.ends_with("_async") { s } else { format!("{s}_async") }
            } else {
                func.name.to_snake_case()
            };
            let doc_line_raw = func.doc.lines().next().unwrap_or("Function");
            // Elixir @doc strings use double-quote delimiters; escape any embedded quotes.
            let doc_line = doc_line_raw.replace('"', "\\\"");
            let doc_line = doc_line.as_str();

            let param_types: Vec<String> = func
                .params
                .iter()
                .map(|p| {
                    let base = elixir_typespec(&p.ty, &opaque_types, &default_types);
                    if p.optional && !base.ends_with("| nil") {
                        format!("{base} | nil")
                    } else {
                        base
                    }
                })
                .collect();
            let return_spec = elixir_return_typespec(
                &func.return_type,
                func.error_type.is_some(),
                &opaque_types,
                &default_types,
            );
            let all_params: Vec<String> = func.params.iter().map(|p| elixir_safe_param_name(&p.name)).collect();

            // Count how many trailing parameters are optional so we can emit shorter-arity overloads.
            let trailing_optional_count = func.params.iter().rev().take_while(|p| p.optional).count();

            // Detect if this function has a visitor bridge param.
            let visitor_bridge_param_idx: Option<usize> = func.params.iter().position(|p| {
                config.trait_bridges.iter().any(|b| {
                    b.param_name.as_deref() == Some(p.name.as_str()) || {
                        let named = match &p.ty {
                            alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                            alef_core::ir::TypeRef::Optional(inner) => {
                                if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                                    Some(n.as_str())
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        named.map(|n| b.type_alias.as_deref() == Some(n)).unwrap_or(false)
                    }
                })
            });

            // Detect options_field visitor bridge: visitor is embedded in the options struct.
            // Returns (options_param_idx, field_name) when matched.
            let options_field_bridge: Option<(usize, String)> = func.params.iter().enumerate().find_map(|(idx, p)| {
                let type_name = match &p.ty {
                    alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                config.trait_bridges.iter().find_map(|b| {
                    if b.bind_via == BridgeBinding::OptionsField
                        && type_name.is_some_and(|n| b.options_type.as_deref() == Some(n))
                    {
                        let field = b.resolved_options_field().unwrap_or("visitor").to_string();
                        Some((idx, field))
                    } else {
                        None
                    }
                })
            });

            // Emit one @spec/@doc per arity variant (shortest to longest).
            // The shortest arity fills optional params with nil.
            let arity_variants: Vec<usize> = if trailing_optional_count > 0 {
                ((all_params.len() - trailing_optional_count)..=all_params.len()).collect()
            } else {
                vec![all_params.len()]
            };

            for arity in &arity_variants {
                let arity_params = &all_params[..*arity];
                let arity_types = &param_types[..*arity];

                content.push_str(&format!("  @doc \"{doc_line}\"\n"));
                let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", arity_types.join(", "));
                if spec_inline.len() > 98 {
                    let spec_broken = format!(
                        "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                        arity_types.join(", ")
                    );
                    if spec_broken.lines().all(|l| l.len() <= 98) {
                        content.push_str(&spec_broken);
                        content.push('\n');
                    } else {
                        content.push_str(&format!("  @spec {nif_fn_name}(\n"));
                        let len = arity_types.len();
                        for (i, t) in arity_types.iter().enumerate() {
                            if i + 1 < len {
                                content.push_str(&format!("          {t},\n"));
                            } else {
                                content.push_str(&format!("          {t}\n"));
                            }
                        }
                        content.push_str(&format!("        ) :: {return_spec}\n"));
                    }
                } else {
                    content.push_str(&spec_inline);
                    content.push('\n');
                }

                // Build the call: fill missing optional params with nil
                let nif_call_args: Vec<String> = all_params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| if i < *arity { p.clone() } else { "nil".to_string() })
                    .collect();

                // options_field bridge: visitor is embedded in the options map.
                // Extract `:visitor` from options before calling the NIF.
                if let Some((opts_idx, ref field_name)) = options_field_bridge {
                    if *arity > opts_idx {
                        let opts_param = &all_params[opts_idx];
                        // Single clause handles both visitor and no-visitor by inspecting the map.
                        content.push_str(&format!(
                            "  def {nif_fn_name}({p}) when is_map({opts_param}) do\n",
                            p = arity_params.join(", ")
                        ));
                        content.push_str(&format!(
                            "    {{visitor, clean_opts}} = Map.pop({opts_param}, :{field_name})\n"
                        ));
                        content.push_str("    if is_map(visitor) do\n");
                        // Build NIF args: replace opts param with JSON-encoded clean opts, append visitor.
                        let with_visitor_args: Vec<String> = nif_call_args
                            .iter()
                            .enumerate()
                            .map(|(i, a)| {
                                if i == opts_idx {
                                    "if(map_size(clean_opts) == 0, do: nil, else: Jason.encode!(clean_opts))"
                                        .to_string()
                                } else {
                                    a.clone()
                                }
                            })
                            .collect();
                        let with_visitor_args_str = with_visitor_args.join(", ");
                        content.push_str(&format!(
                            "      :ok = {native_mod}.{nif_fn_name}_with_visitor({with_visitor_args_str}, visitor)\n"
                        ));
                        content.push_str("      do_visitor_receive_loop(visitor)\n");
                        content.push_str("    else\n");
                        // No visitor: call regular NIF with options as JSON.
                        let plain_args: Vec<String> = nif_call_args
                            .iter()
                            .enumerate()
                            .map(|(i, a)| {
                                if i == opts_idx {
                                    format!(
                                        "if(map_size({opts_param}) == 0, do: nil, else: Jason.encode!({opts_param}))"
                                    )
                                } else {
                                    a.clone()
                                }
                            })
                            .collect();
                        let plain_args_str = plain_args.join(", ");
                        content.push_str(&format!("      {native_mod}.{nif_fn_name}({plain_args_str})\n"));
                        content.push_str("    end\n");
                        content.push_str("  end\n\n");

                        // Nil clause: options is nil — pass nil directly to the NIF.
                        let nil_clause_params: Vec<String> = arity_params
                            .iter()
                            .enumerate()
                            .map(|(i, p)| if i == opts_idx { "nil".to_string() } else { p.clone() })
                            .collect();
                        let nil_nif_args: Vec<String> = nif_call_args
                            .iter()
                            .enumerate()
                            .map(|(i, a)| if i == opts_idx { "nil".to_string() } else { a.clone() })
                            .collect();
                        content.push_str(&format!("  def {nif_fn_name}({}) do\n", nil_clause_params.join(", ")));
                        content.push_str(&format!(
                            "    {native_mod}.{nif_fn_name}({})\n",
                            nil_nif_args.join(", ")
                        ));
                        content.push_str("  end\n\n");
                        continue;
                    }
                }

                // function_param bridge: visitor is a direct positional parameter.
                // When a visitor is provided (non-nil at the bridge param index), delegate to
                // the async visitor variant which drives a receive loop.
                if let Some(vis_idx) = visitor_bridge_param_idx {
                    if *arity > vis_idx {
                        // Full-arity def: visitor param is present in signature.
                        let vis_param = &all_params[vis_idx];
                        // Emit a two-clause definition: visitor map → receive loop, nil → direct.
                        content.push_str(&format!(
                            "  def {nif_fn_name}({}) when is_map({vis_param}) do\n",
                            arity_params.join(", ")
                        ));
                        let with_visitor_args = nif_call_args.join(", ");
                        content.push_str(&format!(
                            "    :ok = {native_mod}.{nif_fn_name}_with_visitor({with_visitor_args})\n"
                        ));
                        content.push_str(&format!("    do_visitor_receive_loop({vis_param})\n"));
                        content.push_str("  end\n\n");
                        // Nil/no-visitor clause
                        content.push_str(&format!("  @doc \"{doc_line}\"\n"));
                        let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", arity_types.join(", "));
                        if spec_inline.len() > 98 {
                            let spec_broken = format!(
                                "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                                arity_types.join(", ")
                            );
                            if spec_broken.lines().all(|l| l.len() <= 98) {
                                content.push_str(&spec_broken);
                            } else {
                                content.push_str(&format!("  @spec {nif_fn_name}(\n"));
                                let len = arity_types.len();
                                for (i, t) in arity_types.iter().enumerate() {
                                    if i + 1 < len {
                                        content.push_str(&format!("          {t},\n"));
                                    } else {
                                        content.push_str(&format!("          {t}\n"));
                                    }
                                }
                                content.push_str(&format!("        ) :: {return_spec}"));
                            }
                        } else {
                            content.push_str(&spec_inline);
                        }
                        content.push('\n');
                        content.push_str(&format!("  def {nif_fn_name}({}) do\n", arity_params.join(", ")));
                        content.push_str(&format!(
                            "    {native_mod}.{nif_fn_name}({})\n",
                            nif_call_args.join(", ")
                        ));
                        content.push_str("  end\n\n");
                        continue;
                    }
                }

                if arity_params.is_empty() {
                    content.push_str(&format!("  def {nif_fn_name} do\n"));
                    content.push_str(&format!(
                        "    {native_mod}.{nif_fn_name}({})\n",
                        nif_call_args.join(", ")
                    ));
                } else {
                    content.push_str(&format!("  def {nif_fn_name}({})", arity_params.join(", ")));
                    content.push_str(" do\n");
                    content.push_str(&format!(
                        "    {native_mod}.{nif_fn_name}({})\n",
                        nif_call_args.join(", ")
                    ));
                }
                content.push_str("  end\n\n");
            }
        }

        // Emit the visitor receive loop helper if any function has a visitor bridge
        // (function_param or options_field mode).
        let has_visitor_bridges = api.functions.iter().any(|func| {
            func.params.iter().any(|p| {
                let named = match &p.ty {
                    alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                config.trait_bridges.iter().any(|b| {
                    // function_param: match by param_name or type_alias
                    let is_function_param = b.param_name.as_deref() == Some(p.name.as_str())
                        || named.map(|n| b.type_alias.as_deref() == Some(n)).unwrap_or(false);
                    // options_field: match when the param type is the configured options_type
                    let is_options_field = b.bind_via == BridgeBinding::OptionsField
                        && named.is_some_and(|n| b.options_type.as_deref() == Some(n));
                    is_function_param || is_options_field
                })
            })
        });

        if has_visitor_bridges {
            content.push_str(&format!(
                r#"  @doc false
  defp do_visitor_receive_loop(visitor) do
    receive do
      {{:visitor_callback, ref_id, callback_name, args_json}} ->
        result =
          case Map.get(visitor, callback_name) do
            nil -> "continue"
            fun -> apply_visitor_callback(fun, args_json)
          end

        {native_mod}.visitor_reply(ref_id, result)
        do_visitor_receive_loop(visitor)

      {{:ok, result}} ->
        {{:ok, result}}

      {{:error, reason}} ->
        {{:error, reason}}
    after
      30_000 ->
        {{:error, "visitor callback timeout after 30s"}}
    end
  end

  @doc false
  defp apply_visitor_callback(fun, args_json) do
    args = Jason.decode!(args_json)
    result = fun.(args)
    case result do
      :continue -> "continue"
      :skip -> "skip"
      :preserve_html -> "preserve_html"
      {{:custom, value}} -> to_string(value)
      binary when is_binary(binary) -> binary
      _ -> "continue"
    end
  end

"#
            ));
        }

        // Wrapper functions for type methods (e.g., conversionoptions_default)
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            for method in &typ.methods {
                let nif_fn_name = if method.is_async {
                    format!("{}_{}_async", typ.name.to_lowercase(), method.name)
                } else {
                    format!("{}_{}", typ.name.to_lowercase(), method.name)
                };

                let doc_line_raw = method.doc.lines().next().unwrap_or("Method");
                let doc_line_escaped = doc_line_raw.replace('"', "\\\"");
                content.push_str(&format!("  @doc \"{doc_line_escaped}\"\n"));

                // Params: receiver (if any) + method params
                let mut param_names: Vec<String> = Vec::new();
                if method.receiver.is_some() {
                    param_names.push("obj".to_string());
                }
                for p in &method.params {
                    param_names.push(elixir_safe_param_name(&p.name));
                }

                let return_spec = elixir_return_typespec(
                    &method.return_type,
                    method.error_type.is_some(),
                    &opaque_types,
                    &default_types,
                );
                let type_specs: Vec<String> = {
                    let mut specs: Vec<String> = Vec::new();
                    if method.receiver.is_some() {
                        // receiver is the struct itself (non-opaque) or a reference
                        specs.push("map()".to_string());
                    }
                    for p in &method.params {
                        let base = elixir_typespec(&p.ty, &opaque_types, &default_types);
                        specs.push(if p.optional && !base.ends_with("| nil") {
                            format!("{base} | nil")
                        } else {
                            base
                        });
                    }
                    specs
                };
                let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", type_specs.join(", "));
                if spec_inline.len() > 98 {
                    let spec_broken = format!(
                        "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                        type_specs.join(", ")
                    );
                    if spec_broken.lines().all(|l| l.len() <= 98) {
                        content.push_str(&spec_broken);
                        content.push('\n');
                    } else {
                        content.push_str(&format!("  @spec {nif_fn_name}(\n"));
                        let len = type_specs.len();
                        for (i, t) in type_specs.iter().enumerate() {
                            if i + 1 < len {
                                content.push_str(&format!("          {t},\n"));
                            } else {
                                content.push_str(&format!("          {t}\n"));
                            }
                        }
                        content.push_str(&format!("        ) :: {return_spec}\n"));
                    }
                } else {
                    content.push_str(&spec_inline);
                    content.push('\n');
                }

                if param_names.is_empty() {
                    content.push_str(&format!("  def {nif_fn_name} do\n"));
                    content.push_str(&format!("    {native_mod}.{nif_fn_name}()\n"));
                } else {
                    content.push_str(&format!("  def {nif_fn_name}({})", param_names.join(", ")));
                    content.push_str(" do\n");
                    content.push_str(&format!("    {native_mod}.{nif_fn_name}({})\n", param_names.join(", ")));
                }
                content.push_str("  end\n\n");
            }
        }

        // Trim trailing blank lines so `mix format` doesn't see an extra blank before `end`.
        let trimmed = content.trim_end_matches('\n');
        content = format!("{trimmed}\nend\n");

        files.push(GeneratedFile {
            path: PathBuf::from(&output_dir).join(format!("{}.ex", app_name.to_snake_case())),
            content,
            generated_header: false,
        });

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "mix",
            crate_suffix: "-rustler",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

/// Generate the rustler::init! macro invocation.
fn gen_nif_init(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_functions: &AHashSet<&str>,
    exclude_types: &AHashSet<&str>,
) -> String {
    let mut exports = vec![];

    // Custom NIF function registrations (before generated ones)
    if let Some(reg) = config.custom_registrations.for_language(Language::Elixir) {
        for func in &reg.functions {
            exports.push(func.clone());
        }
    }

    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let func_name = if func.is_async {
            let n = func.name.as_str();
            if n.ends_with("_async") {
                n.to_string()
            } else {
                format!("{n}_async")
            }
        } else {
            func.name.clone()
        };
        exports.push(func_name);
    }

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
    {
        for method in typ
            .methods
            .iter()
            .filter(|m| !exclude_functions.contains(m.name.as_str()))
        {
            let method_name = if method.is_async {
                format!("{}_{}_async", typ.name.to_lowercase(), method.name)
            } else {
                format!("{}_{}", typ.name.to_lowercase(), method.name)
            };
            exports.push(method_name);
        }
    }

    // Rustler auto-detects #[rustler::nif] functions; explicit list is deprecated
    let _ = exports; // computed for potential future use
    // The NIF module name must match the `defmodule` in native.ex, which is
    // `{AppModule}.Native` (e.g., `HtmlToMarkdown.Native`).
    let module = config
        .elixir
        .as_ref()
        .map(|e| {
            use heck::ToUpperCamelCase;
            format!(
                "Elixir.{}.Native",
                e.app_name.as_deref().unwrap_or("NativeModule").to_upper_camel_case()
            )
        })
        .unwrap_or_else(|| "Elixir.NativeModule.Native".to_string());
    // Check if any opaque types need Resource registration via on_load
    // Exclude trait types (they shouldn't be registered as Rustler resources)
    // Also exclude types in exclude_types (e.g. VisitorHandle, ConversionOptionsBuilder)
    // which are omitted from the binding layer because they hold !Send+!Sync core types.
    let opaque_types: Vec<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !exclude_types.contains(t.name.as_str()))
        .map(|t| t.name.as_str())
        .collect();
    if !opaque_types.is_empty() {
        let registrations: Vec<String> = opaque_types
            .iter()
            .map(|name| format!("    env.register::<{name}>().expect(\"Failed to register resource type {name}\");"))
            .collect();
        let reg_body = registrations.join("\n");
        format!(
            "fn on_load(env: rustler::Env, _info: rustler::Term) -> bool {{\n{reg_body}\n    true\n}}\n\n\
             rustler::init!(\"{module}\", load = on_load);"
        )
    } else {
        format!("rustler::init!(\"{module}\");")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::new_config::NewAlefConfig;
    use alef_core::ir::ApiSurface;

    fn test_config() -> ResolvedCrateConfig {
        let toml = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "my_lib"
"#;
        let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
        cfg.resolve().expect("test config must resolve").remove(0)
    }

    fn test_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    /// The generated lib.rs must be placed in `{name}_nif/src/lib.rs` by default —
    /// matching the scaffold's Cargo.toml at `{name}_nif/Cargo.toml`.
    ///
    /// Previously the backend used `{name}_rustler/src/` which caused a 3-way mismatch:
    /// scaffold Cargo.toml in `_nif/`, generated lib.rs in `_rustler/`, native.ex `crate:` = `_nif`.
    #[test]
    fn test_generate_bindings_output_path_is_nif_not_rustler() {
        let config = test_config();
        let api = test_api();
        let backend = RustlerBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();
        assert_eq!(files.len(), 1, "expected exactly one generated file");
        let lib_rs_path = files[0].path.to_string_lossy();
        // With ResolvedCrateConfig the output_paths template resolves to packages/elixir/.
        // The important invariant is that the path never falls back to a _rustler/ directory.
        assert!(
            lib_rs_path.ends_with("lib.rs"),
            "generated file must be a lib.rs; got: {lib_rs_path}"
        );
        assert!(
            !lib_rs_path.contains("_rustler"),
            "generated lib.rs must not be inside a _rustler/ directory; got: {lib_rs_path}"
        );
    }

    /// The `crate:` field in native.ex must match the `[package] name` in the scaffold's Cargo.toml.
    /// Both must be `{app_name}_nif` so rustler_precompiled can locate the shared library.
    #[test]
    fn test_native_ex_crate_field_matches_nif_crate_name() {
        let config = test_config();
        let api = test_api();
        let backend = RustlerBackend;
        let files = backend.generate_public_api(&api, &config).unwrap();
        let native_ex = files.iter().find(|f| f.path.ends_with("native.ex")).unwrap();
        assert!(
            native_ex.content.contains("crate: \"my_lib_nif\""),
            "native.ex crate: field must match the _nif Cargo.toml package name; content: {}",
            native_ex.content
        );
    }
}
