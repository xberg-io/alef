//! Service-API codegen for the wasm-bindgen backend.
//!
//! Generates JavaScript/WebAssembly glue that exposes registration methods
//! for handler variant styles (VerbDecorator, Builder, Hybrid) and coordinates
//! with Rust-side service execution.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, RegistrationDef, RegistrationVariant, RegistrationVariantStyle, TypeRef};
use std::path::PathBuf;

/// Convert a `TypeRef` to a JavaScript type annotation string.
fn js_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "number".to_owned(),
                _ => "number".to_owned(),
            }
        }
        TypeRef::Bytes => "Uint8Array".to_owned(),
        TypeRef::Optional(inner) => format!("{} | undefined", js_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("{}[]", js_type_annotation(inner)),
        TypeRef::Map(k, v) => format!("Record<{}, {}>", js_type_annotation(k), js_type_annotation(v)),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "any".to_owned(),
        TypeRef::Path => "string".to_owned(),
        TypeRef::Duration => "number".to_owned(),
    }
}

/// Generate the JavaScript service wrapper (`service.js`).
///
/// For wasm-bindgen, this exports a class that:
/// - Manages handler registrations in a list
/// - Exposes registration methods matching the handler contract's variant style
/// - Provides a `run()` entrypoint that coordinates with Rust-side execution
pub(super) fn gen_service_js(api: &ApiSurface) -> String {
    if api.services.is_empty() {
        return String::new();
    }

    let service = &api.services[0];
    let mut out = String::new();

    let class_name = "App";
    out.push_str(&crate::backends::wasm::template_env::render(
        "service_js_class_open.jinja",
        minijinja::context! {
            class_name => class_name,
        },
    ));

    let constructor_params: Vec<String> = service
        .constructor
        .params
        .iter()
        .map(|p| {
            let ty = js_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} = undefined", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    let constructor_param_names: Vec<&str> = service
        .constructor
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    out.push_str(&crate::backends::wasm::template_env::render(
        "service_js_constructor.jinja",
        minijinja::context! {
            constructor_params => constructor_params.join(", "),
            params => constructor_param_names,
        },
    ));

    for method in &service.configurators {
        let method_params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let ty = js_type_annotation(&p.ty);
                if p.optional {
                    format!("{}: {} = undefined", p.name, ty)
                } else {
                    format!("{}: {}", p.name, ty)
                }
            })
            .collect();

        out.push_str(&crate::backends::wasm::template_env::render(
            "service_js_configurator.jinja",
            minijinja::context! {
                method_name => &method.name,
                method_params => method_params.join(", "),
                doc => method.doc.as_str(),
            },
        ));
    }

    for reg in &service.registrations {
        for variant in &reg.variants {
            gen_registration_variant_js(&mut out, variant, reg);
        }
    }

    out.push_str("  run() {\n");
    out.push_str("    // Coordinate with Rust-side service execution\n");
    out.push_str("    // (impl-specific: may spawn server, call native function, etc.)\n");
    out.push_str("  }\n");

    out.push_str("}\n");

    out
}

/// Emit registration variant methods for a single variant,
/// respecting the `RegistrationVariantStyle`.
fn gen_registration_variant_js(out: &mut String, variant: &RegistrationVariant, _reg: &RegistrationDef) {
    let variant_name = &variant.name;

    let variant_params_no_handler: Vec<String> = variant
        .signature_params
        .iter()
        .map(|p| {
            let ty = js_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} = undefined", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            emit_variant_direct_method_js(out, variant_name, &variant_params_no_handler, variant);
        }
        RegistrationVariantStyle::Builder => {
            emit_variant_decorator_factory_js(out, variant_name, &variant_params_no_handler, variant);
        }
        RegistrationVariantStyle::Hybrid
        | RegistrationVariantStyle::Decorator
        | RegistrationVariantStyle::Attribute
        | RegistrationVariantStyle::Dsl => {
            emit_variant_direct_method_js(out, variant_name, &variant_params_no_handler, variant);
            emit_variant_decorator_factory_js(out, variant_name, &variant_params_no_handler, variant);
        }
    }
}

/// Emit the direct method form: `app.get(path, handler): this`.
fn emit_variant_direct_method_js(
    out: &mut String,
    variant_name: &str,
    variant_params: &[String],
    variant: &RegistrationVariant,
) {
    let mut full_params = variant_params.to_vec();
    full_params.push("handler: (...args: any[]) => any".to_string());
    let full_sig = full_params.join(", ");

    let default_doc;
    let doc = if let Some(doc) = variant.doc.as_deref() {
        doc
    } else {
        default_doc = format!("Register a {variant_name} callback directly.");
        &default_doc
    };
    let doc_lines = variant_doc_lines(doc);
    out.push_str(&crate::backends::wasm::template_env::render(
        "service_js_direct_variant.jinja",
        minijinja::context! {
            doc_lines => doc_lines,
            variant_name => variant_name,
            full_sig => full_sig,
            variant_name_json => format!("{variant_name:?}"),
            metadata_args => variant_metadata_args(variant_params),
        },
    ));
}

/// Emit the decorator-factory form: `app.get(path): (handler) => any`.
fn emit_variant_decorator_factory_js(
    out: &mut String,
    variant_name: &str,
    variant_params: &[String],
    variant: &RegistrationVariant,
) {
    let sig = variant_params.join(", ");

    let default_doc;
    let doc = if let Some(doc) = variant.doc.as_deref() {
        doc
    } else {
        default_doc = format!("Register a {variant_name} callback via decorator factory.");
        &default_doc
    };
    let doc_lines = variant_doc_lines(doc);
    out.push_str(&crate::backends::wasm::template_env::render(
        "service_js_decorator_variant.jinja",
        minijinja::context! {
            doc_lines => doc_lines,
            variant_name => variant_name,
            sig => sig,
            variant_name_json => format!("{variant_name:?}"),
            metadata_args => variant_metadata_args(variant_params),
        },
    ));
}

fn variant_doc_lines(doc: &str) -> Vec<String> {
    doc.trim().lines().map(str::trim).map(str::to_owned).collect()
}

fn variant_metadata_args(variant_params: &[String]) -> String {
    variant_params
        .iter()
        .map(|param| param.split(':').next().unwrap().trim())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate the Rust wasm-bindgen glue (`service.rs`).
///
/// Exports a Rust function that accepts the service registration list
/// from JavaScript, builds the core service, and runs it.
pub(super) fn gen_service_rs(api: &ApiSurface, _config: &ResolvedCrateConfig) -> String {
    if api.services.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    out.push_str("#![allow(clippy::too_many_arguments)]\n\n");
    out.push_str("use wasm_bindgen::prelude::*;\n\n");

    out.push_str("/// Initialize the service with registered handlers.\n");
    out.push_str("#[wasm_bindgen]\n");
    out.push_str("pub fn init_service(registrations: JsValue) -> Result<(), JsValue> {\n");
    out.push_str("    // Implementation: deserialize registrations, build service, wire handlers\n");
    out.push_str("    Ok(())\n");
    out.push_str("}\n\n");

    out.push_str("/// Run the service.\n");
    out.push_str("#[wasm_bindgen]\n");
    out.push_str("pub async fn run_service() -> Result<(), JsValue> {\n");
    out.push_str("    // Implementation: await service.run()\n");
    out.push_str("    Ok(())\n");
    out.push_str("}\n");

    out
}

/// Generate all service-related files for the wasm backend.
///
/// Services with "wasm" in their `skip_languages` config entry are excluded —
/// no files are generated for them and `pub mod service;` must not be emitted.
pub fn gen_service_files(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<GeneratedFile> {
    let mut files = Vec::new();

    let active_services: Vec<_> = api
        .services
        .iter()
        .filter(|svc| {
            !config
                .services
                .iter()
                .any(|sc| sc.owner_type == svc.name && sc.skip_languages.iter().any(|l| l == "wasm"))
        })
        .collect();

    if active_services.is_empty() {
        return files;
    }

    let mut api_active = api.clone();
    api_active.services = active_services.into_iter().cloned().collect();

    let js_content = gen_service_js(&api_active);
    files.push(GeneratedFile {
        path: PathBuf::from("src/service.js"),
        content: js_content,
        generated_header: true,
    });

    let rs_content = gen_service_rs(&api_active, config);
    files.push(GeneratedFile {
        path: PathBuf::from("src/service.rs"),
        content: rs_content,
        generated_header: true,
    });

    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::RegistrationVariantStyle;

    #[test]
    fn test_wasm_registration_verb_decorator_only_emits_direct_method() {
        let mut out = String::new();
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: None,
            signature_params: vec![],
            doc: Some("Register a get handler".to_owned()),
            style: RegistrationVariantStyle::VerbDecorator,
            ..Default::default()
        };

        emit_variant_direct_method_js(&mut out, "get", &[], &variant);

        assert!(out.contains("get(path: string, handler:") || out.contains("get(handler:"));
        assert!(out.contains("this._registrations.push"));
        assert!(out.contains("return this;"));
    }

    #[test]
    fn test_wasm_registration_builder_only_emits_decorator_factory() {
        let mut out = String::new();
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: None,
            signature_params: vec![],
            doc: Some("Register a get handler".to_owned()),
            style: RegistrationVariantStyle::Builder,
            ..Default::default()
        };

        emit_variant_decorator_factory_js(&mut out, "get", &[], &variant);

        assert!(out.contains("return (fn:"));
        assert!(out.contains("return fn;"));
    }

    #[test]
    fn test_wasm_registration_hybrid_emits_both_forms() {
        let mut out = String::new();
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: None,
            signature_params: vec![],
            doc: Some("Register a get handler".to_owned()),
            style: RegistrationVariantStyle::Hybrid,
            ..Default::default()
        };

        emit_variant_direct_method_js(&mut out, "get", &[], &variant);
        emit_variant_decorator_factory_js(&mut out, "get", &[], &variant);

        assert!(out.contains("handler:"));
        assert!(out.contains("return (fn:"));
    }

    #[test]
    fn test_wasm_js_type_annotation() {
        assert_eq!(js_type_annotation(&TypeRef::String), "string");
        assert_eq!(js_type_annotation(&TypeRef::Char), "string");
        assert_eq!(
            js_type_annotation(&TypeRef::Optional(Box::new(TypeRef::String))),
            "string | undefined"
        );
        assert_eq!(js_type_annotation(&TypeRef::Vec(Box::new(TypeRef::String))), "string[]");
    }
}
