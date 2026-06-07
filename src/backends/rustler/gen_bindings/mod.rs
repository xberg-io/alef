mod functions;
mod helpers;
mod service_api;
mod types;

use crate::backends::rustler::template_env;
use crate::backends::rustler::type_map::RustlerMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::ApiSurface;
use crate::core::ir::TypeRef;
use ahash::AHashSet;
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

use functions::{gen_nif_async_function, gen_nif_async_method, gen_nif_function, gen_nif_method};
use helpers::{
    collect_types_for_nif_derives, elixir_return_typespec, elixir_safe_param_name, elixir_typespec,
    gen_elixir_enum_module_with_known_types, gen_elixir_opaque_module, gen_elixir_struct_module, gen_native_ex,
    get_module_info,
};
use types::{
    gen_enum, gen_opaque_resource, gen_rustler_config_impl, gen_rustler_flat_data_enum_from_core,
    gen_rustler_flat_data_enum_to_core, gen_struct,
};

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
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let mapper = RustlerMapper;
        let core_import = config.core_import_name();

        let elixir_config = config.elixir.as_ref();
        let exclude_functions: AHashSet<String> = elixir_config
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        // Service-owner types and handler-contract traits are marked binding_excluded
        // by the service extraction pass: they are emitted by the service-API codegen,
        // not the generic struct/trait/opaque codegen, so skip them in the generic loops too.
        let binding_excluded_names: Vec<String> = api
            .types
            .iter()
            .filter(|t| t.binding_excluded)
            .map(|t| t.name.clone())
            .collect();
        let mut exclude_types: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();
        exclude_types.extend(binding_excluded_names.iter().map(String::as_str));
        let cpu_bound_functions: AHashSet<String> = elixir_config
            .map(|c| c.cpu_bound_functions.iter().cloned().collect())
            .unwrap_or_default();

        // For options_field bridges, the bridge field (e.g. "visitor") is handled at the
        // Elixir layer via Map.pop — it must not appear as a typed struct field in the NIF
        // bindings because VisitorHandle (Rc<RefCell<dyn Trait>>) cannot implement
        // Rustler's Encoder/Decoder or Send+Sync traits.
        // Build a map: type_name -> set of field names to exclude.
        // We also cover update structs (e.g. ParseOptionsUpdate) by scanning all IR types
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
                if binding_fields(&typ.fields).any(|f| {
                    if f.name != field_name {
                        return false;
                    }
                    let type_name = match &f.ty {
                        crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                        crate::core::ir::TypeRef::Optional(inner) => {
                            if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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

        // Include service.rs (if services are configured). The service-API
        // codegen emits additional `#[rustler::nif]` functions that Rustler's
        // init! macro discovers via the module tree.
        if !api.services.is_empty() {
            builder.add_item("pub mod service;");
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

        // Collect all types that need NifMap/NifStruct derives: both top-level and recursively
        // referenced (e.g., CrawlResult has field pages: Vec<CrawlPageResult>, so CrawlPageResult
        // must also derive NifMap). Walk the full type closure reachable from function signatures.
        let mut types_to_emit = collect_types_for_nif_derives(api, &exclude_types);

        // Add opaque types (they were filtered out by collect_types_for_nif_derives but need to be emitted
        // as rustler::Resource wrappers). Opaques must be handled separately from non-opaque types.
        for typ in &api.types {
            if typ.is_opaque && !exclude_types.contains(typ.name.as_str()) {
                types_to_emit.insert(typ.name.clone());
            }
        }

        let empty_set: AHashSet<String> = AHashSet::new();
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && types_to_emit.contains(&typ.name))
        {
            if typ.is_opaque {
                builder.add_item(&gen_opaque_resource(typ, &core_import, &opaque_types));
                // Client constructor
                if let Some(ctor) = config.client_constructors.get(&typ.name) {
                    let ctor_body =
                        crate::codegen::generators::gen_opaque_constructor(ctor, &typ.name, &core_import, "");
                    let ctor_impl = format!("impl {} {{\n{}}}", typ.name, ctor_body);
                    builder.add_item(&ctor_impl);
                }
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
            builder.add_item(&gen_enum(enum_def, &module_prefix));
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
        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Elixir)?;

        // Streaming-adapter method keys ("Owner.method_name") — these methods are emitted
        // as a pair of standalone start/next NIFs from the adapter struct hook, so the
        // regular method-iteration loop must skip them to avoid double-emitting a NIF
        // with the same name.
        let streaming_method_keys: AHashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
            .collect();

        // Emit adapter-generated standalone items (streaming iterators, callback bridges).
        for adapter in &config.adapters {
            match adapter.pattern {
                crate::core::config::AdapterPattern::Streaming => {
                    let key = crate::adapters::stream_struct_key(adapter);
                    if let Some(struct_code) = adapter_bodies.get(&key) {
                        // Post-process: convert default-typed request params to JSON strings so
                        // partial maps from Elixir decode successfully (rustler NifMap is strict).
                        let patched = patch_streaming_default_param(struct_code, adapter, &default_types, &core_import);
                        builder.add_item(&patched);
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

        // Build a name → TypeDef map so codegen can resolve full rust_paths for
        // types that are not re-exported at the crate root (e.g. DrawingType lives
        // at sample_core::extraction::docx::drawing::DrawingType, not sample_core::DrawingType).
        let types_by_name: ahash::AHashMap<&str, &crate::core::ir::TypeDef> =
            api.types.iter().map(|t| (t.name.as_str(), t)).collect();

        for func in api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
        {
            if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &active_bridges) {
                continue;
            }
            let bridge_param = crate::backends::rustler::trait_bridge::find_bridge_param(func, &active_bridges);
            let bridge_field =
                crate::codegen::generators::trait_bridge::find_bridge_field(func, &api.types, &active_bridges);
            // Skip sanitized functions when there's no trait bridge that can replace the
            // sanitized parameter — such functions have non-bindable types (e.g. Result<T, Box<dyn Error>>)
            // and cannot be auto-delegated. Functions whose only "sanitized" param is a configured
            // trait_bridge param are emitted via gen_bridge_function.
            if func.sanitized && bridge_param.is_none() && bridge_field.is_none() {
                continue;
            }
            if let Some((param_idx, bridge_cfg)) = bridge_param {
                builder.add_item(&crate::backends::rustler::trait_bridge::gen_bridge_function(
                    api,
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                ));
            } else if let Some(ref bm) = bridge_field {
                builder.add_item(&crate::backends::rustler::trait_bridge::gen_bridge_field_function(
                    api,
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
                    &types_by_name,
                ));
            } else {
                builder.add_item(&gen_nif_function(
                    func,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                    &cpu_bound_functions,
                    &types_by_name,
                ));
            }
        }

        // Trait bridge wrappers — generate Rustler bridge structs that delegate to Elixir terms
        let has_trait_bridges = config
            .trait_bridges
            .iter()
            .any(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"));

        for bridge_cfg in config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"))
        {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                let bridge = crate::backends::rustler::trait_bridge::gen_trait_bridge(
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

        // Emit support NIFs once after all trait bridges to avoid duplicates
        if has_trait_bridges {
            let ctx = minijinja::context! {};
            builder.add_item(&crate::backends::rustler::template_env::render(
                "trait_support_nifs.rs.jinja",
                ctx,
            ));
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
                .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
                .filter(|m| {
                    // Skip methods whose return type references an excluded type.
                    // E.g. ParseOptions::builder() returns ParseOptionsBuilder which
                    // is excluded because it holds !Send + !Sync core types.
                    !crate::codegen::conversions::field_references_excluded_type(
                        &m.return_type,
                        &exclude_types.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    )
                })
            {
                let core_path = crate::codegen::conversions::core_type_path(typ, &core_import);
                if method.is_async {
                    builder.add_item(&gen_nif_async_method(
                        &typ.name,
                        &core_path,
                        method,
                        &mapper,
                        typ.is_opaque,
                        &opaque_types,
                        &default_types,
                        &core_import,
                        &adapter_bodies,
                        &types_by_name,
                    ));
                } else {
                    builder.add_item(&gen_nif_method(
                        &typ.name,
                        &core_path,
                        method,
                        &mapper,
                        typ.is_opaque,
                        &opaque_types,
                        &default_types,
                        &core_import,
                        &adapter_bodies,
                        &types_by_name,
                    ));
                }
            }
        }

        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = crate::codegen::conversions::input_type_names(api);
        // Flat data enums are output-only structs with no binding→core From impl.
        // Pass their names as `from_binding_skip_types` so that containing structs
        // (e.g. Metadata.format: Option<FormatMetadata>) use Default::default()
        // instead of .map(Into::into) in generated From impls.
        let flat_data_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| {
                let has_data = e.variants.iter().any(|v| !v.fields.is_empty());
                has_data && e.variants.iter().filter(|v| !v.fields.is_empty()).all(|v| v.is_tuple)
            })
            .map(|e| e.name.clone())
            .collect();

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

        // From/Into conversions — only for types that have NIF wrapper structs (types_to_emit).
        for typ in api.types.iter().filter(|typ| {
            !typ.is_trait && !exclude_types.contains(typ.name.as_str()) && types_to_emit.contains(&typ.name)
        }) {
            let rustler_struct_cfg = crate::codegen::conversions::ConversionConfig {
                map_as_string: false,
                exclude_types: &bridge_conv_exclude_types,
                // Flat data enums have no binding→core impl; use Default::default() for their fields.
                from_binding_skip_types: &flat_data_enum_names_vec,
                ..Default::default()
            };
            if input_types.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &rustler_struct_cfg,
                ));
            }
            if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
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
            // Flat data enums (all data variants are single-field tuple variants) use a flat
            // NifStruct representation on the Elixir side. Their core→binding conversion is
            // generated by gen_rustler_flat_data_enum_from_core instead of the generic enum
            // arm-matching codepath. They are output-only (no binding→core direction).
            let is_flat_data = has_data && e.variants.iter().filter(|v| !v.fields.is_empty()).all(|v| v.is_tuple);

            if is_flat_data {
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&gen_rustler_flat_data_enum_from_core(e, &core_import));
                }
                // Emit binding→core for input-typed flat data enums so they round-trip through
                // public function arguments (e.g. Vec<Message> in ChatCompletionRequest). The
                // discriminator field on the local struct selects the matching core variant.
                if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&gen_rustler_flat_data_enum_to_core(e, &core_import));
                }
            } else {
                let rustler_conv_config = crate::codegen::conversions::ConversionConfig {
                    binding_enums_have_data: has_data,
                    ..Default::default()
                };
                if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(
                        e,
                        &core_import,
                        &rustler_conv_config,
                    ));
                }
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(
                        e,
                        &core_import,
                        &rustler_conv_config,
                    ));
                }
            }
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&crate::codegen::error_gen::gen_rustler_error_converter(
                error,
                &core_import,
            ));
        }

        // NIF shims for whitelisted error introspection methods.
        // Each shim takes a string error message (the current Rustler error term
        // representation) and returns the method's default value.  When the
        // error-passing model is upgraded to structured terms these bodies will
        // be replaced with real dispatch; for now the declarations are emitted
        // so the Elixir public-API wrappers have matching NIF counterparts.
        for error in &api.errors {
            for method in error.methods.iter().filter(|m| !m.sanitized) {
                let fn_name = format!("{}_{}", error.name.to_lowercase(), method.name);
                let return_type = mapper.map_type(&method.return_type);
                let default_val = rustler_default_for_type(&method.return_type);
                let shim = format!(
                    "/// Introspection NIF: returns the `{method_name}` value carried by the error.\n\
                     /// Planned: extend to accept a structured error term once error passing is upgraded.\n\
                     #[allow(dead_code)]\n\
                     #[rustler::nif]\n\
                     fn {fn_name}(_msg: String) -> {return_type} {{\n    {default_val}\n}}\n",
                    method_name = method.name,
                    fn_name = fn_name,
                    return_type = return_type,
                    default_val = default_val,
                );
                builder.add_item(&shim);
            }
        }

        // from_json NIF shims for Gleam e2e tests.
        // Only emit for types that have a corresponding NIF wrapper struct (types_to_emit).
        for typ in api.types.iter().filter(|t| {
            !t.is_trait
                && !t.is_opaque
                && !t.fields.is_empty()
                && t.has_serde
                && !exclude_types.contains(t.name.as_str())
                && types_to_emit.contains(&t.name)
        }) {
            builder.add_item(&gen_from_json_nif(typ, &core_import));
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
        let exclude_functions: AHashSet<String> = elixir_config
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        // Skip binding-excluded types (service owners / handler-contract traits) — they are
        // emitted/exported by the service-API codegen, not the generic public-API listing.
        let binding_excluded_names: Vec<String> = api
            .types
            .iter()
            .filter(|t| t.binding_excluded)
            .map(|t| t.name.clone())
            .collect();
        let mut exclude_types: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();
        exclude_types.extend(binding_excluded_names.iter().map(String::as_str));

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
                Some((
                    e.name.clone(),
                    crate::codegen::naming::pascal_to_snake(&default_variant.name),
                ))
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

        // ── 2b. Opaque-type wrapper modules ───────────────────────────────────
        // Idiomatic per-type modules (e.g. `SampleLanguagePack.Parser`) that
        // wrap a Rustler ResourceArc reference in a struct (`%__MODULE__{ref: ...}`)
        // and expose the type's methods as Elixir functions that delegate to the
        // corresponding NIF (`{type_lower}_{method_name}`).
        for typ in api
            .types
            .iter()
            .filter(|typ| typ.is_opaque && !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            let opaque_content = gen_elixir_opaque_module(typ, &app_module, config);
            let file_name = format!("{}.ex", typ.name.to_snake_case());
            files.push(GeneratedFile {
                path: PathBuf::from(&output_dir)
                    .join(app_name.to_snake_case())
                    .join(file_name),
                content: opaque_content,
                generated_header: false,
            });
        }

        // ── 3. Enum modules ───────────────────────────────────────────────────
        // Build a set of known type names for resolving union payload types
        let known_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
        for enum_def in &api.enums {
            let enum_content = gen_elixir_enum_module_with_known_types(enum_def, &app_module, &known_type_names);
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
        let mut content = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        content.push_str(&template_env::render(
            "elixir_module_header.jinja",
            minijinja::context! {
                app_module => &app_module,
                moduledoc => &format!("High-level API for {app_name}"),
            },
        ));

        // Wrapper functions for top-level API functions
        for func in api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
        {
            let nif_fn_name = if func.is_async {
                let s = func.name.to_snake_case();
                if s.ends_with("_async") { s } else { format!("{s}_async") }
            } else {
                func.name.to_snake_case()
            };
            let doc_line_raw = if func.doc.is_empty() {
                "Function".to_string()
            } else {
                crate::codegen::doc_emission::doc_first_paragraph_joined(&func.doc)
            };
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

            // Count how many trailing parameters are optional (either p.optional=true or typespec has "| nil").
            // This ensures we catch Option<T> params that may have .optional=false but emit "| nil" typespecs.
            let trailing_optional_count = func
                .params
                .iter()
                .rev()
                .zip(param_types.iter().rev())
                .take_while(|(p, type_str)| p.optional || type_str.contains("| nil"))
                .count();

            // Detect if this function has a visitor bridge param.
            let visitor_bridge_param_idx: Option<usize> = func.params.iter().position(|p| {
                config.trait_bridges.iter().any(|b| {
                    b.param_name.as_deref() == Some(p.name.as_str()) || {
                        let named = match &p.ty {
                            crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                            crate::core::ir::TypeRef::Optional(inner) => {
                                if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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
                    crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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

            // Determine whether trailing optional params should be collapsed into a single
            // `opts \\ []` keyword argument (Elixir idiom) rather than N arity overloads.
            // Visitor-bridge params keep their positional form (handled below).
            let visitor_bridge_idx =
                visitor_bridge_param_idx.or_else(|| options_field_bridge.as_ref().map(|(idx, _)| *idx));
            let trailing_keyword_count = if visitor_bridge_idx.is_some() {
                // Visitor bridge present — no keyword collapsing for safety.
                0
            } else {
                trailing_optional_count
            };
            // Use keyword-opts collapsing (`opts \\ []`) for multiple trailing optionals only.
            // Single trailing optional params (e.g., `config: Option<T>`) stay positional with `\\ nil`
            // so e2e codegen can pass them as positional arguments. This preserves the common
            // config-parameter pattern where a single JSON string or nil is passed directly.
            let use_keyword_opts = trailing_keyword_count >= 2;

            // Emit one @spec/@doc per arity variant (shortest to longest).
            // The shortest arity fills optional params with nil.
            let arity_variants: Vec<usize> = if !use_keyword_opts && trailing_optional_count > 0 {
                ((all_params.len() - trailing_optional_count)..=all_params.len()).collect()
            } else if use_keyword_opts {
                // Keyword-opts path: single arity (required params + opts).
                vec![]
            } else {
                vec![all_params.len()]
            };

            // Keyword-opts path: emit a single `def f(required, opts \\ []) do` with
            // `Keyword.get(opts, :param)` for each trailing optional param.
            if use_keyword_opts {
                let required_count = all_params.len() - trailing_keyword_count;
                let required_params = &all_params[..required_count];
                let required_types = &param_types[..required_count];
                let optional_ir_params = &func.params[required_count..];

                content.push_str(&template_env::render(
                    "elixir_doc_line.jinja",
                    minijinja::context! { doc_line => doc_line },
                ));

                // @spec: required types + keyword()
                let mut spec_types: Vec<String> = required_types.to_vec();
                spec_types.push("keyword()".to_string());
                let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", spec_types.join(", "));
                if spec_inline.len() > 98 {
                    let spec_broken = format!(
                        "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                        spec_types.join(", ")
                    );
                    if spec_broken.lines().all(|l| l.len() <= 98) {
                        content.push_str(&spec_broken);
                        content.push('\n');
                    } else {
                        content.push_str(&template_env::render(
                            "elixir_spec_multiline.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                param_types => &spec_types,
                                return_spec => &return_spec,
                            },
                        ));
                    }
                } else {
                    content.push_str(&spec_inline);
                    content.push('\n');
                }

                // def fn_name(req_param, opts \\ []) do
                let mut def_parts: Vec<String> = required_params.to_vec();
                def_parts.push("opts \\\\ []".to_string());
                let def_params = def_parts.join(", ");

                // NIF call args: required positionally, optional via Keyword.get
                let mut nif_call_parts: Vec<String> = required_params.to_vec();
                for opt_p in optional_ir_params {
                    let safe_name = elixir_safe_param_name(&opt_p.name);
                    nif_call_parts.push(format!("Keyword.get(opts, :{safe_name})"));
                }
                let nif_call_str = nif_call_parts.join(",\n      ");
                content.push_str(&template_env::render(
                    "elixir_keyword_opts_wrapper.ex.jinja",
                    minijinja::context! {
                        func_name => &nif_fn_name,
                        params => &def_params,
                        native_mod => &native_mod,
                        nif_call_args => &nif_call_str,
                    },
                ));
            } else if arity_variants.is_empty() && trailing_optional_count == 0 && !all_params.is_empty() {
                // Single-arity, no keyword opts, no optional trailing params, but may have
                // optional (| nil) params in the typespec. Emit the def with defaults for
                // all params that have "| nil" in their typespec.
                let param_with_defaults: Vec<String> = param_types
                    .iter()
                    .zip(&all_params)
                    .map(|(type_str, param_name)| {
                        if type_str.contains("| nil") {
                            format!("{param_name} \\\\ nil")
                        } else {
                            param_name.clone()
                        }
                    })
                    .collect();

                content.push_str(&template_env::render(
                    "elixir_doc_line.jinja",
                    minijinja::context! { doc_line => doc_line },
                ));
                let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", param_types.join(", "));
                if spec_inline.len() > 98 {
                    let spec_broken = format!(
                        "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                        param_types.join(", ")
                    );
                    if spec_broken.lines().all(|l| l.len() <= 98) {
                        content.push_str(&spec_broken);
                        content.push('\n');
                    } else {
                        content.push_str(&template_env::render(
                            "elixir_spec_multiline.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                param_types => &param_types,
                                return_spec => &return_spec,
                            },
                        ));
                    }
                } else {
                    content.push_str(&spec_inline);
                    content.push('\n');
                }

                content.push_str(&template_env::render(
                    "elixir_def_simple.jinja",
                    minijinja::context! {
                        func_name => &nif_fn_name,
                        params => &param_with_defaults.join(", "),
                    },
                ));
                content.push_str(&template_env::render(
                    "elixir_def_nif_call.jinja",
                    minijinja::context! {
                        native_mod => &native_mod,
                        func_name => &nif_fn_name,
                        args => &all_params.join(", "),
                    },
                ));
                content.push_str("  end\n\n");
            }

            for arity in &arity_variants {
                let arity_params_slice = &all_params[..*arity];
                let arity_types = &param_types[..*arity];

                // For arity variants with positional defaults, append `\\ nil` to params
                // that have "| nil" in their typespec OR are trailing optional.
                // This allows fixtures to call functions with any intermediate arity.
                //
                // Defaults are only safe when this function emits a SINGLE clause. When
                // multiple arity variants are emitted, each shorter arity is already an
                // explicit clause; a `\\ nil` default on a longer clause would generate
                // an implicit lower-arity head that collides with it, producing a
                // "this clause cannot match" warning (fatal under --warnings-as-errors).
                let required_count = all_params.len() - trailing_optional_count;
                let single_clause = arity_variants.len() == 1;
                let arity_params: Vec<String> = arity_params_slice
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let has_nil_option = param_types.get(i).map(|t| t.contains("| nil")).unwrap_or(false);
                        if single_clause && ((i >= required_count && i < *arity) || has_nil_option) {
                            // Trailing optional param or param with | nil typespec: add default
                            format!("{p} \\\\ nil")
                        } else {
                            p.clone()
                        }
                    })
                    .collect();

                // Ensure blank line before @doc (mix format requirement between defs)
                if !content.is_empty() && !content.ends_with("\n\n") {
                    content.push('\n');
                }
                content.push_str(&template_env::render(
                    "elixir_doc_line.jinja",
                    minijinja::context! {
                        doc_line => doc_line,
                    },
                ));
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
                        content.push_str(&template_env::render(
                            "elixir_spec_multiline.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                param_types => &arity_types,
                                return_spec => &return_spec,
                            },
                        ));
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
                        content.push_str(&template_env::render(
                            "elixir_def_with_guard.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                params => &arity_params.join(", "),
                                guard_param => opts_param,
                            },
                        ));
                        content.push_str(&template_env::render(
                            "elixir_map_pop_unpack.jinja",
                            minijinja::context! {
                                opts_param => opts_param,
                                field_name => field_name,
                            },
                        ));
                        // mix format: blank line after Map.pop before if block.
                        content.push('\n');
                        content.push_str("    if is_map(visitor) do\n");
                        // Build NIF args: replace opts param with JSON-encoded clean opts, then append visitor.
                        // The _with_visitor NIF has arity = base NIF arity + 1; the trailing arg is the popped visitor map.
                        let mut with_visitor_args: Vec<String> = nif_call_args
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
                        with_visitor_args.push("visitor".to_string());
                        let with_visitor_args_str = with_visitor_args.join(", ");
                        // Emit visitor NIF call. Check line length to decide between single-line
                        // and multi-line format (mix format wraps at 98 chars).
                        let single_line = format!(
                            "      {{:ok, _}} = {native_mod}.{nif_fn_name}_with_visitor({with_visitor_args_str})\n"
                        );
                        if single_line.len() > 98 {
                            // Multi-line format that mix format produces for long calls:
                            // every positional arg on its own line. Splitting on the first
                            // ", " only would leave the 2nd+ args concatenated on one line
                            // which mix format would then rewrap on every check, breaking
                            // prek's mix-format hook.
                            content.push_str(&template_env::render(
                                "elixir_visitor_call_multiline.ex.jinja",
                                minijinja::context! {
                                    native_mod => &native_mod,
                                    func_name => &nif_fn_name,
                                    args => &with_visitor_args,
                                },
                            ));
                        } else {
                            content.push_str(&single_line);
                        }
                        content.push('\n'); // mix format: blank line before do_visitor_receive_loop.
                        content.push_str(&template_env::render(
                            "elixir_visitor_receive.jinja",
                            minijinja::context! {
                                visitor_param => "visitor",
                            },
                        ));
                        content.push_str("    else\n");
                        // No visitor: call regular NIF with options as JSON.
                        // mix format indents else body to 6 spaces (same as if body).
                        // Use clean_opts (visitor already popped) to avoid sending unknown fields to Rust.
                        let plain_args: Vec<String> = nif_call_args
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
                        let plain_args_str = plain_args.join(", ");
                        content.push_str(&template_env::render(
                            "elixir_visitor_plain_call.ex.jinja",
                            minijinja::context! {
                                native_mod => &native_mod,
                                func_name => &nif_fn_name,
                                args => &plain_args_str,
                            },
                        ));
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
                        content.push_str(&template_env::render(
                            "elixir_def_simple.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                params => &nil_clause_params.join(", "),
                            },
                        ));
                        content.push_str(&template_env::render(
                            "elixir_def_nif_call.jinja",
                            minijinja::context! {
                                native_mod => &native_mod,
                                func_name => &nif_fn_name,
                                args => &nil_nif_args.join(", "),
                            },
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
                        content.push_str(&template_env::render(
                            "elixir_def_with_guard.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                params => &arity_params.join(", "),
                                guard_param => vis_param,
                            },
                        ));
                        let with_visitor_args = nif_call_args.join(", ");
                        content.push_str(&template_env::render(
                            "elixir_visitor_call.jinja",
                            minijinja::context! {
                                native_mod => &native_mod,
                                func_name => &nif_fn_name,
                                args => &with_visitor_args,
                            },
                        ));
                        content.push_str(&template_env::render(
                            "elixir_visitor_receive.jinja",
                            minijinja::context! {
                                visitor_param => vis_param,
                            },
                        ));
                        content.push_str("  end\n\n");
                        // Nil/no-visitor clause
                        content.push_str(&template_env::render(
                            "elixir_doc_line.jinja",
                            minijinja::context! {
                                doc_line => &doc_line,
                            },
                        ));
                        let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", arity_types.join(", "));
                        if spec_inline.len() > 98 {
                            let spec_broken = format!(
                                "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                                arity_types.join(", ")
                            );
                            if spec_broken.lines().all(|l| l.len() <= 98) {
                                content.push_str(&spec_broken);
                            } else {
                                content.push_str(&template_env::render(
                                    "elixir_spec_multiline.jinja",
                                    minijinja::context! {
                                        func_name => &nif_fn_name,
                                        param_types => &arity_types,
                                        return_spec => &return_spec,
                                    },
                                ));
                            }
                        } else {
                            content.push_str(&spec_inline);
                        }
                        content.push('\n');
                        content.push_str(&template_env::render(
                            "elixir_def_simple.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                params => &arity_params.join(", "),
                            },
                        ));
                        content.push_str(&template_env::render(
                            "elixir_def_nif_call.jinja",
                            minijinja::context! {
                                native_mod => &native_mod,
                                func_name => &nif_fn_name,
                                args => &nif_call_args.join(", "),
                            },
                        ));
                        content.push_str("  end\n\n");
                        continue;
                    }
                }

                if arity_params.is_empty() {
                    content.push_str(&template_env::render(
                        "elixir_def_zero_arity.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_def_nif_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &nif_call_args.join(", "),
                        },
                    ));
                } else {
                    content.push_str(&template_env::render(
                        "elixir_def_simple.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            params => &arity_params.join(", "),
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_def_nif_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &nif_call_args.join(", "),
                        },
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
                    crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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
            let visitor_result_metadata = config.trait_bridges.iter().find_map(|bridge_cfg| {
                match crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg) {
                    Ok(metadata) => Some(metadata),
                    Err(err) => {
                        eprintln!(
                            "[alef] gen_bindings(rustler): skip visitor helper metadata for trait bridge `{}`: {err}",
                            bridge_cfg.trait_name
                        );
                        None
                    }
                }
            });
            if let Some(visitor_result_metadata) = visitor_result_metadata {
                let unit_result_variants = visitor_result_metadata
                    .unit_variants
                    .iter()
                    .map(|variant| {
                        let atom_name = variant
                            .wire_name
                            .chars()
                            .all(|c| c == '_' || c.is_ascii_alphanumeric())
                            .then(|| variant.wire_name.clone());
                        minijinja::context! {
                            wire_name => variant.wire_name.clone(),
                            atom_name => atom_name,
                        }
                    })
                    .collect::<Vec<_>>();
                content.push_str(&template_env::render(
                    "elixir_visitor_helper_functions.jinja",
                    minijinja::context! {
                        native_mod => &native_mod,
                        default_result_wire_name => visitor_result_metadata.default_variant.wire_name,
                        unit_result_variants => unit_result_variants,
                    },
                ));
            } else {
                eprintln!(
                    "[alef] gen_bindings(rustler): skip visitor helper functions because no configured result enum metadata is available"
                );
            }
        }

        // Streaming-adapter method keys — these methods are emitted as start/next
        // Type methods are now emitted in their respective type modules
        // (gen_elixir_struct_module for structs, gen_elixir_opaque_module for opaque types).
        // This avoids emitting Rust-idiomatic wrappers with no Elixir equivalents.

        // Elixir public-API wrappers for whitelisted error introspection methods.
        // Each emits an `@spec` + `def` that delegates to the corresponding NIF shim.
        for error in &api.errors {
            for method in error.methods.iter().filter(|m| !m.sanitized) {
                let nif_fn_name = format!("{}_{}", error.name.to_lowercase(), method.name);
                let return_spec = elixir_return_typespec(&method.return_type, false, &opaque_types, &default_types);
                let doc_line = if method.doc.is_empty() {
                    format!("Returns the `{}` value for the given error message.", method.name)
                } else {
                    crate::codegen::doc_emission::doc_first_paragraph_joined(&method.doc).replace('"', "\\\"")
                };
                content.push_str(&template_env::render(
                    "elixir_doc_line.jinja",
                    minijinja::context! {
                        doc_line => &doc_line,
                    },
                ));
                content.push_str(&template_env::render(
                    "elixir_error_spec.ex.jinja",
                    minijinja::context! {
                        func_name => &nif_fn_name,
                        return_spec => &return_spec,
                    },
                ));
                content.push_str(&template_env::render(
                    "elixir_def_simple.jinja",
                    minijinja::context! {
                        func_name => &nif_fn_name,
                        params => "msg",
                    },
                ));
                content.push_str(&template_env::render(
                    "elixir_def_nif_call.jinja",
                    minijinja::context! {
                        native_mod => &native_mod,
                        func_name => &nif_fn_name,
                        args => "msg",
                    },
                ));
                content.push_str("  end\n\n");
            }
        }

        // Streaming-adapter wrappers: emit the underlying `_start` / `_next` defs
        // (delegating to NIFs) plus a high-level `{name}/2` (or `/3`) function
        // returning an Elixir `Stream` driven by `Stream.unfold/2`.
        let streaming_adapters: Vec<_> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .collect();

        // StreamError exception module is emitted AFTER the outer `defmodule
        // <AppModule>` closes (see post-trim block below). Emitting `defmodule
        // <AppModule>.StreamError` INSIDE `defmodule <AppModule>` produces a
        // doubly-namespaced `Elixir.<AppModule>.<AppModule>.StreamError` because
        // Elixir treats nested `defmodule <Outer>.<Suffix>` as relative — and
        // the rebind to the doubly-nested name also breaks every plain
        // `<AppModule>.Native.X` reference in the wrapper bodies, producing
        // `Elixir.<AppModule>.<AppModule>.Native.X is undefined` warnings.

        for adapter in streaming_adapters {
            let Some(owner) = adapter.owner_type.as_deref() else {
                continue;
            };
            let owner_lc = owner.to_lowercase();
            let start_fn = format!("{owner_lc}_{}_start", adapter.name);
            let next_fn = format!("{owner_lc}_{}_next", adapter.name);
            // The high-level Stream.unfold wrapper is the public streaming entry
            // point — it must be named after the adapter (`crawl_stream`), not the
            // owner-prefixed internal form (`crawlenginehandle_crawl_stream`), so
            // callers reach it as `Module.crawl_stream/2` like every other binding.
            let stream_fn = adapter.name.to_snake_case();

            // Build the wrapper-arg list: receiver + adapter params (binding type
            // gets JSON-encoded via Jason for the NIF boundary).
            let mut start_param_names: Vec<String> = vec!["client".to_string()];
            for p in &adapter.params {
                start_param_names.push(elixir_safe_param_name(&p.name));
            }
            let start_call_args = start_param_names.join(", ");

            // _start delegate
            content.push_str(&template_env::render(
                "elixir_streaming_start_wrapper.jinja",
                minijinja::context! {
                    core_path => &adapter.core_path,
                    start_fn => &start_fn,
                    start_call_args => &start_call_args,
                    native_mod => &native_mod,
                },
            ));
            // mix-format requires a blank line before each `@doc`. The template
            // source's trailing newlines get stripped by end-of-file-fixer, so
            // insert the separator explicitly here.
            content.push('\n');

            // _next delegate
            content.push_str(&template_env::render(
                "elixir_streaming_next_wrapper.jinja",
                minijinja::context! {
                    next_fn => &next_fn,
                    native_mod => &native_mod,
                },
            ));
            content.push('\n');

            // High-level Stream.unfold wrapper. The request map is passed directly
            // to the NIF (Rustler decodes via NifMap); the NIF returns chunk JSON
            // which is decoded back into a map by the wrapper.
            let req_param = adapter
                .params
                .first()
                .map(|p| elixir_safe_param_name(&p.name))
                .unwrap_or_else(|| "request".to_string());
            let exception_module = format!("{app_module}.StreamError");
            content.push_str(&template_env::render(
                "elixir_streaming_unfold_wrapper.jinja",
                minijinja::context! {
                    core_path => &adapter.core_path,
                    stream_fn => &stream_fn,
                    req_param => &req_param,
                    native_mod => &native_mod,
                    start_fn => &start_fn,
                    next_fn => &next_fn,
                    exception_module => &exception_module,
                },
            ));
            // mix-format requires a blank line before each top-level def.
            // The next adapter iteration will emit `@doc` for the _start wrapper,
            // so insert the separator here.
            content.push('\n');
        }

        // Top-level flat wrappers for non-streaming methods on opaque types
        // (e.g. `defaultclient_chat_async/2`). The idiomatic Elixir API is exposed
        // via per-type submodules (`SampleLlm.DefaultClient.chat/2`), but consumers —
        // including the e2e fixture suite — also call the underlying NIFs through
        // flat top-level functions on the main module to mirror the streaming-wrapper
        // convention (`defaultclient_chat_stream/2`). These delegates are intentionally
        // thin: each `def` forwards directly to the corresponding `Native.*` NIF.
        let opaque_type_names: AHashSet<&str> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !t.is_trait && !exclude_types.contains(t.name.as_str()))
            .map(|t| t.name.as_str())
            .collect();
        let streaming_method_keys: AHashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
            .collect();
        for typ in api.types.iter().filter(|t| opaque_type_names.contains(t.name.as_str())) {
            let type_lc = typ.name.to_lowercase();
            for method in typ
                .methods
                .iter()
                .filter(|m| !exclude_functions.contains(m.name.as_str()))
                .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
            {
                let method_name = method.name.to_snake_case();
                let nif_fn = if method.is_async {
                    if method.name.ends_with("_async") {
                        format!("{type_lc}_{method_name}")
                    } else {
                        format!("{type_lc}_{method_name}_async")
                    }
                } else {
                    format!("{type_lc}_{method_name}")
                };

                let mut def_args: Vec<String> = Vec::new();
                if method.receiver.is_some() {
                    def_args.push("obj".to_string());
                }
                for p in &method.params {
                    def_args.push(elixir_safe_param_name(&p.name));
                }
                let args_str = def_args.join(", ");
                let doc_first = method.doc.lines().next().unwrap_or("").replace('"', "\\\"");
                content.push_str(&template_env::render(
                    "elixir_top_level_opaque_method.ex.jinja",
                    minijinja::context! {
                        doc_first => &doc_first,
                        func_name => &nif_fn,
                        args => &args_str,
                        native_mod => &native_mod,
                    },
                ));
            }
        }

        // Emit register_*, unregister_*, and clear_* delegates for every trait bridge.
        // These are excluded from the main function loop (via exclude_functions) because
        // the trait bridge generator handles them, but they must also be surfaced in the
        // public module so e2e tests can call them. However, if a clear_fn is already in
        // api.functions (emitted above with the correct Result return type), skip it here
        // to avoid duplicate clauses with conflicting specs.
        let api_fn_names: AHashSet<String> = api.functions.iter().map(|f| f.name.clone()).collect();
        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg
                .exclude_languages
                .iter()
                .any(|l| l == "elixir" || l == "rustler")
            {
                continue;
            }

            // Emit register_* delegate
            if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
                let fn_name = register_fn.to_snake_case();
                content.push_str(&template_env::render(
                    "elixir_trait_register_delegate.ex.jinja",
                    minijinja::context! {
                        trait_name => &bridge_cfg.trait_name,
                        func_name => &fn_name,
                        native_mod => &native_mod,
                    },
                ));
            }

            // Emit unregister_* delegate
            if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
                let fn_name = unregister_fn.to_snake_case();
                content.push_str(&template_env::render(
                    "elixir_trait_unregister_delegate.ex.jinja",
                    minijinja::context! {
                        trait_name => &bridge_cfg.trait_name,
                        func_name => &fn_name,
                        native_mod => &native_mod,
                    },
                ));
            }

            // Emit clear_* delegate only if not already in api.functions.
            // If the function is in api.functions, it's already emitted above with the
            // correct Result return type. Emitting again here with :: :ok | :error would
            // create duplicate clauses with incompatible specs, triggering a compile error.
            if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
                let fn_name = clear_fn.to_snake_case();
                if !api_fn_names.contains(fn_name.as_str()) {
                    content.push_str(&template_env::render(
                        "elixir_trait_clear_delegate.ex.jinja",
                        minijinja::context! {
                            trait_name => &bridge_cfg.trait_name,
                            func_name => &fn_name,
                            native_mod => &native_mod,
                        },
                    ));
                }
            }
        }

        // Trim trailing blank lines so `mix format` doesn't see an extra blank before `end`.
        let trimmed = content.trim_end_matches('\n');
        content = format!("{trimmed}\nend\n");

        // Emit the StreamError exception module at top level AFTER the outer
        // `defmodule <AppModule>` closes — Elixir's nested-defmodule semantics
        // would otherwise create `<AppModule>.<AppModule>.StreamError` (the
        // outer name is prepended) AND shadow the `<AppModule>` alias so every
        // `<AppModule>.Native.X` body reference becomes undefined.
        let streaming_adapters: Vec<_> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .collect();
        if !streaming_adapters.is_empty() {
            let exception_module = format!("{app_module}.StreamError");
            let rendered = template_env::render(
                "elixir_stream_error_exception.jinja",
                minijinja::context! {
                    exception_module => &exception_module,
                },
            );
            // The template indents by two spaces (for nested-in-defmodule usage).
            // At top level the leading spaces must be stripped so `mix format`
            // doesn't reflow the block.
            let dedented = rendered
                .lines()
                .map(|line| line.strip_prefix("  ").unwrap_or(line))
                .collect::<Vec<_>>()
                .join("\n");
            content.push('\n');
            content.push_str(&dedented);
            if !content.ends_with('\n') {
                content.push('\n');
            }
        }

        files.push(GeneratedFile {
            path: PathBuf::from(&output_dir).join(format!("{}.ex", app_name.to_snake_case())),
            content,
            generated_header: false,
        });

        Ok(files)
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
            tool: "mix",
            crate_suffix: "-rustler",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

/// Return the Rust default value literal for the given `TypeRef`.
///
/// Used to populate the body of error-introspection NIF stubs before structured
/// error terms are available.  The literal must be valid Rust and match the
/// return type produced by `RustlerMapper::map_type`.
fn rustler_default_for_type(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::Primitive(_) => "0",
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String::new()",
        _ => "Default::default()",
    }
}

/// Generate a from_json NIF shim for one serde-capable struct type.
fn gen_from_json_nif(typ: &crate::core::ir::TypeDef, core_import: &str) -> String {
    let type_name = &typ.name;
    let snake = type_name.to_snake_case();
    let fn_name = format!("{snake}_from_json");
    let core_ty = if typ.rust_path.is_empty() {
        format!("{core_import}::{type_name}")
    } else {
        typ.rust_path.replace('-', "_")
    };
    format!(
        "#[rustler::nif]\npub fn {fn_name}(json: String) -> Result<{type_name}, String> {{\n    \
        serde_json::from_str::<{core_ty}>(&json)\n        \
        .map({type_name}::from)\n        \
        .map_err(|e| e.to_string())\n}}\n"
    )
}

/// Generate the rustler::init! macro invocation.
fn gen_nif_init(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_functions: &AHashSet<String>,
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

    // Add service NIFs (emitted by service_api.rs)
    if !api.services.is_empty() {
        exports.push("complete_trait_call".to_string());
        exports.push("app_run".to_string());
        exports.push("app_into_router".to_string());
        for http_method in &[
            "get", "post", "put", "patch", "delete", "head", "options", "connect", "trace",
        ] {
            exports.push(format!("app_{}", http_method));
        }
    }

    // Deduplicate and sort for deterministic output
    exports.sort();
    exports.dedup();
    // The NIF module name must match the `defmodule` in native.ex, which is
    // `{AppModule}.Native` (e.g., `SampleMarkdown.Native`).
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
    // Also exclude types in exclude_types (e.g. VisitorHandle, ParseOptionsBuilder)
    // which are omitted from the binding layer because they hold !Send+!Sync core types.
    let opaque_types: Vec<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !exclude_types.contains(t.name.as_str()))
        .map(|t| t.name.as_str())
        .collect();

    // Streaming-adapter handle resources (e.g. `DefaultClientChatStreamHandle`).
    // These are not IR types — they are emitted by the streaming adapter — so we
    // explicitly register them here.
    let streaming_handle_types: Vec<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|a| {
            let owner = a.owner_type.as_deref()?;
            Some(format!(
                "{}{}Handle",
                streaming_handle_type_component(owner),
                streaming_handle_type_component(&a.name)
            ))
        })
        .collect();

    if !opaque_types.is_empty() || !streaming_handle_types.is_empty() {
        let mut registrations: Vec<String> = opaque_types
            .iter()
            .map(|name| {
                template_env::render(
                    "rustler_resource_registration.rs.jinja",
                    minijinja::context! {
                        type_name => name,
                    },
                )
                .trim_end()
                .to_string()
            })
            .collect();
        for name in &streaming_handle_types {
            registrations.push(
                template_env::render(
                    "rustler_resource_registration.rs.jinja",
                    minijinja::context! {
                        type_name => name,
                    },
                )
                .trim_end()
                .to_string(),
            );
        }
        let reg_body = registrations.join("\n");
        template_env::render(
            "rustler_init_with_load.rs.jinja",
            minijinja::context! {
                registrations => &reg_body,
                module => &module,
                nifs => &exports,
            },
        )
        .trim_end()
        .to_string()
    } else {
        template_env::render(
            "rustler_init.rs.jinja",
            minijinja::context! {
                module => &module,
                nifs => &exports,
            },
        )
        .trim_end()
        .to_string()
    }
}

/// Return the public type-name component used in generated Rustler resource structs.
fn streaming_handle_type_component(name: &str) -> String {
    public_host_identifier(Language::Elixir, PublicIdentifierKind::Type, name)
}

/// Patch a generated streaming `_start` NIF so its first parameter — when typed as
/// a default-typed (has_default) core type — is taken as `Option<String>` JSON and
/// deserialized to the core type before the inner method call.
///
/// Mirrors the approach used in `gen_nif_function` / `gen_nif_method` for non-streaming
/// methods. Without this patch, the generated `_start` function would expect a
/// fully-populated `NifMap` from Elixir, which fails for any partial map.
fn patch_streaming_default_param(
    code: &str,
    adapter: &crate::core::config::AdapterConfig,
    default_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let Some(first_param) = adapter.params.first() else {
        return code.to_string();
    };
    let core_ty = first_param.ty.as_str();
    if !default_types.contains(core_ty) {
        return code.to_string();
    }
    let param_name = first_param.name.as_str();

    // 1. Replace the typed param with `Option<String>`.
    let typed_param = format!("{param_name}: {core_ty},");
    let json_param = format!("{param_name}: Option<String>,");
    let mut patched = code.replace(&typed_param, &json_param);

    // 2. Replace the existing `let core_{name}: ... = {name}.into();` binding with a
    //    JSON deserialization line.
    let old_binding = format!("let core_{param_name}: {core_import}::{core_ty} = {param_name}.into();");
    let new_binding = template_env::render(
        "streaming_default_deser_binding.rs.jinja",
        minijinja::context! {
            param_name => param_name,
            core_import => core_import,
            core_ty => core_ty,
        },
    )
    .trim_end()
    .to_string();
    patched = patched.replace(&old_binding, &new_binding);

    patched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::ir::ApiSurface;

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
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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

    /// When services are present, lib.rs must declare `pub mod service;` so that
    /// the `service.rs` module (containing `#[rustler::nif]` functions) is included
    /// in the module tree and discovered by the `rustler::init!` macro.
    #[test]
    fn test_service_module_included_when_services_present() {
        use crate::core::ir::{EntrypointDef, EntrypointKind, MethodDef, ServiceDef, TypeRef};

        let config = test_config();
        let mut api = test_api();

        // Add a minimal service to trigger service.rs generation.
        let service = ServiceDef {
            name: "TestService".to_string(),
            rust_path: "test::TestService".to_string(),
            constructor: MethodDef {
                name: "new".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                is_static: true,
                error_type: None,
                doc: "Create service".to_string(),
                receiver: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![EntrypointDef {
                method: "run".to_string(),
                kind: EntrypointKind::Run,
                is_async: true,
                params: vec![],
                return_type: TypeRef::Unit,
                error_type: None,
                doc: "Run service".to_string(),
            }],
            doc: "Test service".to_string(),
            cfg: None,
        };

        api.services.push(service);

        let backend = RustlerBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();

        let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(
            lib_rs.content.contains("pub mod service;"),
            "lib.rs must declare 'pub mod service;' when services are present; content:\n{}",
            lib_rs.content
        );
    }

    /// Conversely, when no services are present, lib.rs should not declare the service module.
    #[test]
    fn test_service_module_omitted_when_no_services() {
        let config = test_config();
        let api = test_api();
        let backend = RustlerBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();

        let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(
            !lib_rs.content.contains("pub mod service;"),
            "lib.rs must NOT declare 'pub mod service;' when no services are present; content:\n{}",
            lib_rs.content
        );
    }
}
