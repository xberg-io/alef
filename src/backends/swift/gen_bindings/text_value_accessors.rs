//! The public accessor that returns the whole value of a text-projected field.
//!
//! The getter of a field listed in `untagged_union_text_types` returns the plain text of the
//! union, so the parts of a list variant are lost. The bridge crate emits a second getter that
//! returns the value as JSON. This module emits an extension of the bridge type that decodes that
//! JSON into the Swift union type.

use crate::backends::swift::gen_rust_crate::type_bridge::{text_projected_getter_type, text_projected_value_names};
use crate::backends::swift::gen_rust_crate::wrappers::is_unbridgeable_getter;
use crate::backends::swift::naming::swift_source_ident;
use crate::codegen::generators::type_paths::build_type_path_lookup;
use crate::core::ir::{ApiSurface, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

pub(super) fn emit(
    api: &ApiSurface,
    text_types: &[String],
    exclude_types: &HashSet<String>,
    exclude_fields: &HashSet<String>,
    configured_features: &HashSet<&str>,
    out: &mut String,
) {
    if text_types.is_empty() {
        return;
    }
    let configured: HashSet<&str> = text_types.iter().map(String::as_str).collect();
    let type_paths = build_type_path_lookup(api);
    let no_serde_names: HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.has_serde)
        .map(|t| t.name.as_str())
        .collect();
    for ty in api
        .types
        .iter()
        .filter(|t| !t.is_trait && !exclude_types.contains(&t.name))
    {
        let accessors: Vec<_> = ty
            .fields
            .iter()
            .filter(|field| {
                !is_unbridgeable_getter(
                    ty,
                    field,
                    exclude_fields,
                    &type_paths,
                    &no_serde_names,
                    configured_features,
                )
            })
            .filter_map(|field| {
                text_projected_getter_type(field, &configured)?;
                let named = match &field.ty {
                    TypeRef::Optional(inner) => inner.as_ref(),
                    other => other,
                };
                let TypeRef::Named(enum_name) = named else { return None };
                let optional = field.optional || matches!(field.ty, TypeRef::Optional(_));
                let names = text_projected_value_names(&field.name);
                let enum_name = swift_source_ident(enum_name);
                Some(crate::alef_context! {
                    field => swift_source_ident(&field.name.to_lower_camel_case()),
                    name => names.accessor,
                    json_getter => names.json_getter_swift,
                    value_type => if optional { format!("{enum_name}?") } else { enum_name.clone() },
                    enum_name => enum_name,
                    optional => optional,
                })
            })
            .collect();
        if accessors.is_empty() {
            continue;
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_text_value_accessors.swift.jinja",
            crate::alef_context! { type_name => &ty.name, accessors => accessors },
        ));
        out.push('\n');
    }
}

#[cfg(test)]
#[path = "text_value_accessors_tests.rs"]
mod tests;
