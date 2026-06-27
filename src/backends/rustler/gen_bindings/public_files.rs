use crate::backends::rustler::gen_bindings::helpers::{
    gen_elixir_enum_module_with_known_types, gen_elixir_opaque_module, gen_elixir_struct_module, gen_native_ex,
};
use crate::backends::rustler::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::HashMap;
use std::path::PathBuf;

pub(super) struct PublicFileContext<'a> {
    pub(super) app_name: &'a str,
    pub(super) app_module: &'a str,
    pub(super) crate_name: &'a str,
    pub(super) exclude_functions: &'a AHashSet<String>,
    pub(super) exclude_types: &'a AHashSet<&'a str>,
    pub(super) opaque_types: &'a AHashSet<String>,
}

pub(super) fn generated_module_files(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    context: PublicFileContext<'_>,
) -> (String, Vec<GeneratedFile>) {
    let output_dir = elixir_output_dir(config);
    let enum_defaults = enum_defaults(api);
    let known_struct_types = known_public_struct_types(api, context.exclude_types);
    let mut files = Vec::new();

    push_native_module_file(api, config, &context, &output_dir, &mut files);
    push_struct_module_files(
        api,
        &context,
        &output_dir,
        &enum_defaults,
        &known_struct_types,
        &mut files,
    );
    push_opaque_module_files(api, config, &context, &output_dir, &mut files);
    push_enum_module_files(api, &context, &output_dir, &mut files);

    (output_dir, files)
}

pub(super) fn append_stream_error_exception(content: &mut String, config: &ResolvedCrateConfig, app_module: &str) {
    let streaming_adapters: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .collect();
    if streaming_adapters.is_empty() {
        return;
    }

    let exception_module = format!("{app_module}.StreamError");
    let rendered = template_env::render(
        "elixir_stream_error_exception.jinja",
        minijinja::context! {
            exception_module => &exception_module,
        },
    );
    let dedented = rendered
        .lines()
        .map(|line| line.strip_prefix("  ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    content.push('\n');
    content.push_str(&dedented);
    if !content.ends_with('\n') {
        content.push('\n');
    }
}

fn elixir_output_dir(config: &ResolvedCrateConfig) -> String {
    if let Some(elixir_output) = config.output_paths.get("elixir") {
        let path = elixir_output.to_string_lossy();
        if let Some(idx) = path.find("/native/") {
            format!("{}/lib/", &path[..idx])
        } else {
            path.into_owned()
        }
    } else {
        "packages/elixir/lib/".to_owned()
    }
}

fn enum_defaults(api: &ApiSurface) -> HashMap<String, String> {
    api.enums
        .iter()
        .filter_map(|enum_def| {
            let default_variant = enum_def
                .variants
                .iter()
                .find(|variant| variant.is_default)
                .or_else(|| enum_def.variants.first())?;
            Some((
                enum_def.name.clone(),
                crate::codegen::naming::pascal_to_snake(&default_variant.name),
            ))
        })
        .collect()
}

fn known_public_struct_types(api: &ApiSurface, exclude_types: &AHashSet<&str>) -> AHashSet<String> {
    api.types
        .iter()
        .filter(|typ| !typ.is_trait && !typ.is_opaque && !typ.fields.is_empty())
        .filter(|typ| !exclude_types.contains(typ.name.as_str()))
        .map(|typ| typ.name.clone())
        .collect()
}

fn push_native_module_file(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    context: &PublicFileContext<'_>,
    output_dir: &str,
    files: &mut Vec<GeneratedFile>,
) {
    let native_content = gen_native_ex(
        api,
        context.app_name,
        context.app_module,
        context.crate_name,
        config,
        context.exclude_functions,
        context.exclude_types,
    );
    files.push(GeneratedFile {
        path: PathBuf::from(output_dir)
            .join(context.app_name.to_snake_case())
            .join("native.ex"),
        content: native_content,
        generated_header: false,
    });
}

fn push_struct_module_files(
    api: &ApiSurface,
    context: &PublicFileContext<'_>,
    output_dir: &str,
    enum_defaults: &HashMap<String, String>,
    known_struct_types: &AHashSet<String>,
    files: &mut Vec<GeneratedFile>,
) {
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !context.exclude_types.contains(typ.name.as_str()))
    {
        if typ.is_opaque || typ.fields.is_empty() {
            continue;
        }
        let struct_content = gen_elixir_struct_module(
            typ,
            context.app_module,
            enum_defaults,
            context.opaque_types,
            known_struct_types,
        );
        let file_name = format!("{}.ex", typ.name.to_snake_case());
        files.push(GeneratedFile {
            path: PathBuf::from(output_dir)
                .join(context.app_name.to_snake_case())
                .join(file_name),
            content: struct_content,
            generated_header: false,
        });
    }
}

fn push_opaque_module_files(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    context: &PublicFileContext<'_>,
    output_dir: &str,
    files: &mut Vec<GeneratedFile>,
) {
    for typ in api
        .types
        .iter()
        .filter(|typ| typ.is_opaque && !typ.is_trait && !context.exclude_types.contains(typ.name.as_str()))
    {
        let opaque_content = gen_elixir_opaque_module(typ, context.app_module, config);
        let file_name = format!("{}.ex", typ.name.to_snake_case());
        files.push(GeneratedFile {
            path: PathBuf::from(output_dir)
                .join(context.app_name.to_snake_case())
                .join(file_name),
            content: opaque_content,
            generated_header: false,
        });
    }
}

fn push_enum_module_files(
    api: &ApiSurface,
    context: &PublicFileContext<'_>,
    output_dir: &str,
    files: &mut Vec<GeneratedFile>,
) {
    let known_type_names: AHashSet<String> = api.types.iter().map(|typ| typ.name.clone()).collect();
    for enum_def in &api.enums {
        let enum_content = gen_elixir_enum_module_with_known_types(enum_def, context.app_module, &known_type_names);
        let file_name = format!("{}.ex", enum_def.name.to_snake_case());
        files.push(GeneratedFile {
            path: PathBuf::from(output_dir)
                .join(context.app_name.to_snake_case())
                .join(file_name),
            content: enum_content,
            generated_header: false,
        });
    }
}
