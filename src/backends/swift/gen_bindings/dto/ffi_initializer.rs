use super::*;
use crate::core::config::ResolvedCrateConfig;
use std::collections::HashMap;

pub(crate) type TextVariants = HashMap<String, (String, String)>;

pub(crate) fn text_variants(api: &ApiSurface, configured: &[String]) -> anyhow::Result<TextVariants> {
    api.enums
        .iter()
        .filter(|definition| configured.contains(&definition.name))
        .map(|definition| {
            let variant = definition
                .variants
                .iter()
                .find(|variant| variant.fields.len() == 1 && matches!(variant.fields[0].ty, TypeRef::String))
                .ok_or_else(|| {
                    anyhow::anyhow!("text-projected enum {} has no single-String variant", definition.name)
                })?;
            Ok((
                definition.name.clone(),
                (
                    super::super::enums::swift_variant_case(&variant.name),
                    super::super::enums::swift_associated_label(&variant.fields[0].name, 0),
                ),
            ))
        })
        .collect()
}

pub(crate) fn error_type_name(api: &ApiSurface, config: &ResolvedCrateConfig, module: &str) -> String {
    let name = api
        .errors
        .first()
        .map(|error| error.name.clone())
        .unwrap_or_else(|| config.error_type_name());
    if name == "Error" {
        format!("{module}Error")
    } else {
        name
    }
}

fn projected_read_expression(field: &FieldDef, variants: &TextVariants) -> Option<String> {
    let configured = variants.keys().map(String::as_str).collect();
    crate::backends::swift::gen_rust_crate::type_bridge::text_projected_getter_type(field, &configured)?;
    let named = match &field.ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    let TypeRef::Named(name) = named else { return None };
    variants.get(name)?;
    let names = crate::backends::swift::gen_rust_crate::type_bridge::text_projected_value_names(&field.name);
    Some(format!("try rb.{}()", names.accessor))
}

pub(super) fn assignments(context: Initializer<'_>) -> String {
    let mut output = String::new();
    for field in binding_fields(&context.ty.fields) {
        output.push_str(&crate::backends::swift::template_env::render(
            "swift_self_assignment.swift.jinja",
            crate::alef_context! { field => swift_case_ident(&field.name.to_lower_camel_case()),
            expr => context.expression(field) },
        ));
    }
    output
}

pub(super) struct Initializer<'a> {
    pub(super) ty: &'a TypeDef,
    pub(super) mapper: &'a SwiftMapper,
    pub(super) exclude_fields: &'a HashSet<String>,
    pub(super) known_dto_names: &'a HashSet<String>,
    pub(super) unit_enum_names: &'a HashSet<String>,
    pub(super) untagged_enum_names: &'a HashSet<String>,
    pub(super) serde_struct_names: &'a HashSet<String>,
    pub(super) error_type_name: &'a str,
    pub(super) text_variants: &'a TextVariants,
}

impl Initializer<'_> {
    fn expression(&self, field: &FieldDef) -> String {
        let optional = field.optional || matches!(field.ty, TypeRef::Optional(_));
        if is_field_unbridgeable_for_init(self.ty, field, self.exclude_fields, self.known_dto_names) {
            return if optional {
                "nil".to_string()
            } else {
                format!(
                    "try JSONDecoder().decode({}.self, from: Data(\"null\".utf8))",
                    self.mapper.map_type(&field.ty)
                )
            };
        }
        let accessor = swift_ident(&field.name.to_lower_camel_case());
        if field.optional && matches!(field.ty, TypeRef::Vec(_)) {
            return json_read(&accessor, &format!("{}?", self.mapper.map_type(&field.ty)), false);
        }
        if is_vec_of_serde_struct(&field.ty, self.serde_struct_names) {
            return self.serde_vector(field, &accessor, optional);
        }
        if let Some(expression) = projected_read_expression(field, self.text_variants) {
            return expression;
        }
        if is_untagged_enum_type(&field.ty, self.untagged_enum_names) {
            return json_read(
                &accessor,
                &optional_type(self.mapper.map_type(&field.ty), field),
                optional
                    && !crate::backends::swift::gen_rust_crate::type_bridge::field_needs_json_bridge(
                        &field.ty,
                        field.optional,
                    ),
            );
        }
        if needs_json_bridge_for_swift(&field.ty) {
            return json_read(
                &accessor,
                &optional_type(swift_field_type(self.mapper, &field.ty), field),
                false,
            );
        }
        swift_ffi_read_expr(
            &field.ty,
            optional,
            &accessor,
            self.known_dto_names,
            self.unit_enum_names,
            self.untagged_enum_names,
            self.error_type_name,
        )
    }

    fn serde_vector(&self, field: &FieldDef, accessor: &str, optional: bool) -> String {
        if optional {
            return json_read(accessor, &optional_type(self.mapper.map_type(&field.ty), field), false);
        }
        if let TypeRef::Vec(inner) = &field.ty
            && let TypeRef::Named(name) = inner.as_ref()
        {
            return crate::backends::swift::template_env::render(
                "swift_serde_vector_read.swift.jinja",
                crate::alef_context! { accessor => accessor, name => name },
            )
            .trim_end()
            .to_string();
        }
        format!("rb.{accessor}()")
    }
}

fn optional_type(swift_type: String, field: &FieldDef) -> String {
    if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
        format!("{swift_type}?")
    } else {
        swift_type
    }
}

fn json_read(accessor: &str, swift_type: &str, optional_accessor: bool) -> String {
    let source = if optional_accessor {
        format!("rb.{accessor}()?.toString() ?? \"null\"")
    } else {
        format!("rb.{accessor}().toString()")
    };
    format!("try JSONDecoder().decode({swift_type}.self, from: (({source}).data(using: .utf8) ?? Data(\"null\".utf8)))")
}
