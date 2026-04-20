use crate::type_map::RustlerMapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_codegen::shared;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::{ApiSurface, EnumDef, FieldDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

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

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let mapper = RustlerMapper;
        let core_import = config.core_import();

        let elixir_config = config.elixir.as_ref();
        let exclude_functions: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: AHashSet<&str> = elixir_config
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits)");
        builder.add_import("rustler::ResourceArc");

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
                builder.add_item(&gen_struct(typ, &mapper, &module_prefix));
                // Generate config constructor if type has Default
                if typ.has_default && !typ.fields.is_empty() {
                    let config_impl = gen_rustler_config_impl(typ, &mapper);
                    builder.add_item(&config_impl);
                }
            }
        }

        for enum_def in &api.enums {
            builder.add_item(&gen_enum(enum_def));
        }

        // Types with has_default=true accept JSON strings at the NIF boundary so
        // partial maps can be passed without every field being required.
        let default_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_default && !t.is_opaque)
            .map(|t| t.name.clone())
            .collect();

        for func in api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
        {
            if func.is_async {
                builder.add_item(&gen_nif_async_function(
                    func,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                ));
            } else {
                builder.add_item(&gen_nif_function(
                    func,
                    &mapper,
                    &opaque_types,
                    &default_types,
                    &core_import,
                ));
            }
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
                if method.is_async {
                    builder.add_item(&gen_nif_async_method(
                        &typ.name,
                        method,
                        &mapper,
                        typ.is_opaque,
                        &opaque_types,
                        &core_import,
                    ));
                } else {
                    builder.add_item(&gen_nif_method(
                        &typ.name,
                        method,
                        &mapper,
                        typ.is_opaque,
                        &opaque_types,
                        &core_import,
                    ));
                }
            }
        }

        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        // From/Into conversions
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core(typ, &core_import));
            }
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding(
                    typ,
                    &core_import,
                    &opaque_types,
                ));
            }
        }
        for e in &api.enums {
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core(
                    e,
                    &core_import,
                ));
            }
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding(
                    e,
                    &core_import,
                ));
            }
        }

        // Error converter functions
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_rustler_error_converter(
                error,
                &core_import,
            ));
        }

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Elixir)?;

        builder.add_item(&gen_nif_init(api, config, &exclude_functions, &exclude_types));

        let content = builder.build();

        let output_dir = resolve_output_dir(
            config.output.elixir.as_ref(),
            &config.crate_config.name,
            "packages/elixir/native/{name}_nif/src/",
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let app_name = config.elixir_app_name();
        let app_module = app_name.to_pascal_case();
        let native_mod = format!("{app_module}.Native");
        let crate_name = config.crate_config.name.replace('-', "_");

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
                Some((e.name.clone(), default_variant.name.to_snake_case()))
            })
            .collect();

        let mut files: Vec<GeneratedFile> = Vec::new();

        // Elixir .ex files belong in the Elixir lib/ directory, not the Rust native/src/ dir.
        // If config.output.elixir points at a native/ path (e.g. packages/elixir/native/.../src/),
        // derive the lib/ sibling by stripping everything from "/native/" onwards.
        let output_dir = if let Some(elixir_output) = config.output.elixir.as_ref() {
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

        // ── 3. Main wrapper module ────────────────────────────────────────────
        let mut content = String::from("# This file is auto-generated by alef. DO NOT EDIT.\n");
        content.push_str(&format!("defmodule {app_module} do\n"));
        content.push_str(&format!("  @moduledoc \"High-level API for {app_name}.\"\n\n"));

        // Wrapper functions for top-level API functions
        for func in &api.functions {
            let nif_fn_name = if func.is_async {
                format!("{}_async", func.name.to_snake_case())
            } else {
                func.name.to_snake_case()
            };
            let doc_line = func.doc.lines().next().unwrap_or("Function");

            let param_types: Vec<String> = func
                .params
                .iter()
                .map(|p| {
                    let base = elixir_typespec(&p.ty, &opaque_types, &default_types);
                    if p.optional { format!("{base} | nil") } else { base }
                })
                .collect();
            let return_spec = elixir_return_typespec(
                &func.return_type,
                func.error_type.is_some(),
                &opaque_types,
                &default_types,
            );
            let all_params: Vec<String> = func.params.iter().map(|p| p.name.to_snake_case()).collect();

            // Count how many trailing parameters are optional so we can emit shorter-arity overloads.
            let trailing_optional_count = func.params.iter().rev().take_while(|p| p.optional).count();

            // Emit one @spec/@doc per arity variant (shortest to longest).
            // The shortest arity fills optional params with nil.
            let arity_variants: Vec<usize> = if trailing_optional_count > 0 {
                ((all_params.len() - trailing_optional_count)..=all_params.len()).collect()
            } else {
                vec![all_params.len()]
            };

            for arity in &arity_variants {
                let arity_params = &all_params[..*arity];
                let arity_types = &param_types[..*arity];

                content.push_str(&format!("  @doc \"{doc_line}\"\n"));
                content.push_str(&format!("  @spec {nif_fn_name}("));
                content.push_str(&arity_types.join(", "));
                content.push_str(&format!(") :: {return_spec}\n"));

                // Build the call: fill missing optional params with nil
                let nif_call_args: Vec<String> = all_params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| if i < *arity { p.clone() } else { "nil".to_string() })
                    .collect();

                if arity_params.is_empty() {
                    content.push_str(&format!("  def {nif_fn_name} do\n"));
                    content.push_str(&format!(
                        "    {native_mod}.{nif_fn_name}({})\n",
                        nif_call_args.join(", ")
                    ));
                } else {
                    content.push_str(&format!("  def {nif_fn_name}({})", arity_params.join(", ")));
                    content.push_str(" do\n");
                    content.push_str(&format!(
                        "    {native_mod}.{nif_fn_name}({})\n",
                        nif_call_args.join(", ")
                    ));
                }
                content.push_str("  end\n\n");
            }
        }

        // Wrapper functions for type methods (e.g., conversionoptions_default)
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        {
            for method in &typ.methods {
                let nif_fn_name = if method.is_async {
                    format!("{}_{}_async", typ.name.to_lowercase(), method.name)
                } else {
                    format!("{}_{}", typ.name.to_lowercase(), method.name)
                };

                let doc_line = method.doc.lines().next().unwrap_or("Method");
                content.push_str(&format!("  @doc \"{doc_line}\"\n"));

                // Params: receiver (if any) + method params
                let mut param_names: Vec<String> = Vec::new();
                if method.receiver.is_some() {
                    param_names.push("obj".to_string());
                }
                for p in &method.params {
                    param_names.push(p.name.to_snake_case());
                }

                let return_spec = elixir_return_typespec(
                    &method.return_type,
                    method.error_type.is_some(),
                    &opaque_types,
                    &default_types,
                );
                content.push_str(&format!("  @spec {nif_fn_name}("));
                let type_specs: Vec<String> = {
                    let mut specs: Vec<String> = Vec::new();
                    if method.receiver.is_some() {
                        // receiver is the struct itself (non-opaque) or a reference
                        specs.push("map()".to_string());
                    }
                    for p in &method.params {
                        let base = elixir_typespec(&p.ty, &opaque_types, &default_types);
                        specs.push(if p.optional { format!("{base} | nil") } else { base });
                    }
                    specs
                };
                content.push_str(&type_specs.join(", "));
                content.push_str(&format!(") :: {return_spec}\n"));

                if param_names.is_empty() {
                    content.push_str(&format!("  def {nif_fn_name} do\n"));
                    content.push_str(&format!("    {native_mod}.{nif_fn_name}()\n"));
                } else {
                    content.push_str(&format!("  def {nif_fn_name}({})", param_names.join(", ")));
                    content.push_str(" do\n");
                    content.push_str(&format!("    {native_mod}.{nif_fn_name}({})\n", param_names.join(", ")));
                }
                content.push_str("  end\n\n");
            }
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
            depends_on_ffi: false,
            post_build: vec![],
        })
    }
}

/// Get module name and prefix from config or derive from crate name.
fn get_module_info(_api: &ApiSurface, config: &AlefConfig) -> (String, String) {
    let app_name = config.elixir_app_name();
    let module_prefix = {
        use heck::ToPascalCase;
        app_name.to_pascal_case()
    };
    (app_name, module_prefix)
}

/// Generate an opaque Rustler resource struct with inner Arc.
///
/// Also generates `RefUnwindSafe` impl so the resource can be used across
/// the `catch_unwind` boundary that `#[rustler::nif]` inserts. This is safe
/// because the inner value is behind `Arc` (shared ownership, no mutation
/// through the Arc itself) and Rustler's ResourceArc already guarantees
/// thread-safe access.
fn gen_opaque_resource(typ: &TypeDef, core_import: &str, _opaque_types: &AHashSet<String>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("#[derive(Clone)]\n");
    out.push_str(&format!("pub struct {} {{\n", typ.name));
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    out.push_str(&format!("    inner: Arc<{}>,\n", core_path));
    out.push_str("}\n\n");
    // SAFETY: The inner value is behind Arc (immutable shared reference) and
    // Rustler's ResourceArc ensures thread-safe access.
    out.push_str(&format!(
        "// SAFETY: See gen_opaque_resource in alef-backend-rustler for rationale.\n\
         impl std::panic::RefUnwindSafe for {} {{}}\n\n\
         impl rustler::Resource for {} {{}}\n",
        typ.name, typ.name
    ));
    out
}

/// Generate a Rustler NIF struct definition using the shared TypeMapper.
/// Rustler 0.37: NifStruct is a derive macro with #[module = "..."] attribute.
fn gen_struct(typ: &TypeDef, mapper: &RustlerMapper, module_prefix: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);
    if typ.has_default {
        // Config types use NifMap so partial maps can be passed —
        // unspecified keys use Rust Default values instead of Elixir zero values.
        // Binding types always derive Default, Serialize, and Deserialize.
        writeln!(
            out,
            "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifMap)]"
        )
        .ok();
    } else {
        // Binding types always derive Serialize and Deserialize for FFI/type conversion.
        writeln!(
            out,
            "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifStruct)]"
        )
        .ok();
        writeln!(out, "#[module = \"{}.{}\"]", module_prefix, typ.name).ok();
    }
    writeln!(out, "pub struct {} {{", typ.name).ok();

    for field in &typ.fields {
        let field_type = if field.optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        writeln!(out, "    pub {}: {},", field.name, field_type).ok();
    }

    write!(out, "}}").ok();
    out
}

/// Generate a Rustler config constructor impl for a type with `has_default`.
fn gen_rustler_config_impl(typ: &TypeDef, mapper: &RustlerMapper) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "impl {} {{", typ.name).ok();

    // Generate kwargs constructor using config_gen helper
    let map_fn = |ty: &TypeRef| mapper.map_type(ty);
    let config_method = alef_codegen::config_gen::gen_rustler_kwargs_constructor(typ, &map_fn);
    write!(out, "    {}", config_method).ok();

    writeln!(out, "}}").ok();
    out
}

/// Generate a Rustler NIF enum definition (unit enum).
fn gen_enum(enum_def: &EnumDef) -> String {
    let mut lines = vec![
        "#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rustler::NifUnitEnum)]".to_string(),
        format!("pub enum {} {{", enum_def.name),
    ];

    for variant in &enum_def.variants {
        lines.push(format!("    {},", variant.name));
    }

    lines.push("}".to_string());

    // Default impl for config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {} {{", enum_def.name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", first.name));
        lines.push("}".to_string());
    }

    lines.join("\n")
}

/// Wrap a return expression for Rustler (opaque types get ResourceArc wrapping).
fn gen_rustler_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    _type_name: &str,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
) -> String {
    match return_type {
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("ResourceArc::new({n} {{ inner: Arc::new({expr}.clone()) }})")
            } else {
                format!("ResourceArc::new({n} {{ inner: Arc::new({expr}) }})")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => format!("{expr}.into()"),
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Duration => format!("{expr}.as_millis() as u64"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                format!("{expr}.into_iter().map(|v| ResourceArc::new({n} {{ inner: Arc::new(v) }})).collect()")
            }
            TypeRef::Named(_) => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Build call argument expressions for Rustler opaque method (receiver is `resource`).
fn gen_rustler_method_call_args(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                format!("&{}.inner", p.name)
            }
            TypeRef::Named(_) => {
                if p.optional {
                    if p.is_ref {
                        format!("{}.as_ref().map(Into::into)", p.name)
                    } else {
                        format!("{}.map(Into::into)", p.name)
                    }
                } else if p.is_ref {
                    format!("&{}.clone().into()", p.name)
                } else {
                    format!("{}.into()", p.name)
                }
            }
            TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                format!("{}.as_deref()", p.name)
            }
            TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
            TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
            TypeRef::String | TypeRef::Char => p.name.clone(),
            TypeRef::Path => {
                if p.is_ref {
                    format!("&std::path::PathBuf::from({})", p.name)
                } else {
                    format!("std::path::PathBuf::from({})", p.name)
                }
            }
            TypeRef::Bytes => format!("&{}", p.name),
            TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
            TypeRef::Vec(_) => {
                if p.is_ref {
                    format!("&{}", p.name)
                } else {
                    p.name.to_string()
                }
            }
            _ => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate a Rustler NIF free function using the shared TypeMapper.
fn gen_nif_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let params_str = func
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    return format!("{}: ResourceArc<{}>", p.name, n);
                }
                // Default (has_default) types are passed as JSON strings so that
                // partial maps work — serde_json::from_str respects #[serde(default)].
                if default_types.contains(n) {
                    return format!("{}: Option<String>", p.name);
                }
                if p.optional {
                    return format!("{}: Option<{}>", p.name, n);
                }
            }
            let mapped = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{}>", p.name, mapped)
            } else {
                format!("{}: {}", p.name, mapped)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type = map_return_type(&func.return_type, mapper, opaque_types);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    // A function can be auto-delegated when all params (after JSON deserialization)
    // map cleanly to core types.  We treat default-typed params as delegatable by
    // building the JSON deserialization preamble ourselves.
    let has_default_params = func
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types) || has_default_params;

    let body = if can_delegate {
        // Build per-param deserialization lines and call-arg expressions.
        let mut deser_lines: Vec<String> = Vec::new();
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = format!("{core_import}::{n}");
                        // Optional JSON string → Option<CoreType> via serde
                        deser_lines.push(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s)).transpose().map_err(|e| e.to_string())?;",
                            p.name, core_ty
                        ));
                        // Handle based on whether core function expects reference or option
                        if p.optional {
                            // Core expects Option<T> → pass as-is
                            return format!("{}_core", p.name);
                        } else if p.is_ref {
                            // Core expects &T → use as_ref() to get Option<&T>, then unwrap
                            return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                        } else {
                            // Core expects T → unwrap or use default
                            return format!("{}_core.unwrap_or_default()", p.name);
                        }
                    }
                }
                // Fall back to the standard call-arg logic for all other types.
                match &p.ty {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        format!("&{}.inner", p.name)
                    }
                    TypeRef::Named(_) => {
                        if p.optional {
                            if p.is_ref {
                                // Option<T> where core expects &T → use .as_ref()
                                format!("{}.as_ref().map(Into::into)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
                            }
                        } else if p.is_ref {
                            // T where core expects &T → take reference of converted value
                            format!("&{}.clone().into()", p.name)
                        } else {
                            format!("{}.into()", p.name)
                        }
                    }
                    // String params: handle optional and reference cases.
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        // Option<String> where core expects Option<&str>
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => {
                        // Option<String> where core expects Option<String>
                        p.name.to_string()
                    }
                    TypeRef::String | TypeRef::Char if p.is_ref => {
                        // String where core expects &str
                        format!("&{}", p.name)
                    }
                    TypeRef::String | TypeRef::Char => {
                        // String where core expects String
                        p.name.clone()
                    }
                    TypeRef::Path => {
                        if p.is_ref {
                            // &Path expected → pass reference to PathBuf
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            // PathBuf expected
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => format!("&{}", p.name),
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            // Vec<T> where core expects &[T] → pass as slice
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = if deser_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n    ", deser_lines.join("\n    "))
        };

        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({})", call_args.join(", "));
        if func.error_type.is_some() {
            let wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
            format!("{preamble}let result = {core_call}.map_err(|e| e.to_string())?;\n    Ok({wrap})")
        } else {
            format!(
                "{preamble}{}",
                gen_rustler_wrap_return(&core_call, &func.return_type, "", opaque_types, func.returns_ref)
            )
        }
    } else {
        gen_rustler_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some())
    };
    format!(
        "#[rustler::nif]\npub fn {}({params_str}) -> {return_annotation} {{\n    \
         {body}\n}}",
        func.name
    )
}

/// Generate a Rustler NIF async free function (sync wrapper scheduled on DirtyCpu).
fn gen_nif_async_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let params_str = func
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    return format!("{}: ResourceArc<{}>", p.name, n);
                }
                // Default (has_default) types are passed as JSON strings.
                if default_types.contains(n) {
                    return format!("{}: Option<String>", p.name);
                }
                if p.optional {
                    return format!("{}: Option<{}>", p.name, n);
                }
            }
            let mapped = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{mapped}>", p.name)
            } else {
                format!("{}: {mapped}", p.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type = map_return_type(&func.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // function itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let has_default_params = func
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types) || has_default_params;

    let body = if can_delegate {
        let mut deser_lines: Vec<String> = Vec::new();
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = format!("{core_import}::{n}");
                        deser_lines.push(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s)).transpose().map_err(|e| e.to_string())?;",
                            p.name, core_ty
                        ));
                        // Handle based on whether core function expects reference or option
                        if p.optional {
                            // Core expects Option<T> → pass as-is
                            return format!("{}_core", p.name);
                        } else if p.is_ref {
                            // Core expects &T → use as_ref() to get Option<&T>, then unwrap
                            return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                        } else {
                            // Core expects T → unwrap or use default
                            return format!("{}_core.unwrap_or_default()", p.name);
                        }
                    }
                }
                match &p.ty {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        format!("&{}.inner", p.name)
                    }
                    TypeRef::Named(_) => {
                        if p.optional {
                            if p.is_ref {
                                format!("{}.as_ref().map(Into::into)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
                            }
                        } else if p.is_ref {
                            format!("&{}.clone().into()", p.name)
                        } else {
                            format!("{}.into()", p.name)
                        }
                    }
                    // String params: handle optional and reference cases.
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => {
                        p.name.to_string()
                    }
                    TypeRef::String | TypeRef::Char if p.is_ref => {
                        format!("&{}", p.name)
                    }
                    TypeRef::String | TypeRef::Char => {
                        p.name.clone()
                    }
                    TypeRef::Path => {
                        if p.is_ref {
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => format!("&{}", p.name),
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = if deser_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n    ", deser_lines.join("\n    "))
        };

        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({})", call_args.join(", "));
        let result_wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
        if func.error_type.is_some() {
            format!(
                "{preamble}let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| e.to_string())?;\n    \
                 Ok({result_wrap})"
            )
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            format!(
                "{preamble}let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 Ok({result_wrap})"
            )
        }
    } else {
        gen_rustler_unimplemented_body(&func.return_type, &format!("{}_async", func.name), true)
    };
    format!(
        "#[rustler::nif(schedule = \"DirtyCpu\")]\npub fn {}_async({params_str}) -> {return_annotation} {{\n    \
         {body}\n\
         }}",
        func.name
    )
}

/// Generate a Rustler NIF method for a struct using the shared TypeMapper.
fn gen_nif_method(
    struct_name: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let method_fn_name = format!("{}_{}", struct_name.to_lowercase(), method.name);

    let mut params = if method.receiver.is_some() {
        if is_opaque {
            vec![format!("resource: ResourceArc<{}>", struct_name)]
        } else {
            vec![format!("obj: {}", struct_name)]
        }
    } else {
        vec![]
    };

    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                params.push(format!("{}: ResourceArc<{}>", p.name, n));
                continue;
            }
        }
        let param_type = mapper.map_type(&p.ty);
        params.push(format!("{}: {}", p.name, param_type));
    }

    let return_type = map_return_type(&method.return_type, mapper, opaque_types);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types);
        let core_call = if is_opaque && method.receiver.is_some() {
            format!("resource.inner.as_ref().clone().{}({})", method.name, call_args)
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            let inner_ty = format!("{core_import}::{struct_name}");
            format!("{inner_ty}::{}({})", method.name, call_args)
        } else if method.receiver.is_some() {
            // Instance method on non-opaque: convert binding struct to core type, then call
            format!(
                "{core_import}::{}::from(obj).{}({})",
                struct_name, method.name, call_args
            )
        } else {
            // Static method on non-opaque: call directly on core type
            format!("{core_import}::{}::{}({})", struct_name, method.name, call_args)
        };
        if method.error_type.is_some() {
            let wrap = gen_rustler_wrap_return(
                "result",
                &method.return_type,
                struct_name,
                opaque_types,
                method.returns_ref,
            );
            format!("let result = {core_call}.map_err(|e| e.to_string())?;\n    Ok({wrap})")
        } else {
            gen_rustler_wrap_return(
                &core_call,
                &method.return_type,
                struct_name,
                opaque_types,
                method.returns_ref,
            )
        }
    } else {
        gen_rustler_unimplemented_body(&method.return_type, &method_fn_name, method.error_type.is_some())
    };
    format!(
        "#[rustler::nif]\npub fn {}({}) -> {} {{\n    \
         {body}\n}}",
        method_fn_name,
        params.join(", "),
        return_annotation
    )
}

/// Generate a Rustler NIF async method for a struct (sync wrapper scheduled on DirtyCpu).
fn gen_nif_async_method(
    struct_name: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let method_fn_name = format!("{}_{}_async", struct_name.to_lowercase(), method.name);

    let mut params = if method.receiver.is_some() {
        if is_opaque {
            vec![format!("resource: ResourceArc<{}>", struct_name)]
        } else {
            vec![format!("obj: {}", struct_name)]
        }
    } else {
        vec![]
    };

    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                params.push(format!("{}: ResourceArc<{}>", p.name, n));
                continue;
            }
        }
        let param_type = mapper.map_type(&p.ty);
        params.push(format!("{}: {}", p.name, param_type));
    }

    let return_type = map_return_type(&method.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // method itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types);
        let core_call = if is_opaque && method.receiver.is_some() {
            format!("resource.inner.as_ref().clone().{}({})", method.name, call_args)
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            let inner_ty = format!("{core_import}::{struct_name}");
            format!("{inner_ty}::{}({})", method.name, call_args)
        } else if method.receiver.is_some() {
            format!(
                "{core_import}::{}::from(obj).{}({})",
                struct_name, method.name, call_args
            )
        } else {
            // Static method on non-opaque: call directly on core type
            format!("{core_import}::{}::{}({})", struct_name, method.name, call_args)
        };
        let result_wrap = gen_rustler_wrap_return(
            "result",
            &method.return_type,
            struct_name,
            opaque_types,
            method.returns_ref,
        );
        if method.error_type.is_some() {
            format!(
                "let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| e.to_string())?;\n    \
                 Ok({result_wrap})"
            )
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            format!(
                "let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 Ok({result_wrap})"
            )
        }
    } else {
        gen_rustler_unimplemented_body(&method.return_type, &method_fn_name, true)
    };
    format!(
        "#[rustler::nif(schedule = \"DirtyCpu\")]\npub fn {}({}) -> {} {{\n    \
         {body}\n\
         }}",
        method_fn_name,
        params.join(", "),
        return_annotation
    )
}

/// Generate a type-appropriate unimplemented body for Rustler (no todo!()).
fn gen_rustler_unimplemented_body(return_type: &alef_core::ir::TypeRef, fn_name: &str, has_error: bool) -> String {
    use alef_core::ir::TypeRef;
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(String::from(\"{err_msg}\"))")
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

/// Map a return type, wrapping opaque Named types in ResourceArc.
fn map_return_type(ty: &alef_core::ir::TypeRef, mapper: &RustlerMapper, opaque_types: &AHashSet<String>) -> String {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) if opaque_types.contains(n) => format!("ResourceArc<{n}>"),
        _ => mapper.map_type(ty),
    }
}

/// Generate the rustler::init! macro invocation.
fn gen_nif_init(
    api: &ApiSurface,
    config: &AlefConfig,
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
            format!("{}_async", func.name)
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
    let opaque_types: Vec<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();
    if !opaque_types.is_empty() {
        let registrations: Vec<String> = opaque_types
            .iter()
            .map(|name| format!("    env.register::<{name}>().expect(\"Failed to register resource type {name}\");"))
            .collect();
        let reg_body = registrations.join("\n");
        format!(
            "fn on_load(env: rustler::Env, _info: rustler::Term) -> bool {{\n{reg_body}\n    true\n}}\n\n\
             rustler::init!(\"{module}\", load = on_load);"
        )
    } else {
        format!("rustler::init!(\"{module}\");")
    }
}

/// Generate the `{AppModule}.Native` Elixir module with NIF stubs for all functions and methods.
fn gen_native_ex(
    api: &ApiSurface,
    app_name: &str,
    app_module: &str,
    _crate_name: &str,
    config: &AlefConfig,
    exclude_functions: &AHashSet<&str>,
    exclude_types: &AHashSet<&str>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(1024);

    let repo_url = config.github_repo();
    // The env var that forces a local source build: {APP_NAME_UPPER}_BUILD
    let build_env_var = format!("{}_BUILD", app_name.to_uppercase());

    let _ = writeln!(out, "# This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "defmodule {app_module}.Native do");
    let _ = writeln!(out, "  @moduledoc false");
    let _ = writeln!(out);
    let _ = writeln!(out, "  use RustlerPrecompiled,");
    let _ = writeln!(out, "    otp_app: :{app_name},");
    let _ = writeln!(out, "    crate: \"{app_name}_nif\",");
    let _ = writeln!(
        out,
        "    base_url: \"{repo_url}/releases/download/v#{{Mix.Project.config()[:version]}}\","
    );
    let _ = writeln!(out, "    version: Mix.Project.config()[:version],");
    let _ = writeln!(
        out,
        "    force_build: System.get_env(\"{build_env_var}\") in [\"1\", \"true\"] or Mix.env() in [:test, :dev],"
    );
    let _ = writeln!(
        out,
        "    targets: ~w(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu),"
    );
    let _ = writeln!(out, "    nif_versions: [\"2.16\", \"2.17\"]");
    let _ = writeln!(out);

    // Stubs for top-level API functions
    let mut last_was_multiline = false;
    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let fn_name = if func.is_async {
            format!("{}_async", func.name)
        } else {
            func.name.clone()
        };
        let underscored_params: Vec<String> = func
            .params
            .iter()
            .map(|p| format!("_{}", p.name.to_snake_case()))
            .collect();
        last_was_multiline = write_nif_stub(&mut out, &fn_name, &underscored_params, last_was_multiline);
    }

    // Stubs for type methods
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
            let nif_fn_name = if method.is_async {
                format!("{}_{}_async", typ.name.to_lowercase(), method.name)
            } else {
                format!("{}_{}", typ.name.to_lowercase(), method.name)
            };

            let mut underscored_params: Vec<String> = Vec::new();
            if method.receiver.is_some() {
                underscored_params.push("_obj".to_string());
            }
            for p in &method.params {
                underscored_params.push(format!("_{}", p.name.to_snake_case()));
            }

            last_was_multiline = write_nif_stub(&mut out, &nif_fn_name, &underscored_params, last_was_multiline);
        }
    }

    let _ = writeln!(out, "end");
    out
}

/// Write a NIF stub line, splitting onto two lines when the single-line form exceeds 98 chars.
///
/// `prev_was_multiline` should be `true` when the previous stub was multi-line. This is used
/// to insert a single blank separator line around multi-line defs (mix format requirement):
/// - single → multi: blank before multi
/// - multi → single: blank before single
/// - multi → multi: single blank between them (not double)
/// - single → single: no blank
///
/// Returns `true` when this stub was written in multi-line form.
///
/// Single-line form:  `  def fn_name(args), do: :erlang.nif_error(:nif_not_loaded)`
/// Two-line form:
/// ```elixir
///   def fn_name(args),
///     do: :erlang.nif_error(:nif_not_loaded)
/// ```
fn write_nif_stub(out: &mut String, fn_name: &str, params: &[String], prev_was_multiline: bool) -> bool {
    use std::fmt::Write;
    let args = params.join(", ");
    // Elixir convention: omit parens on zero-arg defs
    let sig = if args.is_empty() {
        fn_name.to_string()
    } else {
        format!("{fn_name}({args})")
    };
    // "  def <sig>, do: :erlang.nif_error(:nif_not_loaded)"
    let single_line_len = 6 + sig.len() + 40;
    if single_line_len > 98 {
        if !prev_was_multiline {
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "  def {sig},");
        let _ = writeln!(out, "    do: :erlang.nif_error(:nif_not_loaded)");
        let _ = writeln!(out);
        true
    } else {
        let _ = writeln!(out, "  def {sig}, do: :erlang.nif_error(:nif_not_loaded)");
        false
    }
}

/// Generate a `defmodule {AppModule}.{TypeName}` file with a `defstruct` for a non-opaque type.
fn gen_elixir_struct_module(
    typ: &TypeDef,
    app_module: &str,
    enum_defaults: &std::collections::HashMap<String, String>,
    opaque_types: &AHashSet<String>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    let _ = writeln!(out, "# This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "defmodule {app_module}.{} do", typ.name);

    if !typ.doc.is_empty() {
        let doc_first = typ.doc.lines().next().unwrap_or("").replace('"', "\\\"");
        let _ = writeln!(out, "  @moduledoc \"{doc_first}\"");
    } else {
        let _ = writeln!(out, "  @moduledoc false");
    }
    let _ = writeln!(out);

    // defstruct with defaults - use bare keyword list style (mix format compliant)
    let fields: Vec<_> = typ.fields.iter().collect();
    if fields.is_empty() {
        let _ = writeln!(out, "  defstruct []");
    } else {
        let _ = write!(out, "  defstruct ");
        for (i, field) in fields.iter().enumerate() {
            let default = elixir_field_default(field, &field.ty, enum_defaults, opaque_types);
            let name = field.name.to_snake_case();
            if i == 0 {
                let _ = write!(out, "{name}: {default}");
            } else {
                let _ = write!(out, ",\n            {name}: {default}");
            }
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "end");
    out
}

/// Format an integer literal with underscore separators for Elixir conventions.
/// E.g. 5242880 → "5_242_880". Numbers < 1000 are returned unchanged.
fn elixir_format_integer(n: i64) -> String {
    let (neg, s) = if n < 0 {
        (true, (-n).to_string())
    } else {
        (false, n.to_string())
    };
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push('_');
        }
        result.push(c);
    }
    let formatted: String = result.chars().rev().collect();
    if neg { format!("-{formatted}") } else { formatted }
}

/// Derive an Elixir default expression for a struct field.
fn elixir_field_default(
    field: &FieldDef,
    ty: &TypeRef,
    enum_defaults: &std::collections::HashMap<String, String>,
    _opaque_types: &AHashSet<String>,
) -> String {
    use alef_core::ir::DefaultValue;

    if let Some(td) = &field.typed_default {
        return match td {
            DefaultValue::BoolLiteral(b) => (if *b { "true" } else { "false" }).to_string(),
            DefaultValue::StringLiteral(s) => format!("\"{}\"", s.replace('"', "\\\"")),
            DefaultValue::IntLiteral(i) => elixir_format_integer(*i),
            DefaultValue::FloatLiteral(f) => format!("{f}"),
            DefaultValue::EnumVariant(v) => format!(":{}", v.to_snake_case()),
            DefaultValue::Empty => elixir_zero_value(ty, enum_defaults),
            DefaultValue::None => "nil".to_string(),
        };
    }

    // No typed_default: use optional flag or type-appropriate zero
    if field.optional {
        return "nil".to_string();
    }
    elixir_zero_value(ty, enum_defaults)
}

/// Generate a type-appropriate zero/default value for Elixir.
fn elixir_zero_value(ty: &TypeRef, enum_defaults: &std::collections::HashMap<String, String>) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "false".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"".to_string(),
        TypeRef::Bytes => "\"\"".to_string(),
        TypeRef::Duration => "0".to_string(),
        TypeRef::Vec(_) => "[]".to_string(),
        TypeRef::Map(_, _) => "%{}".to_string(),
        TypeRef::Optional(_) => "nil".to_string(),
        TypeRef::Unit => "nil".to_string(),
        TypeRef::Named(name) => {
            if let Some(variant) = enum_defaults.get(name) {
                format!(":{variant}")
            } else {
                "nil".to_string()
            }
        }
    }
}

/// Map a TypeRef to an Elixir typespec string for `@spec` annotations.
///
/// `default_types` lists types that are passed as JSON strings at the NIF boundary
/// (types with `has_default = true`).  Their typespec is `String.t() | nil` rather
/// than `map()` because callers encode them with `Jason.encode!/1`.
fn elixir_typespec(ty: &TypeRef, opaque_types: &AHashSet<String>, default_types: &AHashSet<String>) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String.t()".to_string(),
        TypeRef::Bytes => "binary()".to_string(),
        TypeRef::Unit => "nil".to_string(),
        TypeRef::Duration => "non_neg_integer()".to_string(),
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "boolean()".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "float()".to_string(),
            alef_core::ir::PrimitiveType::U8
            | alef_core::ir::PrimitiveType::U16
            | alef_core::ir::PrimitiveType::U32
            | alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::Usize => "non_neg_integer()".to_string(),
            alef_core::ir::PrimitiveType::I8
            | alef_core::ir::PrimitiveType::I16
            | alef_core::ir::PrimitiveType::I32
            | alef_core::ir::PrimitiveType::I64
            | alef_core::ir::PrimitiveType::Isize => "integer()".to_string(),
        },
        TypeRef::Named(name) => {
            if opaque_types.contains(name) {
                "reference()".to_string()
            } else if default_types.contains(name) {
                // Passed as an optional JSON string; nil means use defaults.
                "String.t() | nil".to_string()
            } else {
                "map()".to_string()
            }
        }
        TypeRef::Optional(inner) => {
            format!("{} | nil", elixir_typespec(inner, opaque_types, default_types))
        }
        TypeRef::Vec(inner) => {
            format!("[{}]", elixir_typespec(inner, opaque_types, default_types))
        }
        TypeRef::Map(_, _) => "map()".to_string(),
    }
}

/// Map a return TypeRef to an Elixir typespec for `@spec` return annotations.
fn elixir_return_typespec(
    ty: &TypeRef,
    has_error: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    let base = elixir_typespec(ty, opaque_types, default_types);
    if has_error {
        format!("{{:ok, {}}} | {{:error, String.t()}}", base)
    } else {
        base
    }
}
