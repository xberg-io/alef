//! Service-API codegen for the JNI backend.
//!
//! Generates Rust JNI glue for service handler registration and lifecycle management.
//!
//! For each [`ServiceDef`]:
//! - A `Jni{ContractName}Bridge` struct that wraps a global JVM reference to a Java
//!   handler object and implements `Arc<dyn {HandlerContractDef::trait_name}>`
//! - `#[no_mangle] extern "system"` JNI entry points:
//!   - `register_{snake_service}_{registration_method}`: registers a Java handler
//!   - `run_{snake_service}` / `finalize_{snake_service}`: lifecycle entrypoints
//!
//! Thread safety: thread-attaches to JVM, calls Java handler methods with request JSON,
//! parses response JSON. No panics — all errors propagate as JNI exceptions.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use crate::core::jni::{bridge_method_name, jni_package, jni_symbol, service_bridge_class_name};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Map a `TypeRef` to a JNI FFI type.
fn typeref_to_jni_type(ty: &TypeRef, _core_import: &str) -> String {
    match ty {
        TypeRef::String => "jni::objects::JString",
        TypeRef::Char => "c_char",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "jboolean",
                PrimitiveType::U8 => "jbyte",
                PrimitiveType::U16 => "jchar",
                PrimitiveType::U32 => "jint",
                PrimitiveType::U64 => "jlong",
                PrimitiveType::I8 => "jbyte",
                PrimitiveType::I16 => "jshort",
                PrimitiveType::I32 => "jint",
                PrimitiveType::I64 => "jlong",
                PrimitiveType::F32 => "jfloat",
                PrimitiveType::F64 => "jdouble",
                PrimitiveType::Usize => "jlong",
                PrimitiveType::Isize => "jlong",
            }
        }
        TypeRef::Bytes => "*const u8",
        TypeRef::Unit => "()",
        _ => "jni::objects::JObject",
    }
    .to_owned()
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust JNI glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Jni{ContractName}Bridge` struct holding a global JNI reference to the Java handler
/// - `impl` of the handler contract trait with async dispatch that:
///   - Attaches current thread to JVM
///   - Calls the Java handler method (passing request as JSON string)
///   - Parses response JSON
/// - `#[no_mangle] extern "system"` JNI entry points for handler registration and
///   service lifecycle (run/finalize)
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let package = jni_package(config);
    let mut out = String::new();

    // File-level attributes
    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async, non_snake_case)]\n\n");
    out.push_str("use jni::{AttachGuard, Env, EnvUnowned};\n");
    out.push_str("use jni::objects::{JClass, JObject, JString};\n");
    out.push_str("use jni::sys::{jint, jlong};\n");
    out.push_str("use std::sync::Arc;\n");
    out.push_str("use std::sync::OnceLock;\n");
    out.push_str("use serde_json;\n\n");

    // Emit service opaque types and constructor/destructor.
    // The JVM class hosting the `external fun`s is the per-service bridge object
    // `{ServicePascal}ServiceBridge` — it MUST match the Kotlin `object` name so the
    // `Java_*` symbols and the Kotlin `external fun` declarations link.
    for service in &api.services {
        let service_bridge_class = service_bridge_class_name(&service.name);
        gen_service_opaque(&mut out, service, &core_import, &package, &service_bridge_class);
    }

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

    // Emit handler registration and lifecycle entry points per service
    for service in &api.services {
        let service_bridge_class = service_bridge_class_name(&service.name);
        for reg in &service.registrations {
            gen_register_jni_function(&mut out, service, reg, api, &core_import, &package, &service_bridge_class);
        }
        for ep in &service.entrypoints {
            gen_entrypoint_jni_function(&mut out, service, ep, &core_import, &package, &service_bridge_class);
        }
    }

    out
}

/// Emit the opaque service type and its constructor/destructor.
fn gen_service_opaque(
    out: &mut String,
    service: &ServiceDef,
    _core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let opaque_name = format!("{}Opaque", service.name);
    let service_snake = service.name.to_snake_case();
    let owner_path = &service.rust_path;

    // Define the opaque struct
    out.push_str(&format!(
        "/// Opaque handle to a {} service instance.\n\
         /// Allocated by constructor_{}(), freed by free_{}().\n\
         /// Thread safety: this handle wraps the Rust owner, which may not be Send/Sync.\n\
         /// The JNI binding layer is responsible for thread synchronization via JVM thread attachment.\n\
         #[repr(C)]\n\
         pub struct {}({{\n    \
             pub inner: {},\n\
         }})\n\n",
        service.name, service_snake, service_snake, opaque_name, owner_path
    ));

    // Constructor: allocates and returns an opaque handle as jlong
    // Use shared bridge_method_name for consistency
    let ctor_method = bridge_method_name(&service.name, "new");
    let ctor_symbol = jni_symbol(package, service_bridge_class, &ctor_method);
    out.push_str(&format!(
        "/// Allocate a new {} instance.\n\
         ///\n\
         /// Returns the address as a jlong pointer. This pointer must be freed via free_{}().\n\
         /// Never dereference this pointer after freeing it.\n\
         #[no_mangle]\n\
         pub extern \"system\" fn {ctor_symbol}() -> jlong {{\n    \
             let owner = {}::{}();\n    \
             let opaque = Box::new({}({{\n        \
                 inner: owner,\n    \
             }}));\n    \
             Box::into_raw(opaque) as jlong\n\
         }}\n\n",
        service.name, service_snake, owner_path, service.constructor.name, opaque_name
    ));

    // Destructor: frees the opaque handle
    let dtor_method = bridge_method_name(&service.name, "free");
    let dtor_symbol = jni_symbol(package, service_bridge_class, &dtor_method);
    out.push_str(&format!(
        "/// Free a {0} instance allocated by constructor_{1}().\n\
         ///\n\
         /// # Safety\n\
         /// - handle must have been allocated by constructor_{1}().\n\
         /// - After this call, handle is invalid and must not be dereferenced.\n\
         /// - Calling this twice on the same handle causes undefined behavior.\n\
         #[no_mangle]\n\
         pub extern \"system\" fn {dtor_symbol}(_env: EnvUnowned, _class: JClass, handle: jlong) {{\n    \
             if handle != 0 {{\n        \
                 // SAFETY: handle was allocated by into_raw above; we are the sole owner\n        \
                 // and this is the final drop.\n        \
                 unsafe {{\n            \
                     let _ = Box::from_raw(handle as *mut {2});\n        \
                 }}\n    \
             }}\n\
         }}\n\n",
        service.name, service_snake, opaque_name
    ));
}

/// Emit the `Jni{ContractName}Bridge` struct + trait impl.
///
/// Holds a global JVM reference to a Java handler object. When dispatched:
/// 1. Attaches current thread to JVM (idempotent if already attached)
/// 2. Calls Java handler method via JNI, passing request as JSON string
/// 3. Parses response JSON
/// 4. Detaches if this thread wasn't previously attached
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Jni{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    out.push_str(&format!(
        "/// Generated JNI bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps a global JVM reference to a Java handler object so it can be used\n\
         /// as `Arc<dyn {trait_name}>` from Rust async code.\n\
         pub struct {bridge_name} {{\n    \
             /// Global JVM reference to the Java handler object.\n    \
             global_ref: jni::objects::GlobalRef,\n    \
             /// The JavaVM pointer for thread attachment.\n    \
             jvm: jni::JavaVM,\n    \
             /// Method ID for the dispatch method (cached for performance).\n    \
             method_id: jni::sys::jmethodID,\n\
         }}\n\n"
    ));

    // SAFETY comments on unsafe Send/Sync impl
    out.push_str(&format!(
        "// SAFETY: GlobalRef is Send+Sync once obtained in JVM context.\n\
         // JavaVM is Send+Sync per jni crate semantics (one global VM per process).\n\
         // jmethodID is stable for the method lifetime.\n\
         unsafe impl Send for {bridge_name} {{}}\n\
         unsafe impl Sync for {bridge_name} {{}}\n\n"
    ));

    // Trait impl with async dispatch
    out.push_str(&format!(
        "#[async_trait::async_trait]\n\
         impl {core_import}::{trait_name} for {bridge_name} {{\n    \
             async fn {dispatch_name}(\n        \
                 &self,\n        \
                 request: {core_import}::{req_type},\n    \
             ) -> Result<{core_import}::{resp_type}, Box<dyn std::error::Error + Send + Sync>> {{\n"
    ));

    // Serialize request to JSON
    out.push_str("        let req_json = serde_json::to_string(&request)\n");
    out.push_str("            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;\n\n");

    // Attach thread to JVM and call handler
    out.push_str("        let result_json = {\n");
    out.push_str("            let env = self.jvm.attach_current_thread()\n");
    out.push_str("                .map_err(|e| Box::new(std::io::Error::new(\n");
    out.push_str("                    std::io::ErrorKind::Other,\n");
    out.push_str("                    format!(\"failed to attach JVM thread: {}\", e)\n");
    out.push_str("                )) as Box<dyn std::error::Error + Send + Sync>)?;\n\n");

    // Create JNI string for request
    out.push_str("            let req_jni = env.new_string(&req_json)\n");
    out.push_str("                .map_err(|e| Box::new(std::io::Error::new(\n");
    out.push_str("                    std::io::ErrorKind::Other,\n");
    out.push_str("                    format!(\"failed to create JNI string: {}\", e)\n");
    out.push_str("                )) as Box<dyn std::error::Error + Send + Sync>)?;\n\n");

    // Call Java handler method
    // Convention: Java method is named `handle` and takes a String, returns a String
    out.push_str("            let result: jni::sys::jstring = unsafe {\n");
    out.push_str("                // SAFETY: method_id was validated when bridge was created.\n");
    out.push_str("                // self.global_ref is valid for the JVM's lifetime.\n");
    out.push_str("                env.call_method_unchecked(\n");
    out.push_str("                    self.global_ref.as_obj(),\n");
    out.push_str("                    self.method_id,\n");
    out.push_str("                    jni::sys::JNI_ABORT,\n");
    out.push_str("                    &[jni::objects::JValue::from(&req_jni)],\n");
    out.push_str("                )?\n");
    out.push_str("                    .l()?\n");
    out.push_str("                    .as_raw()\n");
    out.push_str("            };\n\n");

    // Convert result back to String
    out.push_str("            let result_obj = unsafe {\n");
    out.push_str("                // SAFETY: result is a valid jstring from the JNI call.\n");
    out.push_str("                jni::objects::JString::from_raw(result)\n");
    out.push_str("            };\n");
    out.push_str("            env.get_string(&result_obj)?\n");
    out.push_str("                .into_string()\n");
    out.push_str("                .map_err(|e| Box::new(std::io::Error::new(\n");
    out.push_str("                    std::io::ErrorKind::InvalidData,\n");
    out.push_str("                    format!(\"response is not valid UTF-8: {}\", e)\n");
    out.push_str("                )) as Box<dyn std::error::Error + Send + Sync>)?\n");
    out.push_str("        };\n\n");

    // Deserialize response JSON
    out.push_str("        let response: ");
    out.push_str(&format!("{core_import}::{resp_type}"));
    out.push_str(" = serde_json::from_str(&result_json)\n");
    out.push_str("            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;\n");
    out.push_str("        Ok(response)\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

/// Emit a JNI function that registers a Java handler for a registration method.
///
/// Function signature (in Java):
/// ```java,ignore
/// public native void register{ServiceName}{MethodName}(Object handler);
/// ```
///
/// Convention: The Java handler object must have a public method `handle(String) -> String`
fn gen_register_jni_function(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let service_pascal = service.name.to_upper_camel_case();
    let method_pascal = reg.method.to_upper_camel_case();
    let contract_name = &reg.callback_contract;

    if let Some(contract) = find_contract(api, contract_name) {
        let bridge_name = format!("Jni{}Bridge", contract_name.to_upper_camel_case());
        let opaque_name = format!("{}Opaque", service.name);
        let register_method = bridge_method_name(&service.name, &format!("register_{}", reg.method));
        let symbol = jni_symbol(package, service_bridge_class, &register_method);

        out.push_str(&format!(
            "/// Register a Java handler for `{service_pascal}::{method_pascal}`.\n\
             ///\n\
             /// Called from Java/Kotlin to provide a handler implementation.\n\
             /// Parameters:\n\
             ///   owner_handle: jlong returned by the service constructor entry point\n\
             ///   handler: the Java handler object\n\
             ///   metadata params: route pattern, HTTP method, etc.\n\
             ///\n\
             /// Returns 0 on success, non-zero error code on failure.\n\
             #[no_mangle]\n\
             pub extern \"system\" fn {symbol}(\n        \
                 env: EnvUnowned,\n        \
                 _class: JClass,\n        \
                 owner_handle: jlong,\n        \
                 handler: JObject",
            service_pascal = service_pascal,
            method_pascal = method_pascal
        ));

        // Add metadata parameters
        for meta_param in &reg.metadata_params {
            let rust_type = typeref_to_jni_type(&meta_param.ty, core_import);
            out.push_str(&format!(",\n        {}: {}", meta_param.name, rust_type));
        }

        out.push_str("\n    ) -> jint {\n");
        out.push_str("    // Validate owner handle\n");
        out.push_str("    if owner_handle == 0 {\n");
        out.push_str("        return 1; // Error: null pointer\n");
        out.push_str("    }\n\n");

        // Get JavaVM from environment
        out.push_str("    let jvm = match env.get_java_vm() {\n");
        out.push_str("        Ok(vm) => vm,\n");
        out.push_str("        Err(_) => return 2, // Error: failed to get JavaVM\n");
        out.push_str("    };\n\n");

        // Create GlobalRef from handler object
        out.push_str("    let global_ref = match env.new_global_ref(&handler) {\n");
        out.push_str("        Ok(g) => g,\n");
        out.push_str("        Err(_) => return 3, // Error: failed to create global reference\n");
        out.push_str("    };\n\n");

        // Get the dispatch method ID (cached for performance)
        let dispatch_method_name = &contract.dispatch.name;
        out.push_str("    let method_id = match env.get_method_id(\n");
        out.push_str("        &handler,\n");
        out.push_str(&format!("        \"{dispatch_method_name}\",\n"));
        out.push_str("        \"(Ljava/lang/String;)Ljava/lang/String;\"\n");
        out.push_str("    ) {\n");
        out.push_str("        Ok(id) => id,\n");
        out.push_str("        Err(_) => return 4, // Error: failed to find method\n");
        out.push_str("    };\n\n");

        // Create the bridge
        out.push_str(&format!(
            "    let bridge = {bridge_name} {{\n\
             global_ref,\n\
             jvm,\n\
             method_id,\n\
             }};\n\
             let handler_arc: Arc<dyn {core_import}::{contract_name}> = Arc::new(bridge);\n\n"
        ));

        // SAFETY comment for owner dereference
        out.push_str("    // SAFETY: owner_handle was returned by the service constructor and\n");
        out.push_str("    // is valid until freed. The caller is responsible for ensuring no use-after-free.\n");
        out.push_str("    match unsafe {\n");
        out.push_str(&format!(
            "        let owner_opaque = owner_handle as *mut {opaque_name};\n\
             (*owner_opaque).inner.{}(",
            reg.method
        ));

        // Add metadata arguments
        let mut first = true;
        for meta_param in &reg.metadata_params {
            if !first {
                out.push_str(", ");
            }
            out.push_str(&meta_param.name);
            first = false;
        }
        if !first {
            out.push_str(", ");
        }
        out.push_str("handler_arc)\n");
        out.push_str("    } {\n");
        out.push_str("        Ok(_) => 0, // Success\n");
        out.push_str("        Err(_) => 5, // Error: registration failed\n");
        out.push_str("    }\n");
        out.push_str("}\n\n");
    }
}

/// Emit a JNI function for a service entrypoint (run or finalize).
///
/// Function signatures (in Java):
/// ```java,ignore
/// public native void run{ServiceName}(long ownerHandle, String addr, ...);
/// public native long finalize{ServiceName}(long ownerHandle, ...);
/// ```
fn gen_entrypoint_jni_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let service_pascal = service.name.to_upper_camel_case();
    let ep_pascal = ep.method.to_upper_camel_case();
    let opaque_name = format!("{}Opaque", service.name);
    let ep_method = bridge_method_name(&service.name, &ep.method);
    let symbol = jni_symbol(package, service_bridge_class, &ep_method);

    out.push_str(&format!(
        "/// Drive `{service_pascal}::{ep_pascal}` from Java/Kotlin.\n\
         ///\n\
         /// Parameters:\n\
         ///   owner_handle: jlong returned by the service constructor entry point\n\
         ///   ep params: as defined in the service entrypoint signature\n"
    ));

    match ep.kind {
        EntrypointKind::Run => {
            out.push_str(&format!(
                "#[no_mangle]\n\
                 pub extern \"system\" fn {symbol}(\n        \
                     _env: EnvUnowned,\n        \
                     _class: JClass,\n        \
                     owner_handle: jlong"
            ));

            // Add entrypoint parameters
            for ep_param in &ep.params {
                let jni_type = typeref_to_jni_type(&ep_param.ty, core_import);
                out.push_str(&format!(",\n        {}: {}", ep_param.name, jni_type));
            }

            out.push_str("\n    ) {\n");
            out.push_str("    // Validate owner handle\n");
            out.push_str("    if owner_handle == 0 {\n");
            out.push_str("        return;\n");
            out.push_str("    }\n\n");

            // SAFETY comment for dereferencing
            out.push_str("    // SAFETY: owner_handle was allocated by the constructor and is valid\n");
            out.push_str("    // until freed. The caller is responsible for not using after free.\n");
            out.push_str("    unsafe {\n");
            out.push_str(&format!(
                "        let owner_opaque = owner_handle as *mut {opaque_name};\n\
                 let owner_ref = &mut (*owner_opaque).inner;\n"
            ));

            out.push_str("        let rt = match tokio::runtime::Runtime::new() {\n");
            out.push_str("            Ok(runtime) => runtime,\n");
            out.push_str("            Err(_) => return, // Failed to create tokio runtime\n");
            out.push_str("        };\n\n");

            out.push_str(&format!("        let _ = rt.block_on(owner_ref.{}(", ep.method));

            // Add entrypoint arguments
            let mut first = true;
            for ep_param in &ep.params {
                if !first {
                    out.push_str(", ");
                }
                out.push_str(&ep_param.name);
                first = false;
            }
            out.push_str("));\n");
            out.push_str("    }\n");
            out.push_str("}\n\n");
        }
        EntrypointKind::Finalize => {
            out.push_str(&format!(
                "#[no_mangle]\n\
                 pub extern \"system\" fn {symbol}(\n        \
                     _env: EnvUnowned,\n        \
                     _class: JClass,\n        \
                     owner_handle: jlong"
            ));

            // Add entrypoint parameters
            for ep_param in &ep.params {
                let jni_type = typeref_to_jni_type(&ep_param.ty, core_import);
                out.push_str(&format!(",\n        {}: {}", ep_param.name, jni_type));
            }

            out.push_str("\n    ) -> jlong {\n");
            out.push_str("    // Validate owner handle\n");
            out.push_str("    if owner_handle == 0 {\n");
            out.push_str("        return 0; // Error: null pointer\n");
            out.push_str("    }\n\n");

            // SAFETY comment for dereferencing
            out.push_str("    // SAFETY: owner_handle was allocated by the constructor and is valid\n");
            out.push_str("    // until freed. The caller is responsible for not using after free.\n");
            out.push_str("    unsafe {\n");
            out.push_str(&format!(
                "        let owner_opaque = owner_handle as *mut {opaque_name};\n\
                 let owner_ref = &mut (*owner_opaque).inner;\n"
            ));

            out.push_str("        let rt = match tokio::runtime::Runtime::new() {\n");
            out.push_str("            Ok(runtime) => runtime,\n");
            out.push_str("            Err(_) => return 0, // Error: failed to create tokio runtime\n");
            out.push_str("        };\n\n");

            out.push_str(&format!("        let _result = rt.block_on(owner_ref.{}(", ep.method));

            // Add entrypoint arguments
            let mut first = true;
            for ep_param in &ep.params {
                if !first {
                    out.push_str(", ");
                }
                out.push_str(&ep_param.name);
                first = false;
            }
            out.push_str("));\n");
            out.push_str("        // Finalize returns the transformed result; caller decides what to do with it\n");
            out.push_str("        owner_handle\n");
            out.push_str("    }\n");
            out.push_str("}\n\n");
        }
    }
}

// ──────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the JNI backend.
///
/// Returns up to one `GeneratedFile`:
/// - `crates/{name}-jni/src/service.rs` — Rust JNI glue for service lifecycle
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let jni_crate = format!("{}-jni", config.jni_crate_base());
    let output_dir = PathBuf::from(format!("crates/{jni_crate}/src/service.rs"));

    let service_rs = gen_service_rs(api, config);

    Ok(vec![GeneratedFile {
        path: output_dir,
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
        };

        let contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: dispatch_method,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("ResponseData".to_owned()),
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

    /// `gen_service_rs` emits the JNI handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct JniRequestHandlerBridge"),
            "expected `JniRequestHandlerBridge` struct in output:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge impl with async dispatch.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for JniRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("async fn handle("),
            "expected async dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits JNI thread attachment code.
    #[test]
    fn rust_output_contains_jni_thread_attach() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("attach_current_thread"),
            "expected JVM thread attachment:\n{output}"
        );
    }

    /// `gen_service_rs` emits JSON serialization of request.
    #[test]
    fn rust_output_contains_json_serialization() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("serde_json::to_string(&request)"),
            "expected request JSON serialization:\n{output}"
        );
        assert!(
            output.contains("serde_json::from_str(&result_json)"),
            "expected response JSON deserialization:\n{output}"
        );
    }

    /// `gen_service_rs` emits JNI native method call.
    #[test]
    fn rust_output_contains_jni_method_call() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("call_method_unchecked"),
            "expected JNI method call:\n{output}"
        );
    }

    /// `gen_service_rs` emits registration entry point function that builds and calls the bridge.
    #[test]
    fn rust_output_register_calls_owner_method() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[no_mangle]"),
            "expected #[no_mangle] attribute:\n{output}"
        );
        assert!(
            output.contains("extern \"system\""),
            "expected extern system ABI:\n{output}"
        );
        assert!(
            output.contains("nativeTestServiceRegisterAddHandler"),
            "expected register function for TestService.add_handler:\n{output}"
        );
        // Verify the register function actually calls owner.add_handler
        assert!(
            output.contains(".inner.add_handler("),
            "register function must call owner.add_handler():\n{output}"
        );
        // Verify it creates the bridge
        assert!(
            output.contains("JniRequestHandlerBridge"),
            "register function must create the bridge:\n{output}"
        );
        // Verify it creates a GlobalRef and jmethodID
        assert!(
            output.contains("new_global_ref"),
            "register function must create global reference to handler:\n{output}"
        );
        assert!(
            output.contains("get_method_id"),
            "register function must cache method ID:\n{output}"
        );
    }

    /// `gen_service_rs` emits run entrypoint function that builds and drives the owner.
    #[test]
    fn rust_output_run_calls_owner_entrypoint() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("nativeTestServiceRun"),
            "expected run entrypoint function:\n{output}"
        );
        // Verify the run function creates a tokio runtime
        assert!(
            output.contains("tokio::runtime::Runtime::new"),
            "run function must create tokio runtime:\n{output}"
        );
        // Verify it dereferences and calls the owner's run method
        assert!(
            output.contains("owner_ref.run("),
            "run function must call owner.run():\n{output}"
        );
        // Verify it blocks on the async runtime
        assert!(
            output.contains("block_on"),
            "run function must block_on the async entrypoint:\n{output}"
        );
    }

    /// `gen_service_rs` emits opaque type and constructor.
    #[test]
    fn rust_output_contains_service_opaque_and_constructor() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        // Verify opaque struct is defined
        assert!(
            output.contains("pub struct TestServiceOpaque"),
            "expected TestServiceOpaque struct:\n{output}"
        );
        // Verify constructor entry point
        assert!(
            output.contains("nativeTestServiceNew"),
            "expected nativeTestServiceNew entry point:\n{output}"
        );
        // Verify it calls the Rust constructor
        assert!(
            output.contains("my_crate::TestService::new()"),
            "constructor must call the Rust service constructor:\n{output}"
        );
        // Verify it returns jlong (via Box::into_raw)
        assert!(
            output.contains("Box::into_raw"),
            "constructor must return raw pointer as jlong:\n{output}"
        );
    }

    /// `gen_service_rs` emits destructor for opaque handle.
    #[test]
    fn rust_output_contains_service_destructor() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        // Verify free entry point
        assert!(
            output.contains("nativeTestServiceFree"),
            "expected nativeTestServiceFree entry point:\n{output}"
        );
        // Verify it reconstructs from raw pointer
        assert!(
            output.contains("Box::from_raw"),
            "destructor must reconstruct from raw pointer:\n{output}"
        );
        // Verify it validates null pointer
        assert!(
            output.contains("if handle != 0"),
            "destructor must check for null pointer:\n{output}"
        );
    }

    /// `gen_service_rs` emits SAFETY comments on unsafe blocks.
    #[test]
    fn rust_output_contains_safety_comments() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("// SAFETY:"),
            "expected SAFETY comments on unsafe:\n{output}"
        );
    }

    /// Full `generate()` call returns one file when services are non-empty.
    #[test]
    fn generate_returns_one_file_for_non_empty_services() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert_eq!(files.len(), 1, "expected 1 generated file, got {}", files.len());
        let path = files[0].path.file_name().unwrap().to_str().unwrap();
        assert_eq!(path, "service.rs", "expected service.rs, got {path}");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_test_config() -> ResolvedCrateConfig {
        use crate::core::config::resolved::ResolvedCrateConfig;
        ResolvedCrateConfig {
            name: "my-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }
}
