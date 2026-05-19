mod functions;
mod helpers;
mod types;

use crate::template_env;
use crate::type_map::RustlerMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_codegen::shared::binding_fields;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::ir::ApiSurface;
use alef_core::ir::TypeRef;
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

use functions::{gen_nif_async_function, gen_nif_async_method, gen_nif_function, gen_nif_method};
use helpers::{
    elixir_return_typespec, elixir_safe_param_name, elixir_typespec, gen_elixir_enum_module_with_known_types,
    gen_elixir_opaque_module, gen_elixir_struct_module, gen_native_ex, get_module_info,
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
                if binding_fields(&typ.fields).any(|f| {
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
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Elixir)?;

        // Streaming-adapter method keys ("Owner.method_name") — these methods are emitted
        // as a pair of standalone start/next NIFs from the adapter struct hook, so the
        // regular method-iteration loop must skip them to avoid double-emitting a NIF
        // with the same name.
        let streaming_method_keys: AHashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
            .collect();

        // Emit adapter-generated standalone items (streaming iterators, callback bridges).
        for adapter in &config.adapters {
            match adapter.pattern {
                alef_core::config::AdapterPattern::Streaming => {
                    let key = alef_adapters::stream_struct_key(adapter);
                    if let Some(struct_code) = adapter_bodies.get(&key) {
                        // Post-process: convert default-typed request params to JSON strings so
                        // partial maps from Elixir decode successfully (rustler NifMap is strict).
                        let patched = patch_streaming_default_param(struct_code, adapter, &default_types, &core_import);
                        builder.add_item(&patched);
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

        // Build a name → TypeDef map so codegen can resolve full rust_paths for
        // types that are not re-exported at the crate root (e.g. DrawingType lives
        // at kreuzberg::extraction::docx::drawing::DrawingType, not kreuzberg::DrawingType).
        let types_by_name: ahash::AHashMap<&str, &alef_core::ir::TypeDef> =
            api.types.iter().map(|t| (t.name.as_str(), t)).collect();

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

        // Emit support NIFs once after all trait bridges to avoid duplicates
        if has_trait_bridges {
            let ctx = minijinja::context! {};
            builder.add_item(&crate::template_env::render("trait_support_nifs.rs.jinja", ctx));
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
                    // E.g. ConversionOptions::builder() returns ConversionOptionsBuilder which
                    // is excluded because it holds !Send + !Sync core types.
                    !alef_codegen::conversions::field_references_excluded_type(
                        &m.return_type,
                        &exclude_types.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    )
                })
            {
                let core_path = alef_codegen::conversions::core_type_path(typ, &core_import);
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

        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
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

        // From/Into conversions
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            let rustler_struct_cfg = alef_codegen::conversions::ConversionConfig {
                map_as_string: false,
                exclude_types: &bridge_conv_exclude_types,
                // Flat data enums have no binding→core impl; use Default::default() for their fields.
                from_binding_skip_types: &flat_data_enum_names_vec,
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
            // Flat data enums (all data variants are single-field tuple variants) use a flat
            // NifStruct representation on the Elixir side. Their core→binding conversion is
            // generated by gen_rustler_flat_data_enum_from_core instead of the generic enum
            // arm-matching codepath. They are output-only (no binding→core direction).
            let is_flat_data = has_data && e.variants.iter().filter(|v| !v.fields.is_empty()).all(|v| v.is_tuple);

            if is_flat_data {
                if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&gen_rustler_flat_data_enum_from_core(e, &core_import));
                }
                // Emit binding→core for input-typed flat data enums so they round-trip through
                // public function arguments (e.g. Vec<Message> in ChatCompletionRequest). The
                // discriminator field on the local struct selects the matching core variant.
                if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&gen_rustler_flat_data_enum_to_core(e, &core_import));
                }
            } else {
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
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_rustler_error_converter(
                error,
                &core_import,
            ));
        }

        // from_json NIF shims for Gleam e2e tests.
        for typ in api.types.iter().filter(|t| {
            !t.is_trait
                && !t.is_opaque
                && !t.fields.is_empty()
                && t.has_serde
                && !exclude_types.contains(t.name.as_str())
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
                Some((
                    e.name.clone(),
                    alef_codegen::naming::pascal_to_snake(&default_variant.name),
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
        // Idiomatic per-type modules (e.g. `TreeSitterLanguagePack.Parser`) that
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
        let mut content = alef_core::hash::header(alef_core::hash::CommentStyle::Hash);
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
                alef_codegen::doc_emission::doc_first_paragraph_joined(&func.doc)
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
            // Single trailing optional → arity overloads (`def f(req)` + `def f(req, opt)`),
            // matching the e2e codegen's positional call shape. Collapse to `opts \\ []`
            // keyword form only when 2+ optionals make the keyword form materially clearer.
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
                if def_parts.len() == 1 && def_parts[0] == "opts \\\\ []" {
                    // zero required params
                    content.push_str(&format!("  def {nif_fn_name}(opts \\\\ []) do\n"));
                } else {
                    content.push_str(&format!("  def {nif_fn_name}({}) do\n", def_parts.join(", ")));
                }

                // NIF call args: required positionally, optional via Keyword.get
                let mut nif_call_parts: Vec<String> = required_params.to_vec();
                for opt_p in optional_ir_params {
                    let safe_name = elixir_safe_param_name(&opt_p.name);
                    nif_call_parts.push(format!("Keyword.get(opts, :{safe_name})"));
                }
                let nif_call_str = nif_call_parts.join(",\n      ");
                content.push_str(&format!(
                    "    {native_mod}.{nif_fn_name}(\n      {nif_call_str}\n    )\n"
                ));
                content.push_str("  end\n\n");
            }

            for arity in &arity_variants {
                let arity_params = &all_params[..*arity];
                let arity_types = &param_types[..*arity];

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
                        // Emit visitor NIF call. Check line length to decide between single-line
                        // and multi-line format (mix format wraps at 98 chars).
                        let single_line = format!(
                            "      {{:ok, _}} = {native_mod}.{nif_fn_name}_with_visitor({with_visitor_args_str})\n"
                        );
                        if single_line.len() > 98 {
                            // Multi-line format that mix format produces for long calls.
                            content.push_str("      {:ok, _} =\n");
                            content.push_str(&format!("        {native_mod}.{nif_fn_name}_with_visitor(\n"));
                            let args_parts: Vec<&str> = with_visitor_args_str.splitn(2, ", ").collect();
                            if args_parts.len() == 2 {
                                content.push_str(&format!("          {},\n", args_parts[0]));
                                content.push_str(&format!("          {}\n", args_parts[1]));
                            } else {
                                content.push_str(&format!("          {with_visitor_args_str}\n"));
                            }
                            content.push_str("        )\n");
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
                    content.push_str("  def ");
                    content.push_str(&nif_fn_name);
                    content.push_str(" do\n");
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
            content.push_str(&template_env::render(
                "elixir_visitor_helper_functions.jinja",
                minijinja::context! {
                    native_mod => &native_mod,
                },
            ));
        }

        // Streaming-adapter method keys — these methods are emitted as start/next
        // pairs (see below) plus a high-level `Stream.unfold/2` wrapper, so the
        // regular method-wrapper loop must skip them.
        let streaming_method_keys: AHashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
            .collect();

        // Wrapper functions for type methods (e.g., conversionoptions_default)
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            for method in typ
                .methods
                .iter()
                .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
            {
                let nif_fn_name = if method.is_async {
                    format!("{}_{}_async", typ.name.to_lowercase(), method.name)
                } else {
                    format!("{}_{}", typ.name.to_lowercase(), method.name)
                };

                let doc_line_raw = if method.doc.is_empty() {
                    "Method".to_string()
                } else {
                    alef_codegen::doc_emission::doc_first_paragraph_joined(&method.doc)
                };
                let doc_line_escaped = doc_line_raw.replace('"', "\\\"");
                content.push_str(&template_env::render(
                    "elixir_doc_line.jinja",
                    minijinja::context! {
                        doc_line => &doc_line_escaped,
                    },
                ));

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
                        content.push_str(&template_env::render(
                            "elixir_spec_multiline.jinja",
                            minijinja::context! {
                                func_name => &nif_fn_name,
                                param_types => &type_specs,
                                return_spec => &return_spec,
                            },
                        ));
                    }
                } else {
                    content.push_str(&spec_inline);
                    content.push('\n');
                }

                if param_names.is_empty() {
                    content.push_str("  def ");
                    content.push_str(&nif_fn_name);
                    content.push_str(" do\n");
                    content.push_str(&template_env::render(
                        "elixir_def_nif_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => "",
                        },
                    ));
                } else {
                    content.push_str(&template_env::render(
                        "elixir_def_simple.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            params => &param_names.join(", "),
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_def_nif_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &param_names.join(", "),
                        },
                    ));
                }
                content.push_str("  end\n\n");
            }
        }

        // Streaming-adapter wrappers: emit the underlying `_start` / `_next` defs
        // (delegating to NIFs) plus a high-level `{name}/2` (or `/3`) function
        // returning an Elixir `Stream` driven by `Stream.unfold/2`.
        for adapter in config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
        {
            let Some(owner) = adapter.owner_type.as_deref() else {
                continue;
            };
            let owner_lc = owner.to_lowercase();
            let start_fn = format!("{owner_lc}_{}_start", adapter.name);
            let next_fn = format!("{owner_lc}_{}_next", adapter.name);
            let stream_fn = format!("{owner_lc}_{}", adapter.name);

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
            content.push_str(&template_env::render(
                "elixir_streaming_unfold_wrapper.jinja",
                minijinja::context! {
                    core_path => &adapter.core_path,
                    stream_fn => &stream_fn,
                    req_param => &req_param,
                    native_mod => &native_mod,
                    start_fn => &start_fn,
                    next_fn => &next_fn,
                },
            ));
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

/// Generate a from_json NIF shim for one serde-capable struct type.
fn gen_from_json_nif(typ: &alef_core::ir::TypeDef, core_import: &str) -> String {
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

    // Streaming-adapter handle resources (e.g. `DefaultClientChatStreamHandle`).
    // These are not IR types — they are emitted by the streaming adapter — so we
    // explicitly register them here.
    let streaming_handle_types: Vec<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
        .filter_map(|a| {
            let owner = a.owner_type.as_deref()?;
            Some(format!(
                "{}{}Handle",
                pascal_case_simple(owner),
                pascal_case_simple(&a.name)
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
            },
        )
        .trim_end()
        .to_string()
    } else {
        template_env::render(
            "rustler_init.rs.jinja",
            minijinja::context! {
                module => &module,
            },
        )
        .trim_end()
        .to_string()
    }
}

/// Convert snake_case (or already-PascalCase) to PascalCase. Used for synthesising
/// streaming handle struct names from adapter `name` and `owner_type`.
fn pascal_case_simple(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
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
    adapter: &alef_core::config::AdapterConfig,
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
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
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
