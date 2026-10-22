//! Typed Ruby exception classes and the converters that raise them.
//!
//! The class family is the one the PHP backend uses (`php::classes`): a base class per error and
//! one subclass per variant. Each exception object carries the error's introspection values as
//! instance variables, read through attribute readers on the base class.

use super::php::{classes, error_info};
use super::shared::{error_variant_wildcard_pattern, python_exception_name, to_snake_case};
use crate::core::ir::{ApiSurface, ErrorDef};

fn rust_path_of(error: &ErrorDef, core_import: &str) -> String {
    if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    }
}

/// `ruby_init` statements that define the exception classes and their attribute readers.
pub fn gen_magnus_error_class_registrations(api: &ApiSurface) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    for class in classes(api, "", None)? {
        let var = format!("{}_class", to_snake_case(&class.name));
        let parent = match &class.parent {
            Some(parent) => format!("{}_class", to_snake_case(parent)),
            None => "ruby.exception_standard_error()".to_string(),
        };
        lines.push(format!(
            "    let {var} = module.define_error(\"{}\", {parent})?;",
            class.name
        ));
        for field in &class.info {
            lines.push(format!(
                "    {var}.define_attr(\"{}\", magnus::Attr::Read)?;",
                field.name
            ));
            if field.name == "is_transient" {
                lines.push(format!("    {var}.define_alias(\"transient?\", \"is_transient\")?;"));
            }
        }
    }
    Ok(lines)
}

/// Generate a converter function that maps a core error to a typed Ruby exception.
pub fn gen_magnus_error_converter(error: &ErrorDef, core_import: &str, module_name: &str) -> String {
    let rust_path = rust_path_of(error, core_import);
    let variants = error
        .variants
        .iter()
        .map(|variant| {
            crate::alef_context! {
                pattern => error_variant_wildcard_pattern(&rust_path, variant),
                class_name => python_exception_name(&variant.name, &error.name),
            }
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/magnus_error_converter.jinja",
        crate::alef_context! {
            rust_path => rust_path,
            fn_name => format!("{}_to_magnus_err", to_snake_case(&error.name)),
            module_name => module_name,
            base_class => error.name.as_str(),
            info => error_info(error),
            variants => variants,
        },
    )
}

/// `fn alef_magnus_err`: turns the error a core call returned into the typed exception of its
/// family. An error type that no converter handles becomes a plain `RuntimeError`.
pub fn gen_magnus_error_dispatch(api: &ApiSurface, core_import: &str) -> String {
    let arms = api
        .errors
        .iter()
        .map(|error| {
            crate::alef_context! {
                rust_path => rust_path_of(error, core_import),
                fn_name => format!("{}_to_magnus_err", to_snake_case(&error.name)),
            }
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/magnus_error_dispatch.jinja",
        crate::alef_context! { arms => arms },
    )
}
