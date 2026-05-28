//! Service-API codegen for the FFI backend.
//!
//! Generates a single output per `ApiSurface` with non-empty services:
//!
//! **`service.rs`** — C ABI contract for service ownership and handler registration.
//!
//! Exports:
//! - For each [`ServiceDef`]: opaque `*mut <service_name>` handle + constructor/destructor.
//! - For each [`RegistrationDef`]: a registration function accepting a callback + metadata
//!   (callback is a C function pointer `extern "C" fn(*mut c_void, *const c_char) -> *mut c_char`).
//! - For each [`EntrypointDef`]: a run/finalize function that builds the service, registers
//!   callbacks via a Rust bridge, and invokes the entrypoint.
//! - A callback typedef shared across all handler contracts.
//!
//! Ownership: Every `*mut T` is caller-owned; each service type has a matching `_free` function.
//! Error handling: C callbacks return null-terminated JSON strings; parsing errors are
//! logged and cause the handler dispatch to return an error JSON response.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ──────────────────────────────────────────────────── C Header (.h output) ──

/// Generate the C FFI header that declares the callback typedef and service API.
///
/// This header is an input to cbindgen for human-readable API documentation,
/// but the actual exported Rust functions below (`extern "C"`) are the binding contract.
#[allow(dead_code)]
fn gen_service_h(api: &ApiSurface, crate_name: &str) -> String {
    let mut out = String::new();
    let header_guard = format!("{}_SERVICE_H", crate_name.to_uppercase().replace('-', "_"));

    out.push_str(&format!(
        "#ifndef {header_guard}\n\
         #define {header_guard}\n\n\
         #include <stdint.h>\n\
         #include <stdbool.h>\n\n"
    ));

    out.push_str("/* Handler registration callback typedef.\n");
    out.push_str(" * Signature: fn(*mut c_void, *const c_char) -> *mut c_char\n");
    out.push_str(" * The context pointer is opaque to the C side and is passed through\n");
    out.push_str(" * to the callback on each invocation for stateful handlers.\n");
    out.push_str(" * The request is a JSON string (null-terminated); the callback returns\n");
    out.push_str(" * a newly-allocated JSON response string (null-terminated) that must be\n");
    out.push_str(" * freed via {crate_prefix}_free_string(). */\n");
    out.push_str("typedef char* (*handler_callback_t)(\n");
    out.push_str("    void* context,\n");
    out.push_str("    const char* request_json\n");
    out.push_str(");\n\n");

    // Forward-declare each service opaque type.
    for service in &api.services {
        let opaque_name = format!("{}Opaque", service.name);
        out.push_str(&format!("typedef struct {opaque_name} {opaque_name};\n"));
    }
    out.push('\n');

    // Service API declarations for each service.
    for service in &api.services {
        gen_service_h_decls(&mut out, service, api, crate_name);
    }

    out.push_str(&format!("\n#endif /* {header_guard} */\n"));
    out
}

#[allow(dead_code)]
fn gen_service_h_decls(out: &mut String, service: &ServiceDef, _api: &ApiSurface, prefix: &str) {
    let service_snake = service.name.to_snake_case();
    let opaque_name = format!("{}Opaque", service.name);
    let prefix_lower = prefix.to_lowercase();

    // Constructor: allocates and returns an opaque handle
    out.push_str(&format!(
        "/* Create a new {0} instance. */\n\
         {1}{2}* {1}_{3}_new(void);\n\n",
        service.name, prefix_lower, opaque_name, service_snake
    ));

    // Destructor: frees the opaque handle
    out.push_str(&format!(
        "/* Destroy a {0} instance. */\n\
         void {1}_{2}_free({1}{3}* ptr);\n\n",
        service.name, prefix_lower, service_snake, opaque_name
    ));

    // Registration functions
    for reg in &service.registrations {
        let reg_method_snake = reg.method.to_snake_case();
        out.push_str(&format!(
            "/* Register a handler for method '{0}'. */\n\
             int {1}_{2}_register_{3}(\n    \
                 {1}{4}* owner,\n    \
                 handler_callback_t callback,\n    \
                 void* context",
            reg.method, prefix_lower, service_snake, reg_method_snake, opaque_name
        ));

        // Metadata parameters
        for meta_param in &reg.metadata_params {
            let c_type = typeref_to_c_type(&meta_param.ty);
            out.push_str(&format!(",\n    {} {}", c_type, meta_param.name));
        }
        out.push_str("\n);\n\n");
    }

    // Entrypoint functions
    for ep in &service.entrypoints {
        let ep_name_snake = ep.method.to_snake_case();
        let return_type = typeref_to_c_type(&ep.return_type);

        out.push_str(&format!(
            "/* {0} the service. */\n\
             {1} {2}_{3}_ep_{4}(\n    \
                 {2}{5}* owner",
            if ep.kind == EntrypointKind::Run {
                "Run"
            } else {
                "Finalize"
            },
            return_type,
            prefix_lower,
            service_snake,
            ep_name_snake,
            opaque_name
        ));

        // Parameters
        for ep_param in &ep.params {
            let c_type = typeref_to_c_type(&ep_param.ty);
            out.push_str(&format!(",\n    {} {}", c_type, ep_param.name));
        }

        out.push_str("\n);\n\n");
    }
}

/// Map a `TypeRef` to a C type string.
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
        _ => "void*".to_owned(), // Json, Vec, Map, etc. go through JSON serialization
    }
}

/// Map a `TypeRef` to a Rust FFI-compatible type string.
fn typeref_to_rust_ffi_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String => "*const c_char".to_owned(),
        TypeRef::Char => "c_char".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "u8".to_owned(),
                PrimitiveType::U16 => "u16".to_owned(),
                PrimitiveType::U32 => "u32".to_owned(),
                PrimitiveType::U64 => "u64".to_owned(),
                PrimitiveType::I8 => "i8".to_owned(),
                PrimitiveType::I16 => "i16".to_owned(),
                PrimitiveType::I32 => "i32".to_owned(),
                PrimitiveType::I64 => "i64".to_owned(),
                PrimitiveType::F32 => "f32".to_owned(),
                PrimitiveType::F64 => "f64".to_owned(),
                PrimitiveType::Usize => "usize".to_owned(),
                PrimitiveType::Isize => "isize".to_owned(),
            }
        }
        TypeRef::Bytes => "*const u8".to_owned(),
        TypeRef::Unit => "()".to_owned(),
        TypeRef::Named(n) => {
            if core_import.is_empty() {
                n.clone()
            } else {
                format!("{core_import}::{n}")
            }
        }
        _ => "serde_json::Value".to_owned(),
    }
}

// ──────────────────────────────────────────────── Rust glue (extern "C") ──

/// Generate the Rust FFI glue module (`service.rs`).
///
/// For each service this emits:
/// - An opaque `struct <ServiceName>Opaque(Box<...>)` wrapping the Rust owner type.
/// - Constructor + destructor functions.
/// - Handler bridge structs implementing the contract trait, wrapping C callback pointers.
/// - Registration functions.
/// - Entrypoint runners.
fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let prefix = config.ffi_prefix();
    let mut out = String::new();

    out.push_str("#![allow(clippy::too_many_arguments, unused_variables, unused_mut)]\n\n");
    out.push_str("use std::ffi::{c_char, c_void, CStr, CString};\n");
    out.push_str("use std::sync::Arc;\n");
    out.push_str("use std::panic;\n\n");

    // Emit one handler bridge per unique handler contract referenced by any registration
    let referenced_contracts: Vec<&HandlerContractDef> = {
        let mut names: Vec<&str> = api
            .services
            .iter()
            .flat_map(|s| s.registrations.iter())
            .map(|r| r.callback_contract.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names.iter().filter_map(|n| find_contract(api, n)).collect()
    };

    for contract in &referenced_contracts {
        gen_handler_bridge(&mut out, contract, &core_import);
    }

    // Emit service opaques, constructors, destructors, and registration/entrypoint functions
    for service in &api.services {
        gen_service_opaque(&mut out, service, &core_import, &prefix);
        gen_service_functions(&mut out, service, api, &core_import, &prefix);
    }

    out
}

/// Emit the opaque service type and its constructor/destructor.
fn gen_service_opaque(out: &mut String, service: &ServiceDef, _core_import: &str, prefix: &str) {
    let opaque_name = format!("{}Opaque", service.name);
    let service_snake = service.name.to_snake_case();
    let owner_path = &service.rust_path;
    let prefix_lower = prefix.to_lowercase();

    // Define the opaque struct
    out.push_str(&format!(
        "/// Opaque handle to a {} service instance.\n\
         /// Allocated by {}_{}_new(), freed by {}_{}_free().\n\
         #[repr(C)]\n\
         pub struct {}({{\n    \
             inner: Box<{}>,\n\
         }})\n\n",
        service.name, prefix_lower, service_snake, prefix_lower, service_snake, opaque_name, owner_path
    ));

    // Constructor
    out.push_str(&format!(
        "/// Allocate a new {} instance.\n\
         ///\n\
         /// # Safety\n\
         /// The returned pointer must be freed via {}_{}_free().\n\
         /// Never access the pointer after freeing it.\n\
         #[no_mangle]\n\
         pub extern \"C\" fn {}_{}_new() -> *mut {} {{\n    \
             let owner = {}::{}();\n    \
             Box::into_raw(Box::new({}({{\n        \
                 inner: Box::new(owner),\n    \
             }})))\n\
         }}\n\n",
        service.name,
        prefix_lower,
        service_snake,
        prefix_lower,
        service_snake,
        opaque_name,
        owner_path,
        service.constructor.name,
        opaque_name
    ));

    // Destructor
    out.push_str(&format!(
        "/// Free a {} instance allocated by {}_{}_new().\n\
         ///\n\
         /// # Safety\n\
         /// - `ptr` must have been allocated by {}_{}_new().\n\
         /// - After this call, `ptr` is invalid and must not be dereferenced.\n\
         /// - Calling this twice on the same pointer causes undefined behavior.\n\
         #[no_mangle]\n\
         pub extern \"C\" fn {}_{}_free(ptr: *mut {}) {{\n    \
             if !ptr.is_null() {{\n        \
                 // SAFETY: ptr was allocated by into_raw above;\n        \
                 // we are the sole owner and this is the final drop.\n        \
                 unsafe {{\n            \
                     drop(Box::from_raw(ptr));\n        \
                 }}\n    \
             }}\n\
         }}\n\n",
        service.name,
        prefix_lower,
        service_snake,
        prefix_lower,
        service_snake,
        prefix_lower,
        service_snake,
        opaque_name
    ));
}

/// Emit the handler bridge struct for one contract.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Ffi{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    out.push_str(&format!(
        "/// FFI handler bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps a C callback function pointer so it can be called from Rust async code.\n\
         /// The callback receives JSON-serialized request and returns JSON response.\n\
         pub struct {bridge_name} {{\n    \
             callback: extern \"C\" fn(*mut c_void, *const c_char) -> *mut c_char,\n    \
             context: *mut c_void,\n\
         }}\n\n"
    ));

    out.push_str(&format!(
        "// SAFETY: The C callback function pointer and context pointer are opaque handles.\n\
         // The caller is responsible for maintaining the invariant that the context\n\
         // pointer remains valid for the lifetime of the bridge. The callback itself\n\
         // must be safe to call from async Rust code.\n\
         unsafe impl Send for {bridge_name} {{}}\n\
         unsafe impl Sync for {bridge_name} {{}}\n\n"
    ));

    // Determine wire types — use plain serde_json::Value as fallback
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Strip leading core import prefix if present
    let req_type = if req_type.contains("::") {
        req_type.split("::").last().unwrap_or(req_type)
    } else {
        req_type
    };
    let resp_type = if resp_type.contains("::") {
        resp_type.split("::").last().unwrap_or(resp_type)
    } else {
        resp_type
    };

    // Leading dispatch parameters (extra params the bridge ignores)
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    // Build full request and response paths
    let req_path = if req_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{resp_type}")
    };

    // Build the future's Output type
    let box_err = "Box<dyn std::error::Error + Send + Sync>";
    let wire_output = format!("Result<{resp_path}, {box_err}>");
    let output_type = contract
        .dispatch_return_type
        .clone()
        .unwrap_or_else(|| wire_output.clone());
    let tail = match &contract.response_adapter {
        Some(adapter) => format!("{adapter}(outcome)"),
        None => "outcome".to_string(),
    };

    // Trait impl. Returns a boxed future directly (canonical object-safe
    // async-trait shape) instead of via the async_trait macro, matching a
    // contract whose dispatch method is hand-written as
    // `-> Pin<Box<dyn Future<..> + Send + '_>>`.
    out.push_str(&format!(
        "impl {core_import}::{trait_name} for {bridge_name} {{\n    \
             fn {dispatch_name}(\n        \
                 &self{extra_param},\n        \
                 {wire_name}: {req_path},\n    \
             ) -> std::pin::Pin<Box<dyn std::future::Future<Output = {output_type}> + Send + '_>> {{\n        \
                 Box::pin(async move {{\n"
    ));

    // Serialize request to JSON and call the C callback (all fallible work
    // lives inside the wire-result block so the only outer expression is the
    // response adapter tail).
    out.push_str(&format!(
        "            \
             let outcome: {wire_output} = async move {{\n                \
                 // Serialize request to JSON\n                \
                 let req_json = serde_json::to_string(&{wire_name})\n                    \
                     .map_err(|e| Box::new(e) as {box_err})?;\n                \
                 let req_c_str = CString::new(req_json)\n                    \
                     .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                 // Call the C callback in a blocking context to avoid blocking the async executor\n                \
                 let resp_ptr = tokio::task::spawn_blocking({{\n                    \
                     let callback = self.callback;\n                    \
                     let context = self.context;\n                    \
                     let req_ptr = req_c_str.as_ptr();\n                    \
                     move || (callback)(context, req_ptr)\n                \
                 }})\n                \
                 .await\n                \
                 .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                 if resp_ptr.is_null() {{\n                    \
                     return Err(\"C callback returned null response\".into());\n                \
                 }}\n\n                \
                 // SAFETY: resp_ptr was returned by the C callback and must be a null-terminated string.\n                \
                 let resp_c_str = unsafe {{\n                    \
                     CStr::from_ptr(resp_ptr)\n                \
                 }};\n                \
                 let resp_json = resp_c_str.to_string_lossy();\n\n                \
                 // Deserialize response from JSON\n                \
                 let response: {resp_path} = serde_json::from_str(&resp_json)\n                    \
                     .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                 // Free the C-allocated response string.\n                \
                 // SAFETY: resp_ptr is null-checked above, and we assume the C side\n                \
                 // allocated it via libc::malloc or equivalent.\n                \
                 unsafe {{\n                    \
                     libc::free(resp_ptr as *mut c_void);\n                \
                 }}\n\n                \
                 Ok(response)\n            \
             }}\n            \
             .await;\n\n            \
             {tail}\n        \
             }})\n    \
             }}\n\
         }}\n\n"
    ));
}

/// Emit registration and entrypoint functions for one service.
fn gen_service_functions(out: &mut String, service: &ServiceDef, api: &ApiSurface, core_import: &str, prefix: &str) {
    let opaque_name = format!("{}Opaque", service.name);

    // Registration functions
    for reg in &service.registrations {
        gen_registration_function(out, service, reg, api, core_import, prefix, &opaque_name);
    }

    // Entrypoint functions
    for ep in &service.entrypoints {
        gen_entrypoint_function(out, service, ep, api, core_import, prefix, &opaque_name);
    }
}

fn gen_registration_function(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    core_import: &str,
    prefix: &str,
    opaque_name: &str,
) {
    let service_snake = service.name.to_snake_case();
    let reg_method_snake = reg.method.to_snake_case();
    let fn_name = format!(
        "{}_{}_register_{}",
        prefix.to_lowercase(),
        service_snake,
        reg_method_snake
    );

    let contract = find_contract(api, &reg.callback_contract).expect("contract not found");
    let bridge_name = format!("Ffi{}Bridge", contract.trait_name.to_upper_camel_case());

    out.push_str(&format!(
        "/// Register a handler callback for method '{0}'.\n\
         ///\n\
         /// # Safety\n\
         /// - `owner` must be a valid pointer returned by `{1}_{2}_new()` and not yet freed.\n\
         /// - `callback` must be a valid function pointer that remains valid for the lifetime\n\
         ///   of this service instance.\n\
         /// - `context` is an opaque pointer passed to the callback on each invocation.\n\
         ///   The caller is responsible for keeping it valid.\n\
         /// Returns 0 on success, non-zero error code on failure.\n\
         #[no_mangle]\n\
         pub extern \"C\" fn {fn_name}(\n    \
             owner: *mut {opaque_name},\n    \
             callback: extern \"C\" fn(*mut c_void, *const c_char) -> *mut c_char,\n    \
             context: *mut c_void",
        reg.method,
        prefix.to_lowercase(),
        service_snake
    ));

    // Add metadata parameters
    for meta_param in &reg.metadata_params {
        let rust_type = typeref_to_rust_ffi_type(&meta_param.ty, core_import);
        out.push_str(&format!(",\n    {} {}", rust_type, meta_param.name));
    }

    out.push_str("\n) -> i32 {\n");
    out.push_str("    if owner.is_null() {\n");
    out.push_str("        return 1; // Error: null pointer\n");
    out.push_str("    }\n\n");

    out.push_str("    let bridge = ");
    out.push_str(&bridge_name);
    out.push_str(" {\n");
    out.push_str("        callback,\n");
    out.push_str("        context,\n");
    out.push_str("    };\n");
    out.push_str("    let handler: Arc<dyn ");
    out.push_str(&format!("{}::{}", core_import, contract.trait_name));
    out.push_str("> = Arc::new(bridge);\n\n");

    out.push_str("    // SAFETY: owner was allocated by _new() and is valid until freed.\n");
    out.push_str("    match unsafe {\n");
    out.push_str("        let owner_ref = &mut (*owner).inner;\n");
    out.push_str(&format!("        owner_ref.{}(", reg.method));

    // Metadata arguments
    for meta_param in &reg.metadata_params {
        out.push_str(&meta_param.name);
        out.push_str(", ");
    }
    out.push_str("handler)\n");
    out.push_str("    } {\n");
    out.push_str("        Ok(_) => 0, // Success\n");
    out.push_str("        Err(_) => 1, // Error\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

fn gen_entrypoint_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    _api: &ApiSurface,
    _core_import: &str,
    prefix: &str,
    opaque_name: &str,
) {
    let service_snake = service.name.to_snake_case();
    let ep_name_snake = ep.method.to_snake_case();
    let fn_name = format!("{}_{}_ep_{}", prefix.to_lowercase(), service_snake, ep_name_snake);
    let return_type = typeref_to_c_type(&ep.return_type);

    out.push_str(&format!(
        "/// Run the service entrypoint '{0}'.\n\
         ///\n\
         /// # Safety\n\
         /// - `owner` must be a valid pointer returned by `{1}_{2}_new()` and not yet freed.\n\
         #[no_mangle]\n\
         pub extern \"C\" fn {fn_name}(\n    \
             owner: *mut {opaque_name}",
        ep.method,
        prefix.to_lowercase(),
        service_snake
    ));

    // Add entrypoint parameters
    for ep_param in &ep.params {
        let rust_type = typeref_to_rust_ffi_type(&ep_param.ty, "");
        out.push_str(&format!(",\n    {} {}", rust_type, ep_param.name));
    }

    out.push_str(&format!("\n) -> {return_type} {{\n"));

    out.push_str("    if owner.is_null() {\n");
    match return_type.as_str() {
        "void" => out.push_str("        return;\n"),
        _ => out.push_str("        return 0;\n"),
    }
    out.push_str("    }\n\n");

    // SAFETY comment for dereferencing
    out.push_str("    // SAFETY: owner was allocated by _new() and is valid until freed.\n");
    out.push_str("    unsafe {\n");
    out.push_str("        let owner_ref = &mut (*owner).inner;\n");

    if ep.is_async {
        out.push_str("        let rt = tokio::runtime::Runtime::new().expect(\"failed to create tokio runtime\");\n");
        out.push_str(&format!("        rt.block_on(owner_ref.{}(", ep.method));
    } else {
        out.push_str(&format!("        owner_ref.{}(", ep.method));
    }

    // Entrypoint arguments
    for ep_param in &ep.params {
        out.push_str(&ep_param.name);
        out.push_str(", ");
    }
    out.push_str("))");
    if ep.is_async {
        out.push_str("\n    }\n");
    } else {
        out.push_str(";\n    }\n");
    }

    out.push_str("}\n\n");
}

// ──────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the FFI backend.
///
/// Returns one `GeneratedFile` when services are present:
/// - `{output_dir}/service.rs`   — Rust FFI glue
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let output_dir = config
        .output_for("ffi")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("crates/{}-ffi/src/", config.name));

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("service.rs"),
        content: service_rs,
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
    fn test_gen_service_rs_produces_valid_rust() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let rs = gen_service_rs(&api, &config);

        // Verify that the generated Rust contains expected FFI markers
        assert!(rs.contains("#[no_mangle]"));
        assert!(rs.contains("extern \"C\""));
        assert!(rs.contains("TestServiceOpaque"));
        assert!(rs.contains("test_service_new"));
        assert!(rs.contains("test_service_free"));
        assert!(rs.contains("FfiRequestHandlerBridge"));
        assert!(rs.contains("Pin<Box<dyn std::future::Future"));
    }

    #[test]
    fn test_handler_bridge_struct_is_generated() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let rs = gen_service_rs(&api, &config);

        // The bridge must have callback and context fields
        assert!(rs.contains("struct FfiRequestHandlerBridge"));
        assert!(rs.contains("callback: extern \"C\" fn"));
        assert!(rs.contains("context: *mut c_void"));
    }

    #[test]
    fn test_opaque_has_constructor_and_destructor() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let rs = gen_service_rs(&api, &config);

        // Constructor and destructor should be present
        assert!(rs.contains("pub extern \"C\" fn test_crate_test_service_new()"));
        assert!(rs.contains("pub extern \"C\" fn test_crate_test_service_free"));
    }

    #[test]
    fn test_registration_function_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let rs = gen_service_rs(&api, &config);

        // Registration function should be present for each registration
        assert!(rs.contains("test_crate_test_service_register_add_handler"));
        // The callback function pointer type is used in the handler bridge
        assert!(rs.contains("extern \"C\" fn(*mut c_void, *const c_char) -> *mut c_char"));
    }

    #[test]
    fn test_entrypoint_function_exists() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let rs = gen_service_rs(&api, &config);

        // Entrypoint function should be present
        assert!(rs.contains("test_crate_test_service_ep_run"));
        assert!(rs.contains("tokio::runtime::Runtime"));
    }
}
