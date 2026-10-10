use std::collections::HashSet;

use crate::core::ir::{DefaultValue, FieldDef, TypeDef, TypeRef};

use super::shared::{constructor_fields, default_value_for_field_in_type, use_unwrap_or_default};

/// Whether this field's default must be left to `Default::default()` instead of being spelled
/// as a literal in the generated `new(opts)` constructor.
///
/// Two shapes cannot be spelled here. A `Named` field's binding-side type is a generated wrapper,
/// so the core-crate path the renderer produces would not typecheck. And an `EnumVariant` default
/// on a `String`-typed field renders as the variant's *wire* string, which the binding struct's
/// field cannot be initialised from without knowing the enum — the shared renderer snake-cases it
/// for a reason that only holds on the far side of serde.
///
/// This used to be decided by sniffing the rendered Rust source for `::` or a leading quote.
/// `StringLiteral("hello")` renders as `"hello".to_string()`, which starts with a quote, so an
/// ordinary `String` field with a real default was misread as an enum variant and collapsed to
/// `.unwrap_or_default()` — `""` where the source crate says `"hello"`. Asking `typed_default`
/// what the value *is*, rather than asking the rendered text what it looks like, cannot make that
/// mistake. ~keep
fn defers_to_field_type_default(field: &FieldDef) -> bool {
    if matches!(&field.ty, TypeRef::Named(_)) {
        return true;
    }
    matches!(&field.typed_default, Some(DefaultValue::EnumVariant(_)))
        && matches!(&field.ty, TypeRef::String | TypeRef::Char)
}

pub fn gen_rustler_kwargs_constructor_with_exclude(
    typ: &TypeDef,
    _type_mapper: &dyn Fn(&TypeRef) -> String,
    exclude_fields: &HashSet<String>,
) -> String {
    let fields: Vec<_> = constructor_fields(typ)
        .filter(|f| !exclude_fields.contains(&f.name))
        .map(|field| {
            let assignment = if field.optional {
                format!("opts.get(\"{}\").and_then(|t| t.decode().ok()),", field.name)
            } else if use_unwrap_or_default(field) {
                format!(
                    "opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                    field.name
                )
            } else {
                if defers_to_field_type_default(field) {
                    format!(
                        "opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                        field.name
                    )
                } else {
                    format!(
                        "opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or({}),",
                        field.name,
                        default_value_for_field_in_type(field, "rust", typ)
                    )
                }
            };

            crate::alef_context! {
                name => crate::codegen::naming::internal_rust_identifier(&field.name),
                assignment => assignment,
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/rustler_kwargs_constructor.jinja",
        crate::alef_context! {
            fields => fields,
        },
    )
}

/// Generate a Rustler (Elixir) kwargs constructor for a type with `has_default`.
/// Accepts keyword list or map, applies defaults for missing fields.
pub fn gen_rustler_kwargs_constructor(typ: &TypeDef, _type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let fields: Vec<_> = constructor_fields(typ)
        .map(|field| {
            let assignment = if field.optional {
                format!("opts.get(\"{}\").and_then(|t| t.decode().ok()),", field.name)
            } else if use_unwrap_or_default(field) {
                format!(
                    "opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                    field.name
                )
            } else {
                if defers_to_field_type_default(field) {
                    format!(
                        "opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                        field.name
                    )
                } else {
                    format!(
                        "opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or({}),",
                        field.name,
                        default_value_for_field_in_type(field, "rust", typ)
                    )
                }
            };

            crate::alef_context! {
                name => crate::codegen::naming::internal_rust_identifier(&field.name),
                assignment => assignment,
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/rustler_kwargs_constructor.jinja",
        crate::alef_context! {
            fields => fields,
        },
    )
}
