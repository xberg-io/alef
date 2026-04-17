use crate::type_map::WasmMapper;
use ahash::AHashSet;
use alef_codegen::builder::{ImplBuilder, RustFileBuilder};
use alef_codegen::generators::{self};
use alef_codegen::naming::to_node_name;
use alef_codegen::shared::{self, constructor_parts};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, FieldDef, FunctionDef, MethodDef, TypeDef, TypeRef};
use std::fmt::Write;
use std::path::PathBuf;

/// Check if a TypeRef references a Named type that is in the exclude set.
/// Used to skip fields whose types were excluded from WASM generation,
/// preventing references to non-existent Js* wrapper types.
fn field_references_excluded_type(ty: &TypeRef, exclude_types: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => exclude_types.iter().any(|e| e == name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_excluded_type(inner, exclude_types),
        TypeRef::Map(k, v) => {
            field_references_excluded_type(k, exclude_types) || field_references_excluded_type(v, exclude_types)
        }
        _ => false,
    }
}

/// Check if a TypeRef is a Copy type that shouldn't be cloned.
/// `enum_names` contains the set of enum type names that derive Copy.
fn is_copy_type(ty: &TypeRef, enum_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) => true, // All primitives are Copy
        TypeRef::Duration => true,     // Duration maps to u64 (secs), which is Copy
        TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Path | TypeRef::Json => false,
        TypeRef::Optional(inner) => is_copy_type(inner, enum_names), // Option<Copy> is Copy
        TypeRef::Vec(_) | TypeRef::Map(_, _) => false,
        TypeRef::Named(n) => enum_names.contains(n), // WASM enums derive Copy
        TypeRef::Unit => true,
    }
}

pub struct WasmBackend;

impl Backend for WasmBackend {
    fn name(&self) -> &str {
        "wasm"
    }

    fn language(&self) -> Language {
        Language::Wasm
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
        let wasm_config = config.wasm.as_ref();
        let exclude_functions = wasm_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
        let exclude_types = wasm_config.map(|c| c.exclude_types.clone()).unwrap_or_default();
        let type_overrides = wasm_config.map(|c| c.type_overrides.clone()).unwrap_or_default();
        let prefix = config.wasm_type_prefix();

        let mapper = WasmMapper::new(type_overrides, prefix.clone());
        let core_import = config.core_import();

        // Note: custom modules and registrations handled below after builder creation

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code)");
        builder.add_import("wasm_bindgen::prelude::*");

        // Import traits needed for trait method dispatch
        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        // Note: HashMap is intentionally not imported here.
        // The WasmMapper always converts Map types to JsValue (wasm-bindgen cannot
        // pass HashMap<K, V> across the JS boundary), so HashMap is never referenced
        // in the generated WASM binding code.

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::Wasm);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && !exclude_types.contains(&t.name))
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
        }

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            if typ.is_opaque {
                builder.add_item(&gen_opaque_struct(typ, &core_import, &prefix));
                builder.add_item(&gen_opaque_struct_methods(
                    typ,
                    &mapper,
                    &opaque_types,
                    &core_import,
                    &prefix,
                ));
            } else {
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // so no separate Default impl is needed.
                builder.add_item(&gen_struct(typ, &mapper, &exclude_types, &prefix));
                builder.add_item(&gen_struct_methods(
                    typ,
                    &mapper,
                    &exclude_types,
                    &core_import,
                    &opaque_types,
                    &api.enums,
                    &prefix,
                ));
            }
        }

        for enum_def in &api.enums {
            if !exclude_types.contains(&enum_def.name) {
                builder.add_item(&gen_enum(enum_def, &prefix));
            }
        }

        for func in &api.functions {
            if !exclude_functions.contains(&func.name) {
                builder.add_item(&gen_function(func, &mapper, &core_import, &opaque_types, &prefix));
            }
        }

        let wasm_conv_config = alef_codegen::conversions::ConversionConfig {
            type_name_prefix: &prefix,
            map_uses_jsvalue: true,
            option_duration_on_defaults: true,
            exclude_types: &exclude_types,
            ..Default::default()
        };
        let convertible = alef_codegen::conversions::convertible_types(api);
        let core_to_binding_convertible = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        // From/Into conversions using shared parameterized generators
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if exclude_types.contains(&typ.name) {
                continue;
            }
            let is_strict = alef_codegen::conversions::can_generate_conversion(typ, &convertible);
            let is_relaxed = alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding_convertible);
            if is_strict {
                // Both directions
                if input_types.contains(&typ.name) {
                    builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                        typ,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &wasm_conv_config,
                ));
            } else if is_relaxed {
                // Only core→binding (sanitized fields prevent binding→core)
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &wasm_conv_config,
                ));
            }
        }
        for e in &api.enums {
            if !exclude_types.contains(&e.name) {
                if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                        e,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
                if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                        e,
                        &core_import,
                        &wasm_conv_config,
                    ));
                }
            }
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_wasm_error_converter(error, &core_import));
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Wasm)?;

        let content = builder.build();

        let output_dir = resolve_output_dir(
            config.output.wasm.as_ref(),
            &config.crate_config.name,
            "crates/{name}-wasm/src/",
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let wasm_config = config.wasm.as_ref();
        let exclude_functions = wasm_config.map(|c| c.exclude_functions.clone()).unwrap_or_default();
        let exclude_types = wasm_config.map(|c| c.exclude_types.clone()).unwrap_or_default();
        let prefix = config.wasm_type_prefix();

        // Collect all exported names from the API
        let mut exports = vec![];

        // Collect all types (exported with prefix from WASM module)
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if !exclude_types.contains(&typ.name) {
                exports.push(format!("{prefix}{}", typ.name));
            }
        }

        // Collect all enums (exported with prefix from WASM module)
        for enum_def in &api.enums {
            if !exclude_types.contains(&enum_def.name) {
                exports.push(format!("{prefix}{}", enum_def.name));
            }
        }

        // Collect all functions (exported from WASM module)
        for func in &api.functions {
            if !exclude_functions.contains(&func.name) {
                // Convert snake_case to camelCase for JavaScript naming
                let js_name = to_node_name(&func.name);
                exports.push(js_name);
            }
        }

        // Collect all error types (exported from WASM module)
        for error in &api.errors {
            exports.push(error.name.clone());
        }

        // Sort for consistent output
        exports.sort();

        // Generate the index.ts re-export file
        let mut lines = vec![
            "// This file is auto-generated by alef. DO NOT EDIT.".to_string(),
            "".to_string(),
        ];

        if !exports.is_empty() {
            lines.push("export {".to_string());
            for (i, name) in exports.iter().enumerate() {
                let comma = if i < exports.len() - 1 { "," } else { "" };
                lines.push(format!("  {}{}", name, comma));
            }
            lines.push("} from './wasm';".to_string());
        }

        let content = lines.join("\n");

        // Output path: packages/wasm/src/index.ts
        let output_path = PathBuf::from("packages/wasm/src/index.ts");

        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "wasm-pack",
            crate_suffix: "-wasm",
            depends_on_ffi: false,
            post_build: vec![],
        })
    }
}

/// Generate an opaque wasm-bindgen struct with inner Arc.
fn gen_opaque_struct(typ: &TypeDef, core_import: &str, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", typ.name);

    // We can't use StructBuilder for private fields, so build manually
    let mut out = String::with_capacity(256);
    writeln!(out, "#[derive(Clone)]").ok();
    writeln!(out, "#[wasm_bindgen]").ok();
    writeln!(out, "pub struct {} {{", js_name).ok();
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    writeln!(out, "    inner: Arc<{}>,", core_path).ok();
    write!(out, "}}").ok();
    out
}

/// Generate wasm-bindgen methods for an opaque struct.
fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    prefix: &str,
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    for method in &typ.methods {
        if method.is_static {
            impl_builder.add_method(&gen_opaque_static_method(
                method,
                mapper,
                &typ.name,
                opaque_types,
                core_import,
                prefix,
            ));
        } else {
            impl_builder.add_method(&gen_opaque_method(method, mapper, &typ.name, opaque_types, prefix));
        }
    }

    impl_builder.build()
}

/// Generate a method for an opaque wasm-bindgen struct that delegates to self.inner.
fn gen_opaque_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{}>", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let async_kw = if method.is_async { "async " } else { "" };

    // Check if the core method takes ownership (Owned receiver).
    // If so, we must clone out of Arc since wasm_bindgen methods take &self.
    let needs_clone = matches!(method.receiver, Some(alef_core::ir::ReceiverKind::Owned));

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let core_call = if needs_clone {
            format!("(*self.inner).clone().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.is_async {
            // WASM async: native async fn becomes a Promise automatically
            let result_wrap = wasm_wrap_return(
                "result",
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
            );
            if method.error_type.is_some() {
                format!(
                    "let result = {core_call}.await\n        \
                     .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                     Ok({result_wrap})"
                )
            } else {
                format!("let result = {core_call}.await;\n    Ok({result_wrap})")
            }
        } else if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok(())")
            } else {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                );
                format!("let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})")
            }
        } else {
            wasm_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
            )
        }
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
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
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub {async_kw}fn {}(&self, {}) -> {} {{\n    \
         {body}\n}}",
        method.name,
        params.join(", "),
        return_annotation
    )
}

/// Generate a static method for an opaque wasm-bindgen struct.
/// Static methods call CoreType::method() instead of self.inner.method().
fn gen_opaque_static_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    prefix: &str,
) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{}>", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let core_call = format!("{core_import}::{type_name}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let wrap = wasm_wrap_return(
                "result",
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
            );
            format!("let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})")
        } else {
            wasm_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
            )
        }
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
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
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
         {body}\n}}",
        method.name,
        params.join(", "),
        return_annotation
    )
}

/// Generate a wasm-bindgen struct definition with private fields.
fn gen_struct(typ: &TypeDef, mapper: &WasmMapper, exclude_types: &[String], prefix: &str) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut out = String::with_capacity(512);
    if typ.has_default {
        writeln!(out, "#[derive(Clone, Default)]").ok();
    } else {
        writeln!(out, "#[derive(Clone)]").ok();
    }
    writeln!(out, "#[wasm_bindgen]").ok();
    writeln!(out, "pub struct {} {{", js_name).ok();

    for field in &typ.fields {
        // Skip cfg-gated fields — they depend on features that may not be enabled
        if field.cfg.is_some() {
            continue;
        }
        // Skip fields whose type references an excluded type (the Js* wrapper won't exist)
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        // On has_default types, non-optional Duration fields are stored as Option<u64> so the
        // wasm constructor can omit them and the From conversion falls back to the core default.
        let force_optional = typ.has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
        let field_type = if field.optional || force_optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        // Fields are private (no pub)
        writeln!(out, "    {}: {},", field.name, field_type).ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate wasm-bindgen methods for a struct.
fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    core_import: &str,
    opaque_types: &AHashSet<String>,
    api_enums: &[EnumDef],
    prefix: &str,
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    if !typ.fields.is_empty() {
        impl_builder.add_method(&gen_new_method(typ, mapper, exclude_types, prefix));
    }

    // Collect enum names for Copy detection in getters.
    // Use unprefixed names since TypeRef::Named stores the original name without Js prefix.
    let enum_names: AHashSet<String> = api_enums.iter().map(|e| e.name.clone()).collect();

    for field in &typ.fields {
        // Skip fields whose type references an excluded type (the Js* wrapper won't exist)
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        impl_builder.add_method(&gen_getter(field, mapper, &enum_names, typ.has_default));
        impl_builder.add_method(&gen_setter(field, mapper, typ.has_default));
    }

    if !exclude_types.contains(&typ.name) {
        for method in &typ.methods {
            // Skip methods whose params or return type reference excluded types
            let refs_excluded = method
                .params
                .iter()
                .any(|p| field_references_excluded_type(&p.ty, exclude_types))
                || field_references_excluded_type(&method.return_type, exclude_types);
            if refs_excluded {
                continue;
            }
            impl_builder.add_method(&gen_method(
                method,
                mapper,
                &typ.name,
                core_import,
                opaque_types,
                prefix,
            ));
        }
    }

    impl_builder.build()
}

/// Generate a constructor method.
fn gen_new_method(typ: &TypeDef, mapper: &WasmMapper, exclude_types: &[String], prefix: &str) -> String {
    let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);

    // Filter out fields whose types reference excluded types
    let filtered_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| !field_references_excluded_type(&f.ty, exclude_types))
        .cloned()
        .collect();

    // For types with has_default, generate optional kwargs-style constructor.
    // Pass option_duration_on_defaults=true so Duration fields are Option<u64> params,
    // matching the Option<u64> field type emitted by gen_struct for has_default types.
    let (param_list, _, assignments) = if typ.has_default {
        alef_codegen::shared::config_constructor_parts_with_options(&filtered_fields, &map_fn, true)
    } else {
        constructor_parts(&filtered_fields, &map_fn)
    };

    // Suppress too_many_arguments when the constructor has >7 params
    let field_count = filtered_fields.iter().filter(|f| f.cfg.is_none()).count();
    let allow_attr = if field_count > 7 {
        "#[allow(clippy::too_many_arguments)]\n"
    } else {
        ""
    };

    format!(
        "{allow_attr}#[wasm_bindgen(constructor)]\npub fn new({param_list}) -> {prefix}{} {{\n    {prefix}{} {{ {assignments} }}\n}}",
        typ.name, typ.name
    )
}

/// Generate a getter method for a field.
fn gen_getter(field: &FieldDef, mapper: &WasmMapper, enum_names: &AHashSet<String>, has_default: bool) -> String {
    // On has_default types, non-optional Duration fields are stored as Option<u64>.
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let field_type = if field.optional || force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let js_name = to_node_name(&field.name);
    let js_name_attr = if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    };

    // Only clone non-Copy types; Copy types are returned directly.
    // For Optional fields, check the inner type — Option<Copy> is Copy.
    let effective_ty = if field.optional {
        // The field type in the struct is Option<mapped_ty>; check if the mapped type is Copy
        &field.ty
    } else {
        &field.ty
    };
    let return_expr = if is_copy_type(effective_ty, enum_names) {
        format!("self.{}", field.name)
    } else {
        format!("self.{}.clone()", field.name)
    };

    format!(
        "#[wasm_bindgen(getter{js_name_attr})]\npub fn {}(&self) -> {} {{\n    {}\n}}",
        field.name, field_type, return_expr
    )
}

/// Generate a setter method for a field.
fn gen_setter(field: &FieldDef, mapper: &WasmMapper, has_default: bool) -> String {
    // On has_default types, non-optional Duration fields are stored as Option<u64>.
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let field_type = if field.optional || force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let js_name = to_node_name(&field.name);
    let js_name_attr = if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    };

    format!(
        "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{}(&mut self, value: {}) {{\n    self.{} = value;\n}}",
        field.name, field_type, field.name
    )
}

/// Generate a method binding for a struct method.
fn gen_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{}>", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let mut attrs = String::new();
    // Per-item clippy suppression: too_many_arguments when >7 params (including &self for instance methods)
    let effective_param_count = if method.is_static {
        method.params.len()
    } else {
        method.params.len() + 1
    };
    if effective_param_count > 7 {
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

    if method.is_async {
        let let_bindings = if alef_codegen::generators::has_named_params(&method.params, opaque_types) {
            alef_codegen::generators::gen_named_let_bindings_pub(&method.params, opaque_types, core_import)
        } else {
            String::new()
        };
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&method.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
        };
        let core_call = format!(
            "{core_import}::{type_name}::from(self.clone()).{method_name}({call_args})",
            method_name = method.name
        );
        let body = if method.error_type.is_some() {
            format!(
                "{let_bindings}let result = {core_call}.await\n        \
                 .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                 Ok({}::from(result))",
                return_type
            )
        } else {
            format!(
                "{let_bindings}let result = {core_call}.await;\n    \
                 Ok({}::from(result))",
                return_type
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub async fn {}(&self, {}) -> {} {{\n    \
             {body}\n}}",
            method.name,
            params.join(", "),
            return_annotation
        )
    } else if method.is_static {
        let body = if can_delegate {
            let let_bindings = if alef_codegen::generators::has_named_params(&method.params, opaque_types) {
                alef_codegen::generators::gen_named_let_bindings_pub(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let call_args = if let_bindings.is_empty() {
                generators::gen_call_args(&method.params, opaque_types)
            } else {
                generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
            };
            let core_call = format!("{core_import}::{type_name}::{}({call_args})", method.name);
            if method.error_type.is_some() {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                );
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
                )
            } else {
                format!(
                    "{let_bindings}{}",
                    wasm_wrap_return(
                        &core_call,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                    )
                )
            }
        } else {
            gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            method.name,
            params.join(", "),
            return_annotation
        )
    } else {
        let body = if can_delegate {
            let let_bindings = if alef_codegen::generators::has_named_params(&method.params, opaque_types) {
                alef_codegen::generators::gen_named_let_bindings_pub(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let call_args = if let_bindings.is_empty() {
                generators::gen_call_args(&method.params, opaque_types)
            } else {
                generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
            };
            let core_call = format!(
                "{core_import}::{type_name}::from(self.clone()).{method_name}({call_args})",
                method_name = method.name
            );
            if method.error_type.is_some() {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                );
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
                )
            } else {
                format!(
                    "{let_bindings}{}",
                    wasm_wrap_return(
                        &core_call,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                    )
                )
            }
        } else {
            gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}(&self, {}) -> {} {{\n    \
             {body}\n}}",
            method.name,
            params.join(", "),
            return_annotation
        )
    }
}

/// Generate a wasm-bindgen enum definition.
fn gen_enum(enum_def: &EnumDef, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", enum_def.name);
    let mut lines = vec![
        "#[wasm_bindgen]".to_string(),
        "#[derive(Clone, Copy, PartialEq, Eq)]".to_string(),
        format!("pub enum {} {{", js_name),
    ];

    for (idx, variant) in enum_def.variants.iter().enumerate() {
        lines.push(format!("    {} = {},", variant.name, idx));
    }

    lines.push("}".to_string());

    // Default impl (first variant) for use in config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {} {{", js_name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", first.name));
        lines.push("}".to_string());
    }

    lines.join("\n")
}

/// Generate a free function binding.
fn gen_function(
    func: &FunctionDef,
    mapper: &WasmMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{}>", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types);

    let mut attrs = String::new();
    // Per-item clippy suppression: too_many_arguments when >7 params
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    if func.is_async {
        let let_bindings = if alef_codegen::generators::has_named_params(&func.params, opaque_types) {
            alef_codegen::generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        // Build the return expression: handle Vec<Named> with collect pattern (turbofish),
        // plain Named with From::from, and everything else as passthrough.
        let return_expr = match &func.return_type {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    format!(
                        "result.into_iter().map(|v| {} {{ inner: Arc::new(v) }}).collect::<Vec<_>>()",
                        mapper.map_type(inner)
                    )
                }
                TypeRef::Named(_) => {
                    let inner_mapped = mapper.map_type(inner);
                    format!("result.into_iter().map({inner_mapped}::from).collect::<Vec<_>>()")
                }
                _ => "result".to_string(),
            },
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let prefixed = mapper.map_type(&func.return_type);
                format!("{prefixed} {{ inner: Arc::new(result) }}")
            }
            TypeRef::Named(_) => {
                format!("{return_type}::from(result)")
            }
            TypeRef::Unit => "result".to_string(),
            _ => "result".to_string(),
        };
        let body = if func.error_type.is_some() {
            format!(
                "{let_bindings}let result = {core_call}.await\n        \
                 .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                 Ok({return_expr})"
            )
        } else {
            format!(
                "{let_bindings}let result = {core_call}.await;\n    \
                 {return_expr}"
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub async fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        )
    } else if can_delegate {
        let let_bindings = if alef_codegen::generators::has_named_params(&func.params, opaque_types) {
            alef_codegen::generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let body = if func.error_type.is_some() {
            let wrap = wasm_wrap_return_fn("result", &func.return_type, opaque_types, func.returns_ref, prefix);
            format!(
                "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        } else {
            format!(
                "{let_bindings}{}",
                wasm_wrap_return_fn(&core_call, &func.return_type, opaque_types, func.returns_ref, prefix)
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        )
    } else {
        let body = gen_wasm_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some());
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        )
    }
}

/// Generate a type-appropriate unimplemented body for WASM (no todo!()).
fn gen_wasm_unimplemented_body(return_type: &TypeRef, fn_name: &str, has_error: bool) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(JsValue::from_str(\"{err_msg}\"))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!("panic!(\"alef: {fn_name} not auto-delegatable\")"),
        }
    }
}

/// WASM-specific return wrapping for opaque methods (adds prefix for opaque Named returns).
#[allow(clippy::too_many_arguments)]
fn wasm_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
) -> String {
    match return_type {
        // Self-returning opaque method
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        // Other opaque Named return: needs prefix
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        // Optional<opaque>: wrap with prefix
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        // Vec<opaque>: wrap with prefix
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
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

/// WASM-specific return wrapping for free functions (no type_name context, adds prefix).
fn wasm_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    prefix: &str,
) -> String {
    match return_type {
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
