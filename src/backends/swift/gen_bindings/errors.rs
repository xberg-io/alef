use crate::backends::swift::naming::swift_rust_shim_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ErrorDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

/// Emits a Swift `Swift.Error`-conforming `public enum` for the given `ErrorDef`.
///
/// When the Rust error type is named `Error`, Swift would parse `public enum Error: Error`
/// as a circular raw-type binding rather than protocol conformance. In that case the enum
/// is renamed to `{module_name}Error` (e.g. `SampleLanguagePackError`) to avoid the
/// clash. The protocol reference is always qualified as `Swift.Error` for clarity.
pub(super) fn emit_error(error: &ErrorDef, module_name: &str, out: &mut String, mapper: &SwiftMapper) {
    // Rename bare `Error` to `{ModuleName}Error` to avoid the Swift parser ambiguity
    // where `public enum Error: Error` is interpreted as a circular raw-type binding
    // instead of protocol conformance.
    let name = if error.name == "Error" {
        format!("{module_name}Error")
    } else {
        error.name.clone()
    };
    super::client::emit_doc_comment(&error.doc, "", out);
    out.push_str(&crate::backends::swift::template_env::render(
        "error_enum_header.jinja",
        minijinja::context! {
            name => &name,
        },
    ));
    for variant in &error.variants {
        super::client::emit_doc_comment(&variant.doc, "    ", out);
        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
        if variant.is_unit || variant.fields.is_empty() {
            out.push_str(&crate::backends::swift::template_env::render(
                "error_case.jinja",
                minijinja::context! {
                    case_name => &case_name,
                },
            ));
        } else {
            let mut assoc: Vec<String> = Vec::with_capacity(variant.fields.len() + 1);
            let mut seen_message = false;
            let mut labels: BTreeSet<String> = BTreeSet::new();
            for (idx, f) in variant.fields.iter().enumerate() {
                // Honor field.optional (extractor-unwrapped form) in addition to
                // TypeRef::Optional(inner) — both encode "nullable" in the IR.
                let already_optional = matches!(&f.ty, TypeRef::Optional(_));
                let ty_str = mapper.map_type(&f.ty);
                let ty_with_opt = if f.optional && !already_optional {
                    format!("{ty_str}?")
                } else {
                    ty_str
                };
                let mut label = super::enums::swift_associated_label(&f.name, idx);
                // Disambiguate duplicate labels by suffixing the index.
                while labels.contains(&label) {
                    label = format!("{label}{idx}");
                }
                labels.insert(label.clone());
                if label == "message" {
                    seen_message = true;
                }
                assoc.push(format!("{label}: {ty_with_opt}"));
            }
            if !seen_message {
                assoc.insert(0, "message: String".to_string());
            }
            out.push_str(&crate::backends::swift::template_env::render(
                "error_case_with_data.jinja",
                minijinja::context! {
                    case_name => &case_name,
                    associated_values => assoc.join(", "),
                },
            ));
        }
    }
    // Append a synthetic `validation(message:source:)` case used by DTO
    // first-class struct initializers (`dto.rs:675`) when a unit-serde-enum
    // raw value from the Rust bridge cannot be mapped to a known Swift
    // case. Without this the generated Swift refuses to compile against
    // any Rust error type that doesn't already declare this exact variant
    // (e.g. kreuzcrawl `CrawlError`, which has 18 domain variants but no
    // `Validation`). Skipped if the Rust error type already declares it.
    let has_validation_variant = error
        .variants
        .iter()
        .any(|v| swift_case_ident(&v.name.to_lower_camel_case()) == "validation");
    if !has_validation_variant {
        out.push_str("    /// Synthetic case raised when a Rust serde unit-enum raw value cannot\n");
        out.push_str("    /// be mapped to a known Swift case during DTO unmarshaling.\n");
        out.push_str("    case validation(message: String, source: String)\n");
    }
    out.push_str("}\n");
    // Emit a public extension with computed properties for each whitelisted
    // introspection method (e.g. `status_code`, `is_transient`, `error_type`).
    // Each property switches over `self` and delegates to the per-variant
    // associated values or returns a sensible default when the variant carries
    // no such field.  Backends that wire a swift-bridge free function can
    // replace these stubs in a subsequent code-generation pass.
    if !error.methods.is_empty() {
        out.push('\n');
        let mut properties = String::new();
        for method in &error.methods {
            let prop_name = swift_case_ident(&method.name.to_lower_camel_case());
            let return_ty = super::overloads::swift_type_name(&method.return_type);
            let default_val = swift_default_for_type(&method.return_type);
            let mut cases = String::new();
            for variant in &error.variants {
                let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
                // Check whether this variant carries an associated value whose
                // name matches the method (e.g. `status_code` ↔ `status`).
                let field_match = variant.fields.iter().find(|f| {
                    let camel = f.name.to_lower_camel_case();
                    let prop_snake = method.name.as_str();
                    // Exact match or common abbreviation (status_code → status).
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
                            if let Some(fm) = &field_match {
                                if fm.name == f.name {
                                    return format!("{label}: let matched");
                                }
                            }
                            format!("{label}: _")
                        })
                        .collect();
                    // The case declaration above synthesizes a leading
                    // `message: String` parameter when none of the original
                    // fields is named `message`.  The switch pattern must
                    // include the same synthetic label or the tuple lengths
                    // will mismatch.
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
                    minijinja::context! {
                        case_name => &case_name,
                        wildcard => &wildcard,
                        return_expression => &ret_expr,
                    },
                ));
            }
            // Cover the synthetic `validation(message:source:)` case (appended
            // above when not already declared by the user) so Swift's exhaustive
            // switch is satisfied for every introspection-method property.
            if !has_validation_variant {
                cases.push_str(&crate::backends::swift::template_env::render(
                    "swift_error_property_case.swift.jinja",
                    minijinja::context! {
                        case_name => "validation",
                        wildcard => "(message: _, source: _)",
                        return_expression => &default_val,
                    },
                ));
            }
            properties.push_str(&crate::backends::swift::template_env::render(
                "swift_error_property.swift.jinja",
                minijinja::context! {
                    property_name => &prop_name,
                    return_type => &return_ty,
                    cases => cases,
                },
            ));
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_error_extension.swift.jinja",
            minijinja::context! {
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
