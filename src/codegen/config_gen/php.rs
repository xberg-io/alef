use crate::core::ir::{DefaultValue, TypeDef, TypeRef};

use super::shared::{constructor_fields, default_value_for_field_in_type, use_unwrap_or_default};

pub fn gen_php_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    gen_php_kwargs_constructor_with_rest(typ, type_mapper, false)
}

pub(crate) fn gen_php_kwargs_constructor_with_rest(
    typ: &TypeDef,
    type_mapper: &dyn Fn(&TypeRef) -> String,
    default_rest: bool,
) -> String {
    let fields: Vec<_> = constructor_fields(typ)
        .map(|field| {
            let mapped = type_mapper(&field.ty);
            let is_optional_field = field.optional
                || matches!(&field.ty, TypeRef::Optional(_))
                || (typ.has_default && matches!(field.ty, TypeRef::Duration));

            let assignment = if is_optional_field
                && typ.is_constructor_extension(field)
                && matches!(
                    field.typed_default,
                    Some(
                        DefaultValue::BoolLiteral(_)
                            | DefaultValue::IntLiteral(_)
                            | DefaultValue::FloatLiteral(_)
                            | DefaultValue::StringLiteral(_)
                    )
                ) {
                format!(
                    "{}.or_else(|| Some({}))",
                    field.name,
                    default_value_for_field_in_type(field, "rust", typ)
                )
            } else if is_optional_field {
                field.name.clone()
            } else if use_unwrap_or_default(field) {
                format!("{}.unwrap_or_default()", field.name)
            } else {
                let default_str = default_value_for_field_in_type(field, "rust", typ);
                format!("{}.unwrap_or({})", field.name, default_str)
            };

            crate::alef_context! {
                name => field.name.clone(),
                ty => mapped,
                init => crate::codegen::field_init::struct_field_init(&field.name, &assignment),
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/php_kwargs_constructor.jinja",
        crate::alef_context! {
            fields => fields, default_rest => default_rest,
        },
    )
}
