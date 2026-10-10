use crate::core::ir::{TypeDef, TypeRef};

use super::shared::{constructor_fields, default_value_for_field_in_type};

pub fn gen_napi_defaults_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let fields: Vec<_> = constructor_fields(typ)
        .map(|field| {
            crate::alef_context! {
                name => field.name.clone(),
                type => type_mapper(&field.ty),
                default => default_value_for_field_in_type(field, "rust", typ),
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/napi_defaults_constructor.jinja",
        crate::alef_context! {
            fields => fields,
        },
    )
    .trim_end_matches('\n')
    .to_string()
}
