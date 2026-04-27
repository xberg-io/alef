mod functions;
mod helpers;
mod types;

use crate::type_map::RustlerMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
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

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let mapper = RustlerMapper;
        let core_import = config.core_import();

        let elixir_config = config.elixir.as_ref();
        let exclude_functions: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

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
                builder.add_item(&gen_struct(typ, &mapper, &module_prefix));
                // Generate config constructor if type has Default
                if typ.has_default && !typ.fields.is_empty() {
                    let config_impl = gen_rustler_config_impl(typ, &mapper);
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
                    &config.error_type(),
                    &config.error_constructor(),
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
        // From/Into conversions
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core(typ, &core_import));
            }
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding(
                    typ,
                    &core_import,
                    &opaque_types,
                ));
            }
        }
        for e in &api.enums {
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core(
                    e,
                    &core_import,
                ));
            }
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding(
                    e,
                    &core_import,
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
            config.output.elixir.as_ref(),
            &config.crate_config.name,
            "packages/elixir/native/{name}_nif/src/",
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let app_name = config.elixir_app_name();
        let app_module = app_name.to_pascal_case();
        let native_mod = format!("{app_module}.Native");
        let crate_name = config.crate_config.name.replace('-', "_");

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
        // If config.output.elixir points at a native/ path (e.g. packages/elixir/native/.../src/),
        // derive the lib/ sibling by stripping everything from "/native/" onwards.
        let output_dir = if let Some(elixir_output) = config.output.elixir.as_ref() {
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
                format!("{}_async", func.name.to_snake_case())
            } else {
                func.name.to_snake_case()
            };
            let doc_line = func.doc.lines().next().unwrap_or("Function");

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

        // Emit the visitor receive loop helper if any function has a visitor bridge.
        let has_visitor_bridges = api.functions.iter().any(|func| {
            func.params.iter().any(|p| {
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
    if is_binary(result), do: result, else: "continue"
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

                let doc_line = method.doc.lines().next().unwrap_or("Method");
                content.push_str(&format!("  @doc \"{doc_line}\"\n"));

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
    config: &AlefConfig,
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
            format!("{}_async", func.name)
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
    let opaque_types: Vec<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
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
    use alef_core::config::*;
    use alef_core::ir::ApiSurface;

    fn test_config() -> AlefConfig {
        AlefConfig {
            version: None,
            crate_config: CrateConfig {
                name: "my-lib".to_string(),
                sources: vec![],
                version_from: "Cargo.toml".to_string(),
                core_import: None,
                workspace_root: None,
                skip_core_import: false,
                features: vec![],
                path_mappings: std::collections::HashMap::new(),
                auto_path_mappings: Default::default(),
                extra_dependencies: Default::default(),
                source_crates: vec![],
                error_type: None,
                error_constructor: None,
            },
            languages: vec![Language::Elixir],
            exclude: ExcludeConfig::default(),
            include: IncludeConfig::default(),
            output: OutputConfig::default(),
            python: None,
            node: None,
            ruby: None,
            php: None,
            elixir: Some(ElixirConfig {
                app_name: Some("my_lib".to_string()),
                features: None,
                serde_rename_all: None,
                exclude_functions: vec![],
                exclude_types: vec![],
                extra_dependencies: Default::default(),
                scaffold_output: None,
                rename_fields: Default::default(),
                run_wrapper: None,
                extra_lint_paths: vec![],
            }),
            wasm: None,
            ffi: None,
            go: None,
            java: None,
            csharp: None,
            kotlin: None,
            swift: None,
            dart: None,
            gleam: None,
            zig: None,
            r: None,
            scaffold: None,
            readme: None,
            lint: None,
            update: None,
            test: None,
            setup: None,
            clean: None,
            build_commands: None,
            publish: None,
            custom_files: None,
            adapters: vec![],
            custom_modules: CustomModulesConfig::default(),
            custom_registrations: CustomRegistrationsConfig::default(),
            opaque_types: std::collections::HashMap::new(),
            generate: GenerateConfig::default(),
            generate_overrides: std::collections::HashMap::new(),
            dto: Default::default(),
            sync: None,
            e2e: None,
            trait_bridges: vec![],
            tools: ToolsConfig::default(),
            format: FormatConfig::default(),
            format_overrides: std::collections::HashMap::new(),
        }
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
        assert!(
            lib_rs_path.contains("_nif/src/lib.rs"),
            "generated lib.rs must be inside the _nif/ crate directory (not _rustler/); got: {lib_rs_path}"
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
