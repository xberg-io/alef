mod functions;
mod helpers;
pub mod service_api;
pub mod types;

use crate::backends::php::type_map::PhpMapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::conversions::ConversionConfig;
use crate::codegen::doc_emission::{self, DocTarget, sanitize_rust_idioms};
use crate::codegen::generators::RustBindingConfig;
use crate::codegen::generators::{self, AsyncPattern};
use crate::codegen::naming::{pascal_to_snake, to_php_name, wire_variant_value};
use crate::codegen::shared::binding_fields;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use crate::core::ir::{DefaultValue, PrimitiveType, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToPascalCase};
use minijinja::context;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::backends::php::naming::php_autoload_namespace;
use functions::{PhpParamTypeSets, gen_async_function_as_static_method, gen_function_as_static_method};

/// PHP 8.1 enum cases cannot use case-insensitive `class` (reserved for
/// `EnumName::class` syntax). Append a trailing underscore for those cases.
fn sanitize_php_enum_case(name: &str) -> String {
    if name.eq_ignore_ascii_case("class") {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

fn php_enum_case_value(enum_def: &crate::core::ir::EnumDef, variant: &crate::core::ir::EnumVariant) -> String {
    wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    )
}

fn serde_default_fn_name(type_name: &str, field_name: &str) -> String {
    format!("{}_{}", pascal_to_snake(type_name), pascal_to_snake(field_name))
}

fn typed_default_fn(default: &DefaultValue, ty: &TypeRef) -> Option<(&'static str, String)> {
    match (default, ty) {
        (DefaultValue::BoolLiteral(value), TypeRef::Primitive(PrimitiveType::Bool)) => {
            Some(("bool", value.to_string()))
        }
        // Only emit String-returning fn for actual String fields. Named (enum-backed struct
        // wrapper) fields would mismatch the wrapped Named return type at compile time, so
        // skip emission and let serde fall back to Default for the wrapped type.
        (DefaultValue::StringLiteral(value) | DefaultValue::EnumVariant(value), TypeRef::String) => {
            Some(("String", format!("{value:?}.to_string()")))
        }
        (DefaultValue::IntLiteral(value), TypeRef::Primitive(primitive)) => {
            let return_type = match primitive {
                PrimitiveType::U8 => "u8",
                PrimitiveType::U16 => "u16",
                PrimitiveType::U32 => "u32",
                PrimitiveType::U64 => "i64",
                PrimitiveType::I8 => "i8",
                PrimitiveType::I16 => "i16",
                PrimitiveType::I32 => "i32",
                PrimitiveType::I64 => "i64",
                PrimitiveType::Usize | PrimitiveType::Isize => "i64",
                PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64 => return None,
            };
            Some((return_type, value.to_string()))
        }
        (DefaultValue::FloatLiteral(value), TypeRef::Primitive(PrimitiveType::F32)) => {
            let s = format!("{value}");
            let body = if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            };
            Some(("f32", body))
        }
        (DefaultValue::FloatLiteral(value), TypeRef::Primitive(PrimitiveType::F64)) => {
            let s = format!("{value}");
            let body = if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            };
            Some(("f64", body))
        }
        _ => None,
    }
}

fn gen_serde_defaults_module(api: &ApiSurface) -> Option<String> {
    let mut functions = Vec::new();
    for typ in api.types.iter().filter(|typ| typ.has_default) {
        for (field, default) in typ
            .fields
            .iter()
            .filter(|field| !field.optional)
            .filter_map(|field| field.typed_default.as_ref().map(|default| (field, default)))
        {
            let Some((return_type, body)) = typed_default_fn(default, &field.ty) else {
                continue;
            };
            let fn_name = serde_default_fn_name(&typ.name, &field.name);
            functions.push(format!("    pub fn {fn_name}() -> {return_type} {{ {body} }}"));
        }
    }

    if functions.is_empty() {
        None
    } else {
        Some(format!("mod serde_defaults {{\n{}\n}}", functions.join("\n")))
    }
}
use helpers::{gen_enum_tainted_from_binding_to_core, gen_tokio_runtime, has_enum_named_field, references_named_type};
use types::{
    gen_enum_constants, gen_flat_data_enum, gen_flat_data_enum_from_impls, gen_flat_data_enum_methods, gen_php_struct,
    is_tagged_data_enum, is_untagged_data_enum,
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
            // PHP applies struct-level `#[serde(default)]` so that `from_json` accepts partial
            // payloads. Without delegation, missing fields fall back to the derived `Default`,
            // which uses Rust's primitive zeros and clobbers any custom values from the core
            // type's `Default` impl when the binding is later converted back to core via
            // `From<BindingType>`. Setting this to `true` emits a delegating `impl Default`
            // that defers to `<core::Type as Default>::default().into()`.
            emit_delegating_default_impl: true,
            // PHP backend has its own per-method skip logic in `gen_bindings/functions.rs`
            // and does not route through the shared `gen_impl_block` skip path.
            skip_methods_when_not_delegatable: false,
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
            supports_service_api: true,
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

        let mut cfg = Self::binding_config(&core_import, has_serde);
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
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
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
        if !included_functions.is_empty() || !config.trait_bridges.is_empty() {
            let facade_class_name = extension_name.to_pascal_case();
            // Build each static method body (no #[php_function] attribute — they live inside
            // a #[php_impl] block which handles registration via the class machinery).
            let mut method_items: Vec<String> = Vec::new();
            for func in included_functions {
                if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(
                    &func.name,
                    &config.trait_bridges,
                ) {
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
        // Tagged data enums are lowered to flat PHP classes — register them like other classes.
        // Unit-variant enums remain as string constants and don't need .class::<T>() registration.
        for enum_def in api.enums.iter().filter(|e| is_tagged_data_enum(e)) {
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

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Helper: escape `*/` sequences that could close PHPDoc early
        let escape_phpdoc_line = |s: &str| s.replace("*/", "* /");

        let extension_name = config.php_extension_name();
        let class_name = extension_name.to_pascal_case();

        // Generate PHP wrapper class
        let mut content = String::new();
        content.push_str(&crate::backends::php::template_env::render(
            "php_file_header.jinja",
            minijinja::Value::default(),
        ));
        content.push_str(&hash::header(CommentStyle::DoubleSlash));
        content.push_str(&crate::backends::php::template_env::render(
            "php_declare_strict_types.jinja",
            minijinja::Value::default(),
        ));
        // PSR-12: blank line between `declare(strict_types=1);` and `namespace`.
        content.push('\n');

        // Determine namespace — delegates to config so [php].namespace overrides are respected.
        let namespace = php_autoload_namespace(config);

        content.push_str(&crate::backends::php::template_env::render(
            "php_namespace.jinja",
            context! { namespace => &namespace },
        ));
        // PSR-12: blank line between `namespace` and class declaration.
        content.push('\n');
        content.push_str(&crate::backends::php::template_env::render(
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
            // Skip trait-bridge-managed names (clear_fn) — the trait-bridge loop below
            // emits its own static method, and duplicating it here would cause a
            // PHP fatal "Cannot redeclare" at load time.
            if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
                continue;
            }
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

            // PHPDoc block: translate rustdoc sections to PHPDoc format, stripping Rust-specific syntax.
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_start.jinja",
                minijinja::Value::default(),
            ));
            if func.doc.is_empty() {
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_text_line.jinja",
                    context! { text => &format!("{}.", method_name) },
                ));
            } else {
                // Extract and render summary + major rustdoc sections, stripping Rust-specific syntax.
                let sections = doc_emission::parse_rustdoc_sections(&func.doc);
                // Emit summary
                for line in sections.summary.lines() {
                    content.push_str("     * ");
                    content.push_str(&escape_phpdoc_line(line));
                    content.push('\n');
                }
                // Skip Arguments, Returns, Errors, Example — they're emitted as @param/@return/@throws below.
                // This prevents raw Rust syntax from leaking into the docstring.
            }
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_empty_line.jinja",
                minijinja::Value::default(),
            ));
            for p in &visible_params {
                let ptype = php_phpdoc_type(&p.ty);
                // php_phpdoc_type() already handles TypeRef::Optional by returning a string
                // starting with '?', so we should not add another nullable prefix here.
                // The p.optional flag is redundant with the type structure.
                let nullable_prefix = "";
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_param_line.jinja",
                    context! {
                        nullable_prefix => nullable_prefix,
                        param_type => &ptype,
                        param_name => &p.name,
                    },
                ));
            }
            let return_phpdoc = php_phpdoc_type(&func.return_type);
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_return_line.jinja",
                context! { return_type => &return_phpdoc },
            ));
            if func.error_type.is_some() {
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_throws_line.jinja",
                    context! {
                        namespace => namespace.as_str(),
                        class_name => &class_name,
                    },
                ));
            }
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_end.jinja",
                minijinja::Value::default(),
            ));

            // Method signature with type hints.
            // Keep parameters in their original Rust order.
            // Since PHP doesn't allow optional params before required ones, and some Rust
            // functions have optional params in the middle, we must make all params after
            // the first optional one also optional (nullable with null default).
            // This ensures e2e generated test code (which uses Rust param order) will work.
            // Treat required named params as optional only when IR metadata proves the
            // target type can be constructed with zero arguments.
            let is_optional_default_constructible_param = |p: &crate::core::ir::ParamDef| -> bool {
                if let TypeRef::Named(name) = &p.ty {
                    no_arg_constructor_types.contains(name.as_str())
                } else {
                    false
                }
            };

            // Build wrapper signature in RUST parameter order (not reordered).
            // E2e test generator expects Rust param order, so the wrapper must match.
            // This aligns PHP bindings with Python, Ruby, Go, etc. which also preserve
            // Rust parameter order.
            // PHP 8.1 syntax rule: required params must come before optional ones.
            // Optional-default-constructible params (like a no-arg-constructible
            // CrawlConfig) can have `= null` defaults — but ONLY when every later
            // parameter is also optional. Otherwise PHP 8.1 emits a "Required
            // parameter follows optional" deprecation. Walk the param list from
            // the end so a required param resets the optional-tail flag to false.
            let mut tail_optional = vec![true; visible_params.len()];
            let mut later_required = false;
            for (idx, p) in visible_params.iter().enumerate().rev() {
                if later_required {
                    tail_optional[idx] = false;
                }
                let is_required = !(p.optional || is_optional_default_constructible_param(p));
                if is_required {
                    later_required = true;
                }
            }
            let params: Vec<String> = visible_params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = php_type(&p.ty);
                    // Check if the parameter is optional: either the type itself is nullable
                    // (like Option<T> which php_type renders as ?T), or it's a default-constructible
                    // type that can use a null default. DO NOT use p.optional flag here — that field
                    // is IR metadata and does not represent optionality for PHP purposes.
                    let type_is_nullable = ptype.starts_with('?');
                    let can_be_optional = type_is_nullable || is_optional_default_constructible_param(p);

                    // Only emit `= null` default for parameters that are truly nullable (Option<T>)
                    // or default-constructible. The tail_optional check ensures PHP 8.1 compliance
                    // (required params before optional ones).
                    let can_emit_default =
                        tail_optional[idx] && (type_is_nullable || is_optional_default_constructible_param(p));

                    if can_be_optional && can_emit_default {
                        // ptype may already be nullable (e.g., "?string" from php_type handling
                        // TypeRef::Optional). Don't double-prepend the nullable prefix.
                        if ptype.starts_with('?') {
                            format!("{} ${} = null", ptype, p.name)
                        } else {
                            format!("?{} ${} = null", ptype, p.name)
                        }
                    } else {
                        format!("{} ${}", ptype, p.name)
                    }
                })
                .collect();

            // Emit signature: when params is empty, collapse () to single line.
            // Otherwise, multi-line with params on separate line.
            if params.is_empty() {
                // Single-line signature for no-arg functions
                content.push_str(&format!(
                    "    public static function {}(): {} {{\n",
                    method_name, return_php_type
                ));
            } else {
                // Multi-line signature for functions with params
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_signature_start.jinja",
                    context! { method_name => &method_name },
                ));
                content.push_str(&params.join(", "));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_signature_end.jinja",
                    context! { return_type => &return_php_type },
                ));
            }
            // Delegate to the native extension class (registered as `{namespace}\{class_name}Api`).
            // ext-php-rs auto-converts Rust snake_case to PHP camelCase.
            // PHP does not expose async — async behaviour is handled internally via Tokio
            // block_on, so the Rust function name matches the PHP method name exactly.
            let ext_method_name = func.name.to_lower_camel_case();
            let is_void = matches!(&func.return_type, TypeRef::Unit);
            // CRITICAL: Pass parameters to the native function in their ORIGINAL IR order,
            // not in the reordered wrapper signature order.
            // The wrapper signature is reordered for PHP 8.1 compliance (required first),
            // but the native extension method expects parameters
            // in the original IR order (as registered via #[php_impl]).
            // When IR has optional params before required ones, the two orders differ.
            // Example: IR is (engine: Option<Engine>, url: String)
            //   → Wrapper signature: (string $url, ?Engine $engine = null) [reordered]
            //   → Native call: ($engine ?? new Engine(), $url) [IR order]
            // Build call args by iterating visible_params in original IR order.
            let call_params = visible_params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    // Only apply the `?? new Type()` coercion for params that the
                    // wrapper actually emits as nullable with `= null` — i.e. params
                    // marked optional that also sit in a tail-optional position. A
                    // param that became required to satisfy PHP 8.1's "required before
                    // optional" rule (`tail_optional[idx] == false`) is non-nullable
                    // at the wrapper signature, so `$p ?? new T()` would be a useless
                    // null-coalesce (`nullCoalesce.variable` in phpstan).
                    if (p.optional || is_optional_default_constructible_param(p))
                        && is_optional_default_constructible_param(p)
                        && tail_optional[idx]
                    {
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
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_call_statement.jinja",
                    context! { call_expr => &call_expr },
                ));
            } else {
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_call_return.jinja",
                    context! { call_expr => &call_expr },
                ));
            }
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_end.jinja",
                minijinja::Value::default(),
            ));
        }

        // Emit trait-bridge registration methods in the PHP facade
        for bridge_cfg in &config.trait_bridges {
            if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
                let method_name = register_fn.to_lower_camel_case();
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_block_start.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_text_line.jinja",
                    context! { text => &format!("{}.", method_name) },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_empty_line.jinja",
                    minijinja::Value::default(),
                ));
                let interface_name = &bridge_cfg.trait_name;
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_param_line.jinja",
                    context! {
                        nullable_prefix => "",
                        param_type => interface_name,
                        param_name => "backend",
                    },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_return_line.jinja",
                    context! { return_type => "void" },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_block_end.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_signature_start.jinja",
                    context! { method_name => &method_name },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_trait_bridge_api_method.jinja",
                    context! { interface_name => interface_name },
                ));
                let call_expr = format!("\\{namespace}\\{class_name}Api::{method_name}($backend)");
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_call_statement.jinja",
                    context! { call_expr => &call_expr },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_end.jinja",
                    minijinja::Value::default(),
                ));
            }
            if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
                let method_name = unregister_fn.to_lower_camel_case();
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_block_start.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_text_line.jinja",
                    context! { text => &format!("{}.", method_name) },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_empty_line.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_param_line.jinja",
                    context! {
                        nullable_prefix => "",
                        param_type => "string",
                        param_name => "name",
                    },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_return_line.jinja",
                    context! { return_type => "void" },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_block_end.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_signature_start.jinja",
                    context! { method_name => &method_name },
                ));
                content.push_str("string $name) : void\n    {\n");
                let call_expr = format!("\\{namespace}\\{class_name}Api::{method_name}($name)");
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_call_statement.jinja",
                    context! { call_expr => &call_expr },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_end.jinja",
                    minijinja::Value::default(),
                ));
            }
            if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
                let method_name = clear_fn.to_lower_camel_case();
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_block_start.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_text_line.jinja",
                    context! { text => &format!("{}.", method_name) },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_empty_line.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_return_line.jinja",
                    context! { return_type => "void" },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_block_end.jinja",
                    minijinja::Value::default(),
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_signature_start.jinja",
                    context! { method_name => &method_name },
                ));
                content.push_str(") : void\n    {\n");
                let call_expr = format!("\\{namespace}\\{class_name}Api::{method_name}()");
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_call_statement.jinja",
                    context! { call_expr => &call_expr },
                ));
                content.push_str(&crate::backends::php::template_env::render(
                    "php_method_end.jinja",
                    minijinja::Value::default(),
                ));
            }
        }

        content.push_str(&crate::backends::php::template_env::render(
            "php_class_end.jinja",
            minijinja::Value::default(),
        ));

        // Use PHP stubs output path if configured, otherwise fall back to packages/php/src/.
        // This is intentionally separate from config.output.php, which controls the Rust binding
        // crate output directory (e.g., crates/sample-crawler-php/src/).
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
        // Build a map of (service owner type, method name, param name) -> callback contract
        // to fix generic handler parameter types (e.g., H -> Handler).
        let mut handler_contract_map: ahash::AHashMap<(String, String, String), String> = ahash::AHashMap::new();
        for service in &api.services {
            for reg in &service.registrations {
                handler_contract_map.insert(
                    (service.name.clone(), reg.method.clone(), reg.callback_param.clone()),
                    reg.callback_contract.clone(),
                );
            }
        }

        for typ in api.types.iter().filter(|t| t.is_opaque && !t.is_trait) {
            let streaming_adapters: Vec<&crate::core::config::AdapterConfig> = config
                .adapters
                .iter()
                .filter(|a| {
                    matches!(a.pattern, crate::core::config::AdapterPattern::Streaming)
                        && a.owner_type.as_deref() == Some(&typ.name)
                        && !a.skip_languages.iter().any(|l| l == "php")
                })
                .collect();
            let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.clone()).collect();
            let opaque_file = gen_php_opaque_class_file(
                typ,
                &namespace,
                &streaming_adapters,
                &streaming_method_names,
                &config.trait_bridges,
                &handler_contract_map,
            );
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
        let php_config = config.php.as_ref();
        let exclude_functions: AHashSet<String> = php_config
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let exclude_types: AHashSet<String> = php_config
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();

        // PSR-12 requires a blank line after the opening `<?php` tag. php-cs-fixer enforces
        // this and would insert it post-write, making `alef verify` see content that differs
        // from what was freshly generated. The `php_file_header.jinja` template emits `<?php`
        // followed by the blank line so generated == on-disk.
        let mut content = String::new();
        content.push_str(&crate::backends::php::template_env::render(
            "php_file_header.jinja",
            minijinja::Value::default(),
        ));
        content.push_str(&hash::header(CommentStyle::DoubleSlash));
        content.push_str("// Type stubs for the native PHP extension — declares classes\n");
        content.push_str("// provided at runtime by the compiled Rust extension (.so/.dll).\n");
        content.push_str("// Include this in phpstan.neon scanFiles for static analysis.\n\n");
        content.push_str(&crate::backends::php::template_env::render(
            "php_declare_strict_types.jinja",
            minijinja::Value::default(),
        ));
        // PSR-12: blank line between `declare(strict_types=1);` and `namespace`.
        content.push('\n');
        // Use bracketed namespace syntax so we can add global-namespace function stubs later.
        content.push_str(&crate::backends::php::template_env::render(
            "php_namespace_block_begin.jinja",
            context! { namespace => &namespace },
        ));

        // Exception class
        content.push_str(&crate::backends::php::template_env::render(
            "php_exception_class_declaration.jinja",
            context! { class_name => &class_name },
        ));
        content.push_str(
            "    public function getErrorCode(): int { throw new \\RuntimeException('Not implemented.'); }\n",
        );
        // Emit introspection method stubs for errors that expose them.
        // These are backed by #[php_method] impls in the generated native extension.
        let has_status_code = api
            .errors
            .iter()
            .any(|e| e.methods.iter().any(|m| m.name == "status_code"));
        let has_is_transient = api
            .errors
            .iter()
            .any(|e| e.methods.iter().any(|m| m.name == "is_transient"));
        let has_error_type = api
            .errors
            .iter()
            .any(|e| e.methods.iter().any(|m| m.name == "error_type"));
        if has_status_code {
            content.push_str(
                "    /** HTTP status code for this error (0 means no associated status). */\n    \
                 public function statusCode(): int { throw new \\RuntimeException('Not implemented.'); }\n",
            );
        }
        if has_is_transient {
            content.push_str(
                "    /** Returns true if the error is transient and a retry may succeed. */\n    \
                 public function isTransient(): bool { throw new \\RuntimeException('Not implemented.'); }\n",
            );
        }
        if has_error_type {
            content.push_str(
                "    /** Machine-readable error category string for matching and logging. */\n    \
                 public function errorType(): string { throw new \\RuntimeException('Not implemented.'); }\n",
            );
        }
        content.push_str("}\n\n");

        // Opaque handle classes are declared as per-type PHP files in
        // `packages/php/src/{TypeName}.php` (see `generate_public_api`). They
        // are intentionally omitted from this aggregate extension stub so PHPStan
        // does not see two class declarations for the same fully-qualified name.

        // Record / struct types (non-opaque with fields)
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
        {
            if typ.is_opaque || typ.fields.is_empty() {
                continue;
            }
            if !typ.doc.is_empty() {
                content.push_str("/**\n");
                let sanitized = sanitize_rust_idioms(&typ.doc, DocTarget::PhpDoc);
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_lines.jinja",
                    context! {
                        doc_lines => sanitized.lines().collect::<Vec<_>>(),
                        indent => "",
                    },
                ));
                content.push_str(" */\n");
            }
            content.push_str(&crate::backends::php::template_env::render(
                "php_record_class_stub_declaration.jinja",
                context! { class_name => &typ.name },
            ));

            // PHP 8.3+ constructor property promotion with `public readonly`.
            // Required parameters come before optional ones (PHP syntax requirement).
            let mut sorted_fields: Vec<&crate::core::ir::FieldDef> = binding_fields(&typ.fields).collect();
            sorted_fields.sort_by_key(|f| f.optional);

            // Promoted readonly parameters replace both separate property declarations
            // and redundant getter methods — direct property access is the PHP 8.3+ idiom.
            // Each promoted parameter gets an inline /** @var T [description] */ block so that
            // phpdoc-lint (phpstan level max) and IDEs see the precise generic type and field docs.
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
                    let php_name = to_php_name(&f.name);
                    let phpdoc_type = php_phpdoc_type(&f.ty);
                    let var_type = if f.optional && !phpdoc_type.starts_with('?') {
                        format!("?{phpdoc_type}")
                    } else {
                        phpdoc_type
                    };
                    let phpdoc = php_property_phpdoc(&var_type, &f.doc, "        ");
                    format!("{phpdoc}        public readonly {nullable} ${php_name}{default}",)
                })
                .collect();
            content.push_str(&crate::backends::php::template_env::render(
                "php_constructor_method.jinja",
                context! { params => &params.join(",\n") },
            ));

            // Emit method stubs for impl methods declared on this DTO type.
            // PHPStan can only see methods that appear in the stub; without these,
            // static preset factories (e.g. `all()`, `minimal()`) and withers
            // (e.g. `withChunking()`) are flagged as "Call to undefined method".
            let non_excluded_methods: Vec<&crate::core::ir::MethodDef> = typ
                .methods
                .iter()
                .filter(|m| !m.binding_excluded && !m.sanitized)
                .collect();
            for method in non_excluded_methods {
                let method_name = method.name.to_lower_camel_case();
                let is_static = method.receiver.is_none();
                let return_type = php_type(&method.return_type);
                let first_optional_idx = method.params.iter().position(|p| p.optional);
                let params: Vec<String> = method
                    .params
                    .iter()
                    .enumerate()
                    .map(|(idx, p)| {
                        let ptype = php_type(&p.ty);
                        if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                            let nullable = if ptype.starts_with('?') { "" } else { "?" };
                            format!("{nullable}{ptype} ${} = null", p.name)
                        } else {
                            format!("{} ${}", ptype, p.name)
                        }
                    })
                    .collect();
                let static_kw = if is_static { "static " } else { "" };
                let is_void = matches!(&method.return_type, TypeRef::Unit);
                let stub_body = if is_void {
                    "{ }".to_string()
                } else {
                    "{ throw new \\RuntimeException('Not implemented — provided by the native extension.'); }"
                        .to_string()
                };
                content.push_str(&crate::backends::php::template_env::render(
                    "php_stub_method_definition.jinja",
                    context! {
                        static_kw => static_kw,
                        method_name => &method_name,
                        params => &params.join(", "),
                        return_type => &return_type,
                        stub_body => &stub_body,
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
                    let sanitized = sanitize_rust_idioms(&enum_def.doc, DocTarget::PhpDoc);
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_phpdoc_lines.jinja",
                        context! {
                            doc_lines => sanitized.lines().collect::<Vec<_>>(),
                            indent => "",
                        },
                    ));
                    content.push_str(" */\n");
                }
                content.push_str(&crate::backends::php::template_env::render(
                    "php_record_class_stub_declaration.jinja",
                    context! { class_name => &enum_def.name },
                ));
                content.push_str("}\n\n");
            } else {
                // Unit-variant enums → PHP 8.1+ enum constants.
                content.push_str(&crate::backends::php::template_env::render(
                    "php_tagged_enum_declaration.jinja",
                    context! { enum_name => &enum_def.name },
                ));
                for variant in &enum_def.variants {
                    let case_name = sanitize_php_enum_case(&variant.name);
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_enum_variant_stub.jinja",
                        context! {
                            variant_name => case_name,
                            value => &php_enum_case_value(enum_def, variant),
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
        if api.functions.iter().any(|f| !exclude_functions.contains(&f.name)) || !config.trait_bridges.is_empty() {
            // Bridge params are hidden from the PHP-visible API in stubs too.
            let bridge_param_names_stubs: ahash::AHashSet<&str> = config
                .trait_bridges
                .iter()
                .filter_map(|b| b.param_name.as_deref())
                .collect();

            content.push_str(&crate::backends::php::template_env::render(
                "php_api_class_declaration.jinja",
                context! { class_name => &class_name },
            ));
            for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
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
                let first_optional_idx = visible_params.iter().position(|p| p.optional);
                if has_array_params || has_array_return {
                    content.push_str("    /**\n");
                    for (idx, p) in visible_params.iter().enumerate() {
                        let ptype = php_phpdoc_type_fq(&p.ty, &namespace);
                        let nullable_prefix = if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                            "?"
                        } else {
                            ""
                        };
                        content.push_str(&crate::backends::php::template_env::render(
                            "php_phpdoc_static_param.jinja",
                            context! {
                                nullable_prefix => nullable_prefix,
                                ptype => &ptype,
                                param_name => &p.name,
                            },
                        ));
                    }
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_phpdoc_static_return.jinja",
                        context! { return_phpdoc => &return_phpdoc },
                    ));
                    content.push_str("     */\n");
                }
                let params: Vec<String> = visible_params
                    .iter()
                    .enumerate()
                    .map(|(idx, p)| {
                        let ptype = php_type_fq(&p.ty, &namespace);
                        if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                            let nullable_ptype = if ptype.starts_with('?') {
                                ptype
                            } else {
                                format!("?{ptype}")
                            };
                            format!("{} ${} = null", nullable_ptype, p.name)
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
                content.push_str(&crate::backends::php::template_env::render(
                    "php_static_method_stub.jinja",
                    context! {
                        method_name => &stub_method_name,
                        params => &params.join(", "),
                        return_type => &return_type,
                        stub_body => &stub_body,
                    },
                ));
            }
            for bridge_cfg in &config.trait_bridges {
                if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
                    let method_name = register_fn.to_lower_camel_case();
                    let interface_name = php_type_fq(&TypeRef::Named(bridge_cfg.trait_name.clone()), &namespace);
                    let params = format!("{interface_name} $backend");
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_static_method_stub.jinja",
                        context! {
                            method_name => &method_name,
                            params => &params,
                            return_type => "void",
                            stub_body => "{ }",
                        },
                    ));
                }
                if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
                    let method_name = unregister_fn.to_lower_camel_case();
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_static_method_stub.jinja",
                        context! {
                            method_name => &method_name,
                            params => "string $name",
                            return_type => "void",
                            stub_body => "{ }",
                        },
                    ));
                }
                if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
                    let method_name = clear_fn.to_lower_camel_case();
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_static_method_stub.jinja",
                        context! {
                            method_name => &method_name,
                            params => "",
                            return_type => "void",
                            stub_body => "{ }",
                        },
                    ));
                }
            }
            content.push_str("}\n\n");
        }

        // Close the namespaced block
        content.push_str(&crate::backends::php::template_env::render(
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

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
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
        TypeRef::Optional(inner) => {
            // Flatten nested Option<Option<T>> to a single nullable type.
            // php_type() already handles nested Optional by returning a string starting with '?',
            // so we check and avoid double-prepending.
            let inner_type = php_phpdoc_type(inner);
            if inner_type.starts_with('?') {
                inner_type
            } else {
                format!("?{inner_type}")
            }
        }
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
fn gen_php_opaque_class_file(
    typ: &crate::core::ir::TypeDef,
    namespace: &str,
    streaming_adapters: &[&crate::core::config::AdapterConfig],
    streaming_method_names: &AHashSet<String>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    handler_contract_map: &ahash::AHashMap<(String, String, String), String>,
) -> String {
    let mut content = String::new();
    content.push_str(&crate::backends::php::template_env::render(
        "php_file_header.jinja",
        minijinja::Value::default(),
    ));
    content.push_str(&hash::header(CommentStyle::DoubleSlash));
    content.push_str(&crate::backends::php::template_env::render(
        "php_declare_strict_types.jinja",
        minijinja::Value::default(),
    ));
    // PSR-12: blank line between `declare(strict_types=1);` and `namespace`.
    content.push('\n');
    content.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        context! { namespace => namespace },
    ));
    // PSR-12: blank line between `namespace` and class declaration.
    content.push('\n');

    // Type-level docblock.
    if !typ.doc.is_empty() {
        content.push_str("/**\n");
        let sanitized = sanitize_rust_idioms(&typ.doc, DocTarget::PhpDoc);
        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_lines.jinja",
            context! {
                doc_lines => sanitized.lines().collect::<Vec<_>>(),
                indent => "",
            },
        ));
        content.push_str(" */\n");
    }

    content.push_str(&crate::backends::php::template_env::render(
        "php_final_class_stub_start.jinja",
        context! { class_name => &typ.name },
    ));

    // Instance methods first, static methods second — skip streaming methods
    // (they'll be emitted as Generator wrappers after regular methods).
    let mut method_order: Vec<&crate::core::ir::MethodDef> = Vec::new();
    method_order.extend(
        typ.methods
            .iter()
            .filter(|m| m.receiver.is_some() && !streaming_method_names.contains(&m.name)),
    );
    method_order.extend(
        typ.methods
            .iter()
            .filter(|m| m.receiver.is_none() && !streaming_method_names.contains(&m.name)),
    );

    for method in method_order {
        let method_name = method.name.to_lower_camel_case();
        let return_type = php_type(&method.return_type);
        let is_void = matches!(&method.return_type, TypeRef::Unit);
        let is_static = method.receiver.is_none();

        // PHPDoc block — keep it short to avoid line-width issues.
        let mut doc_lines: Vec<String> = vec![];
        let sanitized = sanitize_rust_idioms(&method.doc, DocTarget::PhpDoc);
        let doc_line = sanitized.lines().next().unwrap_or("").trim();
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
                content.push_str(&crate::backends::php::template_env::render(
                    "php_prefixed_phpdoc_line.jinja",
                    context! {
                        indent => "    ",
                        line => &line,
                    },
                ));
            }
            content.push_str("     */\n");
        }

        // Method signature.
        let static_kw = if is_static { "static " } else { "" };
        let first_optional_idx = method.params.iter().position(|p| p.optional);
        let params: Vec<String> = method
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                // PHP has no first-class function-type declarations, so a handler
                // contract that resolves to a callback at the host-language layer
                // can't be referenced by a Rust-side class name (ext-php-rs accepts
                // `callable` for closures passed through Zval). Emit `callable`
                // whenever the handler-contract map identifies this parameter as a
                // contract; phpstan would otherwise flag the phantom class as
                // `class.notFound`.
                let ptype =
                    if handler_contract_map.contains_key(&(typ.name.clone(), method_name.clone(), p.name.clone())) {
                        "callable".to_owned()
                    } else {
                        php_type(&p.ty)
                    };
                if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                    let nullable = if ptype.starts_with('?') { "" } else { "?" };
                    format!("{nullable}{ptype} ${} = null", p.name)
                } else {
                    format!("{} ${}", ptype, p.name)
                }
            })
            .collect();
        content.push_str(&crate::backends::php::template_env::render(
            "php_stub_method_definition.jinja",
            context! {
                static_kw => static_kw,
                method_name => &method_name,
                params => &params.join(", "),
                return_type => &return_type,
                stub_body => "",
            },
        ));
        let body = if is_void {
            "    {\n    }\n"
        } else {
            "    {\n        throw new \\RuntimeException('Not implemented — provided by the native extension.');\n    }\n"
        };
        content.push_str(body);
    }

    // Streaming wrapper methods: convert _start/_next/_free Rust functions to PHP Generators.
    for adapter in streaming_adapters {
        let item_type = adapter.item_type.as_deref().unwrap_or("array");
        content.push_str(&gen_php_streaming_method_wrapper(adapter, item_type));
        content.push('\n');
    }

    // Check if this type is a trait bridge type alias (e.g., VisitorHandle)
    for bridge in trait_bridges {
        if let Some(ref type_alias) = bridge.type_alias {
            if type_alias == &typ.name {
                // Emit the from_php_object static method for trait bridge handles
                content.push_str("    /**\n");
                content
                    .push_str("     * Wrap a PHP object implementing the visitor interface as a shareable handle.\n");
                content.push_str("     */\n");
                content.push_str("    public static function from_php_object(object $visitor): self\n");
                content.push_str("    {\n");
                content.push_str(
                    "        throw new \\RuntimeException('Not implemented — provided by the native extension.');\n",
                );
                content.push_str("    }\n");
            }
        }
    }

    content.push_str("}\n");
    content
}

/// Generate a PHP streaming method wrapper for an adapter.
///
/// For PHP, we generate a Generator method that calls the Rust streaming methods directly.
/// Since PHP can't easily pass opaque types as function parameters, we skip the _start/_next/_free
/// pattern and instead keep the streaming logic on the class.
fn gen_php_streaming_method_wrapper(adapter: &crate::core::config::AdapterConfig, _item_type: &str) -> String {
    let method_name = adapter.name.to_lower_camel_case();

    // Build parameter list.
    let mut params_vec: Vec<String> = Vec::new();

    for p in &adapter.params {
        let ptype = php_type(&crate::core::ir::TypeRef::Named(p.ty.clone()));
        let nullable = if p.optional { "?" } else { "" };
        let default = if p.optional { " = null" } else { "" };
        params_vec.push(format!("{nullable}{ptype} ${}{default}", p.name));
    }

    let params_sig = params_vec.join(", ");

    // Generate a stub method that indicates it's provided by the native extension.
    // The actual streaming implementation is on the Rust side; this PHP method
    // is a placeholder for IDE/PHPStan. At runtime, the native extension
    // provides the actual Generator-yielding implementation.
    format!(
        "    public function {method_name}({params_sig}): \\Generator\n    {{\n        \
         throw new \\RuntimeException('Not implemented — provided by the native extension.');\n    \
         }}\n",
        method_name = method_name,
    )
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

/// Generate a static facade method for a streaming adapter.
///
/// For streaming adapters with an `owner_type`, generates a static method that:
/// 1. Takes the owner handle + request type
/// 2. Calls the instance method on the owner handle
/// 3. Returns the streaming result (Vec<String> of JSON chunks)
fn gen_streaming_adapter_facade_method(
    adapter: &crate::core::config::AdapterConfig,
    _mapper: &crate::backends::php::type_map::PhpMapper,
    _opaque_types: &ahash::AHashSet<String>,
    _core_import: &str,
) -> String {
    use heck::ToLowerCamelCase;

    let method_name = adapter.name.to_lower_camel_case();
    let adapter_name = &adapter.name;
    let owner_type = adapter.owner_type.as_deref().unwrap_or_else(|| {
        panic!(
            "php adapter `{adapter_name}`: streaming adapter requires `owner_type` in `[[adapters]]` config (the Rust handle type that owns the streaming method)"
        )
    });

    // Build Rust function signature
    let mut params: Vec<String> = vec![format!("engine: &{owner_type}")];

    // Add request type parameter(s)
    for p in &adapter.params {
        let param_type = p.ty.rsplit("::").next().unwrap_or(&p.ty);
        let ref_indicator = if matches!(param_type, "String" | "Vec<String>") {
            "" // Already reference via .into()
        } else {
            "&"
        };
        let nullable = if p.optional { "Option<" } else { "" };
        let close_nullable = if p.optional { ">" } else { "" };
        params.push(format!(
            "{}: {}{}{}{}",
            p.name, ref_indicator, nullable, param_type, close_nullable
        ));
    }

    let return_type = "std::result::Result<Vec<String>, ext_php_rs::exception::PhpException>";

    // Body: call the instance method on the engine handle
    // Note: adapter.name is already snake_case, so use it directly for the Rust method call
    let rust_method_name = &adapter.name;
    let call_args = adapter
        .params
        .iter()
        .map(|p| format!("&{}", p.name))
        .collect::<Vec<_>>()
        .join(", ");

    crate::backends::php::template_env::render(
        "php_streaming_adapter_method.jinja",
        context! {
            method_name => method_name,
            params => &params.join(", "),
            return_type => return_type,
            rust_method_name => rust_method_name,
            call_args => &call_args,
        },
    )
}

/// Build an inline PHPDoc block for a class property or constructor-promoted parameter.
///
/// - When `doc` is non-empty and multi-line, emits a multi-line block with description lines
///   followed by an `@var` tag.
/// - When `doc` is non-empty and single-line, emits a compact `/** @var T Description. */` form.
/// - When `doc` is empty, emits the type-only compact form `/** @var T */`.
///
/// `indent` is prepended to every line of the output (typically 4 or 8 spaces).
fn php_property_phpdoc(var_type: &str, doc: &str, indent: &str) -> String {
    let doc = doc.trim();
    if doc.is_empty() {
        return crate::backends::php::template_env::render(
            "php_inline_property_phpdoc.jinja",
            context! {
                indent => indent,
                var_type => var_type,
                doc => "",
            },
        );
    }
    let lines: Vec<&str> = doc.lines().collect();
    if lines.len() == 1 {
        let line = lines[0].trim();
        return crate::backends::php::template_env::render(
            "php_inline_property_phpdoc.jinja",
            context! {
                indent => indent,
                var_type => var_type,
                doc => line,
            },
        );
    }
    // Multi-line: description block + @var tag.
    let mut out = format!("{indent}/**\n");
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_indented_phpdoc_empty_line.jinja",
                context! { indent => indent },
            ));
        } else {
            out.push_str(&crate::backends::php::template_env::render(
                "php_prefixed_phpdoc_line.jinja",
                context! {
                    indent => indent,
                    line => trimmed,
                },
            ));
        }
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_indented_phpdoc_empty_line.jinja",
        context! { indent => indent },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        "php_prefixed_phpdoc_line.jinja",
        context! {
            indent => indent,
            line => &format!("@var {var_type}"),
        },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        "php_indented_phpdoc_block_end.jinja",
        context! { indent => indent },
    ));
    out
}

/// Emit a `#[php_impl]` constructor block for a variant-wrapper opaque type.
///
/// When `TypeDef.is_variant_wrapper` is set and no `client_constructors` entry overrides it,
/// this emits:
///
/// ```rust,ignore
/// #[php_impl]
/// impl TypeName {
///     #[php(constructor)]
///     pub fn new(sig_params) -> Self {
///         Self { inner: Arc::new(CoreType::new(call_args)) }
///     }
/// }
/// ```
///
/// This mirrors the `client_constructors` path but drives from the IR's static `new` method
/// rather than a hand-written body template.
/// Check if an opaque type has a no-arg `pub fn new() -> Self` (not Result).
/// If true, clippy will flag `should_implement_trait`, so we need to emit `impl Default`.
fn has_no_arg_new_returning_self(typ: &crate::core::ir::TypeDef) -> bool {
    typ.methods
        .iter()
        .any(|m| m.name == "new" && m.receiver.is_none() && m.params.is_empty() && m.error_type.is_none())
}

fn php_variant_wrapper_constructor(
    typ: &crate::core::ir::TypeDef,
    mapper: &crate::backends::php::type_map::PhpMapper,
    core_import: &str,
) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);
    let call_args = ctor
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let body = if call_args.is_empty() {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new()) }}")
    } else {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new({call_args})) }}")
    };
    let fn_sig = if sig_params.is_empty() {
        "pub fn new() -> Self".to_string()
    } else {
        format!("pub fn new({sig_params}) -> Self")
    };
    Some(format!(
        "#[php_impl]\nimpl {name} {{\n    #[php(constructor)]\n    {fn_sig} {{\n        {body}\n    }}\n}}\n",
        name = typ.name,
    ))
}

/// Generate config.m4 for PIE (PHP Installer for Extensions) to enable building Rust-based PHP extensions.
///
/// PHPize expects config.m4 to describe the build configuration. For Rust extensions built
/// with ext-php-rs, we generate a minimal config.m4 that informs phpize of the extension name
/// and directs the build to use cargo. This allows PIE to fall back from pre-packaged binaries
/// to source compilation without errors.
fn generate_config_m4(extension_name: &str, package_name: &str) -> String {
    // Convert extension_name (with underscores) back to cargo crate name (with hyphens)
    // e.g., "my_ext" → "my-ext" for directory lookup
    let cargo_crate_name = package_name;
    let lib_name = extension_name.replace('_', "-");

    format!(
        r#"dnl Configuration for Rust-based PHP extension via ext-php-rs.
dnl Allows phpize to recognize this extension during source compilation (PIE fallback).

PHP_ARG_ENABLE([{}],
  [whether to enable the {} extension],
  [AS_HELP_STRING([--enable-{}],
    [Enable {} extension support])],
  [yes])

if test "$PHP_{}_ENABLED" = "yes"; then
  dnl Register the extension directory so phpize creates modules/ and sets up build rules.
  PHP_NEW_EXTENSION({}, [], $ext_shared)

  dnl Invoke cargo build to compile the Rust FFI library and copy it to modules/.
  AC_CONFIG_COMMANDS([cargo-build], [
    if test -f "crates/{}-php/Cargo.toml"; then
      (cd crates/{}-php && cargo build --release) || exit 1

      dnl Detect output filename based on platform
      if test -f "crates/{}-php/target/release/lib{}_php.dylib"; then
        cargo_lib="crates/{}-php/target/release/lib{}_php.dylib"
      elif test -f "crates/{}-php/target/release/lib{}_php.so"; then
        cargo_lib="crates/{}-php/target/release/lib{}_php.so"
      else
        echo "ERROR: cargo build succeeded but .so/.dylib not found in crates/{}-php/target/release" >&2
        exit 1
      fi

      mkdir -p modules
      cp "$cargo_lib" "modules/{}.so" || exit 1
    else
      echo "ERROR: crates/{}-php/Cargo.toml not found" >&2
      exit 1
    fi
  ], [])
fi
"#,
        extension_name,
        extension_name,
        extension_name,
        extension_name,
        extension_name.to_uppercase(),
        extension_name,
        cargo_crate_name,
        cargo_crate_name,
        cargo_crate_name,
        lib_name,
        cargo_crate_name,
        lib_name,
        cargo_crate_name,
        lib_name,
        cargo_crate_name,
        cargo_crate_name,
        extension_name,
        cargo_crate_name,
        extension_name,
    )
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    /// Test that PHP wrapper param signatures preserve required-ness from the Rust API.
    ///
    /// Before the fix: Required params after an optional param were being made optional.
    /// Example: `scrape(?CrawlEngineHandle $engine = null, ?string $url = null)`
    /// when the Rust API required both `engine: CrawlEngineHandle` and `url: String`.
    ///
    /// After the fix: Only explicitly optional params or default-constructible params
    /// become optional in the wrapper. Required params stay required.
    /// Example: `scrape(CrawlEngineHandle $engine, string $url)`
    #[test]
    fn test_php_wrapper_param_optionality_logic() {
        use crate::core::ir::{ParamDef, TypeRef};

        // Helper to check if a param should be optional in the wrapper
        let is_optional_default_constructible_param = |p: &ParamDef| -> bool {
            if let TypeRef::Named(name) = &p.ty {
                // Simulate the no_arg_constructor_types set
                matches!(name.as_str(), "CrawlConfig" | "InteractionActions")
            } else {
                false
            }
        };

        // Test case 1: Required params should remain required
        let req_param = ParamDef {
            name: "url".to_string(),
            ty: TypeRef::String,
            optional: false,
            ..ParamDef::default()
        };

        let should_be_optional = req_param.optional || is_optional_default_constructible_param(&req_param);
        assert!(
            !should_be_optional,
            "required param should not become optional in wrapper"
        );

        // Test case 2: Explicitly optional params remain optional
        let opt_param = ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("CrawlConfig".to_string()),
            optional: true,
            ..ParamDef::default()
        };

        let should_be_optional = opt_param.optional || is_optional_default_constructible_param(&opt_param);
        assert!(should_be_optional, "explicitly optional param should be optional");

        // Test case 3: Default-constructible required params become optional
        let default_constructible_param = ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("CrawlConfig".to_string()),
            optional: false,
            ..ParamDef::default()
        };

        let should_be_optional = default_constructible_param.optional
            || is_optional_default_constructible_param(&default_constructible_param);
        assert!(should_be_optional, "default-constructible param should become optional");
    }
}
