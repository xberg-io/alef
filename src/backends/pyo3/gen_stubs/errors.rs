use crate::backends::pyo3::type_map::python_type;
use crate::codegen::error_gen::{
    ExceptionNaming, exception_families, pyo3_error_attribute_fields, python_exception_name,
};
use crate::codegen::naming::public_field_name;
use crate::core::config::Language;
use crate::core::ir::{ApiSurface, ErrorVariant};

pub(super) fn gen_error_stubs(api: &ApiSurface) -> anyhow::Result<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let mut lines = Vec::new();
    for family in exception_families(api, ExceptionNaming::Python)? {
        let error = &family.error;
        if seen.insert(error.name.clone()) {
            lines.push(render_exception(
                &error.name,
                family.parent.as_deref().unwrap_or("Exception"),
                None,
            ));
        }
        for variant in &error.variants {
            let name = python_exception_name(&variant.name, &error.name);
            if seen.insert(name.clone()) {
                lines.push(render_exception(&name, &error.name, Some(variant)));
            }
        }
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    Ok(lines)
}

fn render_exception(name: &str, base: &str, variant: Option<&ErrorVariant>) -> String {
    let fields = variant.map(pyo3_error_attribute_fields).unwrap_or_default();
    let fields = fields
        .iter()
        .map(|field| {
            let mut field_type = python_type(&field.ty);
            if field.optional && !field_type.contains("| None") {
                field_type.push_str(" | None");
            }
            crate::alef_context! {
                name => public_field_name(Language::Python, &field.name, None),
                python_type => field_type,
            }
        })
        .collect::<Vec<_>>();
    crate::backends::pyo3::template_env::render(
        "exception_variant_stub.jinja",
        crate::alef_context! { name => name, base => base, fields => fields },
    )
    .trim_end()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeRef;

    #[test]
    fn should_declare_named_error_attributes_in_python_stubs() {
        use crate::core::ir::{ErrorDef, ErrorVariant, FieldDef};
        let api = ApiSurface {
            errors: vec![ErrorDef {
                name: "RequestError".to_string(),
                variants: vec![ErrorVariant {
                    name: "Rejected".to_string(),
                    fields: vec![
                        FieldDef {
                            name: "status".to_string(),
                            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
                            optional: true,
                            ..Default::default()
                        },
                        FieldDef {
                            name: "body".to_string(),
                            ty: TypeRef::String,
                            core_wrapper: crate::core::ir::CoreWrapper::StaticStr,
                            optional: true,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                exception_base: None,
                rust_path: "sample::RequestError".to_string(),
                original_rust_path: String::new(),
                doc: String::new(),
                methods: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            ..Default::default()
        };
        let stub = gen_error_stubs(&api).expect("valid stub").join("\n");
        assert!(
            stub.contains("class RejectedError(RequestError):\n    status: int | None"),
            "{stub}"
        );
        assert!(stub.contains("    body: str | None"), "{stub}");
    }
}
