use crate::codegen::conversions::helpers::is_tuple_variant;
use crate::codegen::generators::type_paths::resolve_type_path;
use crate::core::ir::{EnumDef, EnumVariant};
use std::collections::HashMap;

pub(super) fn gen_from_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let core_path = resolve_type_path(&enum_def.name, core_import, type_paths);
    let binding_name = enum_def.name.as_str();
    let arms: Vec<String> = enum_def
        .variants
        .iter()
        .map(|variant| {
            let pattern = binding_pattern(binding_name, variant);
            let expression = core_expression(variant);
            crate::backends::extendr::template_env::render(
                "enum_from_binding_to_core_arm.jinja",
                minijinja::context! {
                    pattern => &pattern,
                    expression => &expression,
                },
            )
        })
        .collect();

    let catch_all = catch_all(enum_def).then(|| {
        crate::backends::extendr::template_env::render(
            "enum_from_binding_to_core_catch_all.jinja",
            minijinja::context! {},
        )
    });

    crate::backends::extendr::template_env::render(
        "enum_from_binding_to_core_impl.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            catch_all => catch_all,
        },
    )
}

pub(super) fn gen_from_core_to_binding(
    enum_def: &EnumDef,
    core_import: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let core_path = resolve_type_path(&enum_def.name, core_import, type_paths);
    let binding_name = enum_def.name.as_str();
    let arms: Vec<String> = enum_def
        .variants
        .iter()
        .map(|variant| {
            let pattern = core_pattern(&core_path, variant);
            let expression = binding_expression(variant);
            crate::backends::extendr::template_env::render(
                "enum_from_core_to_binding_arm.jinja",
                minijinja::context! {
                    pattern => &pattern,
                    expression => &expression,
                },
            )
        })
        .collect();

    let catch_all = catch_all(enum_def).then(|| {
        crate::backends::extendr::template_env::render(
            "enum_from_core_to_binding_catch_all.jinja",
            minijinja::context! {},
        )
    });

    crate::backends::extendr::template_env::render(
        "enum_from_core_to_binding_impl.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            catch_all => catch_all,
        },
    )
}

fn catch_all(enum_def: &EnumDef) -> bool {
    let has_excluded_variants = !enum_def.excluded_variants.is_empty();
    let core_has_struct_variants = enum_def
        .variants
        .iter()
        .any(|variant| !variant.fields.is_empty() && !variant.is_tuple);
    let has_any_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    has_excluded_variants || core_has_struct_variants || has_any_data_variants
}

fn binding_pattern(binding_name: &str, variant: &EnumVariant) -> String {
    format!("{binding_name}::{}", variant.name)
}

fn core_pattern(core_path: &str, variant: &EnumVariant) -> String {
    if variant.fields.is_empty() {
        format!("{core_path}::{}", variant.name)
    } else if is_tuple_variant(&variant.fields) {
        format!("{core_path}::{}(..)", variant.name)
    } else {
        format!("{core_path}::{} {{ .. }}", variant.name)
    }
}

fn core_expression(variant: &EnumVariant) -> String {
    if variant.fields.is_empty() {
        format!("Self::{}", variant.name)
    } else if is_tuple_variant(&variant.fields) {
        let defaults = variant
            .fields
            .iter()
            .map(|_| "Default::default()")
            .collect::<Vec<_>>()
            .join(", ");
        format!("Self::{}({defaults})", variant.name)
    } else {
        let defaults = variant
            .fields
            .iter()
            .map(|field| format!("{}: Default::default()", field.name))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Self::{} {{ {defaults} }}", variant.name)
    }
}

fn binding_expression(variant: &EnumVariant) -> String {
    format!("Self::{}", variant.name)
}
