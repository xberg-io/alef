use crate::backends::php::gen_bindings::functions::{
    PhpParamTypeSets, gen_async_function_as_static_method, gen_function_as_static_method,
};
use crate::backends::php::gen_bindings::helpers::{
    gen_enum_tainted_from_binding_to_core, gen_tokio_runtime, has_enum_named_field, references_named_type,
};
use crate::backends::php::gen_bindings::rust_items::{
    gen_streaming_adapter_facade_method, generate_config_m4, has_no_arg_new_returning_self,
};
use crate::backends::php::gen_bindings::serde_defaults::gen_serde_defaults_module;
use crate::backends::php::gen_bindings::types::{
    self, gen_enum_constants, gen_flat_data_enum, gen_flat_data_enum_from_impls, gen_flat_data_enum_methods,
    gen_php_struct, is_tagged_data_enum, is_untagged_data_enum,
};
use crate::backends::php::layout::php_class_output_dir;
use crate::backends::php::naming::{php_autoload_namespace, php_ext_api_class_name};
use crate::backends::php::type_map::PhpMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::conversions::ConversionConfig;
use crate::codegen::generators::{self, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;
use std::collections::HashMap;
use std::path::PathBuf;

/// Whether the generated PHP binding crate can use serde, and therefore whether `gen_php_struct`
/// puts `#[derive(serde::Serialize, serde::Deserialize)]` on the mirror structs it emits.
///
/// This is deliberately a CRATE-level question, not a per-type one. The `from_json` constructor
/// `gen_struct_methods_impl` emits is `serde_json::from_str::<Self>(..)` where `Self` is alef's own
/// generated mirror struct — never the core type — and that mirror's serde derives come from this
/// same probe. A core type that derives nothing (`TypeDef::has_serde == false`) therefore still gets
/// a working `from_json`, and a core type that derives everything still cannot have one in a crate
/// without serde. Both the runtime bindings and the PHPStan stub must key on this single value or
/// they select different constructor shapes for the same type. ~keep
pub(super) fn php_serde_available(config: &ResolvedCrateConfig) -> bool {
    detect_serde_available(&php_output_dir(config))
}

/// Whether the generated PHP crate *requires* serde, as opposed to merely having it on disk.
///
/// `gen_flat_data_enum`/`gen_flat_data_enum_methods` hardcode serde derives and `from_json` on every
/// tagged data enum with no gate at all, so one such enum forces serde on the crate no matter what
/// the manifest probe finds. Both the runtime bindings and the PHPStan stub must key on THIS value
/// rather than on `php_serde_available` alone: keying the stub on the bare probe lets it describe a
/// regular struct's constructor with the no-serde shape while the runtime — now serde-forced by the
/// enum — emits the serde shape. That is the same stub/runtime divergence bb0787c69 removed,
/// reappearing on the regular-struct axis. Deriving both from one function is what keeps them from
/// drifting; two copies of the same expression would not. ~keep
pub(super) fn php_crate_requires_serde(api: &ApiSurface, config: &ResolvedCrateConfig) -> bool {
    php_serde_available(config) || api.enums.iter().any(is_tagged_data_enum)
}

/// The generated PHP binding crate's source directory. Both the serde probe and the writer that
/// emits `lib.rs`/`config.m4` must resolve it identically: the probe reads the Cargo manifest one
/// level above the directory the writer writes into, so a second copy of the default would let
/// them disagree about which crate is being inspected. ~keep
fn php_output_dir(config: &ResolvedCrateConfig) -> String {
    resolve_output_dir(config.output_paths.get("php"), &config.name, "crates/{name}-php/src/")
}

fn binding_config(core_import: &str, has_serde: bool) -> RustBindingConfig<'_> {
    RustBindingConfig {
        struct_attrs: &["php_class"],
        field_attrs: &[],
        struct_derives: &["Clone"],
        method_block_attr: Some("php_impl"),
        constructor_attr: "",
        static_attr: None,
        function_attr: "#[php_function]",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import,
        async_pattern: AsyncPattern::TokioBlockOn,
        has_serde,
        type_name_prefix: "",
        option_duration_on_defaults: true,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: true,
        skip_methods_when_not_delegatable: false,
        source_crate_remaps: &[],
        emit_delegating_default_for_types: None,
        delegate_deserialize_to_core_for_types: None,
    }
}

pub(super) fn generate_bindings(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    // host method so the generated `#[php_impl]` block does not emit duplicate associated
    let deduped_api = api.with_deduped_functions();
    let api = &deduped_api;

    let data_enum_names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| is_tagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();
    let untagged_data_enum_names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| is_untagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();
    let all_string_enums: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| !is_tagged_data_enum(e) && !is_untagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();

    let json_string_enum_names: AHashSet<String> = all_string_enums
        .iter()
        .filter(|enum_name| {
            api.enums
                .iter()
                .find(|e| &e.name == *enum_name)
                .map(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    let enum_names = all_string_enums;
    let mapper = PhpMapper {
        enum_names: enum_names.clone(),
        data_enum_names: data_enum_names.clone(),
        untagged_data_enum_names: untagged_data_enum_names.clone(),
        json_string_enum_names: json_string_enum_names.clone(),
    };
    let default_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let core_import = config.core_import_name();
    let lang_rename_all = config.serde_rename_all_for_language(Language::Php);
    // This binding's own configured feature set (already expanded through the core crate's
    // `[features]` graph), used to decide whether a FOREIGN-owned cfg-gated enum variant is
    // provably unreachable for this binding -- see
    // `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features` and
    // `codegen::conversions::enum_variant_declaration`. ~keep
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Php);
    let configured_features_set: std::collections::HashSet<&str> =
        enabled_features.iter().map(String::as_str).collect();

    let php_config = config.php.as_ref();
    let exclude_functions = php_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
    let exclude_types = php_config.map(|c| c.exclude_types.clone()).unwrap_or_default();

    let output_dir = php_output_dir(config);
    // `gen_flat_data_enum`/`gen_flat_data_enum_methods` hardcode serde derives and `from_json`
    // unconditionally on every tagged data enum -- deliberately not gated on the probe below,
    // because the PHPStan stub side keys the same methods on `is_tagged_data_enum` alone and must
    // not diverge from the runtime (see commit bb0787c69). A tagged data enum's flat-mirror
    // representation is inherently JSON/serde-shaped (that is the whole point of `from_json`), so
    // a crate cannot meaningfully bind one without serde; treat any such enum as forcing
    // crate-wide serde availability rather than leaving the template's unconditional serde
    // reference unmatched by the crate-wide import/derive gate below. ~keep
    let has_serde = php_crate_requires_serde(api, config);

    // Including them ensures gen_php_struct emits #[serde(skip)] for fields of those types so
    let bridge_type_aliases_php: Vec<String> = crate::backends::php::trait_bridge::active_bridges(config)
        .filter_map(|b| b.type_alias.clone())
        .collect();
    let bridge_type_aliases_set: AHashSet<String> = bridge_type_aliases_php.iter().cloned().collect();
    let mut opaque_names_vec_php: Vec<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    opaque_names_vec_php.extend(bridge_type_aliases_php);

    let mut cfg = binding_config(&core_import, has_serde);
    cfg.opaque_type_names = &opaque_names_vec_php;
    let mut never_skip_cfg_field_names: Vec<String> = crate::backends::php::trait_bridge::active_bridges(config)
        .filter_map(|b| {
            if b.bind_via == crate::core::config::BridgeBinding::OptionsField {
                b.resolved_options_field().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    // `php_binding_keeps_field` (types/structs/constructor_init.rs) drops every field with a
    // `#[cfg(...)]` gate unless its name is listed here -- unlike magnus's own struct generator,
    // which has no cfg filter at all and always keeps gated fields (correct only because Ruby's
    // Cargo.toml happens to enable every gated feature unconditionally). PHP enables the same
    // feature set, so a cfg-gated field whose gate this binding's `enabled_features` already
    // satisfies must be listed here too, or it silently vanishes from the generated PHP struct
    // even though the underlying core field exists and compiles. ~keep
    for typ in api.types.iter().filter(|t| !t.is_trait) {
        for field in &typ.fields {
            if field.cfg.is_some()
                && crate::core::ir::cfg_feature_satisfied(field.cfg.as_deref(), &configured_features_set)
            {
                never_skip_cfg_field_names.push(field.name.clone());
            }
        }
    }
    never_skip_cfg_field_names.sort();
    never_skip_cfg_field_names.dedup();
    cfg.never_skip_cfg_field_names = &never_skip_cfg_field_names;

    let mut builder = RustFileBuilder::new().with_generated_header();
    builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables, missing_docs)");
    builder.add_inner_attribute("allow(unsafe_code)");
    builder.add_inner_attribute("allow(non_snake_case)");
    builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::redundant_field_names, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy, clippy::should_implement_trait, clippy::useless_conversion)");
    if let Some(extra_attr) =
        crate::codegen::shared::format_extra_clippy_allows(&config.extra_clippy_allows, builder.inner_attributes_text())
    {
        builder.add_inner_attribute(&extra_attr);
    }
    for attribute in crate::codegen::shared::format_crate_attributes(&config.crate_attributes) {
        builder.add_inner_attribute(&attribute);
    }
    builder.add_import("ext_php_rs::prelude::*");

    if has_serde {
        builder.add_import("serde_json");
    }

    for trait_path in generators::collect_trait_imports(api) {
        builder.add_import(&trait_path);
    }

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

    builder.add_item(
        "#[derive(Debug, Clone, Default)]\n\
             pub struct PhpBytes(pub Vec<u8>);\n\
             \n\
             impl<'a> ext_php_rs::convert::FromZval<'a> for PhpBytes {\n    \
                 const TYPE: ext_php_rs::flags::DataType = ext_php_rs::flags::DataType::String;\n    \
                 fn from_zval(zval: &'a ext_php_rs::types::Zval) -> Option<Self> {\n        \
                     zval.zend_str().map(|zs| PhpBytes(zs.as_bytes().to_vec()))\n    \
                 }\n\
             }\n\
             \n\
             impl From<PhpBytes> for Vec<u8> {\n    \
                 fn from(b: PhpBytes) -> Self { b.0 }\n\
             }\n\
             \n\
             impl From<Vec<u8>> for PhpBytes {\n    \
                 fn from(v: Vec<u8>) -> Self { PhpBytes(v) }\n\
             }\n",
    );

    let custom_mods = config.custom_modules.for_language(Language::Php);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));

    if has_async {
        builder.add_item(&gen_tokio_runtime());
    }

    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    if !opaque_types.is_empty() {
        builder.add_import("std::sync::Arc");
    }

    let mutex_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && crate::codegen::generators::type_needs_mutex(t))
        .map(|t| t.name.clone())
        .collect();
    if !mutex_types.is_empty() {
        builder.add_import("std::sync::Mutex");
    }

    let extension_name = config.php_extension_name();
    let php_namespace = php_autoload_namespace(config);

    let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Php)?;

    // ~keep Source order, not a hash set: the PHP backend is the only one that *iterates* these keys
    // to emit methods (`types::gen_opaque_struct_methods_with_exclude`), and an `AHashSet` is seeded
    // per process, so the emitted `#[php_impl]` method order flipped between builds of an unchanged
    // tree. A `Vec` also matches the config-declared order used by every other PHP emitter.
    let mut seen_streaming_keys = AHashSet::new();
    let streaming_method_keys: Vec<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
        .filter(|key| seen_streaming_keys.insert(key.clone()))
        .collect();

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

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
    {
        if typ.is_opaque {
            // Generate the opaque struct with separate #[php_class] and
            // #[php(name = "Ns\\Type")] attributes (ext-php-rs 0.15+ syntax).
            let ns_escaped = php_namespace.replace('\\', "\\\\");
            let php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped, typ.name);
            let opaque_attr_arr = ["php_class", php_name_attr.as_str()];
            let opaque_cfg = RustBindingConfig {
                struct_attrs: &opaque_attr_arr,
                ..cfg
            };
            builder.add_item(&generators::gen_opaque_struct(typ, &opaque_cfg));
            builder.add_item(&types::gen_opaque_struct_methods_with_exclude(
                api,
                typ,
                &mapper,
                &opaque_types,
                &core_import,
                &adapter_bodies,
                &mutex_types,
                &streaming_method_keys,
                &config.trait_bridges,
            ));

            if has_no_arg_new_returning_self(typ) {
                let default_impl = format!(
                    "impl Default for {} {{\n    fn default() -> Self {{\n        Self::new()\n    }}\n}}",
                    typ.name
                );
                builder.add_item(&default_impl);
            }

            // Client constructor — emit a #[php_method] impl
            if let Some(ctor) = config.client_constructors.get(&typ.name) {
                let ctor_body = generators::gen_opaque_constructor(ctor, &typ.name, &core_import, "#[php_method]");
                let ctor_impl = format!("#[php_impl]\nimpl {} {{\n{}}}", typ.name, ctor_body);
                builder.add_item(&ctor_impl);
            }
        } else {
            builder.add_item(&gen_php_struct(
                typ,
                &mapper,
                &cfg,
                Some(&php_namespace),
                &enum_names,
                &lang_rename_all,
                api,
            ));
            builder.add_item(&types::gen_struct_methods_with_exclude(
                typ,
                &mapper,
                has_serde,
                &core_import,
                &opaque_types,
                &enum_names,
                &api.enums,
                &exclude_functions,
                &bridge_type_aliases_set,
                &never_skip_cfg_field_names,
                &mutex_types,
                &config.untagged_union_text_types,
            )?);
        }
    }

    for enum_def in &api.enums {
        let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(&core_import, &enum_def.rust_path);
        if is_tagged_data_enum(enum_def) {
            builder.add_item(&gen_flat_data_enum(enum_def, &mapper, Some(&php_namespace)));
            builder.add_item(&gen_flat_data_enum_methods(
                enum_def,
                &mapper,
                &opaque_types,
                &bridge_type_aliases_set,
                &enum_names,
                &core_import,
                Some(&configured_features_set),
            ));
        } else {
            builder.add_item(&gen_enum_constants(
                enum_def,
                Some(&php_namespace),
                is_host_enum,
                Some(&configured_features_set),
            ));
        }
    }

    // Free functions never become `#[php_function]` globals here — see `php_ext_api_class_name`
    // for why they are placed on the facade class instead. ~keep
    let included_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .collect();
    if !included_functions.is_empty() || !config.trait_bridges.is_empty() {
        let php_api_class_name = php_ext_api_class_name(&extension_name);
        // Build each static method body (no #[php_function] attribute — they live inside
        // a #[php_impl] block which handles registration via the class machinery).
        // Deliberately not cfg-gated: `#[php_impl]`'s derive (ext-php-rs) walks every
        // `syn::ImplItem::Fn` in this block and unconditionally emits a `FunctionBuilder`
        // registration entry referencing the method by its Rust identifier — it never inspects
        // `#[cfg]`. A method cfg'd out here would still be referenced by that generated
        // registration array, failing the build (E0599/E0425) regardless of whether a
        // `#[cfg]`-gated *body* itself would have compiled. So facade methods are always
        // emitted; `scaffold_php_cargo` (src/scaffold/languages/php.rs) compensates by requiring
        // each function's underlying core feature(s) unconditionally on the core dependency
        // line, instead of exposing them as toggleable `[features]` on the php crate. ~keep
        let mut method_items: Vec<String> = Vec::new();
        for func in included_functions {
            if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
                continue;
            }
            crate::codegen::mut_writeback::reject_unsupported_writeback(
                &func.name,
                &func.params,
                &func.return_type,
                &opaque_types,
            )?;
            let bridge_param = crate::backends::php::trait_bridge::find_bridge_param(func, &config.trait_bridges)
                .filter(|(_, bridge_cfg)| crate::backends::php::trait_bridge::targets_php(bridge_cfg));
            if let Some((param_idx, bridge_cfg)) = bridge_param {
                let bridge_handle_path =
                    crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, &core_import);
                let item = crate::backends::php::trait_bridge::gen_bridge_function(
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    &bridge_handle_path,
                );
                method_items.push(item);
            } else if func.is_async {
                let item = gen_async_function_as_static_method(
                    func,
                    &mapper,
                    PhpParamTypeSets {
                        opaque: &opaque_types,
                        default: &default_types,
                        enums: &enum_names,
                    },
                    &core_import,
                    &config.trait_bridges,
                    &mutex_types,
                );
                method_items.push(item);
            } else {
                let item = gen_function_as_static_method(
                    func,
                    &mapper,
                    PhpParamTypeSets {
                        opaque: &opaque_types,
                        default: &default_types,
                        enums: &enum_names,
                    },
                    &core_import,
                    &config.trait_bridges,
                    has_serde,
                    &mutex_types,
                );
                method_items.push(item);
            }
        }

        for adapter in &config.adapters {
            if !matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
                continue;
            }
            if adapter.owner_type.is_none() {
                continue;
            }
            method_items.push(gen_streaming_adapter_facade_method(
                adapter,
                &mapper,
                &opaque_types,
                &core_import,
            ));
        }

        for bridge_cfg in &config.trait_bridges {
            if crate::backends::php::trait_bridge::active_bridge_trait(bridge_cfg, api).is_none() {
                continue;
            }
            if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
                let php_name = crate::backends::php::naming::php_bridge_method_name(register_fn);
                method_items.push(format!(
                        "#[php(name = \"{php_name}\")]\n\
                        pub fn {register_fn}(backend: &mut ext_php_rs::types::ZendObject) -> ext_php_rs::prelude::PhpResult<()> {{\n    \
                        crate::{}(backend)\n}}",
                        register_fn
                    ));
            }
            if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
                let php_name = crate::backends::php::naming::php_bridge_method_name(unregister_fn);
                method_items.push(format!(
                    "#[php(name = \"{php_name}\")]\n\
                        pub fn {unregister_fn}(name: String) -> ext_php_rs::prelude::PhpResult<()> {{\n    \
                        crate::{unregister_fn}(name)\n}}",
                ));
            }
            if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
                let php_name = crate::backends::php::naming::php_bridge_method_name(clear_fn);
                method_items.push(format!(
                    "#[php(name = \"{php_name}\")]\n\
                        pub fn {clear_fn}() -> ext_php_rs::prelude::PhpResult<()> {{\n    \
                        crate::{clear_fn}()\n}}",
                ));
            }
        }

        let methods_joined = method_items
            .iter()
            .map(|m| {
                m.lines()
                    .map(|l| {
                        if l.is_empty() {
                            String::new()
                        } else {
                            format!("    {l}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let ns_escaped_facade = php_namespace.replace('\\', "\\\\");
        let php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped_facade, php_api_class_name);
        let facade_struct = format!(
            "#[php_class]\n#[{php_name_attr}]\npub struct {php_api_class_name};\n\n#[php_impl]\nimpl {php_api_class_name} {{\n{methods_joined}\n}}"
        );
        builder.add_item(&facade_struct);

        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = crate::backends::php::trait_bridge::active_bridge_trait(bridge_cfg, api) {
                let bridge = crate::backends::php::trait_bridge::gen_trait_bridge(
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
    }

    let convertible = crate::codegen::conversions::convertible_types(api);
    let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
    let input_types = crate::codegen::conversions::input_type_names(api);
    let enum_names_ref = &mapper.enum_names;
    let bridge_skip_types: Vec<String> = crate::backends::php::trait_bridge::active_bridges(config)
        .filter(|b| !matches!(b.bind_via, crate::core::config::BridgeBinding::OptionsField))
        .filter_map(|b| b.type_alias.clone())
        .collect();
    let trait_bridge_arc_wrapper_field_names: Vec<String> = crate::backends::php::trait_bridge::active_bridges(config)
        .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|b| b.resolved_options_field().map(String::from))
        .collect();
    // Set of opaque type names for ConversionConfig. Combines Rust `#[opaque]`
    let mut conv_opaque_types: AHashSet<String> = opaque_types.clone();
    for bridge in crate::backends::php::trait_bridge::active_bridges(config) {
        if let Some(alias) = &bridge.type_alias {
            conv_opaque_types.insert(alias.clone());
        }
    }
    // First unit (fieldless) variant name per `enum_string_names` entry: the sole safe
    // fallback a non-optional binding->core conversion can construct when a PHP-assigned
    // string fails to parse back into the enum, since `From` cannot bail out early to
    // report the error itself (see `ConversionConfig::enum_string_fallback_variant`).
    // `enum_string_names` also has a `json_string_enum_names` subset with data-carrying
    // variants, so this only fires for the first variant that genuinely takes no fields --
    // a data variant's name alone would not be a constructible expression. ~keep
    let enum_string_fallback_variant_map: AHashMap<String, String> = api
        .enums
        .iter()
        .filter(|e| enum_names_ref.contains(&e.name))
        .filter_map(|e| {
            e.variants
                .iter()
                .find(|v| v.fields.is_empty())
                .map(|v| (e.name.clone(), v.name.clone()))
        })
        .collect();
    let php_conv_config = ConversionConfig {
        cast_large_ints_to_i64: true,
        enum_string_names: Some(enum_names_ref),
        enum_string_fallback_variant: Some(&enum_string_fallback_variant_map),
        untagged_data_enum_names: Some(&mapper.untagged_data_enum_names),
        json_as_value: true,
        include_cfg_metadata: false,
        option_duration_on_defaults: true,
        from_binding_skip_types: &bridge_skip_types,
        never_skip_cfg_field_names: &never_skip_cfg_field_names,
        strip_cfg_fields_from_binding_struct: true,
        opaque_types: Some(&conv_opaque_types),
        trait_bridge_arc_wrapper_field_names: &trait_bridge_arc_wrapper_field_names,
        // Narrows a kept cfg-gated field's `#[cfg(...)]` gate (see `never_skip_cfg_field_names`
        // above) to only the feature names this crate's own `Cargo.toml` declares, so an
        // `any(...)` gate naming a feature php.toml never configures for this binding does not
        // trigger `unexpected_cfg_condition_value` under `-D warnings`. Same set that decided
        // `never_skip_cfg_field_names` via `cfg_feature_satisfied`, so the gate this narrows was
        // already proven satisfiable through it. ~keep
        declared_features: Some(&configured_features_set),
        ..Default::default()
    };
    let mut enum_tainted: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if has_enum_named_field(typ, enum_names_ref) {
            enum_tainted.insert(typ.name.clone());
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !enum_tainted.contains(&typ.name)
                && binding_fields(&typ.fields).any(|f| references_named_type(&f.ty, &enum_tainted))
            {
                enum_tainted.insert(typ.name.clone());
                changed = true;
            }
        }
    }
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if input_types.contains(&typ.name)
            && !enum_tainted.contains(&typ.name)
            && crate::codegen::conversions::can_generate_conversion(typ, &convertible)
        {
            builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                typ,
                &core_import,
                &php_conv_config,
            ));
        } else if input_types.contains(&typ.name) && enum_tainted.contains(&typ.name) {
            builder.add_item(&gen_enum_tainted_from_binding_to_core(
                typ,
                &core_import,
                enum_names_ref,
                &enum_tainted,
                &php_conv_config,
                &api.enums,
                &bridge_type_aliases_set,
            ));
        }
        if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
            builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                typ,
                &core_import,
                &opaque_types,
                &php_conv_config,
            ));
        }
    }

    let mut emitted_binding_to_core: AHashSet<String> = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && input_types.contains(&typ.name))
        .filter(|typ| {
            (enum_tainted.contains(&typ.name))
                || crate::codegen::conversions::can_generate_conversion(typ, &convertible)
        })
        .map(|typ| typ.name.clone())
        .collect();
    for enum_def in api.enums.iter().filter(|e| is_tagged_data_enum(e)) {
        builder.add_item(&gen_flat_data_enum_from_impls(
            enum_def,
            &core_import,
            Some(enabled_features.as_slice()),
        ));
        for variant in &enum_def.variants {
            for field in &variant.fields {
                if let TypeRef::Named(type_name) = &field.ty
                    && let Some(typ) = api.types.iter().find(|t| &t.name == type_name)
                {
                    if emitted_binding_to_core.contains(&typ.name) {
                        continue;
                    }
                    if enum_tainted.contains(&typ.name) {
                        builder.add_item(&gen_enum_tainted_from_binding_to_core(
                            typ,
                            &core_import,
                            enum_names_ref,
                            &enum_tainted,
                            &php_conv_config,
                            &api.enums,
                            &bridge_type_aliases_set,
                        ));
                        emitted_binding_to_core.insert(typ.name.clone());
                    } else if crate::codegen::conversions::can_generate_conversion(typ, &convertible) {
                        builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                            typ,
                            &core_import,
                            &php_conv_config,
                        ));
                        emitted_binding_to_core.insert(typ.name.clone());
                    }
                }
            }
        }
    }

    for typ in api.types.iter().filter(|t| !t.is_trait) {
        if !emitted_binding_to_core.contains(&typ.name) {
            if enum_tainted.contains(&typ.name) {
                builder.add_item(&gen_enum_tainted_from_binding_to_core(
                    typ,
                    &core_import,
                    enum_names_ref,
                    &enum_tainted,
                    &php_conv_config,
                    &api.enums,
                    &bridge_type_aliases_set,
                ));
                emitted_binding_to_core.insert(typ.name.clone());
            } else if crate::codegen::conversions::can_generate_conversion(typ, &convertible) {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &php_conv_config,
                ));
                emitted_binding_to_core.insert(typ.name.clone());
            }
        }
    }

    for error in &api.errors {
        builder.add_item(&crate::codegen::error_gen::gen_php_error_converter(error, &core_import));
        // Emit #[php_class] + #[php_impl] block for errors with introspection methods.
        let methods_impl = crate::codegen::error_gen::gen_php_error_methods_impl(error, &core_import);
        if !methods_impl.is_empty() {
            builder.add_item(&methods_impl);
        }
    }

    // Referenced by #[serde(default = "crate::serde_defaults::...")] on struct fields.
    if has_serde && let Some(serde_module) = gen_serde_defaults_module(api, &enum_names) {
        builder.add_item(&serde_module);
    }

    // `#[php_function]`/`#[php_class]` expansions remain to emit `extern "vectorcall"`
    let php_config = config.php.as_ref();
    builder.add_inner_attribute("cfg_attr(windows, feature(abi_vectorcall))");
    builder.add_inner_attribute("cfg_attr(windows, allow(unused_features))");

    if let Some(feature_name) = php_config.and_then(|c| c.feature_gate.as_deref()) {
        builder.add_inner_attribute(&format!("cfg(feature = \"{feature_name}\")"));
    }

    let mut class_registrations = String::new();
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
    {
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &typ.name },
        ));
    }
    if api.functions.iter().any(|f| !exclude_functions.contains(&f.name)) || !config.trait_bridges.is_empty() {
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &php_ext_api_class_name(&extension_name) },
        ));
    }
    for enum_def in api.enums.iter() {
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &enum_def.name },
        ));
    }
    for error in api.errors.iter().filter(|e| !e.methods.is_empty()) {
        let info_class = format!("{}Info", error.name);
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &info_class },
        ));
    }
    // The #[php_module] macro defaults to env!("CARGO_PKG_NAME"), which may differ from
    let version = &api.version;
    let module_code = format!(
        "static __EXT_PHP_RS_MODULE_STARTUP: ::std::sync::Mutex<::std::option::Option<::ext_php_rs::builders::ModuleStartup>> =\n    ::std::sync::Mutex::new(::std::option::Option::None);\n\nunsafe extern \"C\" fn __ext_php_rs_module_startup(ty: i32, mod_num: i32) -> i32 {{\n    let startup = match __EXT_PHP_RS_MODULE_STARTUP.lock() {{\n        Ok(mut guard) => guard.take(),\n        Err(_) => return 1,\n    }};\n    match startup {{\n        Some(s) => match s.startup(ty, mod_num) {{ Ok(_) => 0, Err(_) => 1 }},\n        None => 1,\n    }}\n}}\n\n#[doc(hidden)]\n#[unsafe(no_mangle)]\npub extern \"C\" fn get_module() -> *mut ::ext_php_rs::zend::ModuleEntry {{\n    static __EXT_PHP_RS_MODULE_ENTRY: ::ext_php_rs::zend::StaticModuleEntry = ::ext_php_rs::zend::StaticModuleEntry::new();\n    __EXT_PHP_RS_MODULE_ENTRY.get_or_init(|| {{\n        let builder = ::ext_php_rs::builders::ModuleBuilder::new(\"{}\", \"{}\")\n            .startup_function(__ext_php_rs_module_startup);\n        let builder = builder{};\n        match builder.try_into() {{\n            Ok((entry, startup)) => {{\n                *__EXT_PHP_RS_MODULE_STARTUP.lock().expect(\"module startup mutex poisoned\") = Some(startup);\n                entry\n            }}\n            Err(e) => panic!(\"Failed to build PHP module: {{:?}}\", e),\n        }}\n    }})\n}}\n",
        extension_name, version, class_registrations
    );
    builder.add_item(&module_code);

    let mut content = builder.build();

    for bridge in crate::backends::php::trait_bridge::active_bridges(config) {
        if let Some(field_name) = bridge.resolved_options_field() {
            let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
            let Some(type_alias) = bridge.type_alias.as_deref() else {
                continue;
            };
            let Some(options_type) = bridge.options_type.as_deref() else {
                continue;
            };
            let builder_type = format!("{}Builder", options_type);
            let bridge_struct = format!("Php{}Bridge", bridge.trait_name);
            let bridge_handle_path =
                crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge, &core_import);

            let old_method = format!(
                "        pub fn {field_name}(&self, {param_name}: Option<&{type_alias}>) -> {builder_type} {{\n        Self {{ inner: Arc::new((*self.inner).clone().{field_name}({param_name}.as_ref().map(|v| &v.inner))) }}\n    }}"
            );
            let new_method = format!(
                "        pub fn {field_name}(&self, {param_name}: &mut ext_php_rs::types::ZendObject) -> {builder_type} {{\n        let bridge = {bridge_struct}::new({param_name});\n        let handle: {bridge_handle_path} = std::sync::Arc::new(std::sync::Mutex::new(bridge));\n        Self {{ inner: Arc::new((*self.inner).clone().{field_name}(Some(handle))) }}\n    }}"
            );

            content = content.replace(&old_method, &new_method);
        }
    }

    let php_stubs_dir = php_class_output_dir(config);

    let php_namespace = php_autoload_namespace(config);

    let mut generated_files = vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("lib.rs"),
        content,
        generated_header: false,
    }];

    let extension_name = config.php_extension_name();
    let config_m4 = generate_config_m4(&extension_name, &config.name);
    let mut config_m4_path = PathBuf::from(&output_dir);
    config_m4_path.pop();
    config_m4_path.pop();
    config_m4_path.pop();
    config_m4_path.push("config.m4");

    generated_files.push(GeneratedFile {
        path: config_m4_path,
        content: config_m4,
        generated_header: false,
    });

    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_type) = crate::backends::php::trait_bridge::active_bridge_trait(bridge_cfg, api) {
            let is_visitor_bridge = bridge_cfg.type_alias.is_some()
                && bridge_cfg.register_fn.is_none()
                && bridge_cfg.super_trait.is_none()
                && bridge_cfg.context_type.is_some()
                && bridge_cfg.result_type.is_some()
                && trait_type.methods.iter().all(|m| m.has_default_impl);

            let interface_content = if is_visitor_bridge {
                crate::backends::php::trait_bridge::gen_visitor_interface(
                    trait_type,
                    bridge_cfg,
                    &php_namespace,
                    &HashMap::new(),
                    api,
                )
            } else {
                crate::backends::php::trait_bridge::gen_registration_interface(
                    trait_type,
                    bridge_cfg,
                    &php_namespace,
                    &HashMap::new(),
                    api,
                )
            };

            // The file basename must match the class name the content above actually declares --
            // PSR-4 requires it, and a visitor bridge's class is `{trait_name}Interface`, not
            // `{trait_name}` (see `visitor_interface_class_name`'s doc for the alef #485 incident
            // this guards against). Naming both from the same helper, rather than recomputing the
            // suffix here, is what keeps this call site from drifting from `gen_visitor_interface`
            // again. ~keep
            let interface_class_name = if is_visitor_bridge {
                crate::backends::php::trait_bridge::visitor_interface_class_name(&bridge_cfg.trait_name)
            } else {
                bridge_cfg.trait_name.clone()
            };
            let interface_filename = format!("{interface_class_name}.php");
            generated_files.push(GeneratedFile {
                path: PathBuf::from(&php_stubs_dir).join(&interface_filename),
                content: interface_content,
                generated_header: false,
            });
        }
    }

    Ok(generated_files)
}
