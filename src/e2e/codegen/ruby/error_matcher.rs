use crate::core::ir::ErrorDef;
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, declared_variant};
use crate::e2e::escape::ruby_regex_literal;
use crate::e2e::fixture::Fixture;

use super::values::qualify_ruby_type;

/// Render the `raise_error` matcher for a fixture's `error` assertion.
///
/// The extension defines no class for an error that is `binding_excluded`, and Ruby resolves a
/// constant only when the matcher runs. The registry is narrowed to the errors the extension
/// defines before any class name is derived from it.
pub(super) fn render_raise_error_clause(fixture: &Fixture, errors: &[ErrorDef], module_name: &str) -> String {
    let errors: Vec<ErrorDef> = errors.iter().filter(|error| !error.binding_excluded).cloned().collect();
    let errors = errors.as_slice();
    let declared = match classify("ruby", fixture, errors) {
        DeclaredErrorAssertion::Assert(value) => Some(value),
        DeclaredErrorAssertion::Undeclared => None,
        DeclaredErrorAssertion::Unsubstantiable(_) => unreachable!("Ruby preserves declared variant classes"),
    };
    let variant = declared_variant(fixture, errors);
    let class_name = variant.map(|(error, variant)| {
        qualify_ruby_type(
            module_name,
            &crate::codegen::error_gen::python_exception_name(&variant.name, &error.name),
        )
    });
    let mut bases: Vec<_> = errors
        .iter()
        .filter(|error| error.exception_base.is_none())
        .map(|error| qualify_ruby_type(module_name, &error.name))
        .collect();
    bases.sort();
    bases.dedup();
    let matcher_class = class_name.unwrap_or_else(|| {
        if bases.is_empty() {
            "RuntimeError"
        } else {
            "StandardError"
        }
        .to_string()
    });
    let regex = declared.filter(|_| variant.is_none()).map(ruby_regex_literal);
    crate::e2e::template_env::render(
        "ruby/error_matcher.jinja",
        crate::alef_context! {
            matcher_class => matcher_class,
            bases => if variant.is_none() { bases } else { Vec::new() },
            regex => regex,
        },
    )
    .trim_end()
    .to_string()
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
