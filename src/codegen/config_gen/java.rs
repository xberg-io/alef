use crate::core::ir::{TypeDef, TypeRef};
use heck::ToPascalCase;

use super::shared::default_value_for_field;

pub fn gen_java_builder(typ: &TypeDef, package: &str, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let fields: Vec<_> = typ
        .fields
        .iter()
        .map(|field| {
            crate::alef_context! {
                name_lower => field.name.to_lowercase(),
                type => type_mapper(&field.ty),
                default => default_value_for_field(field, "java"),
                method_name => format!("with{}", field.name.to_pascal_case()),
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/java_builder.jinja",
        crate::alef_context! {
            package => package,
            type_name => typ.name.clone(),
            fields => fields,
        },
    )
    .trim_end_matches('\n')
    .to_string()
}
