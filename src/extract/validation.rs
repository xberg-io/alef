//! Validation for extracted API surfaces before code generation.

use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, MethodDef, ParamDef, TypeRef};
use anyhow::bail;

const SUGGESTED_FIX: &str = "Expose a binding-safe DTO/newtype for this Rust type, include the referenced type in \
the binding surface, or mark the item with #[cfg_attr(alef, alef(skip))] or #[doc(hidden)].";

/// A lossy public API item that would otherwise be passed to binding generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedPublicApiDiagnostic {
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}

/// Fail when the public binding surface contains sanitized items.
pub fn validate_no_sanitized_public_items(api: &ApiSurface) -> anyhow::Result<()> {
    let diagnostics = sanitized_public_api_diagnostics(api);
    if diagnostics.is_empty() {
        return Ok(());
    }

    bail!("{}", format_sanitized_public_api_error(&diagnostics));
}

/// Return diagnostics for sanitized public fields, parameters, returns, functions, and methods.
pub fn sanitized_public_api_diagnostics(api: &ApiSurface) -> Vec<SanitizedPublicApiDiagnostic> {
    let mut diagnostics = Vec::new();

    for function in &api.functions {
        if function.binding_excluded {
            continue;
        }
        collect_function_diagnostics(function, &mut diagnostics);
    }

    for typ in &api.types {
        if typ.binding_excluded {
            continue;
        }
        for field in &typ.fields {
            if !field.binding_excluded {
                collect_field_diagnostic(&format!("{}.{}", typ.name, field.name), field, &mut diagnostics);
            }
        }
        for method in &typ.methods {
            if !method.binding_excluded {
                collect_method_diagnostics(&format!("{}.{}", typ.name, method.name), method, &mut diagnostics);
            }
        }
    }

    for enum_def in &api.enums {
        if enum_def.binding_excluded {
            continue;
        }
        for variant in &enum_def.variants {
            if variant.binding_excluded {
                continue;
            }
            for field in &variant.fields {
                if !field.binding_excluded {
                    collect_field_diagnostic(
                        &format!("{}::{}.{}", enum_def.name, variant.name, field.name),
                        field,
                        &mut diagnostics,
                    );
                }
            }
        }
    }

    for error_def in &api.errors {
        if error_def.binding_excluded {
            continue;
        }
        for variant in &error_def.variants {
            for field in &variant.fields {
                if !field.binding_excluded {
                    collect_field_diagnostic(
                        &format!("{}::{}.{}", error_def.name, variant.name, field.name),
                        field,
                        &mut diagnostics,
                    );
                }
            }
        }
        for method in &error_def.methods {
            if !method.binding_excluded {
                collect_method_diagnostics(&format!("{}.{}", error_def.name, method.name), method, &mut diagnostics);
            }
        }
    }

    diagnostics
}

fn collect_function_diagnostics(function: &FunctionDef, diagnostics: &mut Vec<SanitizedPublicApiDiagnostic>) {
    let path = format!("function {}", function.name);
    if function.sanitized {
        diagnostics.push(diagnostic(
            path.clone(),
            "function signature contains a sanitized parameter or return type".to_string(),
        ));
    }
    for param in &function.params {
        collect_param_diagnostic(&format!("{path} parameter {}", param.name), param, diagnostics);
    }
    if function.return_sanitized {
        diagnostics.push(diagnostic(
            format!("{path} return"),
            format!(
                "return type was sanitized to `{}`",
                type_ref_label(&function.return_type)
            ),
        ));
    }
}

fn collect_method_diagnostics(path: &str, method: &MethodDef, diagnostics: &mut Vec<SanitizedPublicApiDiagnostic>) {
    if method.sanitized {
        diagnostics.push(diagnostic(
            format!("method {path}"),
            "method signature contains a sanitized parameter or return type".to_string(),
        ));
    }
    for param in &method.params {
        collect_param_diagnostic(&format!("method {path} parameter {}", param.name), param, diagnostics);
    }
    if method.sanitized && !method.params.iter().any(|param| param.sanitized) {
        diagnostics.push(diagnostic(
            format!("method {path} return"),
            format!("return type was sanitized to `{}`", type_ref_label(&method.return_type)),
        ));
    }
}

fn collect_field_diagnostic(path: &str, field: &FieldDef, diagnostics: &mut Vec<SanitizedPublicApiDiagnostic>) {
    if field.sanitized {
        diagnostics.push(diagnostic(
            format!("field {path}"),
            sanitized_type_reason("field type", field.original_type.as_deref(), &field.ty),
        ));
    }
}

fn collect_param_diagnostic(path: &str, param: &ParamDef, diagnostics: &mut Vec<SanitizedPublicApiDiagnostic>) {
    if param.sanitized {
        diagnostics.push(diagnostic(
            path.to_string(),
            sanitized_type_reason("parameter type", param.original_type.as_deref(), &param.ty),
        ));
    }
}

fn diagnostic(item_path: String, reason: String) -> SanitizedPublicApiDiagnostic {
    SanitizedPublicApiDiagnostic {
        item_path,
        reason,
        suggested_fix: SUGGESTED_FIX.to_string(),
    }
}

fn sanitized_type_reason(kind: &str, original_type: Option<&str>, sanitized_type: &TypeRef) -> String {
    let sanitized_type = type_ref_label(sanitized_type);
    match original_type {
        Some(original_type) => format!("{kind} `{original_type}` was sanitized to `{sanitized_type}`"),
        None => format!("{kind} was sanitized to `{sanitized_type}`"),
    }
}

fn format_sanitized_public_api_error(diagnostics: &[SanitizedPublicApiDiagnostic]) -> String {
    let mut message = String::from("public API validation failed: lossy sanitized items would be generated");
    for diagnostic in diagnostics {
        message.push_str("\n- item: ");
        message.push_str(&diagnostic.item_path);
        message.push_str("\n  reason: ");
        message.push_str(&diagnostic.reason);
        message.push_str("\n  suggested fix: ");
        message.push_str(&diagnostic.suggested_fix);
    }
    message
}

fn type_ref_label(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(primitive) => format!("{primitive:?}"),
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "PathBuf".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_label(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_label(inner)),
        TypeRef::Map(key, value) => format!("Map<{}, {}>", type_ref_label(key), type_ref_label(value)),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Duration => "Duration".to_string(),
        TypeRef::Char => "char".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef};

    fn sanitized_field(name: &str, original_type: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: true,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: Default::default(),
            vec_inner_core_wrapper: Default::default(),
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: Some(original_type.to_string()),
        }
    }

    #[test]
    fn reports_sanitized_fields_params_returns_and_items() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "render".to_string(),
                rust_path: "sample::render".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "request".to_string(),
                    ty: TypeRef::String,
                    sanitized: true,
                    original_type: Some("FrameworkRequest".to_string()),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: true,
                return_sanitized: true,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            types: vec![crate::core::ir::TypeDef {
                name: "Config".to_string(),
                rust_path: "sample::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![sanitized_field("timeout", "Duration")],
                methods: vec![MethodDef {
                    name: "build".to_string(),
                    params: Vec::new(),
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: None,
                    sanitized: true,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: Vec::new(),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            }],
            ..ApiSurface::default()
        };

        let error = validate_no_sanitized_public_items(&api)
            .expect_err("sanitized public API items must fail validation")
            .to_string();

        assert!(error.contains("item: function render"), "{error}");
        assert!(error.contains("item: function render parameter request"), "{error}");
        assert!(error.contains("item: function render return"), "{error}");
        assert!(error.contains("item: field Config.timeout"), "{error}");
        assert!(error.contains("item: method Config.build"), "{error}");
        assert!(error.contains("item: method Config.build return"), "{error}");
        assert!(error.contains("suggested fix:"), "{error}");
    }

    #[test]
    fn allows_items_explicitly_binding_excluded() {
        let mut excluded_field = sanitized_field("internal", "InternalType");
        excluded_field.binding_excluded = true;

        let api = ApiSurface {
            types: vec![crate::core::ir::TypeDef {
                name: "Config".to_string(),
                rust_path: "sample::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![excluded_field],
                methods: vec![MethodDef {
                    name: "internal".to_string(),
                    params: Vec::new(),
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: None,
                    sanitized: true,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: true,
                    binding_exclusion_reason: Some("test exclusion".to_string()),
                    version: Default::default(),
                }],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: Vec::new(),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            }],
            functions: vec![FunctionDef {
                name: "internal_fn".to_string(),
                rust_path: "sample::internal_fn".to_string(),
                original_rust_path: String::new(),
                params: Vec::new(),
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: true,
                return_sanitized: true,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: true,
                binding_exclusion_reason: Some("test exclusion".to_string()),
                version: Default::default(),
            }],
            ..ApiSurface::default()
        };

        validate_no_sanitized_public_items(&api).expect("binding-excluded sanitized items must be allowed");
    }

    #[test]
    fn reports_sanitized_enum_and_error_variant_fields() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "Event".to_string(),
                rust_path: "sample::Event".to_string(),
                original_rust_path: String::new(),
                variants: vec![EnumVariant {
                    name: "Created".to_string(),
                    fields: vec![sanitized_field("payload", "InternalPayload")],
                    ..EnumVariant::default()
                }],
                methods: vec![],
                doc: String::new(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                has_default: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
            }],
            errors: vec![ErrorDef {
                name: "Error".to_string(),
                rust_path: "sample::Error".to_string(),
                original_rust_path: String::new(),
                variants: vec![ErrorVariant {
                    name: "Invalid".to_string(),
                    message_template: None,
                    fields: vec![sanitized_field("source", "InternalError")],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                    doc: String::new(),
                }],
                doc: String::new(),
                methods: Vec::new(),
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            ..ApiSurface::default()
        };

        let paths: Vec<_> = sanitized_public_api_diagnostics(&api)
            .into_iter()
            .map(|diagnostic| diagnostic.item_path)
            .collect();

        assert_eq!(
            paths,
            vec!["field Event::Created.payload", "field Error::Invalid.source"]
        );
    }

    #[test]
    fn skips_binding_excluded_enum_variants_with_sanitized_fields() {
        // Regression: a variant marked `#[cfg_attr(alef, alef(skip))]` or
        // `#[doc(hidden)]` wraps an internal type that sanitizes to String.  The
        let excluded_variant = EnumVariant {
            name: "Code".to_string(),
            fields: vec![sanitized_field("_0", "CodeMetadataInner")],
            binding_excluded: true,
            binding_exclusion_reason: Some("alef(skip)".to_string()),
            ..EnumVariant::default()
        };
        let public_variant = EnumVariant {
            name: "Document".to_string(),
            ..EnumVariant::default()
        };

        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "FormatMetadata".to_string(),
                rust_path: "sample::FormatMetadata".to_string(),
                original_rust_path: String::new(),
                variants: vec![excluded_variant, public_variant],
                methods: vec![],
                doc: String::new(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                has_default: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
            }],
            ..ApiSurface::default()
        };

        let diagnostics = sanitized_public_api_diagnostics(&api);
        assert!(
            diagnostics.is_empty(),
            "binding-excluded variant must not produce diagnostics; got: {diagnostics:?}"
        );
    }
}
