use crate::core::ir::{FieldDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};

pub(super) fn emit_optional_kwarg_helper(
    out: &mut String,
    type_name: &str,
    type_snake: &str,
    field: &FieldDef,
    parameter_name: &str,
) -> String {
    let helper_name = format!("_optional_{}_{}", type_snake, field.name.to_snake_case());
    let kwargs_name = format!("_{}{}Kwargs", type_name, field.name.to_pascal_case());
    let value_type = optional_kwarg_python_type(&field.ty);
    out.push_str(&crate::backends::pyo3::template_env::render(
        "converters/optional_kwarg_helper.jinja",
        crate::alef_context! {
            helper_name => &helper_name,
            kwargs_name => &kwargs_name,
            parameter_name => parameter_name,
            value_type => &value_type,
        },
    ));
    out.push_str("\n\n");
    helper_name
}

fn optional_kwarg_python_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named(name) => format!("_rust.{name}"),
        TypeRef::Vec(inner) => format!("list[{}]", optional_kwarg_python_type(inner)),
        TypeRef::Map(key, value) => format!(
            "dict[{}, {}]",
            optional_kwarg_python_type(key),
            optional_kwarg_python_type(value)
        ),
        TypeRef::Optional(inner) => optional_kwarg_python_type(inner),
        other => crate::backends::pyo3::type_map::python_type(other),
    }
}
