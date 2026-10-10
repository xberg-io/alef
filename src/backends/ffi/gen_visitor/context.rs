use heck::ToShoutySnakeCase;

use crate::backends::ffi::template_env::render;
use crate::codegen::visitor_context_abi::{ContextAbiField, ContextFieldShape, context_abi};
use crate::core::ir::{ApiSurface, TypeDef};

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
                crate::alef_context! {
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
                crate::alef_context! {
                    code => variant.code,
                    variant_name => variant.name.clone(),
                },
            ));
        }
    }
    arms.push_str(&render(
        "ffi_visitor_result_default_arm.jinja",
        crate::alef_context! { default_result => default_result.to_owned() },
    ));
    arms
}

/// The `#[repr(C)]` context fields, in the order and at the offsets host bindings read them.
///
/// Delegates to the shared derivation so the Java bridge decodes exactly the struct emitted here.
pub(super) fn context_field_specs(context_def: &TypeDef, api: &ApiSurface) -> Vec<ContextAbiField> {
    context_abi(context_def, api).fields
}

pub(super) fn gen_context_struct_fields(fields: &[ContextAbiField]) -> String {
    fields
        .iter()
        .map(|field| {
            render(
                "ffi_visitor_context_field.jinja",
                crate::alef_context! {
                    doc => field.doc.as_str(),
                    name => field.name.as_str(),
                    c_type => field.scalar.rust_c_type(),
                },
            )
        })
        .collect()
}

pub(super) fn gen_context_setup(fields: &[ContextAbiField]) -> String {
    fields
        .iter()
        .filter_map(|field| match field.shape {
            ContextFieldShape::RequiredString => Some(render(
                "ffi_visitor_context_required_string_setup.jinja",
                crate::alef_context! { name => field.name.as_str() },
            )),
            ContextFieldShape::OptionalString => Some(render(
                "ffi_visitor_context_optional_string_setup.jinja",
                crate::alef_context! { name => field.name.as_str() },
            )),
            ContextFieldShape::Bool | ContextFieldShape::Enum | ContextFieldShape::Integer => None,
        })
        .collect()
}

pub(super) fn gen_context_inits(fields: &[ContextAbiField]) -> String {
    fields
        .iter()
        .map(|field| {
            let template = match field.shape {
                ContextFieldShape::RequiredString => "ffi_visitor_context_required_string_init.jinja",
                ContextFieldShape::OptionalString => "ffi_visitor_context_optional_string_init.jinja",
                ContextFieldShape::Bool => "ffi_visitor_context_bool_init.jinja",
                ContextFieldShape::Enum => "ffi_visitor_context_enum_init.jinja",
                ContextFieldShape::Integer => "ffi_visitor_context_passthrough_init.jinja",
            };
            render(template, crate::alef_context! { name => field.name.as_str() })
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
                crate::alef_context! {
                    variant_name => variant.name.as_str(),
                    constant_name,
                    code => variant.code,
                },
            )
        })
        .collect()
}
