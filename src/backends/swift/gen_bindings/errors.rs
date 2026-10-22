use crate::backends::swift::naming::swift_rust_shim_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

/// The Swift name of the enum for `error`: `{module_name}Error` when the Rust error is named
/// `Error`, because `public enum Error: Error` would parse as a circular raw-type binding.
pub(super) fn swift_error_name(error: &ErrorDef, module_name: &str) -> String {
    if error.name == "Error" {
        format!("{module_name}Error")
    } else {
        error.name.clone()
    }
}

/// Whether the Rust error has a `status_code() -> u16` method.
fn has_status_code_method(error: &ErrorDef) -> bool {
    error.methods.iter().any(|method| {
        method.name == "status_code"
            && !method.is_static
            && !method.is_async
            && method.params.is_empty()
            && matches!(method.return_type, TypeRef::Primitive(PrimitiveType::U16))
    })
}

/// Whether `field` holds the HTTP status of its variant.
fn is_status_field(field: &FieldDef) -> bool {
    field.name == "status" || field.name == "status_code" || field.name.to_lower_camel_case() == "statusCode"
}

/// The error as the Swift enum declares it.
///
/// A Rust error with a `status_code` method reports the status of every variant, also of a
/// variant whose fields hold none (`RateLimited` is always 429). The Swift enum cannot compute
/// that status, so every variant without a status field gets a `status: UInt16` field. The
/// error converted from the bridge fills it and `statusCode` reads it back.
fn swift_view(error: &ErrorDef) -> ErrorDef {
    let mut view = error.clone();
    if !has_status_code_method(error) {
        return view;
    }
    for variant in &mut view.variants {
        if variant.fields.iter().any(is_status_field) {
            continue;
        }
        variant.is_unit = false;
        variant.fields.push(FieldDef {
            name: "status".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U16),
            ..FieldDef::default()
        });
    }
    view
}

/// One associated value of a Swift error case.
struct AssociatedValue<'a> {
    label: String,
    swift_type: String,
    /// The Rust field, or `None` for the synthetic `message` value.
    field: Option<&'a FieldDef>,
}

/// The associated values of the Swift case for `variant`, in declaration order. A variant without
/// a `message` field gets a leading `message: String`.
fn associated_values<'a>(variant: &'a ErrorVariant, mapper: &SwiftMapper) -> Vec<AssociatedValue<'a>> {
    let mut values: Vec<AssociatedValue<'a>> = Vec::with_capacity(variant.fields.len() + 1);
    let mut seen_message = false;
    let mut labels: BTreeSet<String> = BTreeSet::new();
    for (idx, f) in variant.fields.iter().enumerate() {
        let already_optional = matches!(&f.ty, TypeRef::Optional(_));
        let ty_str = mapper.map_type(&f.ty);
        let ty_with_opt = if f.optional && !already_optional {
            format!("{ty_str}?")
        } else {
            ty_str
        };
        let mut label = super::enums::swift_associated_label(&f.name, idx);
        while labels.contains(&label) {
            label = format!("{label}{idx}");
        }
        labels.insert(label.clone());
        if label == "message" {
            seen_message = true;
        }
        values.push(AssociatedValue {
            label,
            swift_type: ty_with_opt,
            field: Some(f),
        });
    }
    if !seen_message {
        values.insert(
            0,
            AssociatedValue {
                label: "message".to_string(),
                swift_type: "String".to_string(),
                field: None,
            },
        );
    }
    values
}

/// Emits a Swift `Swift.Error`-conforming `public enum` for the given `ErrorDef`.
///
/// When the Rust error type is named `Error`, Swift would parse `public enum Error: Error`
/// as a circular raw-type binding rather than protocol conformance. In that case the enum
/// is renamed to `{module_name}Error` (e.g. `SampleLanguagePackError`) to avoid the
/// clash. The protocol reference is always qualified as `Swift.Error` for clarity.
pub(super) fn emit_error(error: &ErrorDef, module_name: &str, out: &mut String, mapper: &SwiftMapper) {
    let name = swift_error_name(error, module_name);
    let error = &swift_view(error);
    super::client::emit_doc_comment(&error.doc, "", out);
    out.push_str(&crate::backends::swift::template_env::render(
        "error_enum_header.jinja",
        crate::alef_context! {
            name => &name,
        },
    ));
    for variant in &error.variants {
        super::client::emit_doc_comment(&variant.doc, "    ", out);
        let case_name = super::enums::swift_variant_case(&variant.name);
        if variant.is_unit || variant.fields.is_empty() {
            out.push_str(&crate::backends::swift::template_env::render(
                "error_case.jinja",
                crate::alef_context! {
                    case_name => &case_name,
                },
            ));
        } else {
            let assoc: Vec<String> = associated_values(variant, mapper)
                .iter()
                .map(|value| format!("{}: {}", value.label, value.swift_type))
                .collect();
            out.push_str(&crate::backends::swift::template_env::render(
                "error_case_with_data.jinja",
                crate::alef_context! {
                    case_name => &case_name,
                    associated_values => assoc.join(", "),
                },
            ));
        }
    }
    let has_validation_variant = error
        .variants
        .iter()
        .any(|v| super::enums::swift_variant_case(&v.name) == "validation");
    if !has_validation_variant {
        out.push_str("    /// Synthetic case raised when a Rust serde unit-enum raw value cannot\n");
        out.push_str("    /// be mapped to a known Swift case during DTO unmarshaling.\n");
        out.push_str("    case validation(message: String, source: String)\n");
    }
    out.push_str("}\n");
    if !error.methods.is_empty() {
        out.push('\n');
        let mut properties = String::new();
        for method in &error.methods {
            let prop_name = swift_case_ident(&method.name.to_lower_camel_case());
            let return_ty = super::overloads::swift_type_name(&method.return_type);
            let default_val = swift_default_for_type(&method.return_type);
            let mut cases = String::new();
            for variant in &error.variants {
                let case_name = super::enums::swift_variant_case(&variant.name);
                let field_match = variant.fields.iter().find(|f| {
                    let camel = f.name.to_lower_camel_case();
                    let prop_snake = method.name.as_str();
                    camel == prop_name
                        || f.name == prop_snake
                        || (prop_snake == "status_code" && (f.name == "status" || camel == "status"))
                });
                let wildcard = if variant.is_unit || variant.fields.is_empty() {
                    String::new()
                } else {
                    let mut args: Vec<String> = variant
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let label = super::enums::swift_associated_label(&f.name, i);
                            if let Some(fm) = &field_match
                                && fm.name == f.name
                            {
                                return format!("{label}: let matched");
                            }
                            format!("{label}: _")
                        })
                        .collect();
                    let has_message_field = variant.fields.iter().any(|f| f.name == "message");
                    if !has_message_field {
                        args.insert(0, "message: _".to_string());
                    }
                    format!("({})", args.join(", "))
                };
                let ret_expr = if field_match.is_some() && !variant.is_unit && !variant.fields.is_empty() {
                    "matched".to_string()
                } else {
                    default_val.clone()
                };
                cases.push_str(&crate::backends::swift::template_env::render(
                    "swift_error_property_case.swift.jinja",
                    crate::alef_context! {
                        case_name => &case_name,
                        wildcard => &wildcard,
                        return_expression => &ret_expr,
                    },
                ));
            }
            if !has_validation_variant {
                cases.push_str(&crate::backends::swift::template_env::render(
                    "swift_error_property_case.swift.jinja",
                    crate::alef_context! {
                        case_name => "validation",
                        wildcard => "(message: _, source: _)",
                        return_expression => &default_val,
                    },
                ));
            }
            properties.push_str(&crate::backends::swift::template_env::render(
                "swift_error_property.swift.jinja",
                crate::alef_context! {
                    property_name => &prop_name,
                    return_type => &return_ty,
                    cases => cases,
                },
            ));
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_error_extension.swift.jinja",
            crate::alef_context! {
                name => &name,
                properties => properties,
            },
        ));
    }
}

/// Returns the Swift zero/default literal for a given `TypeRef`.
pub(super) fn swift_default_for_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            }
        }
        TypeRef::String => "\"\"".to_string(),
        TypeRef::Optional(_) => "nil".to_string(),
        _ => "nil".to_string(),
    }
}

/// The Swift expression that gives a case value its place in a conversion from the bridge error,
/// or `None` when the value has no zero value (the variant is then left to the fallback case).
///
/// `message`, `status` and `retryAfterMs` are the constants the conversion reads from the bridge
/// error before its `switch`.
fn bridge_value_expression(value: &AssociatedValue<'_>) -> Option<String> {
    let Some(field) = value.field else {
        return Some("message".to_string());
    };
    if value.label == "message" {
        return Some("message".to_string());
    }
    if is_status_field(field) {
        let (inner, optional) = match &field.ty {
            TypeRef::Optional(inner) => (inner.as_ref(), true),
            ty => (ty, field.optional),
        };
        if !matches!(inner, TypeRef::Primitive(p) if !matches!(p, PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64))
        {
            return None;
        }
        let integer = SwiftMapper.map_type(inner);
        return Some(if optional {
            format!("status < 0 ? nil : {integer}(clamping: status)")
        } else {
            format!("{integer}(clamping: status)")
        });
    }
    if field.name == "retry_after" {
        match crate::codegen::error_gen::duration_shape(&field.ty) {
            Some(crate::codegen::error_gen::DurationShape::Optional) => {
                return Some("retryAfterMs < 0 ? nil : Duration.milliseconds(retryAfterMs)".to_string());
            }
            Some(crate::codegen::error_gen::DurationShape::Bare) => {
                return Some("Duration.milliseconds(max(retryAfterMs, 0))".to_string());
            }
            None => {}
        }
    }
    if field.optional || matches!(field.ty, TypeRef::Optional(_)) {
        return Some("nil".to_string());
    }
    match &field.ty {
        TypeRef::Primitive(PrimitiveType::Bool) => Some("false".to_string()),
        TypeRef::Primitive(_) => Some("0".to_string()),
        TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json => Some("\"\"".to_string()),
        TypeRef::Vec(_) => Some("[]".to_string()),
        TypeRef::Map(..) => Some("[:]".to_string()),
        TypeRef::Duration => Some("Duration.zero".to_string()),
        _ => None,
    }
}

/// The Swift expression that builds the case for `variant` from the bridge error constants, or
/// `None` when a value of the case has no zero value.
fn bridge_case_expression(variant: &ErrorVariant, mapper: &SwiftMapper) -> Option<String> {
    let case_name = super::enums::swift_variant_case(&variant.name);
    if variant.is_unit || variant.fields.is_empty() {
        return Some(format!(".{case_name}"));
    }
    let mut arguments: Vec<String> = Vec::new();
    for value in associated_values(variant, mapper) {
        arguments.push(format!("{}: {}", value.label, bridge_value_expression(&value)?));
    }
    Some(format!(".{case_name}({})", arguments.join(", ")))
}

/// Emits, for every error the bridge reports, the conversion from the error that the Rust bridge
/// throws, and the helpers that every bridged call goes through.
///
/// The conversion reads the kind, the HTTP status and the retry delay from the bridge error.
/// `alefTypedError` passes an error that is not a bridge error through unchanged.
pub(super) fn emit_bridge_conversions(errors: &[ErrorDef], module_name: &str, out: &mut String, mapper: &SwiftMapper) {
    #[derive(serde::Serialize)]
    struct ErrorName {
        rust: String,
        swift: String,
    }
    let converted: Vec<&ErrorDef> = errors.iter().filter(|error| !error.binding_excluded).collect();
    let mut names: Vec<ErrorName> = Vec::new();
    for error in &converted {
        let name = swift_error_name(error, module_name);
        let view = swift_view(error);
        let declared_validation = view
            .variants
            .iter()
            .find(|v| super::enums::swift_variant_case(&v.name) == "validation");
        let fallback = declared_validation
            .and_then(|v| bridge_case_expression(v, mapper))
            .unwrap_or_else(|| ".validation(message: message, source: \"\")".to_string());
        let mut cases = String::new();
        let mut kinds = String::new();
        for variant in &view.variants {
            let case_name = super::enums::swift_variant_case(&variant.name);
            kinds.push_str(&format!("        case .{case_name}: return \"{}\"\n", variant.name));
            if let Some(expression) = bridge_case_expression(variant, mapper) {
                cases.push_str(&format!(
                    "        case \"{}\":\n            self = {expression}\n",
                    variant.name
                ));
            }
        }
        if declared_validation.is_none() {
            kinds.push_str("        case .validation: return \"Validation\"\n");
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_error_bridge_conversion.swift.jinja",
            crate::alef_context! {
                name => &name,
                cases => cases,
                kinds => kinds,
                fallback => fallback,
                has_kind_method => error.methods.iter().any(|method| method.name == "kind"),
            },
        ));
        out.push('\n');
        names.push(ErrorName {
            rust: error.name.clone(),
            swift: name,
        });
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_error_bridge_helpers.swift.jinja",
        crate::alef_context! {
            errors => &names,
        },
    ));
    out.push('\n');
}
