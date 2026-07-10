//! Java→Kotlin typealias emission for types, enums, and errors.
//!
//! The JVM Kotlin wrapper re-uses the Java facade's records, sealed interfaces,
//! and exception classes via `typealias` so values pass straight through to the
//! JNA-loaded native bridge without conversion.

use crate::core::ir::{ApiSurface, EnumDef, TypeDef};

/// Append `typealias` declarations for all visible types, enums, and errors
/// to `body`, returning whether each section was non-empty.
///
/// `configured_trait_bridges` controls which trait types are aliased to
/// `I{TraitName}` (the Java bridge interface); types without a bridge entry
/// are skipped entirely.
pub(super) fn emit_typealiases(
    api: &ApiSurface,
    java_package: &str,
    exclude_types: &std::collections::HashSet<&str>,
    configured_trait_bridges: &std::collections::HashSet<&str>,
    body: &mut String,
) {
    let visible_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| {
            if exclude_types.contains(t.name.as_str()) {
                return false;
            }
            if t.is_trait && !configured_trait_bridges.contains(t.name.as_str()) {
                return false;
            }
            true
        })
        .collect();
    for ty in &visible_types {
        if ty.is_trait {
            body.push_str(&crate::backends::kotlin::template_env::render(
                "typealias_trait.jinja",
                minijinja::context! {
                    name => &ty.name,
                    java_package => java_package,
                },
            ));
        } else {
            body.push_str(&crate::backends::kotlin::template_env::render(
                "typealias_type.jinja",
                minijinja::context! {
                    name => &ty.name,
                    java_package => java_package,
                },
            ));
        }
    }
    if !visible_types.is_empty() {
        body.push('\n');
    }

    let visible_enums: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(e.name.as_str()))
        .collect();
    for en in &visible_enums {
        body.push_str(&crate::backends::kotlin::template_env::render(
            "typealias_type.jinja",
            minijinja::context! {
                name => &en.name,
                java_package => java_package,
            },
        ));
    }
    if !visible_enums.is_empty() {
        body.push('\n');
    }

    for error in &api.errors {
        body.push_str(&crate::backends::kotlin::template_env::render(
            "typealias_error.jinja",
            minijinja::context! {
                name => &error.name,
                java_package => java_package,
            },
        ));
    }
    if !api.errors.is_empty() {
        body.push('\n');
    }
}
