use heck::ToShoutySnakeCase;

use crate::backends::ffi::template_env::render;
use crate::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};

pub(super) struct ContextFieldSpec {
    name: String,
    c_type: &'static str,
    c_init: ContextFieldInit,
    setup: Option<ContextFieldSetup>,
    doc: String,
}

#[derive(Clone, Copy)]
enum ContextFieldSetupKind {
    RequiredString,
    OptionalString,
}

struct ContextFieldSetup {
    name: String,
    kind: ContextFieldSetupKind,
}

#[derive(Clone, Copy)]
enum ContextFieldInitKind {
    RequiredString,
    OptionalString,
    Bool,
    Enum,
    Passthrough,
}

struct ContextFieldInit {
    name: String,
    kind: ContextFieldInitKind,
}

pub(super) fn gen_result_decode_arms(
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
    default_result: &str,
) -> String {
    let mut seen_codes = std::collections::HashSet::new();
    let mut arms = String::new();
    for variant in &result_metadata.unit_variants {
        if seen_codes.insert(variant.code) {
            arms.push_str(&render(
                "ffi_visitor_result_unit_arm.jinja",
                minijinja::context! {
                    code => variant.code,
                    variant_name => variant.name.clone(),
                },
            ));
        }
    }
    for variant in &result_metadata.string_payload_variants {
        if seen_codes.insert(variant.code) {
            arms.push_str(&render(
                "ffi_visitor_result_string_arm.jinja",
                minijinja::context! {
                    code => variant.code,
                    variant_name => variant.name.clone(),
                },
            ));
        }
    }
    arms.push_str(&render(
        "ffi_visitor_result_default_arm.jinja",
        minijinja::context! { default_result => default_result.to_owned() },
    ));
    arms
}

fn context_c_type(field: &FieldDef, api: &ApiSurface) -> Option<&'static str> {
    match (&field.ty, field.optional) {
        (TypeRef::String, false | true) => Some("*const std::ffi::c_char"),
        (TypeRef::Primitive(PrimitiveType::Bool), false) => Some("i32"),
        (TypeRef::Primitive(PrimitiveType::U8), false) => Some("u8"),
        (TypeRef::Primitive(PrimitiveType::U16), false) => Some("u16"),
        (TypeRef::Primitive(PrimitiveType::U32), false) => Some("u32"),
        (TypeRef::Primitive(PrimitiveType::U64), false) => Some("u64"),
        (TypeRef::Primitive(PrimitiveType::I8), false) => Some("i8"),
        (TypeRef::Primitive(PrimitiveType::I16), false) => Some("i16"),
        (TypeRef::Primitive(PrimitiveType::I32), false) => Some("i32"),
        (TypeRef::Primitive(PrimitiveType::I64), false) => Some("i64"),
        (TypeRef::Primitive(PrimitiveType::Usize), false) => Some("usize"),
        (TypeRef::Primitive(PrimitiveType::Isize), false) => Some("isize"),
        (TypeRef::Named(name), false) if api.enums.iter().any(|e| e.name == *name) => Some("i32"),
        _ => None,
    }
}

pub(super) fn context_field_specs(context_def: &TypeDef, api: &ApiSurface) -> Vec<ContextFieldSpec> {
    context_def
        .fields
        .iter()
        .filter_map(|field| {
            let Some(c_type) = context_c_type(field, api) else {
                eprintln!(
                    "[alef] gen_visitor(ffi): skip context field `{}.{}` with unsupported type {:?}",
                    context_def.name, field.name, field.ty
                );
                return None;
            };
            let doc = field
                .doc
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or("Context field.")
                .to_string();
            let setup = match (&field.ty, field.optional) {
                (TypeRef::String, false) => Some(ContextFieldSetup {
                    name: field.name.clone(),
                    kind: ContextFieldSetupKind::RequiredString,
                }),
                (TypeRef::String, true) => Some(ContextFieldSetup {
                    name: field.name.clone(),
                    kind: ContextFieldSetupKind::OptionalString,
                }),
                _ => None,
            };
            let c_init = match (&field.ty, field.optional) {
                (TypeRef::String, false) => ContextFieldInit {
                    name: field.name.clone(),
                    kind: ContextFieldInitKind::RequiredString,
                },
                (TypeRef::String, true) => ContextFieldInit {
                    name: field.name.clone(),
                    kind: ContextFieldInitKind::OptionalString,
                },
                (TypeRef::Primitive(PrimitiveType::Bool), false) => ContextFieldInit {
                    name: field.name.clone(),
                    kind: ContextFieldInitKind::Bool,
                },
                (TypeRef::Named(name), false) if api.enums.iter().any(|e| e.name == *name) => ContextFieldInit {
                    name: field.name.clone(),
                    kind: ContextFieldInitKind::Enum,
                },
                _ => ContextFieldInit {
                    name: field.name.clone(),
                    kind: ContextFieldInitKind::Passthrough,
                },
            };
            Some(ContextFieldSpec {
                name: field.name.clone(),
                c_type,
                c_init,
                setup,
                doc,
            })
        })
        .collect()
}

pub(super) fn gen_context_struct_fields(fields: &[ContextFieldSpec]) -> String {
    fields
        .iter()
        .map(|field| {
            render(
                "ffi_visitor_context_field.jinja",
                minijinja::context! {
                    doc => field.doc.as_str(),
                    name => field.name.as_str(),
                    c_type => field.c_type,
                },
            )
        })
        .collect()
}

pub(super) fn gen_context_setup(fields: &[ContextFieldSpec]) -> String {
    fields
        .iter()
        .filter_map(|field| field.setup.as_ref())
        .map(|setup| match setup.kind {
            ContextFieldSetupKind::RequiredString => render(
                "ffi_visitor_context_required_string_setup.jinja",
                minijinja::context! { name => setup.name.as_str() },
            ),
            ContextFieldSetupKind::OptionalString => render(
                "ffi_visitor_context_optional_string_setup.jinja",
                minijinja::context! { name => setup.name.as_str() },
            ),
        })
        .collect()
}

pub(super) fn gen_context_inits(fields: &[ContextFieldSpec]) -> String {
    fields
        .iter()
        .map(|field| {
            let template = match field.c_init.kind {
                ContextFieldInitKind::RequiredString => "ffi_visitor_context_required_string_init.jinja",
                ContextFieldInitKind::OptionalString => "ffi_visitor_context_optional_string_init.jinja",
                ContextFieldInitKind::Bool => "ffi_visitor_context_bool_init.jinja",
                ContextFieldInitKind::Enum => "ffi_visitor_context_enum_init.jinja",
                ContextFieldInitKind::Passthrough => "ffi_visitor_context_passthrough_init.jinja",
            };
            render(template, minijinja::context! { name => field.c_init.name.as_str() })
        })
        .collect()
}

pub(super) fn gen_result_constants(
    prefix: &str,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) -> String {
    let visit_prefix = prefix.to_uppercase();
    result_metadata
        .unit_variants
        .iter()
        .chain(result_metadata.string_payload_variants.iter())
        .map(|variant| {
            let constant_name = format!("{}_VISIT_{}", visit_prefix, variant.name.to_shouty_snake_case());
            render(
                "ffi_visitor_result_constant.jinja",
                minijinja::context! {
                    variant_name => variant.name.as_str(),
                    constant_name,
                    code => variant.code,
                },
            )
        })
        .collect()
}
