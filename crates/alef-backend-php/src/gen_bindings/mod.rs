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
use std::path::PathBuf;

use crate::naming::php_autoload_namespace;
use functions::{gen_async_function_as_static_method, gen_function_as_static_method};
use helpers::{gen_enum_tainted_from_binding_to_core, gen_tokio_runtime, has_enum_named_field, references_named_type};
use types::{
    gen_enum_constants, gen_flat_data_enum, gen_flat_data_enum_from_impls, gen_flat_data_enum_methods,
    gen_opaque_struct_methods, gen_php_struct, is_tagged_data_enum,
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
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Separate unit-variant enums (→ String) from tagged data enums (→ flat PHP class).
        let data_enum_names: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| is_tagged_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        let enum_names: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| !is_tagged_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        let mapper = PhpMapper {
            enum_names: enum_names.clone(),
            data_enum_names: data_enum_names.clone(),
        };
        let core_import = config.core_import_name();

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
        let mut opaque_names_vec_php: Vec<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        opaque_names_vec_php.extend(bridge_type_aliases_php);

        let mut cfg = Self::binding_config(&core_import, has_serde);
        cfg.opaque_type_names = &opaque_names_vec_php;

        // Build the inner module content (types, methods, conversions)
        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(unsafe_code)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unnecessary_cast, clippy::unused_unit, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
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
                ));
            } else {
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // so no separate Default impl is needed.
                builder.add_item(&gen_php_struct(typ, &mapper, &cfg, Some(&php_namespace), &enum_names));
                builder.add_item(&types::gen_struct_methods_with_exclude(
                    typ,
                    &mapper,
                    has_serde,
                    &core_import,
                    &opaque_types,
                    &enum_names,
                    &api.enums,
                    &exclude_functions,
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
                    ));
                } else {
                    method_items.push(gen_function_as_static_method(
                        func,
                        &mapper,
                        &opaque_types,
                        &core_import,
                        &config.trait_bridges,
                        has_serde,
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
        let php_conv_config = ConversionConfig {
            cast_large_ints_to_i64: true,
            enum_string_names: Some(enum_names_ref),
            json_to_string: true,
            include_cfg_metadata: false,
            option_duration_on_defaults: true,
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
            class_registrations.push_str(&format!("\n    .class::<{}>()", typ.name));
        }
        // Register the facade class that wraps free functions as static methods.
        if !api.functions.is_empty() {
            let facade_class_name = extension_name.to_pascal_case();
            class_registrations.push_str(&format!("\n    .class::<{facade_class_name}Api>()"));
        }
        // Tagged data enums are lowered to flat PHP classes — register them like other classes.
        // Unit-variant enums remain as string constants and don't need .class::<T>() registration.
        for enum_def in api.enums.iter().filter(|e| is_tagged_data_enum(e)) {
            class_registrations.push_str(&format!("\n    .class::<{}>()", enum_def.name));
        }
        builder.add_item(&format!(
            "#[php_module]\npub fn get_module(module: ModuleBuilder) -> ModuleBuilder {{\n    module{class_registrations}\n}}"
        ));

        let mut content = builder.build();

        // Post-process generated code to fix bridge type builder methods.
        // Builder methods on opaque types with bridge parameters
        // (e.g., visitor: Option<&VisitorHandle>) should not attempt to access .inner,
        // as there is no From impl from Arc<VisitorHandle> to the core visitor type.
        // Replace patterns like .visitor(visitor.as_ref().map(|v| &v.inner))
        // with .visitor(None) to skip setting the visitor on the core builder.
        for bridge in &config.trait_bridges {
            if let Some(field_name) = bridge.resolved_options_field() {
                let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
                let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
                let replacement = format!(".{}(None)", field_name);
                content = content.replace(&pattern, &replacement);
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
        let mut content = String::from("<?php\n\n");
        content.push_str(&hash::header(CommentStyle::DoubleSlash));
        content.push_str("declare(strict_types=1);\n\n");

        // Determine namespace — delegates to config so [php].namespace overrides are respected.
        let namespace = php_autoload_namespace(config);

        content.push_str(&format!("namespace {};\n\n", namespace));
        content.push_str(&format!("final class {}\n", class_name));
        content.push_str("{\n");

        // Build the set of bridge param names so they are excluded from public PHP signatures.
        let bridge_param_names_pub: ahash::AHashSet<&str> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.as_deref())
            .collect();

        // Generate wrapper methods for functions
        for func in &api.functions {
            // For async functions, append "Async" to the method name to match the native function name
            let method_name = if func.is_async {
                format!("{}Async", func.name.to_lower_camel_case())
            } else {
                func.name.to_lower_camel_case()
            };
            let return_php_type = php_type(&func.return_type);

            // Visible params exclude bridge params (not surfaced to PHP callers).
            let visible_params: Vec<_> = func
                .params
                .iter()
                .filter(|p| !bridge_param_names_pub.contains(p.name.as_str()))
                .collect();

            // PHPDoc block
            content.push_str("    /**\n");
            for line in func.doc.lines() {
                if line.is_empty() {
                    content.push_str("     *\n");
                } else {
                    content.push_str(&format!("     * {}\n", line));
                }
            }
            if func.doc.is_empty() {
                content.push_str(&format!("     * {}.\n", method_name));
            }
            content.push_str("     *\n");
            for p in &visible_params {
                let ptype = php_phpdoc_type(&p.ty);
                let nullable_prefix = if p.optional { "?" } else { "" };
                content.push_str(&format!("     * @param {}{} ${}\n", nullable_prefix, ptype, p.name));
            }
            let return_phpdoc = php_phpdoc_type(&func.return_type);
            content.push_str(&format!("     * @return {}\n", return_phpdoc));
            if func.error_type.is_some() {
                content.push_str(&format!("     * @throws \\{}\\{}Exception\n", namespace, class_name));
            }
            content.push_str("     */\n");

            // Method signature with type hints.
            // Keep parameters in their original Rust order.
            // Since PHP doesn't allow optional params before required ones, and some Rust
            // functions have optional params in the middle, we must make all params after
            // the first optional one also optional (nullable with null default).
            // This ensures e2e generated test code (which uses Rust param order) will work.
            // Additionally, config-like parameters (Named types ending in "Config") should
            // be treated as optional for PHP even if not explicitly marked as such in the IR.
            let mut first_optional_idx = None;
            for (idx, p) in visible_params.iter().enumerate() {
                let is_config_param = if let TypeRef::Named(name) = &p.ty {
                    name.ends_with("Config") || name == "config"
                } else {
                    false
                };
                if p.optional || is_config_param {
                    first_optional_idx = Some(idx);
                    break;
                }
            }

            content.push_str(&format!("    public static function {}(", method_name));

            let params: Vec<String> = visible_params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = php_type(&p.ty);
                    // Make param optional if:
                    // 1. It's explicitly optional OR
                    // 2. It's a config parameter OR
                    // 3. It comes after the first optional/config param
                    let is_config_param = if let TypeRef::Named(name) = &p.ty {
                        name.ends_with("Config") || name == "config"
                    } else {
                        false
                    };
                    let should_be_optional =
                        p.optional || is_config_param || first_optional_idx.is_some_and(|first| idx >= first);
                    if should_be_optional {
                        format!("?{} ${} = null", ptype, p.name)
                    } else {
                        format!("{} ${}", ptype, p.name)
                    }
                })
                .collect();
            content.push_str(&params.join(", "));
            content.push_str(&format!("): {}\n", return_php_type));
            content.push_str("    {\n");
            // Async functions are registered in the extension with an `_async` suffix
            // (see gen_async_function which generates `pub fn {name}_async`).
            // Delegate to the native extension class (registered as `{namespace}\{class_name}Api`).
            // ext-php-rs auto-converts Rust snake_case to PHP camelCase
            let ext_method_name = if func.is_async {
                format!("{}_async", func.name).to_lower_camel_case()
            } else {
                func.name.to_lower_camel_case()
            };
            let is_void = matches!(&func.return_type, TypeRef::Unit);
            // Pass parameters to the native function in their ORIGINAL order (not sorted).
            // The native extension expects parameters in the order defined in the Rust function.
            // The PHP facade reorders them only in its own signature for PHP syntax compliance,
            // but must pass them in the original order when calling the native method.
            let call_params = visible_params
                .iter()
                .map(|p| format!("${}", p.name))
                .collect::<Vec<_>>()
                .join(", ");
            let call_expr = format!(
                "\\{}\\{}Api::{}({})",
                namespace, class_name, ext_method_name, call_params
            );
            if is_void {
                content.push_str(&format!(
                    "        {}; // delegate to native extension class\n",
                    call_expr
                ));
            } else {
                content.push_str(&format!(
                    "        return {}; // delegate to native extension class\n",
                    call_expr
                ));
            }
            content.push_str("    }\n\n");
        }

        content.push_str("}\n");

        // Use PHP stubs output path if configured, otherwise fall back to packages/php/src/.
        // This is intentionally separate from config.output.php, which controls the Rust binding
        // crate output directory (e.g., crates/kreuzcrawl-php/src/).
        let output_dir = config
            .php
            .as_ref()
            .and_then(|p| p.stubs.as_ref())
            .map(|s| s.output.to_string_lossy().to_string())
            .unwrap_or_else(|| "packages/php/src/".to_string());

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join(format!("{}.php", class_name)),
            content,
            generated_header: false,
        }])
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
        let mut content = String::from("<?php\n\n");
        content.push_str(&hash::header(CommentStyle::DoubleSlash));
        content.push_str("// Type stubs for the native PHP extension — declares classes\n");
        content.push_str("// provided at runtime by the compiled Rust extension (.so/.dll).\n");
        content.push_str("// Include this in phpstan.neon scanFiles for static analysis.\n\n");
        content.push_str("declare(strict_types=1);\n\n");
        // Use bracketed namespace syntax so we can add global-namespace function stubs later.
        content.push_str(&format!("namespace {} {{\n\n", namespace));

        // Exception class
        content.push_str(&format!(
            "class {}Exception extends \\RuntimeException\n{{\n",
            class_name
        ));
        content.push_str(
            "    public function getErrorCode(): int { throw new \\RuntimeException('Not implemented.'); }\n",
        );
        content.push_str("}\n\n");

        // Opaque handle classes
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                if !typ.doc.is_empty() {
                    content.push_str("/**\n");
                    for line in typ.doc.lines() {
                        if line.is_empty() {
                            content.push_str(" *\n");
                        } else {
                            content.push_str(&format!(" * {}\n", line));
                        }
                    }
                    content.push_str(" */\n");
                }
                content.push_str(&format!("class {}\n{{\n", typ.name));
                // Opaque handles have no public constructors in PHP
                content.push_str("}\n\n");
            }
        }

        // Record / struct types (non-opaque with fields)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque || typ.fields.is_empty() {
                continue;
            }
            if !typ.doc.is_empty() {
                content.push_str("/**\n");
                for line in typ.doc.lines() {
                    if line.is_empty() {
                        content.push_str(" *\n");
                    } else {
                        content.push_str(&format!(" * {}\n", line));
                    }
                }
                content.push_str(" */\n");
            }
            content.push_str(&format!("class {}\n{{\n", typ.name));

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
                    content.push_str(&format!("    /** @var {}{} */\n", nullable_prefix, phpdoc));
                }
                content.push_str(&format!("    public {} ${};\n", prop_type, field.name));
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
                    content.push_str(&format!("     * @param {}{} ${}\n", nullable_prefix, phpdoc, f.name));
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
            content.push_str("    public function __construct(\n");
            content.push_str(&params.join(",\n"));
            content.push_str("\n    ) { }\n\n");

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
                    content.push_str(&format!("    /** @return {}{} */\n", nullable_prefix, phpdoc));
                }
                let is_void_getter = return_type == "void";
                let getter_body = if is_void_getter {
                    "{ }".to_string()
                } else {
                    "{ throw new \\RuntimeException('Not implemented.'); }".to_string()
                };
                content.push_str(&format!(
                    "    public function get{}(): {} {getter_body}\n",
                    getter_name.to_pascal_case(),
                    return_type
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
                    for line in enum_def.doc.lines() {
                        if line.is_empty() {
                            content.push_str(" *\n");
                        } else {
                            content.push_str(&format!(" * {}\n", line));
                        }
                    }
                    content.push_str(" */\n");
                }
                content.push_str(&format!("class {}\n{{\n", enum_def.name));
                content.push_str("}\n\n");
            } else {
                // Unit-variant enums → PHP 8.1+ enum constants.
                content.push_str(&format!("enum {}: string\n{{\n", enum_def.name));
                for variant in &enum_def.variants {
                    content.push_str(&format!("    case {} = '{}';\n", variant.name, variant.name));
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

            content.push_str(&format!("class {}Api\n{{\n", class_name));
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
                        content.push_str(&format!("     * @param {}{} ${}\n", nullable_prefix, ptype, p.name));
                    }
                    content.push_str(&format!("     * @return {}\n", return_phpdoc));
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
                let stub_method_name = if func.is_async {
                    format!("{}_async", func.name).to_lower_camel_case()
                } else {
                    func.name.to_lower_camel_case()
                };
                let is_void_stub = return_type == "void";
                let stub_body = if is_void_stub {
                    "{ }".to_string()
                } else {
                    "{ throw new \\RuntimeException('Not implemented.'); }".to_string()
                };
                content.push_str(&format!(
                    "    public static function {}({}): {} {stub_body}\n",
                    stub_method_name,
                    params.join(", "),
                    return_type
                ));
            }
            content.push_str("}\n\n");
        }

        // Close the namespaced block
        content.push_str("} // end namespace\n");

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
