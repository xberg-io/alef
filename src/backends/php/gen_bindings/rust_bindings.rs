use crate::backends::php::gen_bindings::functions::{
    PhpParamTypeSets, gen_async_function_as_static_method, gen_function_as_static_method,
};
use crate::backends::php::gen_bindings::helpers::{
    gen_enum_tainted_from_binding_to_core, gen_tokio_runtime, has_enum_named_field, references_named_type,
};
use crate::backends::php::gen_bindings::rust_items::{
    gen_streaming_adapter_facade_method, generate_config_m4, has_no_arg_new_returning_self,
    php_variant_wrapper_constructor,
};
use crate::backends::php::gen_bindings::serde_defaults::gen_serde_defaults_module;
use crate::backends::php::gen_bindings::types::{
    self, gen_enum_constants, gen_flat_data_enum, gen_flat_data_enum_from_impls, gen_flat_data_enum_methods,
    gen_php_struct, is_tagged_data_enum, is_untagged_data_enum,
};
use crate::backends::php::naming::php_autoload_namespace;
use crate::backends::php::type_map::PhpMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::conversions::ConversionConfig;
use crate::codegen::generators::{self, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToPascalCase};
use minijinja::context;
use std::collections::HashMap;
use std::path::PathBuf;

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
    }
}

pub(super) fn generate_bindings(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    // Separate unit-variant enums (→ String), tagged data enums (→ flat PHP class),
    // and untagged data enums (→ serde_json::Value, converted via from_value at binding↔core boundary).
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
    // String-mapped enums: everything that is NOT a tagged-data enum AND NOT an untagged-data enum.
    // Includes unit-variant enums (FilePurpose, ToolType, …) which are exposed as PHP string constants.
    let enum_names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| !is_tagged_data_enum(e) && !is_untagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();
    let mapper = PhpMapper {
        enum_names: enum_names.clone(),
        data_enum_names: data_enum_names.clone(),
        untagged_data_enum_names: untagged_data_enum_names.clone(),
    };
    let default_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let core_import = config.core_import_name();
    let lang_rename_all = config.serde_rename_all_for_language(Language::Php);

    // Get exclusion lists from PHP config
    let php_config = config.php.as_ref();
    let exclude_functions = php_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
    let exclude_types = php_config.map(|c| c.exclude_types.clone()).unwrap_or_default();

    let output_dir = resolve_output_dir(config.output_paths.get("php"), &config.name, "crates/{name}-php/src/");
    let has_serde = detect_serde_available(&output_dir);

    // Build the opaque type names list: IR opaque types + bridge type aliases.
    // Bridge type aliases wrap Rc-based handles and cannot implement serde::Serialize/Deserialize.
    // Including them ensures gen_php_struct emits #[serde(skip)] for fields of those types so
    // derives on the enclosing struct still compile.
    let bridge_type_aliases_php: Vec<String> = config
        .trait_bridges
        .iter()
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
    let never_skip_cfg_field_names: Vec<String> = config
        .trait_bridges
        .iter()
        .filter_map(|b| {
            if b.bind_via == crate::core::config::BridgeBinding::OptionsField {
                b.resolved_options_field().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    cfg.never_skip_cfg_field_names = &never_skip_cfg_field_names;

    // Build the inner module content (types, methods, conversions)
    let mut builder = RustFileBuilder::new().with_generated_header();
    builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables, missing_docs)");
    builder.add_inner_attribute("allow(unsafe_code)");
    // PHP parameter names are lowerCamelCase; Rust complains about non-snake_case variables.
    builder.add_inner_attribute("allow(non_snake_case)");
    builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy, clippy::should_implement_trait, clippy::useless_conversion)");
    builder.add_import("ext_php_rs::prelude::*");

    // Import serde_json when available (needed for serde-based param conversion)
    if has_serde {
        builder.add_import("serde_json");
    }

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

    // PhpBytes wrapper: accepts PHP binary strings without UTF-8 validation.
    // ext-php-rs's String FromZval rejects non-UTF-8 strings, so binary content
    // (PDFs, images, etc.) gets "Invalid value given for argument" errors. This
    // wrapper reads the raw bytes via `zend_str()` and exposes them as Vec<u8>.
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

    // Custom module declarations
    let custom_mods = config.custom_modules.for_language(Language::Php);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    // Check if any function or method is async
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));

    if has_async {
        builder.add_item(&gen_tokio_runtime());
    }

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

    // Compute mutex types: opaque types with &mut self methods
    let mutex_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && crate::codegen::generators::type_needs_mutex(t))
        .map(|t| t.name.clone())
        .collect();
    if !mutex_types.is_empty() {
        builder.add_import("std::sync::Mutex");
    }

    // Compute the PHP namespace for namespaced class registration.
    // Delegates to config so [php].namespace overrides are respected.
    let extension_name = config.php_extension_name();
    let php_namespace = php_autoload_namespace(config);

    // Build adapter body map before type iteration so bodies are available for method generation.
    let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Php)?;

    // Streaming-adapter method keys ("Owner.method_name") — these methods are emitted
    // as a triple of standalone functions (start/next/free) from the adapter struct hook, so the
    // regular method-iteration loop must skip them to avoid double-emitting a function
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
            // Escape '\' in the namespace so the generated Rust string literal is valid.
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

            // Emit impl Default if the type has a no-arg new() -> Self.
            // This satisfies clippy's should_implement_trait lint.
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
            // Variant-wrapper constructor — emit a #[php(constructor)] impl so PHP callers
            // can instantiate the wrapper with `new TypeName(args)` constructor syntax, as
            // produced by variant bodies. `client_constructors` takes priority above.
            } else if typ.is_variant_wrapper {
                if let Some(ctor) = php_variant_wrapper_constructor(typ, &mapper, &core_import) {
                    builder.add_item(&ctor);
                }
            }
        } else {
            // gen_php_struct emits an explicit delegating `impl Default for BindingType`
            // for has_default types (driven by `emit_delegating_default_impl: true` in
            // the PHP binding config). The auto-derived `Default` is suppressed in the
            // shared struct generator so the two do not collide.
            builder.add_item(&gen_php_struct(
                typ,
                &mapper,
                &cfg,
                Some(&php_namespace),
                &enum_names,
                &lang_rename_all,
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
            ));
        }
    }

    for enum_def in &api.enums {
        if is_tagged_data_enum(enum_def) {
            // Tagged data enums (struct variants) are lowered to a flat PHP class.
            builder.add_item(&gen_flat_data_enum(enum_def, &mapper, Some(&php_namespace)));
            builder.add_item(&gen_flat_data_enum_methods(enum_def, &mapper));
        } else {
            // Unit-variant enums are lowered to PHP classes with class constants.
            builder.add_item(&gen_enum_constants(enum_def, Some(&php_namespace)));
        }
    }

    // Generate free functions as static methods on a facade class rather than standalone
    // `#[php_function]` items. Standalone functions rely on the `inventory` crate for
    // auto-registration, which does not work in cdylib builds on macOS. Classes registered
    // via `.class::<T>()` in the module builder DO work on all platforms.
    let included_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .collect();
    if !included_functions.is_empty() || !config.trait_bridges.is_empty() {
        let facade_class_name = extension_name.to_pascal_case();
        // Build each static method body (no #[php_function] attribute — they live inside
        // a #[php_impl] block which handles registration via the class machinery).
        let mut method_items: Vec<String> = Vec::new();
        for func in included_functions {
            if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
                continue;
            }
            let bridge_param = crate::backends::php::trait_bridge::find_bridge_param(func, &config.trait_bridges);
            if let Some((param_idx, bridge_cfg)) = bridge_param {
                let bridge_handle_path =
                    crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, &core_import);
                method_items.push(crate::backends::php::trait_bridge::gen_bridge_function(
                    func,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    &bridge_handle_path,
                ));
            } else if func.is_async {
                method_items.push(gen_async_function_as_static_method(
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
                ));
            } else {
                method_items.push(gen_function_as_static_method(
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
                ));
            }
        }

        // Emit streaming adapter facade methods as static methods
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

        // Emit trait-bridge registration functions as static methods
        for bridge_cfg in &config.trait_bridges {
            if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
                let php_name = register_fn.to_lower_camel_case();
                method_items.push(format!(
                        "#[php(name = \"{php_name}\")]\n\
                        pub fn {register_fn}(backend: &mut ext_php_rs::types::ZendObject) -> ext_php_rs::prelude::PhpResult<()> {{\n    \
                        crate::{}(backend)\n}}",
                        register_fn
                    ));
            }
            if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
                let php_name = unregister_fn.to_lower_camel_case();
                method_items.push(format!(
                    "#[php(name = \"{php_name}\")]\n\
                        pub fn {unregister_fn}(name: String) -> ext_php_rs::prelude::PhpResult<()> {{\n    \
                        crate::{unregister_fn}(name)\n}}",
                ));
            }
            if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
                let php_name = clear_fn.to_lower_camel_case();
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
                // Indent each line of each method by 4 spaces
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
        // The PHP-visible class name gets an "Api" suffix to avoid collision with the
        // PHP facade class (e.g. `SampleCrawler\SampleCrawler`) that Composer autoloads.
        let php_api_class_name = format!("{facade_class_name}Api");
        // Escape '\' so the generated Rust string literal is valid (e.g. "Ns\\ClassName").
        let ns_escaped_facade = php_namespace.replace('\\', "\\\\");
        let php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped_facade, php_api_class_name);
        let facade_struct = format!(
            "#[php_class]\n#[{php_name_attr}]\npub struct {facade_class_name}Api;\n\n#[php_impl]\nimpl {facade_class_name}Api {{\n{methods_joined}\n}}"
        );
        builder.add_item(&facade_struct);

        // Trait bridge structs — top-level items (outside the facade class)
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
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
    let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api);
    let input_types = crate::codegen::conversions::input_type_names(api);
    // From/Into conversions with PHP-specific i64 casts.
    // Types with enum Named fields (or that reference such types transitively) can't
    // have binding->core From impls because PHP maps enums to String and there's no
    // From<String> for the core enum type. Core->binding is always safe.
    let enum_names_ref = &mapper.enum_names;
    let bridge_skip_types: Vec<String> = config
        .trait_bridges
        .iter()
        .filter(|b| !matches!(b.bind_via, crate::core::config::BridgeBinding::OptionsField))
        .filter_map(|b| b.type_alias.clone())
        .collect();
    // Trait-bridge fields whose binding-side wrapper holds `inner: Arc<core::T>`
    // (every OptionsField-style bridge in alef follows this convention). Used by
    // `binding_to_core` to emit `val.{f}.map(|v| (*v.inner).clone())` instead of
    // `Default::default()` so the visitor handle survives the `.into()` call.
    let trait_bridge_arc_wrapper_field_names: Vec<String> = config
        .trait_bridges
        .iter()
        .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|b| b.resolved_options_field().map(String::from))
        .collect();
    // Set of opaque type names for ConversionConfig. Combines Rust `#[opaque]`
    // types in the API with trait-bridge type aliases (e.g. VisitorHandle) so the
    // `is_opaque_no_wrapper_field` branch in binding_to_core fires for those
    // fields and emits the Arc-wrapper forwarding pattern.
    let mut conv_opaque_types: AHashSet<String> = opaque_types.clone();
    for bridge in &config.trait_bridges {
        if let Some(alias) = &bridge.type_alias {
            conv_opaque_types.insert(alias.clone());
        }
    }
    let php_conv_config = ConversionConfig {
        cast_large_ints_to_i64: true,
        enum_string_names: Some(enum_names_ref),
        untagged_data_enum_names: Some(&mapper.untagged_data_enum_names),
        // PHP keeps `serde_json::Value` as-is in the binding struct (matches PhpMapper::json).
        // `json_to_string` was previously enabled but caused `from_json` to fail when a JSON
        // object/array landed in a `String`-typed field (e.g. tool `parameters` schema).
        json_as_value: true,
        include_cfg_metadata: false,
        option_duration_on_defaults: true,
        from_binding_skip_types: &bridge_skip_types,
        never_skip_cfg_field_names: &never_skip_cfg_field_names,
        opaque_types: Some(&conv_opaque_types),
        trait_bridge_arc_wrapper_field_names: &trait_bridge_arc_wrapper_field_names,
        ..Default::default()
    };
    // Build transitive set of types that can't have binding->core From
    let mut enum_tainted: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if has_enum_named_field(typ, enum_names_ref) {
            enum_tainted.insert(typ.name.clone());
        }
    }
    // Transitively mark types that reference enum-tainted types
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
        // binding->core: only when not enum-tainted and type is used as input
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
            // Enum-tainted types: generate From with string->enum parsing for enum-Named
            // fields, using first variant as fallback. Data-variant enum fields fill
            // data fields with Default::default().
            // Note: JSON roundtrip was previously used when has_serde=true, but that
            // breaks on non-optional Duration fields (null != u64) and empty-string enum
            // fields ("" is not a valid variant). Field-by-field conversion handles both.
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
        // core->binding: always (enum->String via format, sanitized fields via format)
        if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
            builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                typ,
                &core_import,
                &opaque_types,
                &php_conv_config,
            ));
        }
    }

    // From impls for tagged data enums lowered to flat PHP classes.
    // Track types whose `From<binding> for core` impl has already been emitted by
    // the main loop above (or by a prior variant in this loop) to avoid duplicate
    // impls when the same DTO appears both as a top-level input type and as a
    // variant payload of a tagged enum (e.g. `CrawlPageResult` used directly and
    // inside `CrawlEvent::Page { result: Box<CrawlPageResult> }`).
    // The main loop above emits a `From<binding> for core` impl for any type
    // that is `input_types.contains(&typ.name)` (either via the plain branch
    // or the enum-tainted branch). Pre-seed the dedup set with those.
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
        builder.add_item(&gen_flat_data_enum_from_impls(enum_def, &core_import));
        // Also generate From impls for variant data types (e.g., ArchiveMetadata from FormatMetadata::Archive).
        // These are needed when flat enum binding→core conversion calls `.into()` on variant fields.
        for variant in &enum_def.variants {
            for field in &variant.fields {
                if let TypeRef::Named(type_name) = &field.ty {
                    if let Some(typ) = api.types.iter().find(|t| &t.name == type_name) {
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
    }

    // Emit From impls for all remaining DTO types that are convertible but haven't been
    // emitted yet. This handles nested types that appear as fields in output structures
    // but are not direct input types or enum variant payloads.
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

    // Error converter functions + optional introspection method impl structs
    for error in &api.errors {
        builder.add_item(&crate::codegen::error_gen::gen_php_error_converter(error, &core_import));
        // Emit #[php_class] + #[php_impl] block for errors with introspection methods.
        let methods_impl = crate::codegen::error_gen::gen_php_error_methods_impl(error, &core_import);
        if !methods_impl.is_empty() {
            builder.add_item(&methods_impl);
        }
    }

    // Serde default helpers generated from IR typed-default metadata.
    // Referenced by #[serde(default = "crate::serde_defaults::...")] on struct fields.
    if has_serde {
        if let Some(serde_module) = gen_serde_defaults_module(api) {
            builder.add_item(&serde_module);
        }
    }

    // Always enable abi_vectorcall on Windows — ext-php-rs requires the
    // `vectorcall` calling convention for PHP entry points there. The feature
    // is unstable on stable Rust; consumers either build with nightly or set
    // RUSTC_BOOTSTRAP=1 (the upstream-recommended workaround). This cfg_attr
    // is a no-op on non-windows so it costs nothing on Linux/macOS builds.
    let php_config = config.php.as_ref();
    builder.add_inner_attribute("cfg_attr(windows, feature(abi_vectorcall))");

    // Optional feature gate — when [php].feature_gate is set, the entire crate
    // is conditionally compiled. Use this for parity with PyO3's `extension-module`
    // pattern; most PHP bindings don't need it.
    if let Some(feature_name) = php_config.and_then(|c| c.feature_gate.as_deref()) {
        builder.add_inner_attribute(&format!("cfg(feature = \"{feature_name}\")"));
    }

    // PHP module entry point — explicit class registration required because
    // `inventory` crate auto-registration doesn't work in cdylib on macOS.
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
    // Register the facade class that wraps free functions as static methods.
    if api.functions.iter().any(|f| !exclude_functions.contains(&f.name)) || !config.trait_bridges.is_empty() {
        let facade_class_name = extension_name.to_pascal_case();
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &format!("{facade_class_name}Api") },
        ));
    }
    // All enums are lowered to PHP classes: tagged data enums as flat classes,
    // unit-variant enums as classes with class constants. Register them all.
    for enum_def in api.enums.iter() {
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &enum_def.name },
        ));
    }
    // Register error info classes for errors that expose introspection methods.
    for error in api.errors.iter().filter(|e| !e.methods.is_empty()) {
        let info_class = format!("{}Info", error.name);
        class_registrations.push_str(&crate::backends::php::template_env::render(
            "php_class_registration.jinja",
            context! { class_name => &info_class },
        ));
    }
    // Generate the PHP module entry point explicitly with the correct extension name.
    // The #[php_module] macro defaults to env!("CARGO_PKG_NAME"), which may differ from
    // the publishable extension name (e.g., crate "ts-pack-core-php" vs. extension "tree_sitter_language_pack").
    // By using ModuleBuilder::new(extension_name, version) explicitly, we ensure
    // the registered module name matches the php.ini directive and PIE installation filename.
    let version = &api.version;
    let module_code = format!(
        "#[doc(hidden)]\n#[unsafe(no_mangle)]\npub extern \"C\" fn get_module() -> *mut ::ext_php_rs::zend::ModuleEntry {{\n    static __EXT_PHP_RS_MODULE_ENTRY: ::ext_php_rs::zend::StaticModuleEntry = ::ext_php_rs::zend::StaticModuleEntry::new();\n    __EXT_PHP_RS_MODULE_ENTRY.get_or_init(|| {{\n        let builder = ::ext_php_rs::builders::ModuleBuilder::new(\"{}\", \"{}\");\n        let builder = builder{};\n        match builder.try_into() {{\n            Ok((entry, _startup)) => entry,\n            Err(e) => panic!(\"Failed to build PHP module: {{:?}}\", e),\n        }}\n    }})\n}}\n",
        extension_name, version, class_registrations
    );
    builder.add_item(&module_code);

    let mut content = builder.build();

    // Post-process generated code to replace the bridge builder method.
    // The generated code produces `visitor(Option<&VisitorHandle>)` which is
    // unreachable from PHP. Replace the entire method — signature and body —
    // with one that accepts a ZendObject and builds the proper bridge handle.
    for bridge in &config.trait_bridges {
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

            // Match the verbatim pre-rustfmt output from codegen.
            // gen_instance_method produces 4-space-indented lines (signature + body),
            // then ImplBuilder.build() adds 4 more spaces to every line → 8/8/4 indent.
            // The body is a single-line Self { inner: Arc::new(...) } expression.
            // rustfmt later reformats this to the 4/8/8/4 multi-line style on disk.
            let old_method = format!(
                "        pub fn {field_name}(&self, {param_name}: Option<&{type_alias}>) -> {builder_type} {{\n        Self {{ inner: Arc::new((*self.inner).clone().{field_name}({param_name}.as_ref().map(|v| &v.inner))) }}\n    }}"
            );
            let new_method = format!(
                "        pub fn {field_name}(&self, {param_name}: &mut ext_php_rs::types::ZendObject) -> {builder_type} {{\n        let bridge = {bridge_struct}::new({param_name});\n        let handle: {bridge_handle_path} = std::sync::Arc::new(std::sync::Mutex::new(bridge));\n        Self {{ inner: Arc::new((*self.inner).clone().{field_name}(Some(handle))) }}\n    }}"
            );

            content = content.replace(&old_method, &new_method);
        }
    }

    // Generate PHP interface files for visitor-style bridges.
    // Use PHP stubs output path if configured, otherwise fall back to packages/php/src/.
    let php_stubs_dir = config
        .php
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| s.output.to_string_lossy().to_string())
        .unwrap_or_else(|| "packages/php/src/".to_string());

    let php_namespace = php_autoload_namespace(config);

    let mut generated_files = vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("lib.rs"),
        content,
        generated_header: false,
    }];

    // Generate config.m4 for PIE (PHP Installer for Extensions) builds.
    // When PIE falls back from pre-packaged binaries to source compilation,
    // phpize expects config.m4 to describe the build process.
    let extension_name = config.php_extension_name();
    let config_m4 = generate_config_m4(&extension_name, &config.name);
    // The config.m4 file must be at the repository root (one level above the Cargo.toml)
    // output_dir is like "crates/<name>-core-php/src", so we pop three times to get to root.
    let mut config_m4_path = PathBuf::from(&output_dir);
    config_m4_path.pop(); // remove "src"
    config_m4_path.pop(); // remove crate directory
    config_m4_path.pop(); // remove "crates"
    config_m4_path.push("config.m4");

    generated_files.push(GeneratedFile {
        path: config_m4_path,
        content: config_m4,
        generated_header: false,
    });

    // Emit PHP interface files for all trait bridges (visitor-style and registration-style)
    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
            // Check if this is a visitor-style bridge (has type_alias, no register_fn, all methods have defaults)
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
                    &HashMap::new(), // type_paths not needed for the interface file itself
                )
            } else {
                // Registration-style bridge: generate full interface with super-trait methods
                crate::backends::php::trait_bridge::gen_registration_interface(
                    trait_type,
                    bridge_cfg,
                    &php_namespace,
                    &HashMap::new(), // type_paths not needed for the interface file itself
                )
            };

            let interface_filename = format!("{}.php", bridge_cfg.trait_name);
            generated_files.push(GeneratedFile {
                path: PathBuf::from(&php_stubs_dir).join(&interface_filename),
                content: interface_content,
                generated_header: false,
            });
        }
    }

    Ok(generated_files)
}
