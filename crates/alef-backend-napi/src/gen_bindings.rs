use crate::type_map::NapiMapper;
use ahash::AHashSet;
use alef_codegen::builder::{ImplBuilder, RustFileBuilder, StructBuilder};
use alef_codegen::generators::{self, AsyncPattern, RustBindingConfig};
use alef_codegen::naming::to_node_name;
use alef_codegen::shared::{can_auto_delegate, function_params, partition_methods};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile, PostBuildStep};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::path::PathBuf;

pub struct NapiBackend;

impl NapiBackend {
    fn binding_config<'a>(core_import: &'a str, prefix: &'a str, has_serde: bool) -> RustBindingConfig<'a> {
        RustBindingConfig {
            struct_attrs: &["napi"],
            field_attrs: &[],
            struct_derives: &["Clone"],
            method_block_attr: Some("napi"),
            constructor_attr: "#[napi(constructor)]",
            static_attr: None,
            function_attr: "#[napi]",
            enum_attrs: &["napi(string_enum)"],
            enum_derives: &["Clone"],
            needs_signature: false,
            signature_prefix: "",
            signature_suffix: "",
            core_import,
            async_pattern: AsyncPattern::NapiNativeAsync,
            has_serde,
            // NAPI napi(object) structs don't derive Serialize — disable serde bridge
            type_name_prefix: prefix,
            option_duration_on_defaults: true,
        }
    }
}

impl Backend for NapiBackend {
    fn name(&self) -> &str {
        "napi"
    }

    fn language(&self) -> Language {
        Language::Node
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
        let prefix = config.node_type_prefix();
        let mapper = NapiMapper::new(prefix.clone());
        let core_import = config.core_import();

        // Detect serde availability from the output crate's Cargo.toml
        let output_dir = resolve_output_dir(
            config.output.node.as_ref(),
            &config.crate_config.name,
            "crates/{name}-node/src/",
        );
        let has_serde = alef_core::config::detect_serde_available(&output_dir);
        let cfg = Self::binding_config(&core_import, &prefix, has_serde);

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code)");
        builder.add_import("napi::*");
        builder.add_import("napi_derive::napi");

        // Always import serde_json for type conversion in From/Into impls,
        // even if the binding crate doesn't explicitly list it as a dependency.
        // serde_json is needed for conversions of types with serde-serializable fields.
        builder.add_import("serde_json");

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

        // Note: custom_modules for Node are TypeScript-only re-exports
        // (used in generate_public_api), not Rust module declarations.

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

        // NAPI has some unique patterns: Js-prefixed names, Option-wrapped fields,
        // and custom constructor. Use shared generators for enums and functions,
        // but keep struct/method generation custom.
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                builder.add_item(&alef_codegen::generators::gen_opaque_struct_prefixed(
                    typ, &cfg, &prefix,
                ));
                builder.add_item(&gen_opaque_struct_methods(typ, &mapper, &cfg, &opaque_types, &prefix));
            } else {
                // Non-opaque structs use #[napi(object)] — plain JS objects without methods.
                // napi(object) structs cannot have #[napi] impl blocks.
                // gen_struct adds Default to derives when typ.has_default is true.
                builder.add_item(&gen_struct(typ, &mapper, &prefix));
            }
        }

        for enum_def in &api.enums {
            builder.add_item(&gen_enum(enum_def, &prefix));
        }

        for func in &api.functions {
            builder.add_item(&gen_function(func, &mapper, &cfg, &opaque_types, &prefix));
        }

        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        let napi_conv_config = alef_codegen::conversions::ConversionConfig {
            type_name_prefix: &prefix,
            cast_large_ints_to_i64: true,
            cast_f32_to_f64: true,
            // optionalize_defaults: For types with has_default, conversion generators
            // make all fields Option<T> and apply defaults via FromNapiValue,
            // enabling JS users to pass partial objects and omit fields they want defaults for.
            optionalize_defaults: true,
            option_duration_on_defaults: true,
            include_cfg_metadata: true,
            ..Default::default()
        };
        // From/Into conversions using shared parameterized generators
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &napi_conv_config,
                ));
            }
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &napi_conv_config,
                ));
            }
        }
        for e in &api.enums {
            let is_tagged_data_enum = e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty());
            if is_tagged_data_enum {
                // Tagged data enums use flattened struct — generate custom conversions
                builder.add_item(&gen_tagged_enum_binding_to_core(e, &core_import, &prefix));
                builder.add_item(&gen_tagged_enum_core_to_binding(e, &core_import, &prefix));
            } else {
                if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                        e,
                        &core_import,
                        &napi_conv_config,
                    ));
                }
                if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                        e,
                        &core_import,
                        &napi_conv_config,
                    ));
                }
            }
        }

        // Error types (variant name constants + converter functions)
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_napi_error_types(error));
            builder.add_item(&alef_codegen::error_gen::gen_napi_error_converter(error, &core_import));
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Node)?;

        let content = builder.build();

        let output_dir = resolve_output_dir(
            config.output.node.as_ref(),
            &config.crate_config.name,
            "crates/{name}-node/src/",
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.node_type_prefix();

        // Separate exports into functions (plain export) and types (export type)
        let mut type_exports = vec![];
        let mut function_exports = vec![];

        // Collect all types (exported with prefix from native module) - export type
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            type_exports.push(format!("{prefix}{}", typ.name));
        }

        // Collect all enums as value exports (runtime objects).
        // NAPI generates const enum in .d.ts, but we post-process it to regular enum
        // so they can be re-exported as values with verbatimModuleSyntax.
        for enum_def in &api.enums {
            function_exports.push(format!("{prefix}{}", enum_def.name));
        }

        // NAPI errors are thrown as native JS Error objects, not exported as TS types.
        // Skip error types in the public API re-exports.

        // Collect all functions (exported from native module) - plain export
        for func in &api.functions {
            // Convert snake_case to camelCase for JavaScript naming
            let js_name = to_node_name(&func.name);
            function_exports.push(js_name);
        }

        // Sort for consistent output
        type_exports.sort();
        function_exports.sort();

        // Generate the index.ts re-export file using a single export block
        // with inline `type` annotations for verbatimModuleSyntax compatibility.
        let mut lines = vec![
            "// This file is auto-generated by alef. DO NOT EDIT.".to_string(),
            "".to_string(),
        ];

        // Separate value and type exports for isolatedModules compatibility.
        // Value exports (functions + enums) in one block, type exports (structs) in another.
        if !function_exports.is_empty() {
            lines.push("export {".to_string());
            for name in &function_exports {
                lines.push(format!("  {name},"));
            }
            lines.push(format!("}} from '{}';", config.node_package_name()));
            lines.push("".to_string());
        }
        if !type_exports.is_empty() {
            lines.push("export type {".to_string());
            for name in &type_exports {
                lines.push(format!("  {name},"));
            }
            lines.push(format!("}} from '{}';", config.node_package_name()));
        }

        // Append re-exports for custom modules (from [custom_modules] node = [...])
        let custom_mods = config.custom_modules.for_language(Language::Node);
        for module_name in custom_mods {
            lines.push(format!("export * from './{module_name}';"));
        }

        let content = lines.join("\n");

        // Output path: packages/typescript/src/index.ts
        let output_path = PathBuf::from("packages/typescript/src/index.ts");

        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: false,
        }])
    }

    fn generate_type_stubs(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.node_type_prefix();
        let content = gen_dts(api, &prefix);

        // `config.output.node` points to the `src/` directory (e.g., `crates/{name}-node/src/`).
        // `index.d.ts` belongs at the crate root, one level up from `src/`.
        // When the configured path ends in `src/` or `src`, strip that suffix to get the crate root.
        // Falls back to `crates/{name}-node/` if no node output is configured.
        let src_dir = resolve_output_dir(
            config.output.node.as_ref(),
            &config.crate_config.name,
            "crates/{name}-node/src/",
        );
        let crate_root = {
            let p = PathBuf::from(&src_dir);
            match p.file_name().and_then(|n| n.to_str()) {
                Some("src") => p.parent().map(|parent| parent.to_path_buf()).unwrap_or(p),
                _ => p,
            }
        };

        Ok(vec![GeneratedFile {
            path: crate_root.join("index.d.ts"),
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "napi",
            crate_suffix: "-node",
            depends_on_ffi: false,
            post_build: vec![PostBuildStep::PatchFile {
                path: "index.d.ts",
                find: "export declare const enum",
                replace: "export declare enum",
            }],
        })
    }
}

/// Generate a NAPI struct with Js-prefixed name and fields wrapped in Option only if optional.
fn gen_struct(typ: &TypeDef, mapper: &NapiMapper, prefix: &str) -> String {
    let mut struct_builder = StructBuilder::new(&format!("{prefix}{}", typ.name));
    // Use napi(object) so the struct can be used as function/method parameters (FromNapiValue)
    struct_builder.add_attr("napi(object)");
    struct_builder.add_derive("Clone");
    struct_builder.add_derive("serde::Serialize");
    struct_builder.add_derive("serde::Deserialize");
    // Types with has_default get #[derive(Default)] instead of a manual impl.
    if typ.has_default {
        struct_builder.add_derive("Default");
    }

    for field in &typ.fields {
        let mapped_type = mapper.map_type(&field.ty);
        // For types with Default, make all fields optional so JS callers
        // can pass partial objects (missing fields get defaults).
        let field_type = if field.optional || typ.has_default {
            format!("Option<{}>", mapped_type)
        } else {
            mapped_type
        };
        let js_name = to_node_name(&field.name);
        let attrs = if js_name != field.name {
            vec![format!("napi(js_name = \"{}\")", js_name)]
        } else {
            vec![]
        };
        struct_builder.add_field(&field.name, &field_type, attrs);
    }

    struct_builder.build()
}

/// Generate NAPI methods for an opaque struct (delegates to self.inner).
fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let mut impl_builder = ImplBuilder::new(&format!("{prefix}{}", typ.name));
    impl_builder.add_attr("napi");

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        impl_builder.add_method(&gen_opaque_instance_method(
            method,
            mapper,
            typ,
            cfg,
            opaque_types,
            prefix,
        ));
    }
    for method in &statics {
        impl_builder.add_method(&gen_static_method(method, mapper, typ, cfg, opaque_types, prefix));
    }

    impl_builder.build()
}

/// Generate an opaque instance method that delegates to self.inner.
fn gen_opaque_instance_method(
    method: &MethodDef,
    mapper: &NapiMapper,
    typ: &TypeDef,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let async_kw = if method.is_async { "async " } else { "" };

    let type_name = &typ.name;
    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::RefMut));
    let call_args = napi_gen_call_args(&method.params, opaque_types);

    // Use the shared can_auto_delegate check for opaque instance methods.
    // Skip delegation if the receiver is RefMut, since Arc<T> doesn't support &mut T.
    let opaque_can_delegate = !method.sanitized
        && !is_ref_mut_receiver
        && (!is_owned_receiver || typ.is_clone)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && alef_codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && alef_codegen::shared::is_opaque_delegatable_type(&method.return_type);

    let make_core_call = |method_name: &str| -> String {
        if is_owned_receiver {
            format!("(*self.inner).clone().{method_name}({call_args})")
        } else {
            format!("self.inner.{method_name}({call_args})")
        }
    };

    let make_async_core_call = |method_name: &str| -> String { format!("inner.{method_name}({call_args})") };

    let async_result_wrap = napi_wrap_return(
        "result",
        &method.return_type,
        type_name,
        opaque_types,
        true,
        method.returns_ref,
        prefix,
    );

    let body = if !opaque_can_delegate {
        // Try serde-based param conversion for methods with non-opaque Named params
        if cfg.has_serde
            && !method.sanitized
            && generators::has_named_params(&method.params, opaque_types)
            && method.error_type.is_some()
            && alef_codegen::shared::is_opaque_delegatable_type(&method.return_type)
        {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let serde_bindings =
                generators::gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        ");
            let serde_call_args = generators::gen_call_args_with_let_bindings(&method.params, opaque_types);
            let core_call = format!("self.inner.{}({serde_call_args})", method.name);
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{err_conv}?;\n    Ok(())")
            } else {
                let wrap = napi_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                );
                format!("{serde_bindings}let result = {core_call}{err_conv}?;\n    Ok({wrap})")
            }
        } else {
            generators::gen_unimplemented_body(
                &method.return_type,
                &format!("{type_name}.{}", method.name),
                method.error_type.is_some(),
                cfg,
                &method.params,
            )
        }
    } else if method.is_async {
        let inner_clone_line = "let inner = self.inner.clone();\n    ";
        let core_call_str = make_async_core_call(&method.name);
        generators::gen_async_body(
            &core_call_str,
            cfg,
            method.error_type.is_some(),
            &async_result_wrap,
            true,
            inner_clone_line,
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = make_core_call(&method.name);
        if method.error_type.is_some() {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{core_call}{err_conv}?;\n    Ok(())")
            } else {
                let wrap = napi_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                );
                format!("let result = {core_call}{err_conv}?;\n    Ok({wrap})")
            }
        } else {
            napi_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                prefix,
            )
        }
    };

    let mut attrs = String::new();
    // Per-item clippy suppression: too_many_arguments when >7 params (including &self)
    if method.params.len() + 1 > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning methods
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    // Per-item clippy suppression: should_implement_trait for trait-conflicting names
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    format!(
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}(&self, {params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        method.name
    )
}

/// Generate a static method binding.
fn gen_static_method(
    method: &MethodDef,
    mapper: &NapiMapper,
    typ: &TypeDef,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let type_name = &typ.name;
    let core_type_path = typ.rust_path.replace('-', "_");
    let call_args = napi_gen_call_args(&method.params, opaque_types);
    let can_delegate_static = can_auto_delegate(method, opaque_types);

    let async_kw = if method.is_async { "async " } else { "" };

    let body = if !can_delegate_static {
        generators::gen_unimplemented_body(
            &method.return_type,
            &format!("{type_name}::{}", method.name),
            method.error_type.is_some(),
            cfg,
            &method.params,
        )
    } else if method.is_async {
        let core_call = format!("{core_type_path}::{}({call_args})", method.name);
        let return_wrap = napi_wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            typ.is_opaque,
            method.returns_ref,
            prefix,
        );
        generators::gen_async_body(
            &core_call,
            cfg,
            method.error_type.is_some(),
            &return_wrap,
            false,
            "",
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_type_path}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let wrapped = napi_wrap_return(
                "val",
                &method.return_type,
                type_name,
                opaque_types,
                typ.is_opaque,
                method.returns_ref,
                prefix,
            );
            if wrapped == "val" {
                format!("{core_call}{err_conv}")
            } else {
                format!("{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            napi_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                typ.is_opaque,
                method.returns_ref,
                prefix,
            )
        }
    };

    let mut attrs = String::new();
    // Per-item clippy suppression: too_many_arguments when >7 params
    if method.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning methods
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    // Per-item clippy suppression: should_implement_trait for trait-conflicting names
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    format!(
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        method.name
    )
}

/// Generate a NAPI enum definition using string_enum with Js prefix.
/// Generate a NAPI enum definition.
/// For simple enums (no variant fields): generates `#[napi(string_enum)]`.
/// For tagged enums with data fields: generates a flattened `#[napi(object)]` struct
/// with a discriminant field and all variant fields as optional.
fn gen_enum(enum_def: &EnumDef, prefix: &str) -> String {
    let is_tagged_data_enum = enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if is_tagged_data_enum {
        return gen_tagged_enum_as_object(enum_def, prefix);
    }

    // Simple string enum
    let napi_case = enum_def.serde_rename_all.as_deref().and_then(|s| match s {
        "snake_case" => Some("snake_case"),
        "camelCase" => Some("camelCase"),
        "kebab-case" => Some("kebab-case"),
        "SCREAMING_SNAKE_CASE" => Some("UPPER_SNAKE"),
        "lowercase" => Some("lowercase"),
        "UPPERCASE" => Some("UPPERCASE"),
        "PascalCase" => Some("PascalCase"),
        _ => None,
    });

    let string_enum_attr = match napi_case {
        Some(case) => format!("#[napi(string_enum = \"{case}\")]"),
        None => "#[napi(string_enum)]".to_string(),
    };

    let mut lines = vec![
        string_enum_attr,
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]".to_string(),
        format!("pub enum {prefix}{} {{", enum_def.name),
    ];

    for variant in &enum_def.variants {
        lines.push(format!("    {},", variant.name));
    }

    lines.push("}".to_string());

    // Default impl for config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", first.name));
        lines.push("}".to_string());
    }

    lines.join("\n")
}

/// Generate a tagged enum as a flattened `#[napi(object)]` struct.
/// E.g. `AuthConfig { Basic { username, password }, Bearer { token } }` becomes:
/// ```rust,ignore
/// #[napi(object)]
/// struct JsAuthConfig {
///     #[napi(js_name = "type")]
///     pub auth_type: String,
///     pub username: Option<String>,
///     pub password: Option<String>,
///     pub token: Option<String>,
/// }
/// ```
fn gen_tagged_enum_as_object(enum_def: &EnumDef, prefix: &str) -> String {
    use alef_codegen::type_mapper::TypeMapper;
    let mapper = NapiMapper::new(prefix.to_string());

    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    let mut lines = vec![
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]".to_string(),
        "#[napi(object)]".to_string(),
        format!("pub struct {prefix}{} {{", enum_def.name),
        format!("    #[napi(js_name = \"{tag_field}\")]"),
        format!("    pub {tag_field}_tag: String,"),
    ];

    // Collect all unique fields across all variants (all made optional)
    let mut seen_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if seen_fields.insert(field.name.clone()) {
                let field_type = mapper.map_type(&field.ty);
                let js_name = alef_codegen::naming::to_node_name(&field.name);
                if js_name != field.name {
                    lines.push(format!("    #[napi(js_name = \"{js_name}\")]"));
                }
                lines.push(format!("    pub {}: Option<{field_type}>,", field.name));
            }
        }
    }

    lines.push("}".to_string());

    // Default impl
    lines.push(String::new());
    lines.push("#[allow(clippy::derivable_impls)]".to_string());
    lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
    lines.push(format!(
        "    fn default() -> Self {{ Self {{ {tag_field}_tag: String::new(), {} }} }}",
        seen_fields
            .iter()
            .map(|f| format!("{f}: None"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate a free function binding.
fn gen_function(
    func: &FunctionDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params = function_params(&func.params, &|ty| {
        // Opaque Named params must be received by reference since NAPI opaque
        // structs don't implement FromNapiValue (they use Arc<T> internally).
        if let TypeRef::Named(n) = ty {
            if opaque_types.contains(n.as_str()) {
                return format!("&{prefix}{n}");
            }
        }
        mapper.map_type(ty)
    });
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let core_import = cfg.core_import;
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    // Use let-binding pattern for non-opaque Named params, or for Vec<f32> params that need conversion
    let use_let_bindings = generators::has_named_params(&func.params, opaque_types)
        || func.params.iter().any(|p| needs_vec_f32_conversion(&p.ty));
    let call_args = if use_let_bindings {
        let base_args = generators::gen_call_args_with_let_bindings(&func.params, opaque_types);
        napi_apply_primitive_casts_to_call_args(&base_args, &func.params)
    } else {
        napi_gen_call_args(&func.params, opaque_types)
    };

    let can_delegate_fn = alef_codegen::shared::can_auto_delegate_function(func, opaque_types);

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    let async_kw = if func.is_async { "async " } else { "" };

    let body = if !can_delegate_fn {
        // Try serde-based conversion for non-delegatable functions with Named params
        // Only use serde conversion if cfg.has_serde is true (binding crate has serde deps)
        if cfg.has_serde && use_let_bindings && func.error_type.is_some() {
            let serde_bindings =
                generators::gen_serde_let_bindings(&func.params, opaque_types, core_import, err_conv, "    ");
            let core_call = format!("{core_fn_path}({call_args})");
            let await_kw = if func.is_async { ".await" } else { "" };

            if matches!(func.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{await_kw}{err_conv}?;\n    Ok(())")
            } else {
                let wrapped = napi_wrap_return_fn("val", &func.return_type, opaque_types, func.returns_ref, prefix);
                if wrapped == "val" {
                    format!("{serde_bindings}{core_call}{await_kw}{err_conv}")
                } else {
                    format!("{serde_bindings}{core_call}{await_kw}.map(|val| {wrapped}){err_conv}")
                }
            }
        } else {
            generators::gen_unimplemented_body(
                &func.return_type,
                &func.name,
                func.error_type.is_some(),
                cfg,
                &func.params,
            )
        }
    } else if func.is_async {
        // For async delegatable functions, generate let bindings if needed before the async call
        let mut let_bindings = if use_let_bindings {
            generators::gen_named_let_bindings_pub(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Add Vec<f32> conversion bindings for parameters not already handled
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));
        let core_call = format!("{core_fn_path}({call_args})");
        let return_wrap = napi_wrap_return_fn("result", &func.return_type, opaque_types, func.returns_ref, prefix);
        let return_type = mapper.map_type(&func.return_type);
        generators::gen_async_body(
            &core_call,
            cfg,
            func.error_type.is_some(),
            &return_wrap,
            false,
            &let_bindings,
            matches!(func.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_fn_path}({call_args})");
        // Generate let bindings for Named params if needed
        let mut let_bindings = if use_let_bindings {
            generators::gen_named_let_bindings_pub(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Add Vec<f32> conversion bindings for parameters not already handled
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));

        if func.error_type.is_some() {
            let wrapped = napi_wrap_return_fn("val", &func.return_type, opaque_types, func.returns_ref, prefix);
            if wrapped == "val" {
                format!("{let_bindings}{core_call}{err_conv}")
            } else {
                format!("{let_bindings}{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            format!(
                "{let_bindings}{}",
                napi_wrap_return_fn(&core_call, &func.return_type, opaque_types, func.returns_ref, prefix)
            )
        }
    };

    let mut attrs = String::new();
    // Per-item clippy suppression: too_many_arguments when >7 params
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    format!(
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        func.name
    )
}

/// Apply NAPI-specific primitive casts to the call args generated by the generic let-binding handler.
/// Adds i64→usize, i64→isize, f64→f32 casts where needed.
fn napi_apply_primitive_casts_to_call_args(generic_args: &str, params: &[ParamDef]) -> String {
    // Split args by comma and match with params to apply casting
    let args_list: Vec<&str> = generic_args.split(',').map(|s| s.trim()).collect();
    args_list
        .iter()
        .zip(params.iter())
        .map(|(arg, p)| {
            // Special case: Vec<f32> param with is_ref uses the converted variable
            if needs_vec_f32_conversion(&p.ty) && p.is_ref {
                return format!("&{}_f32", p.name);
            }
            match &p.ty {
                TypeRef::Primitive(prim) if needs_napi_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        // Optional: arg might be like "param.map(...)" so re-apply map
                        if arg.contains(".map(") || arg.contains(".as_") {
                            // Already handled, keep as is
                            arg.to_string()
                        } else {
                            format!("{}.map(|v| v as {})", arg, core_ty)
                        }
                    } else {
                        // Non-optional: simple cast
                        format!("{} as {}", arg, core_ty)
                    }
                }
                _ => arg.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate let bindings for Vec<f32> parameters that need f64→f32 conversion.
/// This handles the case where NAPI maps f32→f64, but a function param is Vec<f32> taking a reference.
fn gen_vec_f32_conversion_bindings(params: &[ParamDef]) -> String {
    let mut bindings = String::new();
    for p in params {
        if needs_vec_f32_conversion(&p.ty) && p.is_ref {
            let conv_name = format!("{}_f32", p.name);
            bindings.push_str(&format!(
                "    let {conv_name}: Vec<f32> = {}.iter().map(|&x| x as f32).collect();\n",
                p.name
            ));
        }
    }
    bindings
}

/// NAPI-specific call args that casts i64 params to u64/usize where the core expects it.
/// Properly handles is_ref for reference parameters and complex type conversions.
fn napi_gen_call_args(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .map(|p| {
            // Special case: Vec<f32> param with is_ref uses the converted variable
            if needs_vec_f32_conversion(&p.ty) && p.is_ref {
                return format!("&{}_f32", p.name);
            }
            match &p.ty {
                TypeRef::Primitive(prim) if needs_napi_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        format!("{}.map(|v| v as {})", p.name, core_ty)
                    } else {
                        format!("{} as {}", p.name, core_ty)
                    }
                }
                TypeRef::Duration => {
                    if p.optional {
                        format!("{}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))", p.name)
                    } else {
                        format!("std::time::Duration::from_millis({}.max(0) as u64)", p.name)
                    }
                }
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
                TypeRef::Named(_) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else {
                            format!("{}.map(Into::into)", p.name)
                        }
                    } else {
                        format!("{}.into()", p.name)
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Path => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref().map(std::path::Path::new)", p.name)
                        } else {
                            format!("{}.map(std::path::PathBuf::from)", p.name)
                        }
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{})", p.name)
                    } else {
                        format!("std::path::PathBuf::from({})", p.name)
                    }
                }
                TypeRef::Bytes => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Vec(_) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Map(_, _) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// NAPI-specific return wrapping for opaque instance methods.
/// Extends the shared `wrap_return` with i64 casts for u64/usize/isize primitives.
fn napi_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    prefix: &str,
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_napi_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        // Opaque Named returns need prefix
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        _ => generators::wrap_return(
            expr,
            return_type,
            type_name,
            opaque_types,
            self_is_opaque,
            returns_ref,
            false,
        ),
    }
}

/// NAPI-specific return wrapping for free functions (no type_name context).
fn napi_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    prefix: &str,
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_napi_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
            if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                // Vec<usize>, Vec<f32>, etc. need element-wise casting to i64 or f64
                let target_ty = match p {
                    alef_core::ir::PrimitiveType::F32 => "f64",
                    _ => "i64", // u64, usize, isize, u32
                };
                format!("{expr}.into_iter().map(|v| v as {target_ty}).collect()")
            }
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Check if a type is Vec<f32> which needs element-wise conversion from f64 in NAPI.
fn needs_vec_f32_conversion(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(alef_core::ir::PrimitiveType::F32)))
}

fn needs_napi_cast(p: &alef_core::ir::PrimitiveType) -> bool {
    matches!(
        p,
        alef_core::ir::PrimitiveType::U32
            | alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::Usize
            | alef_core::ir::PrimitiveType::Isize
            | alef_core::ir::PrimitiveType::F32
    )
}

fn core_prim_str(p: &alef_core::ir::PrimitiveType) -> &'static str {
    match p {
        alef_core::ir::PrimitiveType::U32 => "u32",
        alef_core::ir::PrimitiveType::U64 => "u64",
        alef_core::ir::PrimitiveType::Usize => "usize",
        alef_core::ir::PrimitiveType::Isize => "isize",
        alef_core::ir::PrimitiveType::F32 => "f32",
        _ => unreachable!(),
    }
}

/// Generate a global Tokio runtime for NAPI async support.
fn gen_tokio_runtime() -> String {
    "static WORKER_POOL: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect(\"Failed to create Tokio runtime\")
});"
    .to_string()
}

/// Generate an `index.d.ts` file for the NAPI binding crate.
///
/// NAPI-RS generates `const enum` in its auto-generated `.d.ts`, which is incompatible
/// with `verbatimModuleSyntax` (const enums cannot be re-exported as values). This
/// function produces an equivalent `.d.ts` with `export declare enum` (regular enum)
/// so the file can be committed and used directly without a post-build patch step.
///
/// The output format matches what NAPI-RS would generate after patching, using the same
/// alphabetical ordering and type declarations seen in the committed `index.d.ts` files.
fn gen_dts(api: &ApiSurface, prefix: &str) -> String {
    let mut lines: Vec<String> = vec![
        "/* auto-generated by alef */".to_string(),
        "/* eslint-disable */".to_string(),
    ];

    // Collect all declarations: opaque types (classes), plain structs (interfaces), enums, functions.
    // Sort each group alphabetically to produce stable, deterministic output.

    // Opaque types → `export declare class`
    let mut opaque_types: Vec<&TypeDef> = api.types.iter().filter(|t| t.is_opaque).collect();
    opaque_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Plain structs → `export interface`
    let mut plain_types: Vec<&TypeDef> = api.types.iter().filter(|t| !t.is_opaque).collect();
    plain_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Enums → `export declare enum`
    let mut sorted_enums: Vec<&EnumDef> = api.enums.iter().collect();
    sorted_enums.sort_by(|a, b| a.name.cmp(&b.name));

    // Functions → `export declare function`
    let mut sorted_fns: Vec<&FunctionDef> = api.functions.iter().collect();
    sorted_fns.sort_by(|a, b| a.name.cmp(&b.name));

    // Build a merged list of all declarations sorted by their Js-prefixed name so the
    // output is fully alphabetical (matching the committed index.d.ts format).
    enum Decl<'a> {
        Class(&'a TypeDef),
        Interface(&'a TypeDef),
        Enum(&'a EnumDef),
        Function(&'a FunctionDef),
    }

    let mut all_decls: Vec<(String, Decl<'_>)> = Vec::new();
    for t in &opaque_types {
        all_decls.push((format!("{prefix}{}", t.name), Decl::Class(t)));
    }
    for t in &plain_types {
        all_decls.push((format!("{prefix}{}", t.name), Decl::Interface(t)));
    }
    for e in &sorted_enums {
        all_decls.push((format!("{prefix}{}", e.name), Decl::Enum(e)));
    }
    for f in &sorted_fns {
        all_decls.push((to_node_name(&f.name), Decl::Function(f)));
    }
    all_decls.sort_by_key(|a| a.0.to_lowercase());

    for (_, decl) in &all_decls {
        lines.push(String::new());
        match decl {
            Decl::Class(typ) => {
                lines.push(format!("export declare class {prefix}{} {{", typ.name));
                for method in &typ.methods {
                    let js_name = to_node_name(&method.name);
                    let params = dts_params(&method.params, prefix);
                    let ret = dts_return_type(
                        &method.return_type,
                        method.error_type.is_some(),
                        method.is_async,
                        prefix,
                    );
                    if method.is_static {
                        lines.push(format!("  static {js_name}({params}): {ret}"));
                    } else {
                        lines.push(format!("  {js_name}({params}): {ret}"));
                    }
                }
                lines.push("}".to_string());
            }
            Decl::Interface(typ) => {
                lines.push(format!("export interface {prefix}{} {{", typ.name));
                for field in &typ.fields {
                    let js_name = to_node_name(&field.name);
                    let ts_ty = dts_type(&field.ty, prefix);
                    // All fields on plain structs are optional (NAPI napi(object) makes them Option).
                    lines.push(format!("  {js_name}?: {ts_ty}"));
                }
                lines.push("}".to_string());
            }
            Decl::Enum(e) => {
                lines.push(format!("export declare enum {prefix}{} {{", e.name));
                for variant in &e.variants {
                    // NAPI string_enum: variant values are the variant name as a string literal.
                    let value = variant.serde_rename.as_deref().unwrap_or(variant.name.as_str());
                    lines.push(format!("  {} = '{}',", variant.name, value));
                }
                lines.push("}".to_string());
            }
            Decl::Function(func) => {
                let js_name = to_node_name(&func.name);
                let params = dts_params(&func.params, prefix);
                let ret = dts_return_type(&func.return_type, func.error_type.is_some(), func.is_async, prefix);
                lines.push(format!("export declare function {js_name}({params}): {ret}"));
            }
        }
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Map an IR `TypeRef` to its TypeScript equivalent for `.d.ts` generation.
fn dts_type(ty: &TypeRef, prefix: &str) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "boolean".to_string(),
            alef_core::ir::PrimitiveType::U8
            | alef_core::ir::PrimitiveType::U16
            | alef_core::ir::PrimitiveType::U32
            | alef_core::ir::PrimitiveType::I8
            | alef_core::ir::PrimitiveType::I16
            | alef_core::ir::PrimitiveType::I32
            | alef_core::ir::PrimitiveType::F32
            | alef_core::ir::PrimitiveType::F64 => "number".to_string(),
            // NAPI maps u64/usize/isize to i64 on the Rust side; JS sees it as number.
            alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::I64
            | alef_core::ir::PrimitiveType::Usize
            | alef_core::ir::PrimitiveType::Isize => "number".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path => "string".to_string(),
        TypeRef::Bytes => "Uint8Array".to_string(),
        TypeRef::Json => "string".to_string(),
        TypeRef::Duration => "number".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Optional(inner) => format!("{} | undefined | null", dts_type(inner, prefix)),
        TypeRef::Vec(inner) => format!("Array<{}>", dts_type(inner, prefix)),
        TypeRef::Map(k, v) => format!("Record<{}, {}>", dts_type(k, prefix), dts_type(v, prefix)),
        TypeRef::Named(name) => format!("{prefix}{name}"),
    }
}

/// Render a list of parameters as a TypeScript parameter string for `.d.ts`.
fn dts_params(params: &[ParamDef], prefix: &str) -> String {
    params
        .iter()
        .map(|p| {
            let js_name = to_node_name(&p.name);
            let ts_ty = dts_type(&p.ty, prefix);
            if p.optional {
                format!("{js_name}?: {ts_ty} | undefined | null")
            } else {
                format!("{js_name}: {ts_ty}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render the TypeScript return type for a function/method in `.d.ts`.
///
/// Async functions return `Promise<T>`. Functions that can error still return `T`
/// (NAPI throws JS exceptions on error, so the `.d.ts` signature just shows the success type).
fn dts_return_type(ret: &TypeRef, _has_error: bool, is_async: bool, prefix: &str) -> String {
    let base = match ret {
        TypeRef::Unit => "void".to_string(),
        other => dts_type(other, prefix),
    };
    if is_async { format!("Promise<{base}>") } else { base }
}

/// Generate `From<JsTaggedEnum> for core::TaggedEnum` for a flattened struct representation.
fn gen_tagged_enum_binding_to_core(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    use alef_core::ir::TypeRef;
    use std::fmt::Write;
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    let mut out = String::with_capacity(512);
    writeln!(out, "impl From<{binding_name}> for {core_path} {{").ok();
    writeln!(out, "    fn from(val: {binding_name}) -> Self {{").ok();
    writeln!(out, "        match val.{tag_field}_tag.as_str() {{").ok();

    for variant in &enum_def.variants {
        let default_tag = variant.name.to_lowercase();
        let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
        if variant.fields.is_empty() {
            writeln!(out, "            \"{tag_value}\" => Self::{},", variant.name).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            let field_exprs: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    if f.optional {
                        // Optional fields: apply type-specific conversions
                        match &f.ty {
                            TypeRef::Path => {
                                format!("val.{}.map(std::path::PathBuf::from)", f.name)
                            }
                            TypeRef::Named(_) => {
                                format!("val.{}.map(Into::into)", f.name)
                            }
                            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                let core_ty = core_prim_str(p);
                                format!("val.{}.map(|v| v as {core_ty})", f.name)
                            }
                            _ => {
                                format!("val.{}", f.name)
                            }
                        }
                    } else if f.sanitized {
                        "Default::default()".to_string()
                    } else {
                        match &f.ty {
                            TypeRef::Named(_) => {
                                format!("val.{}.unwrap_or_default().into()", f.name)
                            }
                            TypeRef::Path => {
                                format!("val.{}.map(std::path::PathBuf::from).unwrap_or_default()", f.name)
                            }
                            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                let core_ty = core_prim_str(p);
                                format!("val.{}.map(|v| v as {core_ty}).unwrap_or_default()", f.name)
                            }
                            _ => {
                                format!("val.{}.unwrap_or_default()", f.name)
                            }
                        }
                    }
                })
                .collect();
            if is_tuple {
                writeln!(
                    out,
                    "            \"{tag_value}\" => Self::{}({}),",
                    variant.name,
                    field_exprs.join(", ")
                )
                .ok();
            } else {
                let field_inits: Vec<String> = variant
                    .fields
                    .iter()
                    .zip(field_exprs.iter())
                    .map(|(f, expr)| format!("{}: {expr}", f.name))
                    .collect();
                writeln!(
                    out,
                    "            \"{tag_value}\" => Self::{} {{ {} }},",
                    variant.name,
                    field_inits.join(", ")
                )
                .ok();
            }
        }
    }

    // Default fallback to first variant
    if let Some(first) = enum_def.variants.first() {
        if first.fields.is_empty() {
            writeln!(out, "            _ => Self::{},", first.name).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&first.fields);
            if is_tuple {
                let defaults: Vec<&str> = first.fields.iter().map(|_| "Default::default()").collect();
                writeln!(out, "            _ => Self::{}({}),", first.name, defaults.join(", ")).ok();
            } else {
                let defaults: Vec<String> = first
                    .fields
                    .iter()
                    .map(|f| format!("{}: Default::default()", f.name))
                    .collect();
                writeln!(
                    out,
                    "            _ => Self::{} {{ {} }},",
                    first.name,
                    defaults.join(", ")
                )
                .ok();
            }
        }
    }

    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

/// Generate `From<core::TaggedEnum> for JsTaggedEnum` for a flattened struct representation.
fn gen_tagged_enum_core_to_binding(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    use std::fmt::Write;
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    // Collect all field names across all variants
    let all_fields: Vec<String> = {
        let mut fields = std::collections::BTreeSet::new();
        for v in &enum_def.variants {
            for f in &v.fields {
                fields.insert(f.name.clone());
            }
        }
        fields.into_iter().collect()
    };

    let mut out = String::with_capacity(512);
    writeln!(out, "impl From<{core_path}> for {binding_name} {{").ok();
    writeln!(out, "    fn from(val: {core_path}) -> Self {{").ok();
    writeln!(out, "        match val {{").ok();

    for variant in &enum_def.variants {
        let default_tag = variant.name.to_lowercase();
        let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
        let _variant_field_names: std::collections::BTreeSet<String> =
            variant.fields.iter().map(|f| f.name.clone()).collect();

        if variant.fields.is_empty() {
            writeln!(
                out,
                "            {core_path}::{} => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},",
                variant.name,
                all_fields
                    .iter()
                    .map(|f| format!("{f}: None"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .ok();
        } else {
            use alef_core::ir::TypeRef;
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            let variant_field_map: std::collections::BTreeMap<&str, &alef_core::ir::FieldDef> =
                variant.fields.iter().map(|f| (f.name.as_str(), f)).collect();
            let destructured: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    if f.sanitized {
                        if is_tuple {
                            format!("_{}", f.name)
                        } else {
                            format!("{}: _{}", f.name, f.name)
                        }
                    } else {
                        f.name.clone()
                    }
                })
                .collect();
            let field_inits: Vec<String> = all_fields
                .iter()
                .map(|f| {
                    if let Some(field) = variant_field_map.get(f.as_str()) {
                        if field.optional {
                            match &field.ty {
                                TypeRef::Path => format!("{f}: {f}.map(|p| p.to_string_lossy().to_string())"),
                                TypeRef::Named(_) => format!("{f}: {f}.map(Into::into)"),
                                _ => format!("{f}: {f}"),
                            }
                        } else if field.sanitized {
                            format!("{f}: None")
                        } else {
                            match &field.ty {
                                TypeRef::Named(_) => format!("{f}: Some({f}.into())"),
                                TypeRef::Path => format!("{f}: Some({f}.to_string_lossy().to_string())"),
                                // Tagged enum struct fields keep original types, no NAPI cast needed
                                _ => format!("{f}: Some({f})"),
                            }
                        }
                    } else {
                        format!("{f}: None")
                    }
                })
                .collect();
            if is_tuple {
                writeln!(
                    out,
                    "            {core_path}::{}({}) => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},",
                    variant.name,
                    destructured.join(", "),
                    field_inits.join(", ")
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "            {core_path}::{} {{ {} }} => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},",
                    variant.name,
                    destructured.join(", "),
                    field_inits.join(", ")
                )
                .ok();
            }
        }
    }

    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}
