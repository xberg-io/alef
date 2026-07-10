//! Service-API codegen for the Java backend.
//!
//! Generates Java source files for service lifecycle and handler registration using Panama FFM:
//! - Service class wrapping opaque owner handles (via downcalls to C FFI symbols)
//! - Handler functional interface that accepts request JSON and returns response JSON
//! - Registration methods that build upcall stubs from handlers and invoke the C FFI
//! - Entrypoint methods (run/finalize) driving the service lifecycle
//!
//! Panama FFM Pattern:
//! - `Linker.nativeLinker()` + `SymbolLookup.libraryLookup(...)` to locate C symbols
//! - `downcallHandle()` + `FunctionDescriptor` for C function invocations
//! - `upcallStub()` + `MethodHandle` to wrap Java callbacks for C to call back into Java
//! - `Arena` for managing lifetime of callback stubs + context pointers
//! - String marshalling via `MemorySegment` + `getString()` / `CLinker.C_CHAR.byteSize()`

use crate::backends::java::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, ParamDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use minijinja::context;
use std::path::PathBuf;

/// Check if a TypeRef is an opaque (surface-wrapped Named type).
fn is_opaque_metadata(ty: &TypeRef, api: &ApiSurface) -> bool {
    matches!(ty, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n))
}

/// Map TypeRef to Java parameter type.
/// For Named types that are in the API surface, return the wrapper class name (opaque handle).
/// For String/Char/primitives, return the Java type.
fn java_type_for_metadata(ty: &TypeRef, api: &ApiSurface) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean".to_owned(),
                PrimitiveType::U8 | PrimitiveType::I8 => "byte".to_owned(),
                PrimitiveType::U16 | PrimitiveType::I16 => "short".to_owned(),
                PrimitiveType::U32 | PrimitiveType::I32 => "int".to_owned(),
                PrimitiveType::U64 | PrimitiveType::I64 => "long".to_owned(),
                PrimitiveType::F32 => "float".to_owned(),
                PrimitiveType::F64 => "double".to_owned(),
                PrimitiveType::Usize | PrimitiveType::Isize => "long".to_owned(),
            }
        }
        TypeRef::Bytes => "byte[]".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => n.clone(),
        _ => "Object".to_owned(),
    }
}

fn java_layout_for_metadata(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String => "ValueLayout.ADDRESS",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "ValueLayout.JAVA_LONG",
                PrimitiveType::U8 | PrimitiveType::I8 => "ValueLayout.JAVA_LONG",
                PrimitiveType::U16 | PrimitiveType::I16 => "ValueLayout.JAVA_LONG",
                PrimitiveType::U32 | PrimitiveType::I32 => "ValueLayout.JAVA_LONG",
                PrimitiveType::U64 | PrimitiveType::I64 => "ValueLayout.JAVA_LONG",
                PrimitiveType::F32 => "ValueLayout.JAVA_FLOAT",
                PrimitiveType::F64 => "ValueLayout.JAVA_DOUBLE",
                PrimitiveType::Usize | PrimitiveType::Isize => "ValueLayout.ADDRESS",
            }
        }
        _ => "ValueLayout.ADDRESS",
    }
}

/// Build a Vec of (layout, param_name) tuples for Jinja emission.
/// Jinja template is responsible for commas and newlines.
fn descriptor_layouts_vec(params: &[ParamDef]) -> Vec<(String, String)> {
    params
        .iter()
        .map(|param| {
            (
                java_layout_for_metadata(&param.ty).to_owned(),
                param.name.to_lower_camel_case(),
            )
        })
        .collect()
}

fn bool_arg_expr(param_name: &str) -> String {
    template_env::render(
        "service_bool_arg_expr.jinja",
        context! {
            param_name => param_name,
        },
    )
    .trim_end()
    .to_owned()
}

fn metadata_arg_expr(param: &ParamDef, api: &ApiSurface) -> String {
    let param_name = param.name.to_lower_camel_case();
    if is_opaque_metadata(&param.ty, api) {
        format!("{param_name}.handle()")
    } else if matches!(param.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
        bool_arg_expr(&param_name)
    } else {
        param_name
    }
}

fn metadata_arg_comment(param: &ParamDef, api: &ApiSurface, default_comment: &str) -> String {
    if is_opaque_metadata(&param.ty, api) {
        "opaque handle".to_owned()
    } else {
        default_comment.to_owned()
    }
}

/// Generate the idiomatic Java service class wrapper using Panama FFM.
///
/// The class exposes:
/// - Constructor that invokes the C FFI `{prefix}_{service}_new()` via downcall
/// - Registration methods that build upcall stubs from handlers and register them
/// - Run/Finalize entrypoint methods that invoke C FFI entrypoint downcalls
/// - AutoCloseable interface with close() to invoke the C FFI `_free()` downcall
/// - All Panama FFM binding details (Linker, downcallHandle, FunctionDescriptor, etc.)
fn gen_service_class(api: &ApiSurface, service: &ServiceDef, package: &str, config: &ResolvedCrateConfig) -> String {
    let mut out = String::new();

    let class_name = &service.name;
    let service_snake = service.name.to_snake_case();
    let ffi_prefix = config.ffi_prefix().to_lowercase();

    let mut bindings_doc = String::new();
    for reg in &service.registrations {
        bindings_doc.push_str(&template_env::render(
            "service_binding_doc_registration.jinja",
            context! {
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                method_snake => reg.method.to_snake_case(),
            },
        ));
    }
    for ep in &service.entrypoints {
        bindings_doc.push_str(&template_env::render(
            "service_binding_doc_entrypoint.jinja",
            context! {
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                method_snake => ep.method.to_snake_case(),
            },
        ));
    }

    out.push_str(&template_env::render(
        "service_class_header.jinja",
        context! {
            package => package,
            service_name => &service.name,
            service_snake => &service_snake,
            ffi_prefix => &ffi_prefix,
            bindings_doc => bindings_doc,
            class_name => class_name,
        },
    ));

    out.push_str(&template_env::render(
        "service_constructor.jinja",
        context! {
            service_name => &service.name,
            class_name => class_name,
            ffi_prefix => &ffi_prefix,
            service_snake => &service_snake,
        },
    ));

    for reg in &service.registrations {
        let reg_method = &reg.method;
        let reg_method_camel = reg_method.to_upper_camel_case();
        let reg_method_snake = reg_method.to_snake_case();

        let mut metadata_docs = String::new();
        let mut metadata_signature = String::new();
        for meta_param in &reg.metadata_params {
            let java_type = java_type_for_metadata(&meta_param.ty, api);
            let param_name = meta_param.name.to_lower_camel_case();
            metadata_docs.push_str(&template_env::render(
                "service_metadata_param_doc.jinja",
                context! {
                    param_name => &param_name,
                    java_type => &java_type,
                },
            ));
            let signature_param = template_env::render(
                "service_metadata_signature_param.jinja",
                context! {
                    java_type => &java_type,
                    param_name => &param_name,
                },
            );
            metadata_signature.push_str(signature_param.trim_end());
        }

        let descriptor_layouts_vec = descriptor_layouts_vec(&reg.metadata_params);
        let invoke_args_vec: Vec<_> = reg
            .metadata_params
            .iter()
            .map(|meta_param| {
                (
                    metadata_arg_expr(meta_param, api),
                    metadata_arg_comment(meta_param, api, "metadata"),
                )
            })
            .collect();

        out.push_str(&template_env::render(
            "service_registration_method.jinja",
            context! {
                reg_method => reg_method,
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                reg_method_snake => &reg_method_snake,
                metadata_docs => metadata_docs,
                method_name => format!("register{class_name}{reg_method_camel}"),
                metadata_signature => metadata_signature,
                class_name => class_name,
                descriptor_layouts => descriptor_layouts_vec,
                invoke_args => invoke_args_vec,
            },
        ));
    }

    for reg in &service.registrations {
        let reg_method_snake = reg.method.to_snake_case();
        for variant in &reg.variants {
            let variant_method_name = variant.name.to_lower_camel_case();
            let ffi_symbol = format!(
                "{}_{}_register_{}_{}",
                ffi_prefix,
                service_snake,
                reg_method_snake,
                variant.name.to_snake_case()
            );
            let doc = variant.doc.clone();

            let ctx = context! {
                method_name => variant_method_name.clone(),
                variant_name_display => variant.name.to_lower_camel_case(),
                ffi_symbol => ffi_symbol.clone(),
                doc => doc,
            };

            let rendered = template_env::render("registration_variant.java.jinja", ctx);
            out.push_str(&rendered);
            out.push_str("\n\n");
        }
    }

    for ep in &service.entrypoints {
        let ep_method = &ep.method;
        let ep_method_snake = ep_method.to_snake_case();

        let return_type = match ep.kind {
            EntrypointKind::Run => "void",
            EntrypointKind::Finalize => "long",
        };

        let params_signature = ep
            .params
            .iter()
            .map(|param| {
                let java_type = java_type_for_metadata(&param.ty, api);
                let param_name = param.name.to_lower_camel_case();
                format!("{java_type} {param_name}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let return_layout = match ep.kind {
            EntrypointKind::Run => "                ValueLayout.JAVA_INT,    // return int (status)\n",
            EntrypointKind::Finalize => {
                "                ValueLayout.ADDRESS,     // return *mut opaque or int status\n"
            }
        };
        let descriptor_layouts_vec = descriptor_layouts_vec(&ep.params);
        let invoke_args_vec: Vec<String> = ep
            .params
            .iter()
            .map(|param| {
                if is_opaque_metadata(&param.ty, api) {
                    format!("{}.handle()", param.name.to_lower_camel_case())
                } else if matches!(param.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
                    bool_arg_expr(&param.name.to_lower_camel_case())
                } else {
                    param.name.to_lower_camel_case()
                }
            })
            .collect();

        out.push_str(&template_env::render(
            "service_entrypoint_method.jinja",
            context! {
                ep_method => ep_method,
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                ep_method_snake => &ep_method_snake,
                return_type => return_type,
                params_signature => params_signature,
                return_layout => return_layout,
                descriptor_layouts => descriptor_layouts_vec,
                invoke_args => invoke_args_vec,
            },
        ));
    }

    out.push_str(&template_env::render(
        "service_config_method.jinja",
        context! {
            ffi_prefix => &ffi_prefix,
            service_snake => &service_snake,
        },
    ));

    out.push_str(&template_env::render(
        "service_close.jinja",
        context! {
            ffi_prefix => &ffi_prefix,
            service_snake => &service_snake,
        },
    ));

    out
}

/// Generate the @FunctionalInterface Callable interface.
///
/// A simple interface that handlers must implement to be passed to registration methods.
fn gen_callable_interface(package: &str) -> String {
    template_env::render("service_callable_interface.jinja", context! { package => package })
}

/// Generate all service-API files for the Java backend.
///
/// Returns Java source files using Panama FFM:
/// - One service class per [`ServiceDef`] (Panama downcalls + upcalls)
/// - One Callable interface (shared)
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let package = config.java_package();
    let package_path = package.replace('.', "/");

    let output_dir = config
        .output_for("java")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/java/src/main/java/".to_string());

    let base_path = if output_dir.ends_with(&package_path) || output_dir.ends_with(&format!("{}/", package_path)) {
        PathBuf::from(&output_dir)
    } else {
        PathBuf::from(&output_dir).join(&package_path)
    };

    let mut files = Vec::new();

    for service in &api.services {
        let service_class = gen_service_class(api, service, &package, config);
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.java", service.name)),
            content: service_class,
            generated_header: false,
        });
    }

    files.push(GeneratedFile {
        path: base_path.join("Callable.java"),
        content: gen_callable_interface(&package),
        generated_header: false,
    });

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
    };

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one registration, and Run entrypoint
    /// - One [`HandlerContractDef`] with wire request/response DTO names
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
            doc: "Register a request handler.".to_owned(),
            variants: vec![
                crate::core::ir::RegistrationVariant {
                    name: "get".to_owned(),
                    overrides: vec![],
                    wrapper_call: None,
                    signature_params: vec![],
                    doc: Some("Register a GET handler.".to_owned()),
                    style: Default::default(),
                    ..Default::default()
                },
                crate::core::ir::RegistrationVariant {
                    name: "post".to_owned(),
                    overrides: vec![],
                    wrapper_call: None,
                    signature_params: vec![],
                    doc: Some("Register a POST handler.".to_owned()),
                    style: Default::default(),
                    ..Default::default()
                },
            ],
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
            doc: "A test service owner.".to_owned(),
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
            doc: "Async trait for handling requests.".to_owned(),
        };

        ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![contract],
            ..ApiSurface::default()
        }
    }

    fn make_test_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "test-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }

    #[test]
    fn java_class_uses_panama_ffm() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(java.contains("import java.lang.foreign.*;"), "should import Panama FFM");
        assert!(java.contains("Linker.nativeLinker()"), "should use Linker");
        assert!(java.contains("downcallHandle"), "should use downcalls");
        assert!(java.contains("SymbolLookup"), "should lookup C symbols");
        assert!(java.contains("FunctionDescriptor"), "should build function descriptors");
        assert!(java.contains("MemorySegment"), "should use MemorySegment");
        assert!(java.contains("Arena"), "should use Arena for lifetime management");
    }

    #[test]
    fn java_class_contains_service_class() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(java.contains("public class TestService"));
        assert!(java.contains("implements AutoCloseable"));
        assert!(java.contains("private MemorySegment ownerHandle"));
    }

    #[test]
    fn java_class_constructor_uses_downcall() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(java.contains("public TestService()"));
        assert!(
            java.contains("test_crate_test_service_new"),
            "constructor should bind to C symbol"
        );
        assert!(
            java.contains("LINKER.downcallHandle"),
            "constructor should use downcall"
        );
    }

    #[test]
    fn java_class_contains_upcall_stub_for_handler() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(
            java.contains("LINKER.upcallStub"),
            "registration should build upcall stub for handler"
        );
        assert!(java.contains("MethodHandle"), "should use MethodHandle to wrap handler");
    }

    #[test]
    fn java_class_registration_binds_to_c_symbol() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(
            java.contains("test_crate_test_service_register_add_handler"),
            "registration should bind to exact C FFI symbol"
        );
    }

    #[test]
    fn java_class_entrypoint_uses_downcall() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(java.contains("public void run(String addr)"));
        assert!(
            java.contains("test_crate_test_service_ep_run"),
            "entrypoint should bind to C symbol"
        );
        assert!(java.contains("LINKER.downcallHandle"), "entrypoint should use downcall");
    }

    #[test]
    fn java_class_close_frees_via_downcall() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(java.contains("@Override"));
        assert!(java.contains("public void close()"));
        assert!(
            java.contains("test_crate_test_service_free"),
            "close should bind to C symbol"
        );
        assert!(java.contains("LINKER.downcallHandle"), "close should use downcall");
        assert!(java.contains("arena.close()"), "arena lifetime should be managed");
    }

    #[test]
    fn java_class_no_native_method_declarations() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(
            !java.contains("public native ")
                && !java.contains("private native ")
                && !java.contains("protected native ")
                && !java.contains("static native "),
            "should not contain JNI native method declarations:\n{java}"
        );
        assert!(
            !java.contains("System.loadLibrary"),
            "should not load library (Panama manages it)"
        );
        assert!(!java.contains("Java_"), "should not contain Java_ JNI symbols");
    }

    #[test]
    fn callable_interface_is_functional() {
        let iface = gen_callable_interface("com.example");

        assert!(iface.contains("@FunctionalInterface"));
        assert!(iface.contains("public interface Callable"));
        assert!(iface.contains("String handle(String request)"));
    }

    #[test]
    fn generate_returns_service_and_callable() {
        let surface = make_fixture_surface();
        let config = make_test_config();

        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.len() >= 2, "expected at least service class + Callable interface");

        let has_service_class = files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("TestService.java"));
        let has_callable = files.iter().any(|f| f.path.to_string_lossy().contains("Callable.java"));

        assert!(has_service_class, "expected TestService.java");
        assert!(has_callable, "expected Callable.java");
    }

    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();

        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    #[test]
    fn java_class_passes_all_metadata_params() {
        let mut surface = make_fixture_surface();
        let reg = &mut surface.services[0].registrations[0];

        reg.metadata_params.push(ParamDef {
            name: "method".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        });
        reg.metadata_params.push(ParamDef {
            name: "priority".to_owned(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            optional: false,
            default: None,
            ..ParamDef::default()
        });

        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(
            java.contains("public int registerTestServiceAddHandler(Callable handler, String path"),
            "registration method must include all metadata parameters"
        );

        assert!(
            java.contains("ValueLayout.ADDRESS") || java.contains("ValueLayout.JAVA_INT"),
            "registration should build FunctionDescriptor with metadata param layouts"
        );
    }

    #[test]
    fn java_class_emits_registration_variants() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

        assert!(
            java.contains("public int get(String path, Callable handler)"),
            "should emit get variant method"
        );
        assert!(
            java.contains("public int post(String path, Callable handler)"),
            "should emit post variant method"
        );

        assert!(
            java.contains("test_crate_test_service_register_add_handler_get"),
            "should bind get variant to correct C symbol"
        );
        assert!(
            java.contains("test_crate_test_service_register_add_handler_post"),
            "should bind post variant to correct C symbol"
        );

        assert!(
            java.contains("LINKER.downcallHandle"),
            "variant methods should use Panama downcalls"
        );
        assert!(
            java.contains("LINKER.upcallStub"),
            "variant methods should create upcall stubs"
        );
        assert!(
            java.contains("FunctionDescriptor.of"),
            "variant methods should build function descriptors"
        );
    }
}
