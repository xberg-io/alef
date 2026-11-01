//! Typed Ruby exception classes and the converters that raise them.
//!
//! The class family is the one the PHP backend uses (`php::classes`): a base class per error and
//! one subclass per variant. Each exception object carries the error's introspection values as
//! instance variables, read through attribute readers on the base class.

use super::php::{classes, error_info};
use super::shared::{error_variant_wildcard_pattern, python_exception_name, to_snake_case};
use super::{ExceptionNaming, exception_lineage, shared_exception_classes};
use crate::core::ir::{ApiSurface, ErrorDef};
use std::collections::HashSet;

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
    let classes = classes(api, "", None)?;
    let parents: HashSet<_> = classes.iter().filter_map(|class| class.parent.as_deref()).collect();
    for class in &classes {
        let var = format!("{}_class", to_snake_case(&class.name));
        let parent = match &class.parent {
            Some(parent) => format!("{}_class", to_snake_case(parent)),
            None => "ruby.exception_standard_error()".to_string(),
        };
        let registration = format!("module.define_error(\"{}\", {parent})?", class.name);
        if class.info.is_empty() && !parents.contains(class.name.as_str()) {
            lines.push(format!("    {registration};"));
        } else {
            lines.push(format!("    let {var} = {registration};"));
        }
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
    let shared = shared_exception_classes(&api.errors, ExceptionNaming::Python);
    // Every family gets a module, parents first since a family's module includes its parent's.
    let mut families: Vec<Vec<String>> = api
        .errors
        .iter()
        .map(|error| exception_lineage(&api.errors, &error.name))
        .collect();
    families.sort_by_key(Vec::len);
    for lineage in &families {
        lines.push(format!(
            "    module.funcall::<_, _, magnus::Value>(\"module_eval\", ({:?},))?;",
            ruby_family_module(&lineage[0], lineage.get(1).map(String::as_str))
        ));
    }
    for (class, members) in &shared {
        for family in members.families.iter().skip(1) {
            lines.push(format!(
                "    module.funcall::<_, _, magnus::Value>(\"module_eval\", ({:?},))?;",
                format!("{class}.include({family}::Family)")
            ));
        }
    }
    Ok(lines)
}

/// Ruby source, evaluated inside the binding's module, that gives `family` a `Family` module its
/// classes include, and makes `rescue SchemaError` also match a variant class the family shares
/// with another family and so does not extend. The `===` override applies to the family class
/// itself only, so rescuing one of its variant subclasses keeps matching that subclass alone.
/// The module includes the `exception_base` parent's, so a shared class joining `SchemaError` is
/// also a `RootError::Family`, and `rescue RootError` matches it.
fn ruby_family_module(family: &str, parent: Option<&str>) -> String {
    let module = parent.map_or_else(
        || "module Family; end".to_string(),
        |parent| format!("module Family; include {parent}::Family; end"),
    );
    format!(
        "class {family}\n  {module}\n  include Family\n  def self.===(other)\n    \
         super || (equal?({family}) && Family === other)\n  end\nend"
    )
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
