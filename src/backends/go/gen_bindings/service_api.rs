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
use crate::core::ir::{ApiSurface, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef, WrapperConstructorArg};
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

/// Whether an entrypoint's return type can be represented in Go.
///
/// Unit/primitives/strings map to zero values or are implicit; a `Named` type is representable only
/// when this surface wraps it (so it can cross as an opaque pointer wrapper). Anything else
/// is not representable and the entrypoint should be skipped.
fn entrypoint_return_representable(ep: &crate::core::ir::EntrypointDef, api: &ApiSurface) -> bool {
    match &ep.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n),
        _ => false,
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
fn gen_service_go(api: &ApiSurface, config: &ResolvedCrateConfig, pkg_name: &str, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ffi_header = config.ffi_header_name();

    out.push_str(&format!("package {pkg_name}\n\n"));

    // cgo preamble with C headers
    out.push_str("/*\n");
    out.push_str("#include <string.h>\n");
    out.push_str(&format!("#include \"{ffi_header}\"\n"));
    // Forward declaration of the exported Go callback function
    out.push_str("extern char* service_handler_callback(void* ctx, char* req);\n");
    // Define a function pointer typedef for the callback signature
    out.push_str("typedef char* (*ServiceHandlerCallbackPtr)(void*, const char*);\n");
    // Static inline helper to avoid duplicate symbols when preamble is included multiple times
    out.push_str("static inline ServiceHandlerCallbackPtr get_service_handler_callback(void) {\n");
    out.push_str("  return (ServiceHandlerCallbackPtr)service_handler_callback;\n");
    out.push_str("}\n");
    out.push_str("*/\n");
    out.push_str("import \"C\"\n\n");

    // Standard Go imports (separate from cgo import "C")
    out.push_str("import (\n");
    out.push_str("\t\"encoding/json\"\n");
    out.push_str("\t\"errors\"\n");
    out.push_str("\t\"fmt\"\n");
    out.push_str("\t\"sync\"\n");
    out.push_str("\t\"unsafe\"\n");
    out.push_str(")\n\n");

    // Generate C function references (now more of a comment section, not real FFI decls)
    out.push_str("// ──────────────────────────────────────────── Service Definitions ──\n\n");
    for service in &api.services {
        gen_service_c_imports_comment(&mut out, service, api, ffi_prefix);
    }

    // Generate the handler registry and trampoline
    out.push_str("// ──────────────────────────────────────────── Handler Registry ──\n\n");
    gen_handler_registry(&mut out);

    // Generate Go service structs and methods
    out.push_str("// ──────────────────────────────────────────── Go Service API ──\n\n");
    for service in &api.services {
        gen_service_struct(&mut out, service, api, ffi_prefix, api);
    }

    out
}

/// Generate documentation comments for one service's C FFI functions.
///
/// This is purely informational — the actual C declarations come from the header
/// included in the cgo preamble. We emit a comment block for readability.
fn gen_service_c_imports_comment(out: &mut String, service: &ServiceDef, _api: &ApiSurface, ffi_prefix: &str) {
    let service_snake = service.name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();

    out.push_str(&format!("// Service: {}\n", service.name));

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

/// Render a (possibly multi-line) doc string as a Go doc comment block.
///
/// Every line is prefixed with `//` so multi-paragraph docs (e.g. a Markdown `# Errors`
/// section) cannot leak un-commented source into the generated Go file.
fn go_doc_block(doc: &str) -> String {
    let mut out = String::from("//\n");
    for line in doc.trim_end().lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str("//\n");
        } else {
            out.push_str(&format!("// {line}\n"));
        }
    }
    out
}

/// Generate a Go service struct with constructor, registration, and entrypoint methods.
fn gen_service_struct(
    out: &mut String,
    service: &ServiceDef,
    api: &ApiSurface,
    ffi_prefix: &str,
    api_surface: &ApiSurface,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let upper_prefix = ffi_prefix.to_uppercase();

    // Service struct
    out.push_str(&format!(
        "// {} is a wrapper around the native service.\n",
        service_name
    ));
    if !service.doc.is_empty() {
        out.push_str(&go_doc_block(&service.doc));
    }
    out.push_str(&format!(
        "type {} struct {{\n\
         \towner unsafe.Pointer // *{}{}Opaque from C\n\
         \tmu    sync.Mutex\n\
         }}\n\n",
        service_name, upper_prefix, service_name
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
         \t\tC.{service_lower}_{service_snake}_free((*C.{upper_prefix}{service_name}Opaque)(s.owner))\n\
         \t\ts.owner = nil\n\
         \t}}\n\
         }}\n\n"
    ));

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, service, reg, api, ffi_prefix);
    }

    // Registration variant methods (e.g., Get, Post shortcuts)
    for reg in &service.registrations {
        for variant in &reg.variants {
            gen_registration_variant(out, service, reg, variant, ffi_prefix);
        }
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        gen_entrypoint_method(out, service, ep, api_surface, ffi_prefix);
    }

    // StartBackground convenience method for non-blocking server spawn
    gen_start_background_method(out, service, ffi_prefix);
}

/// Generate a registration method for one registration.
fn gen_registration_method(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
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
        let mut go_type = typeref_to_go_type(&meta_param.ty);
        // Opaque types (Named types that wrap FFI pointers) must be passed by pointer
        if let TypeRef::Named(type_name) = &meta_param.ty {
            if api.types.iter().any(|t| t.name == *type_name) {
                go_type = format!("*{}", go_type);
            }
        }
        params.push(format!("{} {}", meta_param.name, go_type));
    }
    let param_sig = params.join(", ");

    out.push_str(&format!(
        "// Register{} registers a handler for the {} registration.\n",
        method_name_pascal, method_name
    ));
    if !reg.doc.is_empty() {
        out.push_str(&go_doc_block(&reg.doc));
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

    // Call C registration function.
    // Pass the exported Go callback function's address as an opaque void* pointer.
    // The FFI function will transmute it back to the proper function pointer type.
    let upper_prefix = ffi_prefix.to_uppercase();
    out.push_str(&format!(
        "\tret := C.{}_{}_register_{}(\n\
         \t\t(*C.{upper_prefix}{service_name}Opaque)(s.owner),\n\
         \t\tC.get_service_handler_callback(),\n\
         \t\tunsafe.Pointer(ctxID),\n",
        service_lower, service_snake, reg_method_snake
    ));

    // Add metadata params as arguments, marshaling opaque types correctly. Go requires a trailing
    // comma on every argument when the closing paren sits on its own line, so each line ends with `,`.
    for meta_param in &reg.metadata_params {
        match &meta_param.ty {
            TypeRef::String => {
                out.push_str(&format!("\t\tC.CString({}),\n", meta_param.name));
            }
            TypeRef::Named(type_name) if api.types.iter().any(|t| t.name == *type_name) => {
                // Opaque type: pass (*C.{PREFIX}{TypeName})(unsafe.Pointer({param}.ptr))
                out.push_str(&format!(
                    "\t\t(*C.{}{})(unsafe.Pointer({}.ptr)),\n",
                    upper_prefix, type_name, meta_param.name
                ));
            }
            _ => {
                // Primitive or other type: pass directly (cast via C type)
                let c_type = typeref_to_c_type(&meta_param.ty);
                out.push_str(&format!("\t\t{c_type}({}),\n", meta_param.name));
            }
        }
    }
    out.push_str("\t)\n\n");

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

/// Generate a registration variant method for one variant shortcut (e.g., Get, Post).
fn gen_registration_variant(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    ffi_prefix: &str,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let variant_name_pascal = variant.name.to_upper_camel_case();
    let variant_name_snake = variant.name.to_snake_case();

    // Build method signature with variant's signature_params + handler
    let mut params = vec!["handler HandlerFunc".to_owned()];
    for sig_param in &variant.signature_params {
        let go_type = typeref_to_go_type(&sig_param.ty);
        params.push(format!("{} {}", sig_param.name, go_type));
    }
    let param_sig = params.join(", ");

    out.push_str(&format!(
        "// {}() registers a handler via the {} variant.\n",
        variant_name_pascal, variant.name
    ));
    if let Some(doc) = &variant.doc {
        out.push_str(&go_doc_block(doc));
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
        "func (s *{}) {}({}) {} {{\n",
        service_name, variant_name_pascal, param_sig, return_sig
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

    // Call the C variant function with fixed overrides + free args
    let upper_prefix = ffi_prefix.to_uppercase();
    // The FFI exports the variant symbol as `{prefix}_{service}_{variant}` —
    // the registration method name is NOT included in the variant symbol name.
    // Get the function pointer from a public C helper function.
    out.push_str(&format!(
        "\tret := C.{}_{}_{}(\n\
         \t\t(*C.{upper_prefix}{service_name}Opaque)(s.owner),\n\
         \t\tC.get_service_handler_callback(),\n\
         \t\tunsafe.Pointer(ctxID),\n",
        service_lower, service_snake, variant_name_snake
    ));

    // Emit the free args that follow the fixed trampoline args.
    //
    // When a wrapper_call is present the FFI function's extra args are the free
    // constructor params in declaration order (fixed params are baked in).
    // When there is no wrapper_call the extra args are the non-overridden base
    // metadata params.
    if let Some(wc) = &variant.wrapper_call {
        for arg in &wc.args {
            if let WrapperConstructorArg::Free { param } = arg {
                match &param.ty {
                    TypeRef::String => {
                        out.push_str(&format!("\t\tC.CString({}),\n", param.name));
                    }
                    TypeRef::Named(type_name) => {
                        out.push_str(&format!(
                            "\t\t(*C.{upper_prefix}{type_name})(unsafe.Pointer({}.ptr)),\n",
                            param.name
                        ));
                    }
                    _ => {
                        let c_type = typeref_to_c_type(&param.ty);
                        out.push_str(&format!("\t\t{c_type}({}),\n", param.name));
                    }
                }
            }
        }
    } else {
        for base_param in &reg.metadata_params {
            if variant.overrides.iter().any(|o| o.param_name == base_param.name) {
                // Fixed override — baked into the FFI function; do not re-emit.
            } else if let Some(sig_param) = variant.signature_params.iter().find(|s| s.name == base_param.name) {
                match &sig_param.ty {
                    TypeRef::String => {
                        out.push_str(&format!("\t\tC.CString({}),\n", sig_param.name));
                    }
                    TypeRef::Named(type_name) => {
                        out.push_str(&format!(
                            "\t\t(*C.{upper_prefix}{type_name})(unsafe.Pointer({}.ptr)),\n",
                            sig_param.name
                        ));
                    }
                    _ => {
                        let c_type = typeref_to_c_type(&sig_param.ty);
                        out.push_str(&format!("\t\t{c_type}({}),\n", sig_param.name));
                    }
                }
            }
        }
    }
    out.push_str("\t)\n\n");

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
    api: &ApiSurface,
    ffi_prefix: &str,
) {
    // Skip finalize entrypoints with non-representable return types (e.g., foreign framework routers).
    use crate::core::ir::EntrypointKind;
    if matches!(ep.kind, EntrypointKind::Finalize) && !entrypoint_return_representable(ep, api) {
        return;
    }
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

    // The C entrypoint returns either a `*mut T` opaque pointer (when this surface wraps the
    // entrypoint's return type) or an `i32` status code (0 = ok). Mirror that ABI here: expose the
    // opaque wrapper as a value; otherwise the call is status-only, reported through `error`. A
    // value type the surface does not wrap (e.g. a foreign framework type) has no C return form, so
    // it collapses to the status call rather than a bogus Go value.
    let upper_prefix = ffi_prefix.to_uppercase();
    let opaque_return = match &ep.return_type {
        TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => Some(n.clone()),
        _ => None,
    };
    let has_err = ep.error_type.is_some();
    let return_sig = match (&opaque_return, has_err) {
        (Some(t), true) => format!(" (*{t}, error)"),
        (Some(t), false) => format!(" *{t}"),
        (None, true) => " error".to_owned(),
        (None, false) => String::new(),
    };

    out.push_str(&format!(
        "// {}() runs the service's {} entrypoint.\n",
        ep_method_pascal, ep_method
    ));
    if !ep.doc.is_empty() {
        out.push_str(&go_doc_block(&ep.doc));
    }

    out.push_str(&format!(
        "func (s *{}) {}({}){} {{\n",
        service_name, ep_method_pascal, param_sig, return_sig
    ));
    out.push_str("\ts.mu.Lock()\n");
    out.push_str("\tdefer s.mu.Unlock()\n");
    out.push_str("\tif s.owner == nil {\n");
    match (&opaque_return, has_err) {
        (Some(_), true) => out.push_str("\t\treturn nil, errors.New(\"service is closed\")\n"),
        (Some(_), false) => out.push_str("\t\treturn nil\n"),
        (None, true) => out.push_str("\t\treturn errors.New(\"service is closed\")\n"),
        (None, false) => out.push_str("\t\tpanic(\"service is closed\")\n"),
    }
    out.push_str("\t}\n\n");

    // Call the C entrypoint, capturing its return when it carries a value or status.
    let capture = if opaque_return.is_some() || has_err {
        "ret := "
    } else {
        ""
    };
    out.push_str(&format!(
        "\t{capture}C.{}_{}_ep_{}(\n\
         \t\t(*C.{upper_prefix}{}Opaque)(s.owner),\n",
        service_lower, service_snake, ep_name_snake, service_name
    ));
    for ep_param in &ep.params {
        out.push_str(&format!("\t\tC.CString({}),\n", ep_param.name));
    }
    out.push_str("\t)\n");

    match (&opaque_return, has_err) {
        (Some(t), true) => {
            out.push_str("\tif ret == nil {\n");
            out.push_str(&format!("\t\treturn nil, errors.New(\"{} failed\")\n", ep_method));
            out.push_str("\t}\n");
            out.push_str(&format!("\treturn &{t}{{ptr: unsafe.Pointer(ret)}}, nil\n"));
        }
        (Some(t), false) => {
            out.push_str(&format!("\treturn &{t}{{ptr: unsafe.Pointer(ret)}}\n"));
        }
        (None, true) => {
            out.push_str("\tif ret != 0 {\n");
            out.push_str(&format!(
                "\t\treturn fmt.Errorf(\"{} failed: error code %d\", ret)\n",
                ep_method
            ));
            out.push_str("\t}\n");
            out.push_str("\treturn nil\n");
        }
        (None, false) => {}
    }

    out.push_str("}\n\n");
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

/// Generate a StartBackground convenience method that spawns the service in a goroutine.
///
/// This method is useful for e2e harnesses that need the server running in the background
/// while the test process continues. It blocks until the TCP socket is bound, ensuring
/// the server is reachable when the call returns.
fn gen_start_background_method(out: &mut String, service: &ServiceDef, _ffi_prefix: &str) {
    let service_name = &service.name;

    // ServerHandle wraps the running service and exposes Stop() for graceful shutdown.
    out.push_str(&format!(
        "// ServerHandle allows stopping a service started via StartBackground.\n\
         type ServerHandle struct {{\n\
         \tservice *{}\n\
         }}\n\n",
        service_name
    ));

    out.push_str(
        "// Stop gracefully shuts down the server.\n\
         func (h *ServerHandle) Stop() error {\n\
         \tif h.service == nil {\n\
         \t\treturn errors.New(\"service already stopped\")\n\
         \t}\n\
         \th.service.Close()\n\
         \th.service = nil\n\
         \treturn nil\n\
         }\n\n",
    );

    // StartBackground spawns the service in a goroutine after binding to the port.
    out.push_str(&format!(
        "// StartBackground starts the service in a background goroutine and returns a handle.\n\
         // It blocks until the TCP socket is bound, so the server is guaranteed to be accepting\n\
         // connections when this call returns.\n\
         func (s *{}) StartBackground(host string, port uint16) (*ServerHandle, error) {{\n\
         \ts.mu.Lock()\n\
         \tdefer s.mu.Unlock()\n\
         \tif s.owner == nil {{\n\
         \t\treturn nil, errors.New(\"service is closed\")\n\
         \t}}\n\n",
        service_name
    ));

    out.push_str(
        "\t// Spawn Run in a goroutine. The C entrypoint will block there,\n\
         \t// and we exit this function once the socket is bound.\n\
         \tgo func() {\n\
         \t\t_ = s.Run()\n\
         \t}()\n\n\
         \t// Return immediately with a handle for shutdown.\n\
         \treturn &ServerHandle{service: s}, nil\n\
         }\n\n",
    );
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

        let get_variant = crate::core::ir::RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![crate::core::ir::RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "\"GET\"".to_owned(),
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
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: Some("HandlerError".to_owned()),
            doc: "Register a request handler.".to_owned(),
            variants: vec![get_variant],
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
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
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

        let go = gen_service_go(&api, &config, "binding", "TEST_CRATE");

        // Verify that the generated Go contains expected markers
        assert!(go.contains("package binding"));
        assert!(go.contains("TestService"));
        assert!(go.contains("NewTestService"));
        assert!(go.contains("RegisterAddHandler"));
        assert!(go.contains("Run"));
        assert!(go.contains("HandlerFunc"));
        assert!(go.contains("handlerRegistry"));
        assert!(go.contains("service_handler_callback"));
        // Verify cgo preamble
        assert!(go.contains("/*\n#include <string.h>"));
        assert!(go.contains("#include \"test_crate.h\""));
        assert!(go.contains("//export service_handler_callback"));
        assert!(go.contains("import \"C\""));
        // Verify prefixed struct names (uppercase prefix)
        assert!(go.contains("*TEST_CRATETestServiceOpaque"));
    }

    #[test]
    fn test_service_struct_is_generated() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "TEST_CRATE");

        // The service struct must be present with prefixed opaque type
        assert!(go.contains("type TestService struct"));
        assert!(go.contains("owner unsafe.Pointer"));
        assert!(go.contains("*TEST_CRATETestServiceOpaque"));
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

        // Constructor should be present with lowercase prefix
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

        // Registration method should be present with correct callback passing
        assert!(go.contains("RegisterAddHandler"));
        assert!(go.contains("handler HandlerFunc"));
        assert!(go.contains("registerHandler(handler)"));
        // Verify callback is obtained from public C helper function.
        assert!(go.contains("C.get_service_handler_callback(),"));
        // Verify prefixed struct names
        assert!(go.contains("(*C.TEST_CRATETestServiceOpaque)"));
    }

    #[test]
    fn test_entrypoint_method_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Entrypoint method should be present with prefixed struct names
        assert!(go.contains("func (s *TestService) Run("));
        assert!(go.contains("test_crate_test_service_ep_run"));
        assert!(go.contains("(*C.TEST_CRATETestServiceOpaque)"));
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

    #[test]
    fn test_registration_variant_method_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Variant method should be present with TitleCase name
        assert!(go.contains("func (s *TestService) Get("));
        assert!(go.contains("handler HandlerFunc"));
        assert!(go.contains("path string"));
        // Verify it calls the variant C function: symbol is {prefix}_{service}_{variant},
        // WITHOUT the registration method name in between.
        assert!(go.contains("C.test_crate_test_service_get"));
        assert!(!go.contains("C.test_crate_test_service_add_handler_get"));
        // Verify that the free wrapper-call arg (path) is marshaled with CString.
        assert!(go.contains("C.CString(path)"));
    }

    #[test]
    fn test_start_background_method_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // StartBackground method and ServerHandle must be present
        assert!(go.contains("func (s *TestService) StartBackground("));
        assert!(go.contains("type ServerHandle struct"));
        assert!(go.contains("func (h *ServerHandle) Stop()"));
        assert!(go.contains("host string, port uint16"));
        assert!(go.contains("*ServerHandle, error"));
    }

    #[test]
    fn test_registration_variant_wrapper_call_emits_free_args() {
        use crate::core::ir::{WrapperConstructorArg, WrapperConstructorCall};

        // Build a surface where the variant uses wrapper_call so free args come from wc.args.
        let mut api = make_fixture_surface();
        let svc = &mut api.services[0];
        let reg = &mut svc.registrations[0];

        // Replace the variant with one that has wrapper_call set.
        reg.variants[0] = crate::core::ir::RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![crate::core::ir::RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "\"GET\"".to_owned(),
            }],
            wrapper_call: Some(WrapperConstructorCall {
                metadata_param: "builder".to_owned(),
                wrapper_type_path: "test_crate::RouteBuilder".to_owned(),
                wrapper_type_name: "RouteBuilder".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![
                    WrapperConstructorArg::Fixed {
                        param_name: "method".to_owned(),
                        value_expr: "\"GET\"".to_owned(),
                    },
                    WrapperConstructorArg::Free {
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
            doc: Some("Register a GET handler.".to_owned()),
            style: Default::default(),
        };

        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let go = gen_service_go(&api, &config, "binding", "test_crate");

        // Free arg from wrapper_call must be emitted as a C arg.
        assert!(go.contains("C.CString(path)"), "missing CString(path) in:\n{go}");
        // Fixed args must NOT be emitted separately (baked into the FFI function).
        assert!(!go.contains("\"GET\""), "fixed arg must not be re-emitted:\n{go}");
    }
}
