use crate::core::ir::{TypeDef, TypeRef};

use super::shared::{constructor_fields, default_value_for_field, use_unwrap_or_default};

pub fn gen_php_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let fields: Vec<_> = constructor_fields(typ)
        .map(|field| {
            let mapped = type_mapper(&field.ty);
            let is_optional_field = field.optional || matches!(&field.ty, TypeRef::Optional(_));

            let assignment = if is_optional_field {
                field.name.clone()
            } else if use_unwrap_or_default(field) {
                format!("{}.unwrap_or_default()", field.name)
            } else {
                let default_str = default_value_for_field(field, "rust");
                format!("{}.unwrap_or({})", field.name, default_str)
            };

            minijinja::context! {
                name => field.name.clone(),
                ty => mapped,
                assignment => assignment,
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/php_kwargs_constructor.jinja",
        minijinja::context! {
            fields => fields,
        },
    )
}
