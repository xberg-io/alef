use super::{swift_type_based_default, swift_typed_default_literal};
use crate::backends::swift::gen_bindings::zero_arg_default::zero_arg_named_default;
use crate::core::ir::{DefaultValue, FieldDef};
use std::collections::HashSet;

pub(super) fn non_optional_fallback(
    field: &FieldDef,
    swift_type: &str,
    zero_arg_constructible_names: &HashSet<String>,
    every_field_may_be_absent: bool,
) -> Option<String> {
    if !every_field_may_be_absent
        && !field.serde_skip_serializing_if
        && !field.has_bare_serde_default()
        && field.typed_default.is_none()
    {
        return None;
    }
    let default = field
        .typed_default
        .clone()
        .or_else(|| field.has_bare_serde_default().then_some(DefaultValue::Empty));
    default.as_ref().and_then(swift_typed_default_literal).or_else(|| {
        // An unresolved function or enum default cannot be replaced with the field type's zero. ~keep
        if matches!(
            default,
            Some(
                DefaultValue::FunctionCall(_)
                    | DefaultValue::PublicFunctionCall(_)
                    | DefaultValue::EnumVariant(_)
                    | DefaultValue::Unresolved(_)
                    | DefaultValue::TupleVariant(..)
                    | DefaultValue::StructVariant(..)
            )
        ) {
            return None;
        }
        zero_arg_named_default(&default, &field.ty, swift_type, zero_arg_constructible_names)
            .or_else(|| swift_type_based_default(&field.ty))
    })
}

pub(super) fn default_assignment_template(
    field: &FieldDef,
    every_field_may_be_absent: bool,
    had_owner_decoder: bool,
) -> &'static str {
    let had_fallback = field.typed_default.is_some()
        || ((every_field_may_be_absent || field.serde_skip_serializing_if)
            && swift_type_based_default(&field.ty).is_some());
    // Newly recognized defaults reject null; previously supported fallback policies stay unchanged. ~keep
    if had_owner_decoder && had_fallback {
        "swift_decode_default_assignment.swift.jinja"
    } else {
        "swift_decode_missing_default_assignment.swift.jinja"
    }
}

pub(super) fn optional_default_assignment_template(had_owner_decoder: bool) -> &'static str {
    if had_owner_decoder {
        "swift_decode_default_assignment.swift.jinja"
    } else {
        "swift_decode_missing_optional_default_assignment.swift.jinja"
    }
}
