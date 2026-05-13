mod functions;
mod helpers;
mod types;

use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::conversions::ConversionConfig;
use alef_codegen::generators::RustBindingConfig;
use alef_codegen::generators::{self, AsyncPattern};
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;
use alef_core::ir::{PrimitiveType, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase};
use minijinja::context;
use std::path::PathBuf;

use crate::naming::php_autoload_namespace;
use functions::{gen_async_function_as_static_method, gen_function_as_static_method};

/// PHP 8.1 enum cases cannot use case-insensitive `class` (reserved for
/// `EnumName::class` syntax). Append a trailing underscore for those cases.
fn sanitize_php_enum_case(name: &str) -> String {
    if name.eq_ignore_ascii_case("class") {
        format!("{name}_")
    } else {
        name.to_string()
    }
}
use helpers::{gen_enum_tainted_from_binding_to_core, gen_tokio_runtime, has_enum_named_field, references_named_type};
use types::{
    gen_enum_constants, gen_flat_data_enum, gen_flat_data_enum_from_impls, gen_flat_data_enum_methods,
    gen_opaque_struct_methods, gen_php_struct, is_tagged_data_enum, is_untagged_data_enum,
};

pub struct PhpBackend;

impl PhpBackend {
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
        }
    }
}

impl Backend for PhpBackend {
    fn name(&self) -> &str {
        "php"
    }

    fn language(&self) -> Language {
        Language::Php
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
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
        let core_import = config.core_import_name();
        let lang_rename_all = config.serde_rename_all_for_language(Language::Php);

        // Get exclusion lists from PHP config
        let php_config = config.php.as_ref();
        let exclude_functions = php_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
        let exclude_types = php_config.map(|c| c.exclude_types.clone()).unwrap_or_default();

        let output_dir = resolve_output_dir(config.output_paths.get("php"), &config.name, "crates/{name}-php/src/");
        let has_serde = detect_serde_available(&output_dir);

        // Build the opaque type names list: IR opaque types + bridge type aliases.
        // Bridge type aliases (e.g. `VisitorHandle`) wrap Rc-based handles and cannot
        // implement serde::Serialize/Deserialize.  Including them ensures gen_php_struct
        // emits #[serde(skip)] for fields of those types so derives on the enclosing
        // struct (e.g. ConversionOptions) still compile.
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

        let mut cfg = Self::binding_config(&core_import, has_serde);
        cfg.opaque_type_names = &opaque_names_vec_php;
        let never_skip_cfg_field_names: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| {
                if b.bind_via == alef_core::config::BridgeBinding::OptionsField {
                    b.resolved_options_field().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
        cfg.never_skip_cfg_field_names = &never_skip_cfg_field_names;

        // Build the inner module content (types, methods, conversions)
        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(unsafe_code)");
        // PHP parameter names are lowerCamelCase; Rust complains about non-snake_case variables.
        builder.add_inner_attribute("allow(non_snake_case)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy)");
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
                .any(|f| matches!(&f.ty, alef_core::ir::TypeRef::Map(_, _)))
        }) || api
            .functions
            .iter()
            .any(|f| matches!(&f.return_type, alef_core::ir::TypeRef::Map(_, _)));
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
            .filter(|t| t.is_opaque && alef_codegen::generators::type_needs_mutex(t))
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
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Php)?;

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
                builder.add_item(&gen_opaque_struct_methods(
                    typ,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    &adapter_bodies,
                    &mutex_types,
                ));
            } else {
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // so no separate Default impl is needed.
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
                builder.add_item(&gen_enum_constants(enum_def));
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
        if !included_functions.is_empty() {
            let facade_class_name = extension_name.to_pascal_case();
            // Build each static method body (no #[php_function] attribute — they live inside
            // a #[php_impl] block which handles registration via the class machinery).
            let mut method_items: Vec<String> = Vec::new();
            for func in included_functions {
                let bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges);
                if let Some((param_idx, bridge_cfg)) = bridge_param {
                    method_items.push(crate::trait_bridge::gen_bridge_function(
                        func,
                        param_idx,
                        bridge_cfg,
                        &mapper,
                        &opaque_types,
                        &core_import,
                    ));
                } else if func.is_async {
                    method_items.push(gen_async_function_as_static_method(
                        func,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &config.trait_bridges,
                        &mutex_types,
                    ));
                } else {
                    method_items.push(gen_function_as_static_method(
                        func,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &config.trait_bridges,
                        has_serde,
                        &mutex_types,
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
            // PHP facade class (e.g. `Kreuzcrawl\Kreuzcrawl`) that Composer autoloads.
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
        }

        let convertible = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        // From/Into conversions with PHP-specific i64 casts.
        // Types with enum Named fields (or that reference such types transitively) can't
        // have binding->core From impls because PHP maps enums to String and there's no
        // From<String> for the core enum type. Core->binding is always safe.
        let enum_names_ref = &mapper.enum_names;
        let bridge_skip_types: Vec<String> = config
            .trait_bridges
            .iter()
            .filter(|b| !matches!(b.bind_via, alef_core::config::BridgeBinding::OptionsField))
            .filter_map(|b| b.type_alias.clone())
            .collect();
        // Trait-bridge fields whose binding-side wrapper holds `inner: Arc<core::T>`
        // (every OptionsField-style bridge in alef follows this convention). Used by
        // `binding_to_core` to emit `val.{f}.map(|v| (*v.inner).clone())` instead of
        // `Default::default()` so the visitor handle survives the `.into()` call.
        let trait_bridge_arc_wrapper_field_names: Vec<String> = config
            .trait_bridges
            .iter()
            .filter(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField)
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
                    && typ.fields.iter().any(|f| references_named_type(&f.ty, &enum_tainted))
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
                && alef_codegen::conversions::can_generate_conversion(typ, &convertible)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
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
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &php_conv_config,
                ));
            }
        }

        // From impls for tagged data enums lowered to flat PHP classes.
        for enum_def in api.enums.iter().filter(|e| is_tagged_data_enum(e)) {
            builder.add_item(&gen_flat_data_enum_from_impls(enum_def, &core_import));
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_php_error_converter(error, &core_import));
        }

        // Serde default helpers for bool fields whose core default is `true`,
        // and for SecurityLimits fields which use struct-level defaults.
        // Referenced by #[serde(default = "crate::serde_defaults::...")] on struct fields.
        if has_serde {
            let serde_module = "mod serde_defaults {\n    pub fn bool_true() -> bool { true }\n\
                   pub fn max_archive_size() -> i64 { 500 * 1024 * 1024 }\n\
                   pub fn max_compression_ratio() -> i64 { 100 }\n\
                   pub fn max_files_in_archive() -> i64 { 10_000 }\n\
                   pub fn max_nesting_depth() -> i64 { 1024 }\n\
                   pub fn max_entity_length() -> i64 { 1024 * 1024 }\n\
                   pub fn max_content_size() -> i64 { 100 * 1024 * 1024 }\n\
                   pub fn max_iterations() -> i64 { 10_000_000 }\n\
                   pub fn max_xml_depth() -> i64 { 1024 }\n\
                   pub fn max_table_cells() -> i64 { 100_000 }\n\
                }";
            builder.add_item(serde_module);
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
            class_registrations.push_str(&crate::template_env::render(
                "php_class_registration.jinja",
                context! { class_name => &typ.name },
            ));
        }
        // Register the facade class that wraps free functions as static methods.
        if !api.functions.is_empty() {
            let facade_class_name = extension_name.to_pascal_case();
            class_registrations.push_str(&crate::template_env::render(
                "php_class_registration.jinja",
                context! { class_name => &format!("{facade_class_name}Api") },
            ));
        }
        // Tagged data enums are lowered to flat PHP classes — register them like other classes.
        // Unit-variant enums remain as string constants and don't need .class::<T>() registration.
        for enum_def in api.enums.iter().filter(|e| is_tagged_data_enum(e)) {
            class_registrations.push_str(&crate::template_env::render(
                "php_class_registration.jinja",
                context! { class_name => &enum_def.name },
            ));
        }
        builder.add_item(&format!(
            "#[php_module]\npub fn get_module(module: ModuleBuilder) -> ModuleBuilder {{\n    module{class_registrations}\n}}"
        ));

        let mut content = builder.build();

        // Post-process generated code to replace the bridge builder method.
        // The generated code produces `visitor(Option<&VisitorHandle>)` which is
        // unreachable from PHP. Replace the entire method — signature and body —
        // with one that accepts a ZendObject and builds the proper bridge handle.
        for bridge in &config.trait_bridges {
            if let Some(field_name) = bridge.resolved_options_field() {
                let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
                let type_alias = bridge.type_alias.as_deref().unwrap_or("VisitorHandle");
                let options_type = bridge.options_type.as_deref().unwrap_or("ConversionOptions");
                let builder_type = format!("{}Builder", options_type);
                let bridge_struct = format!("Php{}Bridge", bridge.trait_name);

                // Match the verbatim pre-rustfmt output from codegen.
                // gen_instance_method produces 4-space-indented lines (signature + body),
                // then ImplBuilder.build() adds 4 more spaces to every line → 8/8/4 indent.
                // The body is a single-line Self { inner: Arc::new(...) } expression.
                // rustfmt later reformats this to the 4/8/8/4 multi-line style on disk.
                let old_method = format!(
                    "        pub fn {field_name}(&self, {param_name}: Option<&{type_alias}>) -> {builder_type} {{\n        Self {{ inner: Arc::new((*self.inner).clone().{field_name}({param_name}.as_ref().map(|v| &v.inner))) }}\n    }}"
                );
                let new_method = format!(
                    "        pub fn {field_name}(&self, {param_name}: &mut ext_php_rs::types::ZendObject) -> {builder_type} {{\n        let bridge = {bridge_struct}::new({param_name});\n        let handle: html_to_markdown_rs::visitor::VisitorHandle = std::rc::Rc::new(std::cell::RefCell::new(bridge));\n        Self {{ inner: Arc::new((*self.inner).clone().{field_name}(Some(handle))) }}\n    }}"
                );

                content = content.replace(&old_method, &new_method);
            }
        }

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
        let extension_name = config.php_extension_name();
        let class_name = extension_name.to_pascal_case();

        // Generate PHP wrapper class
        let mut content = String::new();
        content.push_str(&crate::template_env::render(
            "php_file_header.jinja",
            minijinja::Value::default(),
        ));
        content.push_str(&hash::header(CommentStyle::DoubleSlash));
        content.push_str(&crate::template_env::render(
            "php_declare_strict_types.jinja",
            minijinja::Value::default(),
        ));

        // Determine namespace — delegates to config so [php].namespace overrides are respected.
        let namespace = php_autoload_namespace(config);

        content.push_str(&crate::template_env::render(
            "php_namespace.jinja",
            context! { namespace => &namespace },
        ));
        content.push_str(&crate::template_env::render(
            "php_facade_class_declaration.jinja",
            context! { class_name => &class_name },
        ));

        // Build the set of bridge param names so they are excluded from public PHP signatures.
        let bridge_param_names_pub: ahash::AHashSet<&str> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.as_deref())
            .collect();

        // Config types whose PHP constructors can be called with zero arguments.
        // Only qualifies when ALL fields are optional (PHP constructor needs no required args).
        // `has_default` (Rust Default impl) is NOT sufficient — the PHP constructor is
        // generated from struct fields and still requires non-optional ones.
        let no_arg_constructor_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.fields.iter().all(|f| f.optional))
            .map(|t| t.name.clone())
            .collect();

        // Generate wrapper methods for functions
        for func in &api.functions {
            // PHP method names are based on the Rust source name (camelCased).
            // Async functions do not get a suffix because PHP blocks on async internally
            // via `block_on`, presenting a synchronous API to callers.
            // For example: `scrape` (async in Rust) → `scrape()` (sync from PHP perspective).
            let method_name = func.name.to_lower_camel_case();
            let return_php_type = php_type(&func.return_type);

            // Visible params exclude bridge params (not surfaced to PHP callers).
            let visible_params: Vec<_> = func
                .params
                .iter()
                .filter(|p| !bridge_param_names_pub.contains(p.name.as_str()))
                .collect();

            // PHPDoc block
            content.push_str(&crate::template_env::render(
                "php_phpdoc_block_start.jinja",
                minijinja::Value::default(),
            ));
            if func.doc.is_empty() {
                content.push_str(&crate::template_env::render(
                    "php_phpdoc_text_line.jinja",
                    context! { text => &format!("{}.", method_name) },
                ));
            } else {
                content.push_str(&crate::template_env::render(
                    "php_phpdoc_lines.jinja",
                    context! {
                        doc_lines => func.doc.lines().collect::<Vec<_>>(),
                        indent => "     ",
                    },
                ));
            }
            content.push_str(&crate::template_env::render(
                "php_phpdoc_empty_line.jinja",
                minijinja::Value::default(),
            ));
            for p in &visible_params {
                let ptype = php_phpdoc_type(&p.ty);
                let nullable_prefix = if p.optional { "?" } else { "" };
                content.push_str(&crate::template_env::render(
                    "php_phpdoc_param_line.jinja",
                    context! {
                        nullable_prefix => nullable_prefix,
                        param_type => &ptype,
                        param_name => &p.name,
                    },
                ));
            }
            let return_phpdoc = php_phpdoc_type(&func.return_type);
            content.push_str(&crate::template_env::render(
                "php_phpdoc_return_line.jinja",
                context! { return_type => &return_phpdoc },
            ));
            if func.error_type.is_some() {
                content.push_str(&crate::template_env::render(
                    "php_phpdoc_throws_line.jinja",
                    context! {
                        namespace => namespace.as_str(),
                        class_name => &class_name,
                    },
                ));
            }
            content.push_str(&crate::template_env::render(
                "php_phpdoc_block_end.jinja",
                minijinja::Value::default(),
            ));

            // Method signature with type hints.
            // Keep parameters in their original Rust order.
            // Since PHP doesn't allow optional params before required ones, and some Rust
            // functions have optional params in the middle, we must make all params after
            // the first optional one also optional (nullable with null default).
            // This ensures e2e generated test code (which uses Rust param order) will work.
            // Additionally, config-like parameters (Named types ending in "Config") should
            // be treated as optional for PHP even if not explicitly marked as such in the IR.
            // Helper: a config param is only treated as optional when its type can be
            // constructed with zero arguments (all fields are optional in the IR).
            let is_optional_config_param = |p: &alef_core::ir::ParamDef| -> bool {
                if let TypeRef::Named(name) = &p.ty {
                    (name.ends_with("Config") || name.as_str() == "config")
                        && no_arg_constructor_types.contains(name.as_str())
                } else {
                    false
                }
            };

            let mut first_optional_idx = None;
            for (idx, p) in visible_params.iter().enumerate() {
                if p.optional || is_optional_config_param(p) {
                    first_optional_idx = Some(idx);
                    break;
                }
            }

            content.push_str(&crate::template_env::render(
                "php_method_signature_start.jinja",
                context! { method_name => &method_name },
            ));

            let params: Vec<String> = visible_params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = php_type(&p.ty);
                    // Make param optional if:
                    // 1. It's explicitly optional OR
                    // 2. It's a config parameter with a no-arg constructor OR
                    // 3. It comes after the first optional/config param
                    let should_be_optional = p.optional
                        || is_optional_config_param(p)
                        || first_optional_idx.is_some_and(|first| idx >= first);
                    if should_be_optional {
                        format!("?{} ${} = null", ptype, p.name)
                    } else {
                        format!("{} ${}", ptype, p.name)
                    }
                })
                .collect();
            content.push_str(&params.join(", "));
            content.push_str(&crate::template_env::render(
                "php_method_signature_end.jinja",
                context! { return_type => &return_php_type },
            ));
            // Delegate to the native extension class (registered as `{namespace}\{class_name}Api`).
            // ext-php-rs auto-converts Rust snake_case to PHP camelCase.
            // PHP does not expose async — async behaviour is handled internally via Tokio
            // block_on, so the Rust function name matches the PHP method name exactly.
            let ext_method_name = func.name.to_lower_camel_case();
            let is_void = matches!(&func.return_type, TypeRef::Unit);
            // Pass parameters to the native function in their ORIGINAL order (not sorted).
            // The native extension expects parameters in the order defined in the Rust function.
            // The PHP facade reorders them only in its own signature for PHP syntax compliance,
            // but must pass them in the original order when calling the native method.
            // Config-type params that were made optional (nullable) in the facade must be
            // coerced to their default constructor when null, since the native ext requires
            // non-nullable objects.
            let call_params = visible_params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let should_be_optional = p.optional
                        || is_optional_config_param(p)
                        || first_optional_idx.is_some_and(|first| idx >= first);
                    if should_be_optional && is_optional_config_param(p) {
                        if let TypeRef::Named(type_name) = &p.ty {
                            return format!("${} ?? new {}()", p.name, type_name);
                        }
                    }
                    format!("${}", p.name)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let call_expr = format!("\\{namespace}\\{class_name}Api::{ext_method_name}({call_params})");
            if is_void {
                content.push_str(&crate::template_env::render(
                    "php_method_call_statement.jinja",
                    context! { call_expr => &call_expr },
                ));
            } else {
                content.push_str(&crate::template_env::render(
                    "php_method_call_return.jinja",
                    context! { call_expr => &call_expr },
                ));
            }
            content.push_str(&crate::template_env::render(
                "php_method_end.jinja",
                minijinja::Value::default(),
            ));
        }

        content.push_str(&crate::template_env::render(
            "php_class_end.jinja",
            minijinja::Value::default(),
        ));

        // Use PHP stubs output path if configured, otherwise fall back to packages/php/src/.
        // This is intentionally separate from config.output.php, which controls the Rust binding
        // crate output directory (e.g., crates/kreuzcrawl-php/src/).
        let output_dir = config
            .php
            .as_ref()
            .and_then(|p| p.stubs.as_ref())
            .map(|s| s.output.to_string_lossy().to_string())
            .unwrap_or_else(|| "packages/php/src/".to_string());

        let mut files: Vec<GeneratedFile> = Vec::new();
        files.push(GeneratedFile {
            path: PathBuf::from(&output_dir).join(format!("{}.php", class_name)),
            content,
            generated_header: false,
        });

        // Emit a per-opaque-type PHP class file alongside the facade. These provide
        // method declarations for static analysis (PHPStan) and IDE autocomplete.
        // The native PHP extension registers the same class names at module load
        // (before Composer autoload runs), so these userland files are never
        // included at runtime — the native class always wins.
        for typ in api.types.iter().filter(|t| t.is_opaque && !t.is_trait) {
            let opaque_file = gen_php_opaque_class_file(typ, &namespace);
            files.push(GeneratedFile {
                path: PathBuf::from(&output_dir).join(format!("{}.php", typ.name)),
                content: opaque_file,
                generated_header: false,
            });
        }

        Ok(files)
    }

    fn generate_type_stubs(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let extension_name = config.php_extension_name();
        let class_name = extension_name.to_pascal_case();

        // Determine namespace — delegates to config so [php].namespace overrides are respected.
        let namespace = php_autoload_namespace(config);

        // PSR-12 requires a blank line after the opening `<?php` tag.
        // php-cs-fixer enforces this and would insert it post-write,
        // making `alef verify` see content that differs from what was
        // freshly generated. Emit it here so generated == on-disk.
        let mut content = String::new();
        content.push_str(&crate::template_env::render(
            "php_file_header.jinja",
            minijinja::Value::default(),
        ));
        content.push_str(&hash::header(CommentStyle::DoubleSlash));
        content.push_str("// Type stubs for the native PHP extension — declares classes\n");
        content.push_str("// provided at runtime by the compiled Rust extension (.so/.dll).\n");
        content.push_str("// Include this in phpstan.neon scanFiles for static analysis.\n\n");
        content.push_str(&crate::template_env::render(
            "php_declare_strict_types.jinja",
            minijinja::Value::default(),
        ));
        // Use bracketed namespace syntax so we can add global-namespace function stubs later.
        content.push_str(&crate::template_env::render(
            "php_namespace_block_begin.jinja",
            context! { namespace => &namespace },
        ));

        // Exception class
        content.push_str(&crate::template_env::render(
            "php_exception_class_declaration.jinja",
            context! { class_name => &class_name },
        ));
        content.push_str(
            "    public function getErrorCode(): int { throw new \\RuntimeException('Not implemented.'); }\n",
        );
        content.push_str("}\n\n");

        // Opaque handle classes are declared as per-type PHP files in
        // `packages/php/src/{TypeName}.php` (see `generate_public_api`). They
        // are intentionally omitted from this aggregate extension stub so PHPStan
        // does not see two class declarations for the same fully-qualified name.

        // Record / struct types (non-opaque with fields)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque || typ.fields.is_empty() {
                continue;
            }
            if !typ.doc.is_empty() {
                content.push_str("/**\n");
                content.push_str(&crate::template_env::render(
                    "php_phpdoc_lines.jinja",
                    context! {
                        doc_lines => typ.doc.lines().collect::<Vec<_>>(),
                        indent => "",
                    },
                ));
                content.push_str(" */\n");
            }
            content.push_str(&crate::template_env::render(
                "php_record_class_stub_declaration.jinja",
                context! { class_name => &typ.name },
            ));

            // Public property declarations (ext-php-rs exposes struct fields as properties)
            for field in &typ.fields {
                let is_array = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
                let prop_type = if field.optional {
                    let inner = php_type(&field.ty);
                    if inner.starts_with('?') {
                        inner
                    } else {
                        format!("?{inner}")
                    }
                } else {
                    php_type(&field.ty)
                };
                if is_array {
                    let phpdoc = php_phpdoc_type(&field.ty);
                    let nullable_prefix = if field.optional { "?" } else { "" };
                    content.push_str(&crate::template_env::render(
                        "php_property_type_annotation.jinja",
                        context! {
                            nullable_prefix => nullable_prefix,
                            phpdoc => &phpdoc,
                        },
                    ));
                }
                content.push_str(&crate::template_env::render(
                    "php_property_stub.jinja",
                    context! {
                        prop_type => &prop_type,
                        field_name => &field.name,
                    },
                ));
            }
            content.push('\n');

            // Constructor with typed parameters.
            // PHP requires required parameters to come before optional ones, so sort
            // the fields: required first, then optional (preserving relative order within each group).
            let mut sorted_fields: Vec<&alef_core::ir::FieldDef> = typ.fields.iter().collect();
            sorted_fields.sort_by_key(|f| f.optional);

            // Emit PHPDoc before the constructor for any array-typed fields so PHPStan
            // understands the generic element type (e.g. `@param array<string> $items`).
            let array_fields: Vec<&alef_core::ir::FieldDef> = sorted_fields
                .iter()
                .copied()
                .filter(|f| matches!(&f.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)))
                .collect();
            if !array_fields.is_empty() {
                content.push_str("    /**\n");
                for f in &array_fields {
                    let phpdoc = php_phpdoc_type(&f.ty);
                    let nullable_prefix = if f.optional { "?" } else { "" };
                    content.push_str(&crate::template_env::render(
                        "php_phpdoc_array_param.jinja",
                        context! {
                            nullable_prefix => nullable_prefix,
                            phpdoc => &phpdoc,
                            param_name => &f.name,
                        },
                    ));
                }
                content.push_str("     */\n");
            }

            let params: Vec<String> = sorted_fields
                .iter()
                .map(|f| {
                    let ptype = php_type(&f.ty);
                    let nullable = if f.optional && !ptype.starts_with('?') {
                        format!("?{ptype}")
                    } else {
                        ptype
                    };
                    let default = if f.optional { " = null" } else { "" };
                    format!("        {} ${}{}", nullable, f.name, default)
                })
                .collect();
            content.push_str(&crate::template_env::render(
                "php_constructor_method.jinja",
                context! { params => &params.join(",\n") },
            ));

            // Getter methods for each field
            for field in &typ.fields {
                let is_array = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
                let return_type = if field.optional {
                    let inner = php_type(&field.ty);
                    if inner.starts_with('?') {
                        inner
                    } else {
                        format!("?{inner}")
                    }
                } else {
                    php_type(&field.ty)
                };
                let getter_name = field.name.to_lower_camel_case();
                // Emit PHPDoc for array return types so PHPStan knows the element type.
                if is_array {
                    let phpdoc = php_phpdoc_type(&field.ty);
                    let nullable_prefix = if field.optional { "?" } else { "" };
                    content.push_str(&crate::template_env::render(
                        "php_constructor_doc_return.jinja",
                        context! { return_type => &format!("{nullable_prefix}{phpdoc}") },
                    ));
                }
                let is_void_getter = return_type == "void";
                let getter_body = if is_void_getter {
                    "{ }".to_string()
                } else {
                    "{ throw new \\RuntimeException('Not implemented.'); }".to_string()
                };
                content.push_str(&crate::template_env::render(
                    "php_getter_stub.jinja",
                    context! {
                        getter_name => &format!("get{}", getter_name.to_pascal_case()),
                        return_type => &return_type,
                        getter_body => &getter_body,
                    },
                ));
            }

            content.push_str("}\n\n");
        }

        // Emit tagged data enums as classes (they're lowered to flat PHP classes in the binding).
        // Unit-variant enums → PHP 8.1+ enum constants.
        for enum_def in &api.enums {
            if is_tagged_data_enum(enum_def) {
                // Tagged data enums are lowered to flat classes; emit class stubs.
                if !enum_def.doc.is_empty() {
                    content.push_str("/**\n");
                    content.push_str(&crate::template_env::render(
                        "php_phpdoc_lines.jinja",
                        context! {
                            doc_lines => enum_def.doc.lines().collect::<Vec<_>>(),
                            indent => "",
                        },
                    ));
                    content.push_str(" */\n");
                }
                content.push_str(&crate::template_env::render(
                    "php_record_class_stub_declaration.jinja",
                    context! { class_name => &enum_def.name },
                ));
                content.push_str("}\n\n");
            } else {
                // Unit-variant enums → PHP 8.1+ enum constants.
                content.push_str(&crate::template_env::render(
                    "php_tagged_enum_declaration.jinja",
                    context! { enum_name => &enum_def.name },
                ));
                for variant in &enum_def.variants {
                    let case_name = sanitize_php_enum_case(&variant.name);
                    content.push_str(&crate::template_env::render(
                        "php_enum_variant_stub.jinja",
                        context! {
                            variant_name => case_name,
                            value => &variant.name,
                        },
                    ));
                }
                content.push_str("}\n\n");
            }
        }

        // Extension function stubs — generated as a native `{ClassName}Api` class with static
        // methods. The PHP facade (`{ClassName}`) delegates to `{ClassName}Api::method()`.
        // Using a class instead of global functions avoids the `inventory` crate registration
        // issue on macOS (cdylib builds do not collect `#[php_function]` entries there).
        if !api.functions.is_empty() {
            // Bridge params are hidden from the PHP-visible API in stubs too.
            let bridge_param_names_stubs: ahash::AHashSet<&str> = config
                .trait_bridges
                .iter()
                .filter_map(|b| b.param_name.as_deref())
                .collect();

            content.push_str(&crate::template_env::render(
                "php_api_class_declaration.jinja",
                context! { class_name => &class_name },
            ));
            for func in &api.functions {
                let return_type = php_type_fq(&func.return_type, &namespace);
                let return_phpdoc = php_phpdoc_type_fq(&func.return_type, &namespace);
                // Visible params exclude bridge params.
                let visible_params: Vec<_> = func
                    .params
                    .iter()
                    .filter(|p| !bridge_param_names_stubs.contains(p.name.as_str()))
                    .collect();
                // Stubs declare the ACTUAL native interface, which has parameters in their original order
                // (ext-php-rs doesn't reorder them). DO NOT sort them here.
                // The PHP facade may reorder them for syntax compliance, but the stub must match
                // the actual native extension signature.
                // Emit PHPDoc when any param or the return type is an array, so PHPStan
                // understands generic element types (e.g. array<string> vs bare array).
                let has_array_params = visible_params
                    .iter()
                    .any(|p| matches!(&p.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)));
                let has_array_return = matches!(&func.return_type, TypeRef::Vec(_) | TypeRef::Map(_, _))
                    || matches!(&func.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_) | TypeRef::Map(_, _)));
                if has_array_params || has_array_return {
                    content.push_str("    /**\n");
                    for p in &visible_params {
                        let ptype = php_phpdoc_type_fq(&p.ty, &namespace);
                        let nullable_prefix = if p.optional { "?" } else { "" };
                        content.push_str(&crate::template_env::render(
                            "php_phpdoc_static_param.jinja",
                            context! {
                                nullable_prefix => nullable_prefix,
                                ptype => &ptype,
                                param_name => &p.name,
                            },
                        ));
                    }
                    content.push_str(&crate::template_env::render(
                        "php_phpdoc_static_return.jinja",
                        context! { return_phpdoc => &return_phpdoc },
                    ));
                    content.push_str("     */\n");
                }
                let params: Vec<String> = visible_params
                    .iter()
                    .map(|p| {
                        let ptype = php_type_fq(&p.ty, &namespace);
                        if p.optional {
                            format!("?{} ${} = null", ptype, p.name)
                        } else {
                            format!("{} ${}", ptype, p.name)
                        }
                    })
                    .collect();
                // ext-php-rs auto-converts Rust snake_case to PHP camelCase.
                // PHP does not expose async — async behaviour is handled internally via
                // Tokio block_on, so the stub method name matches the Rust function name.
                let stub_method_name = func.name.to_lower_camel_case();
                let is_void_stub = return_type == "void";
                let stub_body = if is_void_stub {
                    "{ }".to_string()
                } else {
                    "{ throw new \\RuntimeException('Not implemented.'); }".to_string()
                };
                content.push_str(&crate::template_env::render(
                    "php_static_method_stub.jinja",
                    context! {
                        method_name => &stub_method_name,
                        params => &params.join(", "),
                        return_type => &return_type,
                        stub_body => &stub_body,
                    },
                ));
            }
            content.push_str("}\n\n");
        }

        // Close the namespaced block
        content.push_str(&crate::template_env::render(
            "php_namespace_block_end.jinja",
            minijinja::Value::default(),
        ));

        // Use stubs output path if configured, otherwise packages/php/stubs/
        let output_dir = config
            .php
            .as_ref()
            .and_then(|p| p.stubs.as_ref())
            .map(|s| s.output.to_string_lossy().to_string())
            .unwrap_or_else(|| "packages/php/stubs/".to_string());

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join(format!("{}_extension.php", extension_name)),
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-php",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

/// Map an IR [`TypeRef`] to a PHPDoc type string with generic parameters (e.g., `array<string>`).
/// PHPStan at level `max` requires iterable value types in PHPDoc annotations.
fn php_phpdoc_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Vec(inner) => format!("array<{}>", php_phpdoc_type(inner)),
        TypeRef::Map(k, v) => format!("array<{}, {}>", php_phpdoc_type(k), php_phpdoc_type(v)),
        TypeRef::Optional(inner) => format!("?{}", php_phpdoc_type(inner)),
        _ => php_type(ty),
    }
}

/// Map an IR [`TypeRef`] to a fully-qualified PHPDoc type string with generics (e.g., `array<\Ns\T>`).
fn php_phpdoc_type_fq(ty: &TypeRef, namespace: &str) -> String {
    match ty {
        TypeRef::Vec(inner) => format!("array<{}>", php_phpdoc_type_fq(inner, namespace)),
        TypeRef::Map(k, v) => format!(
            "array<{}, {}>",
            php_phpdoc_type_fq(k, namespace),
            php_phpdoc_type_fq(v, namespace)
        ),
        TypeRef::Named(name) => format!("\\{}\\{}", namespace, name),
        TypeRef::Optional(inner) => format!("?{}", php_phpdoc_type_fq(inner, namespace)),
        _ => php_type(ty),
    }
}

/// Map an IR [`TypeRef`] to a fully-qualified PHP type-hint string for use outside the namespace.
fn php_type_fq(ty: &TypeRef, namespace: &str) -> String {
    match ty {
        TypeRef::Named(name) => format!("\\{}\\{}", namespace, name),
        TypeRef::Optional(inner) => {
            let inner_type = php_type_fq(inner, namespace);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        _ => php_type(ty),
    }
}

/// Generate a per-opaque-type PHP class file for `packages/php/src/{TypeName}.php`.
///
/// The native ext-php-rs extension registers the same class at module load time
/// (before Composer autoload runs), so this userland file is never included at
/// runtime — the native class always wins. The file is consumed by PHPStan and
/// IDEs as the authoritative declaration of the type's public API surface.
fn gen_php_opaque_class_file(typ: &alef_core::ir::TypeDef, namespace: &str) -> String {
    let mut content = String::new();
    content.push_str(&crate::template_env::render(
        "php_file_header.jinja",
        minijinja::Value::default(),
    ));
    content.push_str(&hash::header(CommentStyle::DoubleSlash));
    content.push_str(&crate::template_env::render(
        "php_declare_strict_types.jinja",
        minijinja::Value::default(),
    ));
    content.push_str(&crate::template_env::render(
        "php_namespace.jinja",
        context! { namespace => namespace },
    ));

    // Type-level docblock.
    if !typ.doc.is_empty() {
        content.push_str("/**\n");
        content.push_str(&crate::template_env::render(
            "php_phpdoc_lines.jinja",
            context! {
                doc_lines => typ.doc.lines().collect::<Vec<_>>(),
                indent => "",
            },
        ));
        content.push_str(" */\n");
    }

    content.push_str(&format!("final class {}\n{{\n", typ.name));

    // Instance methods first, static methods second.
    let mut method_order: Vec<&alef_core::ir::MethodDef> = Vec::new();
    method_order.extend(typ.methods.iter().filter(|m| m.receiver.is_some()));
    method_order.extend(typ.methods.iter().filter(|m| m.receiver.is_none()));

    for method in method_order {
        let method_name = method.name.to_lower_camel_case();
        let return_type = php_type(&method.return_type);
        let is_void = matches!(&method.return_type, TypeRef::Unit);
        let is_static = method.receiver.is_none();

        // PHPDoc block — keep it short to avoid line-width issues.
        let mut doc_lines: Vec<String> = vec![];
        let doc_line = method.doc.lines().next().unwrap_or("").trim();
        if !doc_line.is_empty() {
            doc_lines.push(doc_line.to_string());
        }

        // Add @param PHPDoc for array parameters so PHPStan knows the element type
        let mut phpdoc_params: Vec<String> = vec![];
        for param in &method.params {
            if matches!(&param.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)) {
                let phpdoc_type = php_phpdoc_type(&param.ty);
                phpdoc_params.push(format!("@param {} ${}", phpdoc_type, param.name));
            }
        }
        doc_lines.extend(phpdoc_params);

        // Add @return PHPDoc for array types so PHPStan knows the element type
        let needs_return_phpdoc = matches!(&method.return_type, TypeRef::Vec(_) | TypeRef::Map(_, _));
        if needs_return_phpdoc {
            let phpdoc_type = php_phpdoc_type(&method.return_type);
            doc_lines.push(format!("@return {phpdoc_type}"));
        }

        // Emit PHPDoc if needed
        if !doc_lines.is_empty() {
            content.push_str("    /**\n");
            for line in doc_lines {
                content.push_str(&format!("     * {}\n", line));
            }
            content.push_str("     */\n");
        }

        // Method signature.
        let static_kw = if is_static { "static " } else { "" };
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let ptype = php_type(&p.ty);
                if p.optional {
                    format!("?{} ${} = null", ptype, p.name)
                } else {
                    format!("{} ${}", ptype, p.name)
                }
            })
            .collect();
        content.push_str(&format!(
            "    public {static_kw}function {method_name}({}): {return_type}\n",
            params.join(", ")
        ));
        let body = if is_void {
            "    {\n    }\n"
        } else {
            "    {\n        throw new \\RuntimeException('Not implemented — provided by the native extension.');\n    }\n"
        };
        content.push_str(body);
    }

    content.push_str("}\n");
    content
}

/// Map an IR [`TypeRef`] to a PHP type-hint string.
fn php_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Json | TypeRef::Bytes | TypeRef::Path => "string".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "float".to_string(),
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => "int".to_string(),
        },
        TypeRef::Optional(inner) => {
            // Flatten nested Option<Option<T>> to a single nullable type.
            // PHP has no double-nullable concept; ?T already covers null.
            let inner_type = php_type(inner);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => "array".to_string(),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Duration => "float".to_string(),
    }
}
