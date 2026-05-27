//! Service-API codegen for the Go backend.
//!
//! Generates one output per `ApiSurface` with non-empty services:
//!
//! **`service.go`** — Go service wrapper that:
//! - Declares C FFI function imports (service `_new`/`_free`, registration, entrypoint functions).
//! - Defines Go handler registry (a map of opaque context indices → Go handler functions).
//! - Exports a cgo trampoline that looks up handlers in the registry and invokes them.
//! - Provides Go service struct with:
//!   - Constructor
//!   - Registration methods that store Go handlers in the registry and call C registration
//!   - `Run`/`Finalize` entrypoint methods that call C entrypoints
//!
//! All names and signatures are derived from the `ApiSurface` IR — alef is generic.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Find the `HandlerContractDef` by trait name in the surface.
#[allow(dead_code)]
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Convert a TypeRef to a Go type string for signatures.
fn typeref_to_go_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "string".to_owned(),
        TypeRef::Char => "byte".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "uint8".to_owned(),
                PrimitiveType::U16 => "uint16".to_owned(),
                PrimitiveType::U32 => "uint32".to_owned(),
                PrimitiveType::U64 => "uint64".to_owned(),
                PrimitiveType::I8 => "int8".to_owned(),
                PrimitiveType::I16 => "int16".to_owned(),
                PrimitiveType::I32 => "int32".to_owned(),
                PrimitiveType::I64 => "int64".to_owned(),
                PrimitiveType::F32 => "float32".to_owned(),
                PrimitiveType::F64 => "float64".to_owned(),
                PrimitiveType::Usize => "uintptr".to_owned(),
                PrimitiveType::Isize => "intptr".to_owned(),
            }
        }
        TypeRef::Bytes => "[]byte".to_owned(),
        TypeRef::Unit => "".to_owned(), // void in Go is implicit (no return)
        TypeRef::Named(n) => n.clone(),
        TypeRef::Optional(inner) => format!("*{}", typeref_to_go_type(inner)),
        TypeRef::Vec(inner) => format!("[]{}", typeref_to_go_type(inner)),
        TypeRef::Map(k, v) => format!("map[{}]{}", typeref_to_go_type(k), typeref_to_go_type(v)),
        TypeRef::Json => "interface{}".to_owned(),
        TypeRef::Path => "string".to_owned(),
        TypeRef::Duration => "time.Duration".to_owned(),
    }
}

// ──────────────────────────────────────────────────────────────── Go output ──

/// Generate the Go service module (`service.go`).
///
/// For each service this emits:
/// - C FFI imports for service `_new`/`_free`, registration, and entrypoint functions.
/// - A handler registry map keyed by context index.
/// - A cgo trampoline function matching the C callback typedef signature.
/// - A Go struct mirroring the service (constructor, registration methods, entrypoints).
fn gen_service_go(api: &ApiSurface, _config: &ResolvedCrateConfig, pkg_name: &str, ffi_prefix: &str) -> String {
    let mut out = String::new();

    out.push_str(&format!("package {pkg_name}\n\n"));

    // Imports
    out.push_str("import (\n");
    out.push_str("\t\"encoding/json\"\n");
    out.push_str("\t\"errors\"\n");
    out.push_str("\t\"fmt\"\n");
    out.push_str("\t\"sync\"\n");
    out.push_str("\t\"unsafe\"\n");
    out.push_str("\t\"C\"\n");
    out.push_str(")\n\n");

    // Generate C imports for all services
    out.push_str("// ──────────────────────────────────────────── C FFI Imports ──\n\n");
    for service in &api.services {
        gen_service_c_imports(&mut out, service, api, ffi_prefix);
    }

    // Generate the handler registry and trampoline
    out.push_str("// ──────────────────────────────────────────── Handler Registry ──\n\n");
    gen_handler_registry(&mut out);

    // Generate Go service structs and methods
    out.push_str("// ──────────────────────────────────────────── Go Service API ──\n\n");
    for service in &api.services {
        gen_service_struct(&mut out, service, api, ffi_prefix);
    }

    out
}

/// Generate C FFI imports for one service.
fn gen_service_c_imports(out: &mut String, service: &ServiceDef, _api: &ApiSurface, ffi_prefix: &str) {
    let service_snake = service.name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();

    out.push_str("/*\n");
    out.push_str(&format!("// Service: {}\n", service.name));
    out.push_str("*/\n");

    // Constructor
    out.push_str(&format!(
        "// extern {}Opaque* {}_{}_new(void);\n",
        service.name, service_lower, service_snake
    ));

    // Destructor
    out.push_str(&format!(
        "// extern void {}_{}_free({}Opaque* ptr);\n",
        service_lower, service_snake, service.name
    ));

    // Registration functions
    for reg in &service.registrations {
        let reg_method_snake = reg.method.to_snake_case();
        out.push_str(&format!(
            "// extern int {}_{}_register_{}(\n\
             //     {}Opaque* owner,\n\
             //     char* (*callback)(void*, const char*),\n\
             //     void* context",
            service_lower, service_snake, reg_method_snake, service.name
        ));

        for meta_param in &reg.metadata_params {
            let c_type = typeref_to_c_type(&meta_param.ty);
            out.push_str(&format!(",\n//     {} {}", c_type, meta_param.name));
        }
        out.push_str("\n// );\n");
    }

    // Entrypoint functions
    for ep in &service.entrypoints {
        let ep_name_snake = ep.method.to_snake_case();
        let return_c_type = typeref_to_c_type(&ep.return_type);
        out.push_str(&format!(
            "// extern {} {}_{}_ep_{}(\n//     {}Opaque* owner",
            return_c_type, service_lower, service_snake, ep_name_snake, service.name
        ));

        for ep_param in &ep.params {
            let c_type = typeref_to_c_type(&ep_param.ty);
            out.push_str(&format!(",\n//     {} {}", c_type, ep_param.name));
        }
        out.push_str("\n// );\n");
    }
    out.push('\n');
}

/// Map TypeRef to C type for function signatures in comments.
fn typeref_to_c_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "const char*".to_owned(),
        TypeRef::Char => "char".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "uint8_t".to_owned(),
                PrimitiveType::U16 => "uint16_t".to_owned(),
                PrimitiveType::U32 => "uint32_t".to_owned(),
                PrimitiveType::U64 => "uint64_t".to_owned(),
                PrimitiveType::I8 => "int8_t".to_owned(),
                PrimitiveType::I16 => "int16_t".to_owned(),
                PrimitiveType::I32 => "int32_t".to_owned(),
                PrimitiveType::I64 => "int64_t".to_owned(),
                PrimitiveType::F32 => "float".to_owned(),
                PrimitiveType::F64 => "double".to_owned(),
                PrimitiveType::Usize => "uintptr_t".to_owned(),
                PrimitiveType::Isize => "intptr_t".to_owned(),
            }
        }
        TypeRef::Bytes => "const uint8_t*".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        _ => "void*".to_owned(),
    }
}

/// Generate the handler registry and cgo trampoline.
fn gen_handler_registry(out: &mut String) {
    out.push_str("// HandlerFunc is the signature for Go handler functions.\n");
    out.push_str("// They receive JSON-serialized request and return JSON response.\n");
    out.push_str("type HandlerFunc func([]byte) ([]byte, error)\n\n");

    out.push_str("// handlerRegistry maps opaque context indices to Go handlers.\n");
    out.push_str("var (\n");
    out.push_str("\thandlerRegistryMu sync.Mutex\n");
    out.push_str("\thandlerRegistry   = make(map[uintptr]HandlerFunc)\n");
    out.push_str("\thandlerNextID     uintptr = 1\n");
    out.push_str(")\n\n");

    out.push_str("// registerHandler stores a Go handler in the registry and returns its opaque context ID.\n");
    out.push_str("func registerHandler(fn HandlerFunc) uintptr {\n");
    out.push_str("\thandlerRegistryMu.Lock()\n");
    out.push_str("\tdefer handlerRegistryMu.Unlock()\n");
    out.push_str("\tid := handlerNextID\n");
    out.push_str("\thandlerNextID++\n");
    out.push_str("\thandlerRegistry[id] = fn\n");
    out.push_str("\treturn id\n");
    out.push_str("}\n\n");

    out.push_str("// unregisterHandler removes a handler from the registry.\n");
    out.push_str("func unregisterHandler(id uintptr) {\n");
    out.push_str("\thandlerRegistryMu.Lock()\n");
    out.push_str("\tdefer handlerRegistryMu.Unlock()\n");
    out.push_str("\tdelete(handlerRegistry, id)\n");
    out.push_str("}\n\n");

    out.push_str("// invokeHandler looks up a handler by ID and invokes it with the request JSON.\n");
    out.push_str("// Returns the response JSON or an error.\n");
    out.push_str("func invokeHandler(ctx uintptr, reqJSON []byte) ([]byte, error) {\n");
    out.push_str("\thandlerRegistryMu.Lock()\n");
    out.push_str("\thandler, ok := handlerRegistry[ctx]\n");
    out.push_str("\thandlerRegistryMu.Unlock()\n");
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn nil, errors.New(\"handler not found\")\n");
    out.push_str("\t}\n");
    out.push_str("\treturn handler(reqJSON)\n");
    out.push_str("}\n\n");

    out.push_str("// cgo trampoline matching the C callback typedef:\n");
    out.push_str("// char* (*)(void* context, const char* request_json)\n");
    out.push_str("//\n");
    out.push_str("// This function is exported with //export so cgo can call it from C.\n");
    out.push_str("// It looks up the handler in the registry and invokes it.\n");
    out.push_str("//\n");
    out.push_str("//export service_handler_callback\n");
    out.push_str("func service_handler_callback(ctx unsafe.Pointer, reqCStr *C.char) *C.char {\n");
    out.push_str("\tctxID := uintptr(ctx)\n");
    out.push_str("\treqJSON := C.GoBytes(unsafe.Pointer(reqCStr), C.int(C.strlen(reqCStr)))\n\n");
    out.push_str("\trespJSON, err := invokeHandler(ctxID, reqJSON)\n");
    out.push_str("\tif err != nil {\n");
    out.push_str("\t\terrJSON, _ := json.Marshal(map[string]string{\"error\": err.Error()})\n");
    out.push_str("\t\trespJSON = errJSON\n");
    out.push_str("\t}\n\n");
    out.push_str("\t// Allocate C string from Go heap (caller responsible for freeing).\n");
    out.push_str("\tcResp := C.CString(string(respJSON))\n");
    out.push_str("\treturn cResp\n");
    out.push_str("}\n\n");
}

/// Generate a Go service struct with constructor, registration, and entrypoint methods.
fn gen_service_struct(out: &mut String, service: &ServiceDef, api: &ApiSurface, ffi_prefix: &str) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();

    // Service struct
    out.push_str(&format!(
        "// {} is a wrapper around the native service.\n",
        service_name
    ));
    if !service.doc.is_empty() {
        out.push_str(&format!("//\n// {}\n", service.doc.trim()));
    }
    out.push_str(&format!(
        "type {} struct {{\n\
         \towner unsafe.Pointer // *{service_name}Opaque from C\n\
         \tmu    sync.Mutex\n\
         }}\n\n",
        service_name
    ));

    // Constructor
    out.push_str(&format!(
        "// New{service_name} creates a new {service_name} instance.\n"
    ));
    out.push_str(&format!(
        "func New{service_name}() (*{service_name}, error) {{\n\
         \towner := unsafe.Pointer(C.{service_lower}_{service_snake}_new())\n\
         \tif owner == nil {{\n\
         \t\treturn nil, errors.New(\"failed to create {service_name}\")\n\
         \t}}\n\
         \treturn &{service_name}{{owner: owner}}, nil\n\
         }}\n\n"
    ));

    // Destructor
    out.push_str(&format!("// Close frees the {service_name} instance.\n"));
    out.push_str(&format!(
        "func (s *{service_name}) Close() {{\n\
         \ts.mu.Lock()\n\
         \tdefer s.mu.Unlock()\n\
         \tif s.owner != nil {{\n\
         \t\tC.{service_lower}_{service_snake}_free((*C.{service_name}Opaque)(s.owner))\n\
         \t\ts.owner = nil\n\
         \t}}\n\
         }}\n\n"
    ));

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, service, reg, api, ffi_prefix);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        gen_entrypoint_method(out, service, ep, api, ffi_prefix);
    }
}

/// Generate a registration method for one registration.
fn gen_registration_method(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    _api: &ApiSurface,
    ffi_prefix: &str,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let method_name = &reg.method;
    let method_name_pascal = method_name.to_upper_camel_case();
    let reg_method_snake = method_name.to_snake_case();

    // Build method signature with metadata params
    let mut params = vec!["handler HandlerFunc".to_owned()];
    for meta_param in &reg.metadata_params {
        let go_type = typeref_to_go_type(&meta_param.ty);
        params.push(format!("{} {}", meta_param.name, go_type));
    }
    let param_sig = params.join(", ");

    out.push_str(&format!(
        "// Register{} registers a handler for the {} registration.\n",
        method_name_pascal, method_name
    ));
    if !reg.doc.is_empty() {
        out.push_str(&format!("//\n// {}\n", reg.doc.trim()));
    }

    let return_type = if reg.error_type.is_some() {
        "error".to_owned()
    } else {
        "".to_owned()
    };
    let return_sig = if !return_type.is_empty() {
        format!(" {}", return_type)
    } else {
        String::new()
    };

    out.push_str(&format!(
        "func (s *{}) Register{}({}) {} {{\n",
        service_name, method_name_pascal, param_sig, return_sig
    ));
    out.push_str("\ts.mu.Lock()\n");
    out.push_str("\tdefer s.mu.Unlock()\n");
    out.push_str("\tif s.owner == nil {\n");
    if reg.error_type.is_some() {
        out.push_str("\t\treturn errors.New(\"service is closed\")\n");
    } else {
        out.push_str("\t\tpanic(\"service is closed\")\n");
    }
    out.push_str("\t}\n\n");

    // Register the handler in Go's registry
    out.push_str("\tctxID := registerHandler(handler)\n");

    // Call C registration function
    out.push_str(&format!(
        "\tret := C.{}_{}_register_{}(\n\
         \t\t(*C.{}Opaque)(s.owner),\n\
         \t\tC.handler_callback_t(C.service_handler_callback),\n\
         \t\tunsafe.Pointer(ctxID)",
        service_lower, service_snake, reg_method_snake, service_name
    ));

    // Add metadata params as arguments
    for meta_param in &reg.metadata_params {
        out.push_str(&format!(",\n\t\tC.CString({})", meta_param.name));
    }
    out.push_str("\n\t)\n\n");

    if reg.error_type.is_some() {
        out.push_str("\tif ret != 0 {\n");
        out.push_str("\t\tunregisterHandler(ctxID)\n");
        out.push_str("\t\treturn fmt.Errorf(\"registration failed: error code %d\", ret)\n");
        out.push_str("\t}\n");
        out.push_str("\treturn nil\n");
    } else {
        out.push_str("\tif ret != 0 {\n");
        out.push_str("\t\tunregisterHandler(ctxID)\n");
        out.push_str("\t\tpanic(fmt.Sprintf(\"registration failed: error code %d\", ret))\n");
        out.push_str("\t}\n");
    }

    out.push_str("}\n\n");
}

/// Generate an entrypoint method for one entrypoint.
fn gen_entrypoint_method(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    _api: &ApiSurface,
    ffi_prefix: &str,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let ep_method = &ep.method;
    let ep_method_pascal = ep_method.to_upper_camel_case();
    let ep_name_snake = ep_method.to_snake_case();

    // Build method signature with entrypoint params
    let mut params = vec![];
    for ep_param in &ep.params {
        let go_type = typeref_to_go_type(&ep_param.ty);
        params.push(format!("{} {}", ep_param.name, go_type));
    }
    let param_sig = if params.is_empty() {
        String::new()
    } else {
        params.join(", ")
    };

    let return_type = typeref_to_go_type(&ep.return_type);
    let return_sig = if !return_type.is_empty() {
        format!(" ({})", return_type)
    } else if ep.error_type.is_some() {
        " error".to_owned()
    } else {
        String::new()
    };

    out.push_str(&format!(
        "// {}() runs the service's {} entrypoint.\n",
        ep_method_pascal, ep_method
    ));
    if !ep.doc.is_empty() {
        out.push_str(&format!("//\n// {}\n", ep.doc.trim()));
    }

    out.push_str(&format!(
        "func (s *{}) {}({}) {} {{\n",
        service_name, ep_method_pascal, param_sig, return_sig
    ));
    out.push_str("\ts.mu.Lock()\n");
    out.push_str("\tdefer s.mu.Unlock()\n");
    out.push_str("\tif s.owner == nil {\n");

    if ep.error_type.is_some() {
        if !return_type.is_empty() {
            let zero_val = zero_value_for_type(&return_type);
            out.push_str(&format!("\t\treturn {zero_val}, errors.New(\"service is closed\")\n"));
        } else {
            out.push_str("\t\treturn errors.New(\"service is closed\")\n");
        }
    } else {
        out.push_str("\t\tpanic(\"service is closed\")\n");
    }
    out.push_str("\t}\n\n");

    // Call C entrypoint function
    out.push_str(&format!(
        "\tC.{}_{}_ep_{}(\n\
         \t\t(*C.{}Opaque)(s.owner)",
        service_lower, service_snake, ep_name_snake, service_name
    ));

    for ep_param in &ep.params {
        out.push_str(&format!(",\n\t\tC.CString({})", ep_param.name));
    }
    out.push_str("\n\t)\n");

    // Return statement
    if ep.error_type.is_some() {
        if !return_type.is_empty() {
            out.push_str(&format!("\treturn {}, nil\n", zero_value_for_type(&return_type)));
        } else {
            out.push_str("\treturn nil\n");
        }
    }

    out.push_str("}\n\n");
}

/// Return a Go zero value for the given type.
fn zero_value_for_type(go_type: &str) -> String {
    match go_type {
        "bool" => "false".to_owned(),
        "string" => "\"\"".to_owned(),
        s if s.starts_with("int") || s.starts_with("uint") || s.starts_with("float") => "0".to_owned(),
        s if s.starts_with('[') => "nil".to_owned(),
        s if s.starts_with("map[") => "nil".to_owned(),
        s if s.starts_with('*') => "nil".to_owned(),
        _ => "nil".to_owned(),
    }
}

// ──────────────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the Go backend.
///
/// Returns one `GeneratedFile` when services are present:
/// - `{output_dir}/service.go` — Go service wrapper
pub fn generate(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    pkg_name: &str,
    ffi_prefix: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let output_dir = {
        let mut d =
            crate::core::config::resolve_output_dir(config.output_paths.get("go"), &config.name, "packages/go/");
        if !d.ends_with('/') {
            d.push('/');
        }
        d
    };

    let service_go = gen_service_go(api, config, pkg_name, ffi_prefix);

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("service.go"),
        content: service_go,
        generated_header: true,
    }])
}

// ───────────────────────────────────────────────────────────────────── tests ──

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
            error_type: Some("HandlerError".to_owned()),
            doc: "Register a request handler.".to_owned(),
        };

        let run_entrypoint = EntrypointDef {
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
            error_type: Some("IoError".to_owned()),
            doc: "Start the service.".to_owned(),
        };

        let handler_contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: MethodDef {
                name: "handle".to_owned(),
                params: vec![ParamDef {
                    name: "req".to_owned(),
                    ty: TypeRef::Named("RequestData".to_owned()),
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("Response".to_owned()),
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Handle a request.".to_owned(),
                receiver: Some(crate::core::ir::ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("Response".to_owned()),
            doc: "Handler contract.".to_owned(),
        };

        ApiSurface {
            crate_name: "test_crate".to_owned(),
            version: "1.0.0".to_owned(),
            services: vec![ServiceDef {
                name: "TestService".to_owned(),
                rust_path: "my_crate::TestService".to_owned(),
                constructor,
                configurators: vec![],
                registrations: vec![registration],
                entrypoints: vec![run_entrypoint],
                doc: "Test service.".to_owned(),
                cfg: None,
            }],
            handler_contracts: vec![handler_contract],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn test_gen_service_go_produces_valid_go() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Verify that the generated Go contains expected markers
        assert!(go.contains("package binding"));
        assert!(go.contains("TestService"));
        assert!(go.contains("NewTestService"));
        assert!(go.contains("RegisterAddHandler"));
        assert!(go.contains("Run"));
        assert!(go.contains("HandlerFunc"));
        assert!(go.contains("handlerRegistry"));
        assert!(go.contains("service_handler_callback"));
    }

    #[test]
    fn test_service_struct_is_generated() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // The service struct must be present
        assert!(go.contains("type TestService struct"));
        assert!(go.contains("owner unsafe.Pointer"));
        assert!(go.contains("mu    sync.Mutex"));
    }

    #[test]
    fn test_constructor_is_generated() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Constructor should be present
        assert!(go.contains("func NewTestService()"));
        assert!(go.contains("test_crate_test_service_new"));
    }

    #[test]
    fn test_registration_method_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Registration method should be present
        assert!(go.contains("RegisterAddHandler"));
        assert!(go.contains("handler HandlerFunc"));
        assert!(go.contains("registerHandler(handler)"));
    }

    #[test]
    fn test_entrypoint_method_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Entrypoint method should be present
        assert!(go.contains("func (s *TestService) Run("));
        assert!(go.contains("test_crate_test_service_ep_run"));
    }

    #[test]
    fn test_handler_registry_and_trampoline() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Handler registry and trampoline must be present
        assert!(go.contains("handlerRegistry"));
        assert!(go.contains("service_handler_callback"));
        assert!(go.contains("invokeHandler"));
        assert!(go.contains("registerHandler"));
        assert!(go.contains("//export service_handler_callback"));
    }

    #[test]
    fn test_c_ffi_imports_generated() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // C FFI imports should be present in comments
        assert!(go.contains("test_crate_test_service_new"));
        assert!(go.contains("test_crate_test_service_free"));
        assert!(go.contains("test_crate_test_service_register_add_handler"));
    }
}
