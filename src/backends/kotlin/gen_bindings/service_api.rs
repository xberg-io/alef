//! Service-API codegen for the Kotlin backend.
//!
//! Emits a coroutine-friendly wrapper that delegates to the Java facade
//! (`{java_package}.{ServiceName}`). The Kotlin wrapper:
//!
//! - Holds an `internal val inner: {java_package}.{ServiceName}` reference
//! - Exposes a no-arg secondary constructor that allocates the Java owner
//! - Re-exposes each registration as a Kotlin lambda taking
//!   `handler: (String) -> String`, converted to a `Callable` SAM and
//!   forwarded to the Java facade's `register{ClassName}{Method}` method
//! - Re-exposes each `Run` entrypoint as a `suspend fun` that hops to
//!   `Dispatchers.IO` before invoking the (blocking) Java call
//! - Re-exposes each representable `Finalize` entrypoint as a `fun`
//! - Implements `AutoCloseable` by forwarding to `inner.close()`
//!
//! The wrapper lives in the Kotlin client package (`{configured_kotlin_package}` or
//! `{java_package}.kt` when the two coincide), mirroring `emit_client_type_file` so
//! the kotlin coroutine surface and the JVM facade live side-by-side.

use crate::backends::kotlin::gen_bindings::shared;
use crate::backends::kotlin::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointDef, EntrypointKind, ParamDef, RegistrationVariant, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Whether an entrypoint's return type can be represented as a Kotlin return
/// (mirrors the gate in every host backend so non-representable `Finalize`
/// entrypoints — e.g. one returning an axum `Router` — are skipped).
fn entrypoint_return_representable(ep: &EntrypointDef, api: &ApiSurface) -> bool {
    match &ep.return_type {
        TypeRef::Unit | TypeRef::Primitive(_) => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n),
        _ => false,
    }
}

/// Whether a parameter type is a surface-wrapped opaque type — these are
/// passed through the Kotlin wrapper's `inner` field when forwarded to Java.
fn param_is_opaque_surface_type(param: &ParamDef, api: &ApiSurface) -> bool {
    matches!(&param.ty, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n))
}

/// Map a `TypeRef` to its Kotlin source representation for service-API params.
///
/// Surface-wrapped `Named` types resolve to the Kotlin wrapper class name
/// (the file emitted by `emit_client_type_file`), so the Kotlin caller works
/// in the coroutine wrapper world. The wrapper's `.inner` field is unwrapped
/// at the delegation site.
fn kotlin_type_for_param(ty: &TypeRef, api: &ApiSurface) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean".to_owned(),
                PrimitiveType::U8 | PrimitiveType::I8 => "Byte".to_owned(),
                PrimitiveType::U16 | PrimitiveType::I16 => "Short".to_owned(),
                PrimitiveType::U32 | PrimitiveType::I32 => "Int".to_owned(),
                PrimitiveType::U64 | PrimitiveType::I64 => "Long".to_owned(),
                PrimitiveType::F32 => "Float".to_owned(),
                PrimitiveType::F64 => "Double".to_owned(),
                PrimitiveType::Usize | PrimitiveType::Isize => "Long".to_owned(),
            }
        }
        TypeRef::Bytes => "ByteArray".to_owned(),
        TypeRef::Unit => "Unit".to_owned(),
        TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => n.clone(),
        TypeRef::Json => "Any".to_owned(),
        _ => "Any".to_owned(),
    }
}

/// Same mapping for `Finalize` entrypoint return types.
fn kotlin_return_type(ty: &TypeRef, api: &ApiSurface) -> String {
    kotlin_type_for_param(ty, api)
}

/// Translate a Rust enum path expression to a Kotlin/JVM enum access expression.
///
/// The `value_expr` stored on `RegistrationVariantOverride` is a fully-qualified
/// Rust path (e.g. `my_crate::Method::Get`).  The generated Java/Kotlin enum
/// keeps the Rust variant name as-is in PascalCase (`Method.Get`) because the
/// JVM codegen preserves Rust variant casing rather than converting to
/// `SCREAMING_SNAKE_CASE`.  This function strips the leading crate/module
/// segments and emits `{TypeName}.{VariantName}`.
fn rust_enum_expr_to_kotlin(value_expr: &str) -> String {
    let parts: Vec<&str> = value_expr.split("::").collect();
    match parts.as_slice() {
        [.., type_name, variant] => format!("{}.{}", type_name, variant),
        _ => value_expr.to_owned(),
    }
}

/// Emit a registration variant (shortcut method) for the given variant.
///
/// `java_package` is needed when the variant has a wrapper constructor: the
/// wrapper's Java factory method is called via the fully-qualified Java class
/// (`{java_package}.{TypeName}.create(...)`) and the result is wrapped in the
/// Kotlin coroutine wrapper (`{TypeName}(javaInstance)`).
fn gen_registration_variant(
    out: &mut String,
    variant: &RegistrationVariant,
    base_reg: &crate::core::ir::RegistrationDef,
    _class_name: &str,
    java_package: &str,
) {
    let variant_method_kt = variant.name.to_lower_camel_case();
    let base_method_kt = base_reg.method.to_lower_camel_case();

    let handler_param = "handler: (String) -> String".to_owned();

    let mut param_parts = vec![handler_param];
    param_parts.extend(variant.signature_params.iter().map(|p| {
        format!(
            "{}: {}",
            p.name.to_lower_camel_case(),
            kotlin_type_for_param(&p.ty, &ApiSurface::default()),
        )
    }));
    let params = param_parts.join(", ");

    let args_str = if let Some(wc) = &variant.wrapper_call {
        let ctor_args: Vec<String> = wc
            .args
            .iter()
            .map(|arg| match arg {
                crate::core::ir::WrapperConstructorArg::Fixed { value_expr, .. } => {
                    rust_enum_expr_to_kotlin(value_expr)
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => param.name.to_lower_camel_case(),
            })
            .collect();
        let type_name = &wc.wrapper_type_name;
        let java_factory = format!("{java_package}.{type_name}.create({})", ctor_args.join(", "));
        let wrapper_expr = format!("{type_name}({java_factory})");
        format!("handler, {wrapper_expr}")
    } else {
        let mut args = vec!["handler".to_owned()];
        for ov in &variant.overrides {
            args.push(rust_enum_expr_to_kotlin(&ov.value_expr));
        }
        for param in &variant.signature_params {
            args.push(param.name.to_lower_camel_case());
        }
        args.join(", ")
    };

    let ctx = minijinja::context! {
        variant_method_kt => variant_method_kt,
        params => params,
        base_method_kt => base_method_kt,
        args => args_str,
        variant => variant,
    };
    let rendered = template_env::render("registration_variant.kt.jinja", ctx);
    out.push_str(&rendered);
    out.push('\n');
}

/// Generate the coroutine-friendly Kotlin wrapper for a single service.
fn gen_service_kotlin(api: &ApiSurface, service: &ServiceDef, package: &str, java_package: &str) -> String {
    let class_name = service.name.to_upper_camel_case();
    let java_fqn = format!("{java_package}.{class_name}");

    let mut imports: BTreeSet<String> = BTreeSet::new();
    imports.insert(format!("import {java_package}.Callable"));

    let has_run = service
        .entrypoints
        .iter()
        .any(|ep| matches!(ep.kind, EntrypointKind::Run));
    if has_run {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }

    let mut body = String::new();

    body.push_str(&template_env::render(
        "service_class_header.jinja",
        minijinja::context! {
            java_fqn => java_fqn,
            class_name => class_name,
        },
    ));

    for reg in &service.registrations {
        let reg_method_kt = reg.method.to_lower_camel_case();
        let reg_method_camel = reg.method.to_upper_camel_case();
        let java_method = format!("register{class_name}{reg_method_camel}");

        let mut params: Vec<String> = vec!["handler: (String) -> String".to_owned()];
        for meta in &reg.metadata_params {
            params.push(format!(
                "{}: {}",
                meta.name.to_lower_camel_case(),
                kotlin_type_for_param(&meta.ty, api)
            ));
        }

        let mut args: Vec<String> = vec!["Callable { request -> handler(request) }".to_owned()];
        for meta in &reg.metadata_params {
            let name = meta.name.to_lower_camel_case();
            if param_is_opaque_surface_type(meta, api) {
                args.push(format!("{name}.inner"));
            } else {
                args.push(name);
            }
        }

        if !reg.doc.is_empty() {
            for line in reg.doc.lines() {
                body.push_str(&template_env::render(
                    "line_comment.jinja",
                    minijinja::context! {
                        indent => "    ",
                        line => line,
                    },
                ));
            }
        }
        body.push_str(&template_env::render(
            "service_registration_method.jinja",
            minijinja::context! {
                method_name => reg_method_kt,
                params => params.join(", "),
                java_method => java_method,
                args => args.join(", "),
            },
        ));

        for variant in &reg.variants {
            gen_registration_variant(&mut body, variant, reg, &class_name, java_package);
        }
    }

    for ep in &service.entrypoints {
        if matches!(ep.kind, EntrypointKind::Finalize) && !entrypoint_return_representable(ep, api) {
            continue;
        }

        let ep_method_kt = ep.method.to_lower_camel_case();
        let params: Vec<String> = ep
            .params
            .iter()
            .map(|p| {
                format!(
                    "{}: {}",
                    p.name.to_lower_camel_case(),
                    kotlin_type_for_param(&p.ty, api)
                )
            })
            .collect();
        let args: Vec<String> = ep
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_lower_camel_case();
                if param_is_opaque_surface_type(p, api) {
                    format!("{name}.inner")
                } else {
                    name
                }
            })
            .collect();

        if !ep.doc.is_empty() {
            for line in ep.doc.lines() {
                body.push_str(&template_env::render(
                    "line_comment.jinja",
                    minijinja::context! {
                        indent => "    ",
                        line => line,
                    },
                ));
            }
        }

        match ep.kind {
            EntrypointKind::Run => {
                body.push_str(&template_env::render(
                    "service_run_method.jinja",
                    minijinja::context! {
                        method_name => ep_method_kt,
                        params => params.join(", "),
                        args => args.join(", "),
                    },
                ));
            }
            EntrypointKind::Finalize => {
                let ret = kotlin_return_type(&ep.return_type, api);
                body.push_str(&template_env::render(
                    "service_finalize_method.jinja",
                    minijinja::context! {
                        method_name => ep_method_kt,
                        params => params.join(", "),
                        args => args.join(", "),
                        return_type => ret,
                        unit_return => ret == "Unit",
                    },
                ));
            }
        }
    }

    body.push_str("    override fun close() { inner.close() }\n");
    body.push_str("}\n");

    shared::assemble_kt_file(package, &imports, &body)
}

/// Generate all service-API files for the Kotlin backend.
///
/// One `GeneratedFile` per service: `{kotlin_root}/src/main/kotlin/{package_path}/{ClassName}.kt`.
/// The package is `config.kotlin_package()` unless that coincides with
/// `config.java_package()`, in which case `.kt` is appended (mirroring
/// `emit_jvm_client_class_with_package`).
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let java_package = config.java_package();
    let configured_kotlin_package = config.kotlin_package();
    let package = if configured_kotlin_package == java_package {
        format!("{configured_kotlin_package}.kt")
    } else {
        configured_kotlin_package.clone()
    };
    let package_path = package.replace('.', "/");

    let kotlin_root = config
        .output_for("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin".to_string());
    let kotlin_root_path = PathBuf::from(&kotlin_root);

    let mut files = Vec::new();
    for service in &api.services {
        let class_name = service.name.to_upper_camel_case();
        let content = gen_service_kotlin(api, service, &package, &java_package);
        let file_name = format!("{class_name}.kt");
        let path = if config.explicit_output.kotlin.is_some() {
            kotlin_root_path.join(&file_name)
        } else {
            kotlin_root_path
                .join("src/main/kotlin")
                .join(&package_path)
                .join(&file_name)
        };
        files.push(GeneratedFile {
            path,
            content,
            generated_header: false,
        });
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
    };

    fn make_fixture_surface() -> ApiSurface {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create a new service owner.".to_owned(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Register a handler.".to_owned(),
            variants: vec![],
            ..Default::default()
        };

        let run_ep = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![ParamDef {
                name: "addr".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("ServiceError".to_owned()),
            doc: "Run the service.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![run_ep],
            doc: "A test service.".to_owned(),
            cfg: None,
        };

        let dispatch_method = MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "request".to_owned(),
                ty: TypeRef::Named("RequestData".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("ResponseData".to_owned()),
            is_async: true,
            is_static: false,
            error_type: Some("HandlerError".to_owned()),
            doc: "Dispatch a request.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: dispatch_method,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("ResponseData".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Handler contract.".to_owned(),
        };

        ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![contract],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn coroutine_wrapper_has_package_and_imports() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("package com.example.kt"));
        assert!(kt.contains("import com.example.Callable"));
        assert!(kt.contains("import kotlinx.coroutines.Dispatchers"));
        assert!(kt.contains("import kotlinx.coroutines.withContext"));
    }

    #[test]
    fn coroutine_wrapper_declares_class_and_constructors() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains(
            "class TestService internal constructor(internal val inner: com.example.TestService) : AutoCloseable"
        ));
        assert!(kt.contains("constructor() : this(com.example.TestService())"));
    }

    #[test]
    fn coroutine_wrapper_delegates_registration_via_sam() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("fun addHandler(handler: (String) -> String, path: String): Int"));
        assert!(kt.contains("inner.registerTestServiceAddHandler(Callable { request -> handler(request) }, path)"));
    }

    #[test]
    fn coroutine_wrapper_run_is_suspend_on_io_dispatcher() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("suspend fun run(addr: String) = withContext(Dispatchers.IO) { inner.run(addr) }"));
    }

    #[test]
    fn coroutine_wrapper_forwards_close() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("override fun close() { inner.close() }"));
    }

    #[test]
    fn coroutine_wrapper_emits_no_jni_artifacts() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(!kt.contains("external fun"));
        assert!(!kt.contains("System.loadLibrary"));
        assert!(!kt.contains("nativeTestService"));
        assert!(!kt.contains("Java_"));
    }

    #[test]
    fn coroutine_wrapper_finalize_representable_uses_return() {
        let mut api = make_fixture_surface();
        api.services[0].entrypoints.push(EntrypointDef {
            method: "shutdown".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
            error_type: None,
            doc: "Shutdown the service.".to_owned(),
        });
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("fun shutdown(): Long = inner.shutdown()"));
    }

    #[test]
    fn coroutine_wrapper_finalize_non_representable_is_skipped() {
        let mut api = make_fixture_surface();
        api.services[0].entrypoints.push(EntrypointDef {
            method: "into_router".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Named("axum::Router".to_owned()),
            error_type: None,
            doc: "Take the router.".to_owned(),
        });
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(!kt.contains("intoRouter"));
    }

    #[test]
    fn coroutine_wrapper_opaque_metadata_unwraps_inner() {
        let mut api = make_fixture_surface();
        api.types.push(crate::core::ir::TypeDef {
            name: "RouteBuilder".to_owned(),
            rust_path: "my_crate::RouteBuilder".to_owned(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
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
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        });
        api.services[0].registrations[0].metadata_params.push(ParamDef {
            name: "builder".to_owned(),
            ty: TypeRef::Named("RouteBuilder".to_owned()),
            optional: false,
            default: None,
            ..ParamDef::default()
        });
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("builder: RouteBuilder"));
        assert!(kt.contains("builder.inner"));
    }

    #[test]
    fn coroutine_wrapper_has_no_stubs() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(!kt.contains("placeholder"));
        assert!(!kt.contains("stub"));
        assert!(!kt.contains("placeholder"));
    }

    #[test]
    fn generate_emits_one_file_per_service() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(!files.is_empty(), "expected at least one file");
        let has_service_file = files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("TestService.kt"));
        assert!(has_service_file, "expected TestService.kt in output");
    }

    #[test]
    fn generate_is_empty_when_no_services() {
        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    #[test]
    fn coroutine_wrapper_emits_registration_variants() {
        let mut api = make_fixture_surface();
        api.services[0].registrations[0].variants.push(RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![crate::core::ir::RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "HttpMethod.GET".to_owned(),
            }],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET handler.".to_owned()),
            style: Default::default(),
            ..Default::default()
        });
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("fun get(handler: (String) -> String, path: String): Int"));
        assert!(kt.contains("Register a GET handler."));
        assert!(!kt.contains("Register a GET handler.    fun get"));
        assert!(kt.contains("addHandler(handler, HttpMethod.GET, path)"));
    }

    #[test]
    fn coroutine_wrapper_variant_with_wrapper_call() {
        let mut api = make_fixture_surface();
        api.services[0].registrations[0].variants.push(RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: Some(crate::core::ir::WrapperConstructorCall {
                metadata_param: "builder".to_owned(),
                wrapper_type_path: "my_crate::RouteBuilder".to_owned(),
                wrapper_type_name: "RouteBuilder".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![
                    crate::core::ir::WrapperConstructorArg::Fixed {
                        param_name: "method".to_owned(),
                        value_expr: "my_crate::Method::Get".to_owned(),
                    },
                    crate::core::ir::WrapperConstructorArg::Free {
                        param: ParamDef {
                            name: "path".to_owned(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            ..ParamDef::default()
                        },
                    },
                ],
            }),
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET route.".to_owned()),
            style: Default::default(),
            ..Default::default()
        });
        let service = &api.services[0];
        let kt = gen_service_kotlin(&api, service, "com.example.kt", "com.example");

        assert!(kt.contains("fun get(handler: (String) -> String, path: String): Int"));
        assert!(kt.contains("com.example.RouteBuilder.create(Method.Get, path)"));
        assert!(kt.contains("RouteBuilder(com.example.RouteBuilder.create(Method.Get, path))"));
        assert!(kt.contains("addHandler(handler, RouteBuilder(com.example.RouteBuilder.create(Method.Get, path))"));
    }
}
