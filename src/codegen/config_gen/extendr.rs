use crate::core::ir::{TypeDef, TypeRef};

pub fn gen_extendr_kwargs_constructor(
    typ: &TypeDef,
    type_mapper: &dyn Fn(&TypeRef) -> String,
    enum_names: &ahash::AHashSet<String>,
) -> String {
    let is_named_enum = |ty: &TypeRef| -> bool { matches!(ty, TypeRef::Named(n) if enum_names.contains(n.as_str())) };
    let is_named_struct =
        |ty: &TypeRef| -> bool { matches!(ty, TypeRef::Named(n) if !enum_names.contains(n.as_str())) };
    let is_optional_named_struct = |ty: &TypeRef| -> bool {
        if let TypeRef::Optional(inner) = ty {
            is_named_struct(inner)
        } else {
            false
        }
    };
    let ty_is_optional = |ty: &TypeRef| -> bool { matches!(ty, TypeRef::Optional(_)) };

    let emittable_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| {
            !f.binding_excluded && f.cfg.is_none() && !is_named_struct(&f.ty) && !is_optional_named_struct(&f.ty)
        })
        .map(|field| {
            let param_type = if is_named_enum(&field.ty) {
                "Option<String>".to_string()
            } else if ty_is_optional(&field.ty) {
                type_mapper(&field.ty)
            } else {
                format!("Option<{}>", type_mapper(&field.ty))
            };

            minijinja::context! {
                name => field.name.clone(),
                type => param_type,
            }
        })
        .collect();

    let body_assignments: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded && f.cfg.is_none() && !is_named_struct(&f.ty) && !is_optional_named_struct(&f.ty))
        .map(|field| {
            let code = if is_named_enum(&field.ty) {
                if field.optional {
                    format!(
                        "if let Some(v) = {} {{ __out.{} = serde_json::from_str(&format!(\"\\\"{{v}}\\\"\")).ok(); }}",
                        field.name, field.name
                    )
                } else {
                    format!(
                        "if let Some(v) = {} {{ if let Ok(parsed) = serde_json::from_str(&format!(\"\\\"{{v}}\\\"\")) {{ __out.{} = parsed; }} }}",
                        field.name, field.name
                    )
                }
            } else if ty_is_optional(&field.ty) || field.optional {
                format!(
                    "if let Some(v) = {} {{ __out.{} = Some(v); }}",
                    field.name, field.name
                )
            } else {
                format!(
                    "if let Some(v) = {} {{ __out.{} = v; }}",
                    field.name, field.name
                )
            };

            minijinja::context! {
                code => code,
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/extendr_kwargs_constructor.jinja",
        minijinja::context! {
            type_name => typ.name.clone(),
            type_name_lower => typ.name.to_lowercase(),
            params => emittable_fields,
            body_assignments => body_assignments,
        },
    )
}
