use crate::type_map::MagnusMapper;
use ahash::AHashSet;
use alef_codegen::builder::{ImplBuilder, RustFileBuilder, StructBuilder};
use alef_codegen::generators;
use alef_codegen::shared::{self, constructor_parts, function_params};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, FieldDef, FunctionDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use std::fmt::Write;
use std::path::PathBuf;

pub struct MagnusBackend;

/// Names that conflict with magnus imports or generated code.
/// `Error` conflicts with `magnus::Error`, `init` conflicts with `#[magnus::init]`.
const MAGNUS_RESERVED_ENUM_NAMES: &[&str] = &["Error"];
const MAGNUS_RESERVED_FN_NAMES: &[&str] = &["init"];

fn is_reserved_enum(name: &str) -> bool {
    MAGNUS_RESERVED_ENUM_NAMES.contains(&name)
}

fn is_reserved_fn(name: &str) -> bool {
    MAGNUS_RESERVED_FN_NAMES.contains(&name)
}

impl Backend for MagnusBackend {
    fn name(&self) -> &str {
        "magnus"
    }

    fn language(&self) -> Language {
        Language::Ruby
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
        let mapper = MagnusMapper;
        let core_import = config.core_import();

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code)");
        builder.add_import(
            "magnus::{function, method, prelude::*, Error, Ruby, IntoValueFromNative, try_convert::TryConvertOwned}",
        );

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

        // Compute module name early so it can be used for class paths in #[magnus::wrap]
        let module_name = get_module_name(&api.crate_name);

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::Ruby);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
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

        // Check if any data enum exists (needs json_to_ruby helper)
        let has_data_enum = api
            .enums
            .iter()
            .any(|e| e.variants.iter().any(|v| !v.fields.is_empty()));
        if has_data_enum {
            // Add json_to_ruby helper for converting serde_json::Value to Magnus values
            builder.add_item(
                "fn json_to_ruby(handle: &Ruby, val: serde_json::Value) -> magnus::Value {\n\
                 \x20   use magnus::IntoValue;\n\
                 \x20   match val {\n\
                 \x20       serde_json::Value::Null => handle.qnil().into_value_with(handle),\n\
                 \x20       serde_json::Value::Bool(b) => b.into_value_with(handle),\n\
                 \x20       serde_json::Value::Number(n) => {\n\
                 \x20           if let Some(i) = n.as_i64() { i.into_value_with(handle) }\n\
                 \x20           else if let Some(f) = n.as_f64() { f.into_value_with(handle) }\n\
                 \x20           else { handle.qnil().into_value_with(handle) }\n\
                 \x20       }\n\
                 \x20       serde_json::Value::String(s) => s.into_value_with(handle),\n\
                 \x20       serde_json::Value::Array(arr) => {\n\
                 \x20           let ruby_arr = handle.ary_new_capa(arr.len());\n\
                 \x20           for item in arr { let _ = ruby_arr.push(json_to_ruby(handle, item)); }\n\
                 \x20           ruby_arr.into_value_with(handle)\n\
                 \x20       }\n\
                 \x20       serde_json::Value::Object(map) => {\n\
                 \x20           let hash = handle.hash_new();\n\
                 \x20           for (k, v) in map {\n\
                 \x20               let key = handle.to_symbol(&k);\n\
                 \x20               let val = json_to_ruby(handle, v);\n\
                 \x20               let _ = hash.aset(key, val);\n\
                 \x20           }\n\
                 \x20           hash.into_value_with(handle)\n\
                 \x20       }\n\
                 \x20   }\n\
                 }",
            );
        }

        // Build set of types with has_default for validation
        let default_types: std::collections::HashSet<&str> = api
            .types
            .iter()
            .filter(|t| t.has_default)
            .map(|t| t.name.as_str())
            .collect();

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                builder.add_item(&gen_opaque_struct(typ, &core_import, &module_name));
                builder.add_item(&gen_opaque_struct_methods(typ, &mapper, &opaque_types));
            } else {
                builder.add_item(&gen_struct(typ, &mapper, &module_name));
                // Only generate Default impl if all fields can be safely defaulted
                if typ.has_default && alef_codegen::generators::can_generate_default_impl(typ, &default_types) {
                    builder.add_item(&alef_codegen::generators::gen_struct_default_impl(typ, ""));
                }
                builder.add_item(&gen_struct_methods(typ, &mapper, &opaque_types, &core_import));
            }
        }

        for enum_def in &api.enums {
            if !is_reserved_enum(&enum_def.name) {
                builder.add_item(&gen_enum(enum_def));
            }
        }

        for func in &api.functions {
            if !is_reserved_fn(&func.name) {
                builder.add_item(&gen_function(func, &mapper, &opaque_types, &core_import));
                if func.is_async {
                    builder.add_item(&gen_async_function(func, &mapper, &opaque_types, &core_import));
                }
            }
        }

        // Magnus binding types are separate structs from core types and need From impls
        // for delegation. Generate both directions where possible.
        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                continue;
            }
            let is_strict = alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core);
            let is_relaxed = alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding);
            if is_strict && input_types.contains(&typ.name) {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core(typ, &core_import));
            }
            if is_relaxed {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding(
                    typ,
                    &core_import,
                    &opaque_types,
                ));
            }
        }
        // Magnus generates data enums with fields, so enable binding_enums_have_data
        let magnus_conv_config = alef_codegen::conversions::ConversionConfig {
            binding_enums_have_data: true,
            ..Default::default()
        };
        for e in &api.enums {
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(
                    e,
                    &core_import,
                    &magnus_conv_config,
                ));
            }
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(
                    e,
                    &core_import,
                    &magnus_conv_config,
                ));
            }
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_magnus_error_converter(
                error,
                &core_import,
            ));
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Ruby)?;

        builder.add_item(&gen_module_init(&module_name, api, config));

        let content = builder.build();

        let output_dir = resolve_output_dir(
            config.output.ruby.as_ref(),
            &config.crate_config.name,
            "packages/ruby/ext/{name}_rb/src/",
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_type_stubs(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let stubs_config = match config.ruby.as_ref().and_then(|c| c.stubs.as_ref()) {
            Some(s) => s,
            None => return Ok(vec![]),
        };

        let content = crate::gen_stubs::gen_stubs(api);

        let stubs_path = resolve_output_dir(
            Some(&stubs_config.output),
            &config.crate_config.name,
            stubs_config.output.to_string_lossy().as_ref(),
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&stubs_path).join("types.rbs"),
            content,
            generated_header: true,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let gem_name = config.ruby_gem_name();
        let module_name = get_module_name(&gem_name);

        // Generate the main Ruby wrapper module file
        let mut content = String::from("# This file is auto-generated by alef. DO NOT EDIT.\n");
        content.push_str("# frozen_string_literal: true\n\n");
        content.push_str(&format!("require_relative '{gem_name}/version'\n"));
        content.push_str(&format!("require_relative '{gem_name}/native'\n\n"));
        content.push_str(&format!("module {module_name}\n"));
        content.push_str("  # Re-export all types and functions from native extension\n");
        content.push_str("end\n");

        // Generate the version file
        let version = if api.version.is_empty() {
            "0.0.0".to_string()
        } else {
            api.version.clone()
        };
        let mut version_content = String::from("# This file is auto-generated by alef. DO NOT EDIT.\n");
        version_content.push_str("# frozen_string_literal: true\n\n");
        version_content.push_str(&format!("module {module_name}\n"));
        version_content.push_str(&format!("  VERSION = \"{version}\"\n"));
        version_content.push_str("end\n");

        let output_dir = resolve_output_dir(
            config.output.ruby.as_ref(),
            &config.crate_config.name,
            "packages/ruby/lib/",
        );

        Ok(vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join(format!("{gem_name}.rb")),
                content,
                generated_header: false,
            },
            GeneratedFile {
                path: PathBuf::from(&output_dir).join(format!("{gem_name}/version.rb")),
                content: version_content,
                generated_header: false,
            },
        ])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-rb",
            depends_on_ffi: false,
            post_build: vec![],
        })
    }
}

/// Convert crate name to PascalCase module name.
fn get_module_name(crate_name: &str) -> String {
    crate_name
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

/// Generate an opaque Magnus-wrapped struct with inner Arc.
fn gen_opaque_struct(typ: &TypeDef, core_import: &str, module_name: &str) -> String {
    let class_path = format!("{}::{}", module_name, typ.name);

    let mut out = String::with_capacity(256);
    writeln!(out, "#[derive(Clone)]").ok();
    writeln!(out, r#"#[magnus::wrap(class = "{}")]"#, class_path).ok();
    writeln!(out, "pub struct {} {{", typ.name).ok();
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    writeln!(out, "    inner: Arc<{}>,", core_path).ok();
    writeln!(out, "}}").ok();
    let name = &typ.name;
    writeln!(out).ok();
    // SAFETY: #[magnus::wrap] already provides IntoValue. This marker trait
    // enables use in Vec<T> returns from Magnus function!/method! macros.
    writeln!(out, "unsafe impl IntoValueFromNative for {name} {{}}").ok();
    // Magnus only provides TryConvert for &T (references) on TypedData types.
    // We need TryConvert for owned T so wrapped types can be used as function parameters.
    writeln!(out, "\nimpl magnus::TryConvert for {name} {{").ok();
    writeln!(
        out,
        "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
    )
    .ok();
    writeln!(out, "        let r: &{name} = magnus::TryConvert::try_convert(val)?;").ok();
    writeln!(out, "        Ok(r.clone())").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    // SAFETY: TryConvert produces an owned value via Clone, satisfying owned conversion.
    write!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();
    out
}

/// Generate Magnus methods for an opaque struct (delegates to self.inner).
fn gen_opaque_struct_methods(typ: &TypeDef, mapper: &MagnusMapper, opaque_types: &AHashSet<String>) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);

    for method in &typ.methods {
        if !method.is_static {
            if method.is_async {
                impl_builder.add_method(&gen_opaque_async_instance_method(
                    method,
                    mapper,
                    &typ.name,
                    opaque_types,
                ));
            } else {
                impl_builder.add_method(&gen_opaque_instance_method(method, mapper, &typ.name, opaque_types));
            }
        }
    }

    impl_builder.build()
}

/// Generate an opaque sync instance method for Magnus (delegates to self.inner).
fn gen_opaque_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        // For owned-receiver (consuming) methods, clone the Arc's inner value before calling,
        // since we cannot move out of an Arc from a &self method.
        let is_owned_receiver = matches!(method.receiver, Some(ReceiverKind::Owned));
        let inner_access = if is_owned_receiver {
            "self.inner.as_ref().clone()".to_string()
        } else {
            "self.inner".to_string()
        };
        let core_call = format!("{inner_access}.{}({})", method.name, call_args);
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!(
                    "{core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok(())"
                )
            } else {
                let wrap = generators::wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                );
                format!(
                    "let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok({wrap})"
                )
            }
        } else {
            generators::wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
            )
        }
    } else {
        gen_magnus_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n    "
    } else {
        ""
    };
    format!(
        "{trait_allow}fn {}(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    }}",
        method.name
    )
}

/// Generate an opaque async instance method for Magnus (block on runtime, delegates to self.inner).
fn gen_opaque_async_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let inner_clone = "let inner = self.inner.clone();\n        ";
        let core_call = format!("inner.{}({})", method.name, call_args);
        let result_wrap = generators::wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            true,
            method.returns_ref,
            method.returns_cow,
        );
        if method.error_type.is_some() {
            format!(
                "{inner_clone}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 Ok({result_wrap})"
            )
        } else {
            format!(
                "{inner_clone}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ {core_call}.await }});\n        \
                 {result_wrap}"
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            method.error_type.is_some(),
        )
    };
    format!(
        "fn {}_async(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    \
         }}",
        method.name
    )
}

/// Generate a Magnus-wrapped struct definition using the shared TypeMapper.
fn gen_struct(typ: &TypeDef, mapper: &MagnusMapper, module_name: &str) -> String {
    let class_path = format!("{}::{}", module_name, typ.name);

    let mut struct_builder = StructBuilder::new(&typ.name);
    struct_builder.add_attr(&format!(r#"magnus::wrap(class = "{}")"#, class_path));

    // Magnus requires Clone for TryConvert on owned types
    struct_builder.add_derive("Clone");
    struct_builder.add_derive("Debug");
    // serde derives allow accepting Ruby hashes via serde_magnus and serializing back
    struct_builder.add_derive("serde::Serialize");
    struct_builder.add_derive("serde::Deserialize");
    if typ.has_default {
        struct_builder.add_attr("serde(default)");
    }

    for field in &typ.fields {
        let field_type = if field.optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        struct_builder.add_field(&field.name, &field_type, vec![]);
    }

    let mut out = struct_builder.build();
    let name = &typ.name;
    // SAFETY: #[magnus::wrap] already provides IntoValue. This marker trait
    // enables use in Vec<T> returns from Magnus function!/method! macros.
    writeln!(out, "\n\nunsafe impl IntoValueFromNative for {name} {{}}").ok();
    // Magnus only provides TryConvert for &T (references) on TypedData types.
    // We need TryConvert for owned T so wrapped types can be used as function parameters.
    writeln!(out, "\nimpl magnus::TryConvert for {name} {{").ok();
    writeln!(
        out,
        "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
    )
    .ok();
    writeln!(out, "        let r: &{name} = magnus::TryConvert::try_convert(val)?;").ok();
    writeln!(out, "        Ok(r.clone())").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    // SAFETY: TryConvert produces an owned value via Clone, satisfying owned conversion.
    write!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();
    out
}

/// Generate Magnus methods for a struct.
fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);

    if !typ.fields.is_empty() {
        let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);

        // Generate config builder if type has Default, otherwise generate normal constructor
        if typ.has_default {
            let config_method = alef_codegen::config_gen::gen_magnus_kwargs_constructor(typ, &map_fn);
            impl_builder.add_method(&config_method);
        } else {
            let (param_list, _, assignments) = constructor_parts(&typ.fields, &map_fn);
            let new_method = format!("fn new({param_list}) -> Self {{\n        Self {{ {assignments} }}\n    }}");
            impl_builder.add_method(&new_method);
        }
    }

    for field in &typ.fields {
        impl_builder.add_method(&gen_field_accessor(field, mapper));
    }

    for method in &typ.methods {
        if !method.is_static {
            if method.is_async {
                impl_builder.add_method(&gen_async_instance_method(
                    method,
                    mapper,
                    typ,
                    opaque_types,
                    core_import,
                ));
            } else {
                impl_builder.add_method(&gen_instance_method(method, mapper, typ, opaque_types, core_import));
            }
        }
    }

    impl_builder.build()
}

/// Generate a field accessor method.
fn gen_field_accessor(field: &FieldDef, mapper: &MagnusMapper) -> String {
    let return_type = if field.optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let body = if is_primitive_copy(&field.ty) {
        format!("self.{}", field.name)
    } else {
        format!("self.{}.clone()", field.name)
    };

    format!(
        "fn {}(&self) -> {} {{\n        {}\n    }}",
        field.name, return_type, body
    )
}

/// Check if a type is a Copy type (primitives and unit).
fn is_primitive_copy(ty: &alef_core::ir::TypeRef) -> bool {
    matches!(ty, alef_core::ir::TypeRef::Primitive(_) | alef_core::ir::TypeRef::Unit)
}

/// Generate an instance method binding for a non-opaque struct.
fn gen_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    typ: &TypeDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let field_conversions = generators::gen_lossy_binding_to_core_fields(typ, core_import);
        let core_call = format!("core_self.{}({})", method.name, call_args);
        let result_wrap = match &method.return_type {
            TypeRef::Named(_) | TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Path => {
                ".into()".to_string()
            }
            _ => String::new(),
        };
        if method.error_type.is_some() {
            format!(
                "{field_conversions}let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok(result{result_wrap})"
            )
        } else {
            format!("{field_conversions}{core_call}{result_wrap}")
        }
    } else {
        gen_magnus_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };
    format!(
        "fn {}(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    }}",
        method.name
    )
}

/// Generate an async instance method binding for Magnus (block on runtime).
fn gen_async_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    typ: &TypeDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let field_conversions = generators::gen_lossy_binding_to_core_fields(typ, core_import);
        let _core_call = format!("core_self.{}({})", method.name, call_args);
        let result_wrap = match &method.return_type {
            TypeRef::Named(_) | TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Path => {
                ".into()".to_string()
            }
            _ => String::new(),
        };
        if method.error_type.is_some() {
            format!(
                "{field_conversions}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ core_self.{name}({call_args}).await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 Ok(result{result_wrap})",
                name = method.name
            )
        } else {
            format!(
                "{field_conversions}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ core_self.{name}({call_args}).await }});\n        \
                 result{result_wrap}",
                name = method.name
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            method.error_type.is_some(),
        )
    };
    format!(
        "fn {}_async(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    \
         }}",
        method.name
    )
}

/// Convert a PascalCase name to snake_case for Ruby symbol mapping.
fn pascal_to_snake(name: &str) -> String {
    let mut result = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_lowercase().next().unwrap_or(ch));
    }
    result
}

/// Generate a Magnus enum definition with IntoValue and TryConvert impls.
/// Unit-variant enums are represented as Ruby Symbols for ergonomic Ruby usage.
/// Map a field type to a Rust type suitable for serde deserialization in data enums.
fn field_type_for_serde(field: &FieldDef) -> String {
    use alef_core::ir::PrimitiveType;
    let base = match &field.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "u8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "u16".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "u32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "u64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "usize".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "i8".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "i16".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "i32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "i64".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "isize".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "f32".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "f64".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => n.clone(),
        _ => "String".to_string(),
    };
    if field.optional {
        format!("Option<{base}>")
    } else {
        base
    }
}

fn gen_enum(enum_def: &EnumDef) -> String {
    let name = &enum_def.name;
    let mut out = String::with_capacity(512);

    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Enum definition
    if has_data {
        // Data enum: can't be Copy, include serde tag attribute
        writeln!(out, "#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]").ok();
        if let Some(tag) = &enum_def.serde_tag {
            writeln!(out, r#"#[serde(tag = "{tag}")]"#).ok();
        }
    } else {
        writeln!(
            out,
            "#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]"
        )
        .ok();
    }
    writeln!(out, "pub enum {name} {{").ok();
    for variant in &enum_def.variants {
        if let Some(rename) = &variant.serde_rename {
            writeln!(out, r#"    #[serde(rename = "{rename}")]"#).ok();
        }
        if variant.fields.is_empty() {
            writeln!(out, "    {},", variant.name).ok();
        } else {
            // Data variant with named fields
            let fields: Vec<String> = variant
                .fields
                .iter()
                .map(|f| format!("{}: {}", f.name, field_type_for_serde(f)))
                .collect();
            writeln!(out, "    {} {{ {} }},", variant.name, fields.join(", ")).ok();
        }
    }
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Default impl for config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        if has_data && !first.fields.is_empty() {
            // Data variant default: use Default::default() for each field
            let field_defaults: Vec<String> = first
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            writeln!(out, "impl Default for {name} {{").ok();
            writeln!(
                out,
                "    fn default() -> Self {{ Self::{} {{ {} }} }}",
                first.name,
                field_defaults.join(", ")
            )
            .ok();
            writeln!(out, "}}").ok();
        } else {
            writeln!(out, "impl Default for {name} {{").ok();
            writeln!(out, "    fn default() -> Self {{ Self::{} }}", first.name).ok();
            writeln!(out, "}}").ok();
        }
        writeln!(out).ok();
    }

    // For data enums, implement IntoValue via serde_json serialization
    // and TryConvert via serde_json deserialization.
    // Uses a json_to_ruby helper to convert serde_json::Value to Magnus values.
    if has_data {
        writeln!(out, "impl magnus::IntoValue for {name} {{").ok();
        writeln!(out, "    fn into_value_with(self, handle: &Ruby) -> magnus::Value {{").ok();
        writeln!(out, "        match serde_json::to_value(&self) {{").ok();
        writeln!(out, "            Ok(v) => json_to_ruby(handle, v),").ok();
        writeln!(out, "            Err(_) => handle.qnil().into_value_with(handle),").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(out, "impl magnus::TryConvert for {name} {{").ok();
        writeln!(
            out,
            "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
        )
        .ok();
        writeln!(out, "        let s: String = magnus::TryConvert::try_convert(val)?;").ok();
        writeln!(
            out,
            "        serde_json::from_str(&s).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_type_error(), e.to_string()))"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(out, "unsafe impl IntoValueFromNative for {name} {{}}").ok();
        writeln!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();
        return out;
    }

    // IntoValue: convert enum variant to Ruby Symbol
    writeln!(out, "impl magnus::IntoValue for {name} {{").ok();
    writeln!(out, "    fn into_value_with(self, handle: &Ruby) -> magnus::Value {{").ok();
    writeln!(out, "        let sym = match self {{").ok();
    for variant in &enum_def.variants {
        let snake = pascal_to_snake(&variant.name);
        writeln!(out, "            {name}::{} => \"{snake}\",", variant.name).ok();
    }
    writeln!(out, "        }};").ok();
    writeln!(out, "        handle.to_symbol(sym).into_value_with(handle)").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // TryConvert: convert Ruby Symbol/String to enum variant
    writeln!(out, "impl magnus::TryConvert for {name} {{").ok();
    writeln!(
        out,
        "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
    )
    .ok();
    writeln!(out, "        let s: String = magnus::TryConvert::try_convert(val)?;").ok();
    writeln!(out, "        match s.as_str() {{").ok();
    for variant in &enum_def.variants {
        let snake = pascal_to_snake(&variant.name);
        writeln!(out, "            \"{snake}\" => Ok({name}::{}),", variant.name).ok();
    }
    writeln!(out, "            other => Err(magnus::Error::new(").ok();
    writeln!(
        out,
        "                unsafe {{ Ruby::get_unchecked() }}.exception_arg_error(),"
    )
    .ok();
    writeln!(out, "                format!(\"invalid {name} value: {{other}}\"),").ok();
    writeln!(out, "            )),").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    // SAFETY: IntoValue is implemented above. This marker trait enables use
    // in Vec<T> returns from Magnus function!/method! macros.
    writeln!(out, "unsafe impl IntoValueFromNative for {name} {{}}").ok();
    // SAFETY: TryConvert produces an owned value, satisfying owned conversion.
    write!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();

    out
}

/// Generate a free function binding.
fn gen_function(
    func: &FunctionDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    // For non-opaque Named params, use magnus::Value and serde_magnus deserialization
    let params = function_params(&func.params, &|ty| {
        if let TypeRef::Named(name) = ty {
            if !opaque_types.contains(name.as_str()) {
                // Accept JSON string for non-opaque Named types (Ruby hashes → JSON → deserialize)
                return "String".to_string();
            }
        }
        mapper.map_type(ty)
    });
    let return_type = mapper.map_type(&func.return_type);
    // Async functions always return Result because Runtime::new() can fail, even when the core
    // function itself has no error type.
    let has_error = func.error_type.is_some() || func.is_async;
    let return_annotation = mapper.wrap_return(&return_type, has_error);

    // Generate serde_magnus deserialization preamble for non-opaque Named params
    let mut deser_lines = Vec::new();
    for p in &func.params {
        if let TypeRef::Named(name) = &p.ty {
            if !opaque_types.contains(name.as_str()) {
                let binding_ty = &p.name;
                if p.optional {
                    deser_lines.push(format!(
                        "let {binding_ty}: Option<{name}> = {binding_ty}.as_deref().filter(|s| *s != \"nil\").map(|s| {{ let core: {core_import}::{name} = serde_json::from_str(s).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_type_error(), e.to_string()))?; Ok::<_, magnus::Error>(core.into()) }}).transpose()?;"
                    ));
                } else {
                    deser_lines.push(format!(
                        "let {binding_ty}: {name} = {{ let core: {core_import}::{name} = serde_json::from_str(&{binding_ty}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_type_error(), e.to_string()))?; core.into() }};"
                    ));
                }
            }
        }
    }
    let deser_preamble = if deser_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", deser_lines.join("\n    "))
    };

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&func.params, opaque_types);
        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({call_args})");
        if func.is_async {
            // Async core function: wrap in tokio runtime block_on.
            // Runtime::new() can fail, so always use map_err and return Ok(...).
            let wrap = generators::wrap_return(
                "result",
                &func.return_type,
                "",
                opaque_types,
                false,
                func.returns_ref,
                false,
            );
            if func.error_type.is_some() {
                format!(
                    "let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
                     let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
                     Ok({wrap})"
                )
            } else {
                format!(
                    "let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
                     let result = rt.block_on(async {{ {core_call}.await }});\n    \
                     Ok({wrap})"
                )
            }
        } else if func.error_type.is_some() {
            let wrap = generators::wrap_return(
                "result",
                &func.return_type,
                "",
                opaque_types,
                false,
                func.returns_ref,
                false,
            );
            format!(
                "let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    Ok({wrap})"
            )
        } else {
            generators::wrap_return(
                &core_call,
                &func.return_type,
                "",
                opaque_types,
                false,
                func.returns_ref,
                false,
            )
        }
    } else {
        gen_magnus_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some())
    };
    format!(
        "fn {}({params}) -> {return_annotation} {{\n    \
         {deser_preamble}{body}\n}}",
        func.name
    )
}

/// Generate an async free function binding for Magnus (block on runtime).
fn gen_async_function(
    func: &FunctionDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    // For non-opaque Named params, use magnus::Value and serde_magnus deserialization
    let params = function_params(&func.params, &|ty| {
        if let TypeRef::Named(name) = ty {
            if !opaque_types.contains(name.as_str()) {
                // Accept JSON string for non-opaque Named types (Ruby hashes → JSON → deserialize)
                return "String".to_string();
            }
        }
        mapper.map_type(ty)
    });
    let return_type = mapper.map_type(&func.return_type);
    // Async functions always return Result because Runtime::new() can fail, even when the core
    // function itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    // Generate serde_magnus deserialization preamble for non-opaque Named params
    let mut deser_lines = Vec::new();
    for p in &func.params {
        if let TypeRef::Named(name) = &p.ty {
            if !opaque_types.contains(name.as_str()) {
                let binding_ty = &p.name;
                if p.optional {
                    deser_lines.push(format!(
                        "let {binding_ty}: Option<{name}> = {binding_ty}.as_deref().filter(|s| *s != \"nil\").map(|s| {{ let core: {core_import}::{name} = serde_json::from_str(s).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_type_error(), e.to_string()))?; Ok::<_, magnus::Error>(core.into()) }}).transpose()?;"
                    ));
                } else {
                    deser_lines.push(format!(
                        "let {binding_ty}: {name} = {{ let core: {core_import}::{name} = serde_json::from_str(&{binding_ty}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_type_error(), e.to_string()))?; core.into() }};"
                    ));
                }
            }
        }
    }
    let deser_preamble = if deser_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", deser_lines.join("\n    "))
    };

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&func.params, opaque_types);
        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let result_wrap = generators::wrap_return(
            "result",
            &func.return_type,
            "",
            opaque_types,
            false,
            func.returns_ref,
            false,
        );
        if func.error_type.is_some() {
            format!(
                "let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
                 Ok({result_wrap})"
            )
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            format!(
                "let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 Ok({result_wrap})"
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &func.return_type,
            &format!("{}_async", func.name),
            func.error_type.is_some(),
        )
    };
    format!(
        "fn {}_async({params}) -> {return_annotation} {{\n    \
         {deser_preamble}{body}\n\
         }}",
        func.name
    )
}

/// Generate a type-appropriate unimplemented body for Magnus (no todo!()).
fn gen_magnus_unimplemented_body(return_type: &alef_core::ir::TypeRef, fn_name: &str, has_error: bool) -> String {
    use alef_core::ir::TypeRef;
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), \"{err_msg}\"))")
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

/// Generate the module initialization function.
fn gen_module_init(module_name: &str, api: &ApiSurface, config: &AlefConfig) -> String {
    let mut lines = vec![
        "#[magnus::init]".to_string(),
        "fn init(ruby: &Ruby) -> Result<(), Error> {".to_string(),
        format!(r#"    let module = ruby.define_module("{}")?;"#, module_name),
        "".to_string(),
    ];

    // Custom registrations (before generated ones)
    if let Some(reg) = config.custom_registrations.for_language(Language::Ruby) {
        for class in &reg.classes {
            lines.push(format!(
                r#"    let class = module.define_class("{class}", ruby.class_object())?;"#
            ));
        }
        for func in &reg.functions {
            lines.push(format!(
                r#"    module.define_module_function("{func}", function!({func}, 0))?;"#
            ));
        }
        lines.push("".to_string());
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        lines.push(format!(
            r#"    let class = module.define_class("{}", ruby.class_object())?;"#,
            typ.name
        ));

        if !typ.is_opaque && !typ.fields.is_empty() {
            // Magnus function! macro only supports arity -2..=15.
            // Types with >15 fields use a hash-based constructor (RHash param = arity 1).
            let arg_count = typ.fields.len();
            let arity = if arg_count > 15 { 1 } else { arg_count };
            lines.push(format!(
                r#"    class.define_singleton_method("new", function!({name}::new, {count}))?;"#,
                name = typ.name,
                count = arity
            ));
        }

        if !typ.is_opaque {
            for field in &typ.fields {
                lines.push(format!(
                    r#"    class.define_method("{name}", method!({typ_name}::{name}, 0))?;"#,
                    name = field.name,
                    typ_name = typ.name
                ));
            }
        }

        for method in &typ.methods {
            if !method.is_static {
                let method_name = if method.is_async {
                    format!("{}_async", method.name)
                } else {
                    method.name.clone()
                };
                let param_count = method.params.len();
                lines.push(format!(
                    r#"    class.define_method("{name}", method!({typ_name}::{fn_name}, {count}))?;"#,
                    name = method_name,
                    typ_name = typ.name,
                    fn_name = method_name,
                    count = param_count
                ));
            }
        }

        lines.push("".to_string());
    }

    for func in &api.functions {
        if is_reserved_fn(&func.name) {
            continue;
        }
        let param_count = func.params.len();
        if func.is_async {
            // Register both sync (blocking) and async variants
            lines.push(format!(
                r#"    module.define_module_function("{name}", function!({name}, {count}))?;"#,
                name = func.name,
                count = param_count
            ));
            lines.push(format!(
                r#"    module.define_module_function("{name}_async", function!({name}_async, {count}))?;"#,
                name = func.name,
                count = param_count
            ));
        } else {
            lines.push(format!(
                r#"    module.define_module_function("{name}", function!({name}, {count}))?;"#,
                name = func.name,
                count = param_count
            ));
        }
    }

    lines.push("".to_string());
    lines.push("    Ok(())".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}
