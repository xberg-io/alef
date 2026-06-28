use crate::backends::rustler::gen_bindings::functions::{
    gen_nif_async_function, gen_nif_async_method, gen_nif_function, gen_nif_method, regate_ungated_same_name_functions,
};
use crate::backends::rustler::gen_bindings::helpers::{collect_types_for_nif_derives, get_module_info};
use crate::backends::rustler::gen_bindings::rust_items::{
    gen_from_json_nif, gen_nif_init, patch_streaming_default_param, rustler_default_for_type,
};
use crate::backends::rustler::gen_bindings::types::{
    gen_enum, gen_opaque_resource, gen_rustler_config_impl, gen_rustler_flat_data_enum_from_core,
    gen_rustler_flat_data_enum_to_core, gen_struct,
};
use crate::backends::rustler::type_map::RustlerMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use std::path::PathBuf;

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
}

pub(super) fn generate_bindings(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
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
                let ctor_body = crate::codegen::generators::gen_opaque_constructor(ctor, &typ.name, &core_import, "");
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
        builder.add_item("type TraitReplyChannel = tokio::sync::oneshot::Sender<std::result::Result<String, String>>;");
        builder.add_item(
            "static TRAIT_REPLY_CHANNELS: std::sync::LazyLock<Mutex<HashMap<u64, TraitReplyChannel>>> = \
                 std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));",
        );
    }

    // Build a name → TypeDef map so codegen can resolve full rust_paths for
    // types that are not re-exported at the crate root (e.g. DrawingType lives
    // at sample_core::extraction::docx::drawing::DrawingType, not sample_core::DrawingType).
    let types_by_name: ahash::AHashMap<&str, &crate::core::ir::TypeDef> =
        api.types.iter().map(|t| (t.name.as_str(), t)).collect();

    // Re-gate ungated NIF definitions that share a name with gated arms so the fallback NIF
    // compiles only when no gated arm does. Without this, an unconditional inline fallback module
    // whose enclosing cfg gate was dropped by extraction would emit a second `#[rustler::nif]`
    // alongside the active cfg arm, aborting `on_load` with a "Duplicate NIF entry" error.
    let regated_functions = regate_ungated_same_name_functions(&api.functions);

    for func in regated_functions
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
            let item = crate::backends::rustler::trait_bridge::gen_bridge_function(
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
        } else if let Some(ref bm) = bridge_field {
            let item = crate::backends::rustler::trait_bridge::gen_bridge_field_function(
                api,
                func,
                bm,
                bm.bridge,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
        } else if func.is_async {
            let item = gen_nif_async_function(
                func,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
                &types_by_name,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
        } else {
            let item = gen_nif_function(
                func,
                &mapper,
                &opaque_types,
                &default_types,
                &core_import,
                &cpu_bound_functions,
                &types_by_name,
            );
            let item = prepend_cfg(func.cfg.as_deref(), item);
            builder.add_item(&item);
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
            // Rustler NIFs run synchronously off the BEAM scheduler with no ambient Tokio
            // runtime, so `tokio::task::spawn_blocking` in the sync trait-callback bodies
            // panics ("there is no reactor running"). Swap the fire-and-forget reply send
            // to a plain OS thread. (Canonical home: the `trait_sync_method_body.rs.jinja`
            // template; patched here to stay within the permitted edit scope.)
            let bridge_code = bridge.code.replace(
                "drop(tokio::task::spawn_blocking(move || {",
                "drop(std::thread::spawn(move || {",
            );
            builder.add_item(&bridge_code);
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
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()) && types_to_emit.contains(&typ.name))
    {
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

    // Always-emitted test-support NIF: lets the e2e harness set process environment
    // variables that native code reads via libc `getenv` (e.g. crawlberg's SSRF loopback
    // allowlist). Erlang's `os:putenv/2` updates only the BEAM's own env table, not the
    // C runtime's `environ`, so a dedicated NIF is required. Auto-registered by the
    // `rustler::init!` module scan; the matching `set_env/2` stub is injected into the
    // generated `{App}.Native` module by the public-API codegen.
    builder.add_item(
        "#[rustler::nif]\npub fn set_env(key: String, value: String) -> bool {\n    \
         // SAFETY: called once from the e2e harness during setup before extraction\n    \
         // NIFs run on other scheduler threads, so no concurrent env access.\n    \
         unsafe { std::env::set_var(&key, &value); }\n    true\n}\n",
    );

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
