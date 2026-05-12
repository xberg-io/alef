//! Java→Kotlin typealias emission for types, enums, and errors.
//!
//! The JVM Kotlin wrapper re-uses the Java facade's records, sealed interfaces,
//! and exception classes via `typealias` so values pass straight through to the
//! JNA-loaded native bridge without conversion.

use alef_core::ir::{ApiSurface, EnumDef, TypeDef};

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
            // Skip trait types that don't have a configured bridge — Java
            // doesn't emit them, so a typealias would fail to resolve.
            if t.is_trait && !configured_trait_bridges.contains(t.name.as_str()) {
                return false;
            }
            true
        })
        .collect();
    for ty in &visible_types {
        if ty.is_trait {
            body.push_str(&crate::template_env::render(
                "typealias_trait.jinja",
                minijinja::context! {
                    name => &ty.name,
                    java_package => java_package,
                },
            ));
        } else {
            body.push_str(&crate::template_env::render(
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
        body.push_str(&crate::template_env::render(
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

    // Error types are aliased with the `Exception` suffix to mirror the Java
    // facade's class name and to avoid collision with a same-named non-error
    // struct in `api.types` (e.g. an error variant `Foo` may coexist with a
    // struct `Foo` in the same surface). Without the suffix, Kotlin emits two
    // `typealias Foo` declarations and `compileKotlin` fails with
    // "Redeclaration:".
    for error in &api.errors {
        body.push_str(&crate::template_env::render(
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
