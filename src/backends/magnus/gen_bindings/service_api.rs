//! Service-API codegen for the Magnus (Ruby) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rb`** — An idiomatic Ruby class mirroring the service's constructor,
//!    configurator methods, registration methods accepting blocks/procs, and a `run`
//!    entrypoint that delegates to the native extension.
//!
//! 2. **`service.rs`** — Magnus glue that wraps each registered Ruby proc as
//!    `Arc<dyn <HandlerContractDef::trait_name>>` via an async callback bridge.
//!    The bridge acquires the GVL (Global VM Lock) to call the proc with request
//!    DTO and interprets the response. Also defines native `#[magnus::function]` for
//!    the run entrypoint that collects registrations, builds the core service,
//!    and drives it.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a simple Ruby type annotation string.
fn ruby_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "true | false".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_owned(),
                _ => "Integer".to_owned(),
            }
        }
        TypeRef::Bytes => "String".to_owned(),
        TypeRef::Optional(inner) => format!("{} | nil", ruby_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("Array[{}]", ruby_type_annotation(inner)),
        TypeRef::Map(k, v) => format!("Hash[{}, {}]", ruby_type_annotation(k), ruby_type_annotation(v)),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "Object".to_owned(),
        TypeRef::Path => "String".to_owned(),
        TypeRef::Duration => "Float".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ─────────────────────────────────────────────────────────── Ruby output ──

/// Generate the idiomatic Ruby service class (`service.rb`).
///
/// Produces a Ruby module containing one class per service. Each class exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration methods from [`ServiceDef::registrations`] that accept blocks.
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`] entrypoint.
pub(super) fn gen_service_rb(api: &ApiSurface, native_module_name: &str) -> String {
    let mut out = String::new();

    out.push_str("# frozen_string_literal: true\n\n");
    out.push_str(&format!("require \"{native_module_name}\"\n\n"));

    for service in &api.services {
        gen_service_class(&mut out, service, api, native_module_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, native_module_name: &str) {
    let class_name = &service.name;

    // Class comment
    if !service.doc.is_empty() {
        out.push_str(&format!("# {}\n", service.doc.trim()));
    }
    out.push_str(&format!("class {class_name}\n"));

    // initialize
    {
        let ctor = &service.constructor;
        let mut init_params = Vec::new();
        let mut stored_args = Vec::new();

        for p in &ctor.params {
            let type_annotation = ruby_type_annotation(&p.ty);
            if p.optional {
                init_params.push(format!("{}: {} | nil = nil", p.name, type_annotation));
            } else {
                init_params.push(format!("{}: {}", p.name, type_annotation));
            }
            stored_args.push(p.name.clone());
        }

        let param_sig = if init_params.is_empty() {
            String::new()
        } else {
            format!("({})", init_params.join(", "))
        };

        out.push_str(&format!("  def initialize{param_sig}\n"));
        if !ctor.doc.is_empty() {
            out.push_str(&format!("    # {}\n", ctor.doc.trim()));
        }
        out.push_str("    @registrations = []\n");

        // Store constructor params as instance state
        for arg in &stored_args {
            out.push_str(&format!("    @{arg} = {arg}\n"));
        }

        out.push_str("  end\n\n");
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            let annotation = ruby_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} | nil = nil", p.name, annotation));
            } else {
                params.push(format!("{}: {}", p.name, annotation));
            }
        }
        let param_sig = if params.is_empty() {
            String::new()
        } else {
            format!("({})", params.join(", "))
        };
        let method_name = &method.name;

        out.push_str(&format!("  def {method_name}{param_sig}\n"));
        if !method.doc.is_empty() {
            out.push_str(&format!("    # {}\n", method.doc.trim()));
        }

        // Store each configurator param as instance state
        for p in &method.params {
            out.push_str(&format!("    @{} = {}\n", p.name, p.name));
        }
        out.push_str("    self\n");
        out.push_str("  end\n\n");
    }

    // Registration methods accepting blocks
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, native_module_name);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let mut params = Vec::new();
        for p in &ep.params {
            let annotation = ruby_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} | nil = nil", p.name, annotation));
            } else {
                params.push(format!("{}: {}", p.name, annotation));
            }
        }
        let param_sig = if params.is_empty() {
            String::new()
        } else {
            format!("({})", params.join(", "))
        };
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&format!("  def {ep_name}{param_sig}\n"));
                if !ep.doc.is_empty() {
                    out.push_str(&format!("    # {}\n", ep.doc.trim()));
                }
                // Convention: native fn is `{snake_service_name}_{entrypoint_name}`
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                out.push_str(&format!("    {native_module_name}.{native_fn}(@registrations"));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n");
                out.push_str("  end\n\n");
            }
            EntrypointKind::Finalize => {
                out.push_str(&format!("  def {ep_name}{param_sig}\n"));
                if !ep.doc.is_empty() {
                    out.push_str(&format!("    # {}\n", ep.doc.trim()));
                }
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                out.push_str(&format!("    {native_module_name}.{native_fn}(@registrations"));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n");
                out.push_str("  end\n\n");
            }
        }
    }

    out.push_str("end\n\n");
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    _native_module_name: &str,
) {
    let method_name = &reg.method;

    // Build metadata param signature (excluding the callback param)
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = ruby_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} | nil = nil", p.name, annotation)
            } else {
                format!("{}: {}", p.name, annotation)
            }
        })
        .collect();

    let param_sig = if meta_params.is_empty() {
        "(&block)".to_owned()
    } else {
        format!("({}, &block)", meta_params.join(", "))
    };

    out.push_str(&format!("  def {method_name}{param_sig}\n"));
    if !reg.doc.is_empty() {
        out.push_str(&format!("    # {}\n", reg.doc.trim()));
    }

    // Collect metadata param names for the tuple
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{}]", meta_names.join(", "))
    };

    out.push_str(&format!(
        "    @registrations.push([\"{method_name}\", {meta_tuple}, block])\n"
    ));
    out.push_str("    self\n");
    out.push_str("  end\n\n");
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Magnus Rust glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Rb{ContractName}Bridge` struct that wraps a `Opaque<Value>` callable
///   and `impl`s the handler contract trait, acquiring the GVL (Global VM Lock)
///   to call the proc synchronously with JSON request/response.
/// - A `#[magnus::function]` `{snake_service}_{entrypoint}` that accepts
///   the collected registrations list and any entrypoint params, builds the
///   native service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    // File-level allow attributes to keep clippy happy in generated code
    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async)]\n\n");
    out.push_str("use magnus::{method, prelude::*, Value, Opaque, RArray, RHash};\n");
    out.push_str("use std::sync::Arc;\n\n");

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

    // Emit one function per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_function(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Rb{ContractName}Bridge` struct + trait impl.
///
/// The bridge wraps a Ruby proc (stored as `Opaque<Value>`) and implements
/// the handler contract trait. It acquires the GVL via `Ruby::with_gvl()` to
/// call the proc with JSON request/response serialization.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Rb{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types — use plain serde_json::Value, not re-exported from core
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Special handling: if the wire type includes the core import prefix, strip it
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

    // Leading dispatch parameters the bridge ignores (e.g. a foreign framework type the
    // contract's dispatch method receives but the wire bridge does not consume). Their concrete
    // types cannot be reconstructed from the sanitized surface, so the library supplies them
    // verbatim via `dispatch_extra_params`. Each is emitted as a `, {decl}` prefix argument.
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    out.push_str(&format!(
        "/// Generated Magnus bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps a Ruby proc so it can be used as `Arc<dyn {trait_name}>`\n\
         /// from Rust async code. Calls the proc with GVL acquired.\n\
         pub struct {bridge_name} {{\n    \
             proc_handle: Opaque<Value>,\n\
         }}\n\n"
    ));

    out.push_str(&format!(
        "impl {bridge_name} {{\n    \
             /// Create a bridge from a Ruby proc.\n    \
             pub fn new(proc_handle: Opaque<Value>) -> Self {{\n        \
                 Self {{ proc_handle }}\n    \
             }}\n\
         }}\n\n"
    ));

    // Safety: Opaque<Value> is Send+Sync because Magnus uses internal locking.
    out.push_str(&format!(
        "// SAFETY: Opaque<Value> is Send+Sync; calls acquire the GVL.\n\
         unsafe impl Send for {bridge_name} {{}}\n\
         unsafe impl Sync for {bridge_name} {{}}\n\n"
    ));

    // Trait impl — build the request and response paths using core_import
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

    // The future's `Output` is the contract dispatch's real return type when the library
    // supplies one (`dispatch_return_type`); otherwise the bridge yields the wire response
    // wrapped in a boxed-error `Result`. When a `response_adapter` is configured, the inner
    // fallible computation produces the wire `Result` and the adapter converts it into the
    // dispatch return type — keeping the generator ignorant of the library's response model.
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

    // Returns a boxed future directly (canonical object-safe async-trait shape)
    // rather than via the async_trait macro, matching a contract whose dispatch
    // method is hand-written as `-> Pin<Box<dyn Future<..> + Send + '_>>`.
    out.push_str(&format!(
        "impl {core_import}::{trait_name} for {bridge_name} {{\n    \
             fn {dispatch_name}(\n        \
                 &self{extra_param},\n        \
                 {wire_name}: {req_path},\n    \
             ) -> std::pin::Pin<Box<dyn std::future::Future<Output = {output_type}> + Send + '_>> {{\n        \
                 Box::pin(async move {{\n            \
                     // Call the Ruby proc with the GVL.\n            \
                     // Ruby procs are synchronous, so we block_on in a spawn_blocking.\n            \
                     let outcome: {wire_output} = async move {{\n                \
                         // Serialize the request to JSON\n                \
                         let req_json = serde_json::to_string(&{wire_name})\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                         let resp_json = tokio::task::spawn_blocking({{\n                    \
                             let proc_handle = self.proc_handle.clone();\n                    \
                             let req_json = req_json.clone();\n                    \
                             move || {{\n                        \
                                 Ruby::with_gvl(|ruby| {{\n                            \
                                     let proc_value = proc_handle.get_inner_with(&ruby);\n\n                            \
                                     // Parse request JSON into a Ruby Hash\n                            \
                                     let json_mod = ruby.eval::<_, Value>(\"JSON\").map_err(|e| {{\n                                \
                                         Box::new(e) as {box_err}\n                            \
                                     }})?;\n                            \
                                     let req_hash = json_mod\n                                \
                                         .funcall::<_, _, Value>(\"parse\", (&req_json,))\n                                \
                                         .map_err(|e| Box::new(e) as {box_err})?;\n\n                            \
                                     // Call the proc with the request hash\n                            \
                                     let result = proc_value\n                                \
                                         .funcall::<_, _, Value>(\"call\", (req_hash,))\n                                \
                                         .map_err(|e| Box::new(e) as {box_err})?;\n\n                            \
                                     // Serialize result back to JSON\n                            \
                                     let resp_json_str = json_mod\n                                \
                                         .funcall::<_, _, String>(\"generate\", (result,))\n                                \
                                         .map_err(|e| Box::new(e) as {box_err})?;\n\n                            \
                                     Ok::<String, {box_err}>(resp_json_str)\n                        \
                                 }})\n                \
                             }}\n            \
                         }})\n            \
                         .await\n            \
                         .map_err(|e| Box::new(e) as {box_err})??\n            \
                         .map_err(|e| Box::new(e) as {box_err})?;\n\n            \
                         // Deserialize the JSON result back into the wire response DTO.\n            \
                         let response: {resp_path} = serde_json::from_str(&resp_json)\n                \
                             .map_err(|e| Box::new(e) as {box_err})?;\n            \
                         Ok(response)\n        \
                     }}\n        \
                     .await;\n\n        \
                     {tail}\n        \
                 }})\n    \
             }}\n\
         }}\n\n"
    ));
}

/// Emit the `#[magnus::function]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations array (each entry is `[method_name, metadata_array, proc]`).
/// 2. Constructs the native service owner via its constructor as a mutable owned value.
/// 3. Iterates registrations (within GVL), wraps each proc in the appropriate bridge,
///    and calls the owner's registration method with the handler trait object.
/// 4. Releases GVL and calls the owner's entrypoint (blocking if sync, driving async via block_on).
fn gen_run_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature: registrations + entrypoint params
    let mut rust_params = vec!["registrations: &Opaque<Value>".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    out.push_str(&format!(
        "/// Drive `{owner_path}::{ep_method}` from Ruby.\n\
         ///\n\
         /// Each entry in `registrations` is a `[method_name, metadata_array, proc]` triple\n\
         /// produced by the Ruby service class. Constructs an owned service instance,\n\
         /// registers all handlers (acquiring GVL for each Ruby proc call), then invokes\n\
         /// the entrypoint.\n\
         #[magnus::function]\n\
         pub fn {fn_name}({param_sig}) -> magnus::error::Result<()> {{\n"
    ));

    // Build the owner instance as a mutable owned value (no Arc<Mutex<…>> wrapping)
    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&format!("    let mut owner = {ctor_call};\n\n"));

    // Iterate registrations and dispatch (within GVL context)
    out.push_str("    Ruby::with_gvl(|ruby| {\n");
    out.push_str("        let regs_value = registrations.get_inner_with(&ruby);\n");
    out.push_str("        let regs_array = RArray::try_convert(regs_value)\n");
    out.push_str("            .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?;\n\n");

    out.push_str("        for entry in regs_array.iter() {\n");
    out.push_str("            let entry_array = RArray::try_convert(entry)\n");
    out.push_str("                .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?;\n");
    out.push_str("            let method_name: String = entry_array.get::<String>(0)\n");
    out.push_str("                .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?;\n");
    out.push_str("            let proc_value = entry_array.get::<Value>(2)\n");
    out.push_str("                .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?;\n\n");

    // Dispatch on method name
    out.push_str("            match method_name.as_str() {\n");
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Rb{}Bridge", contract.trait_name.to_upper_camel_case());
            let meta_count = reg.metadata_params.len();

            out.push_str(&format!("                \"{reg_method}\" => {{\n"));
            out.push_str(&format!(
                "                    let bridge = {bridge_name}::new(Opaque::new(proc_value));\n"
            ));
            // Create handler trait object — no generic parameter needed
            out.push_str(&format!(
                "                    let handler: Arc<dyn {core_import}::{contract_name}> = Arc::new(bridge);\n"
            ));

            if meta_count > 0 {
                out.push_str("                    let meta_array = RArray::try_convert(entry_array.get::<Value>(1)\n");
                out.push_str("                        .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?)\n");
                out.push_str("                        .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?;\n");

                for (i, meta_param) in reg.metadata_params.iter().enumerate() {
                    let rust_ty = typeref_to_rust_type(&meta_param.ty, core_import);
                    let extract_ty = match &meta_param.ty {
                        TypeRef::String | TypeRef::Char => "String".to_owned(),
                        TypeRef::Primitive(p) => {
                            use crate::core::ir::PrimitiveType;
                            match p {
                                PrimitiveType::Bool => "bool".to_owned(),
                                PrimitiveType::F32 | PrimitiveType::F64 => "f64".to_owned(),
                                _ => "i64".to_owned(),
                            }
                        }
                        _ => "Value".to_owned(),
                    };
                    out.push_str(&format!(
                        "                    let {}: {} = meta_array.get::<{}>({})\n",
                        meta_param.name, rust_ty, extract_ty, i
                    ));
                    out.push_str("                        .map_err(|e| magnus::Error::new(ruby.exception_type_error(), e.to_string()))?;\n");
                }

                let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
                out.push_str(&format!(
                    "                    owner.{reg_method}({}, handler)\n",
                    meta_args.join(", ")
                ));
            } else {
                out.push_str(&format!("                    owner.{reg_method}(handler)\n"));
            }

            // Handle error if the registration is fallible
            if reg.error_type.is_some() {
                out.push_str(
                    "                        .map_err(|e| magnus::Error::new(ruby.exception_runtime_error(), e.to_string()))?;\n",
                );
            } else {
                out.push_str("                        ;\n");
            }
            out.push_str("                }\n");
        }
    }
    out.push_str("                _ => {\n");
    out.push_str(
        "                    return Err(magnus::Error::new(\n                        ruby.exception_arg_error(),\n                        format!(\"unknown registration method: {method_name}\"),\n                    ));\n",
    );
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        Ok::<(), magnus::Error>(())\n");
    out.push_str("    }).map_err(|e| e)?;\n\n");

    // Call the entrypoint on the owned, registered owner
    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str("    Ok(())\n}\n\n");
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        // For now, zero-arg constructor. Can be extended to thread constructor params.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
///
/// For async entrypoints that consume `self` (e.g., `run(self)`), `block_on` moves
/// the owned `owner` into the async function. For sync entrypoints, call directly.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");

    if ep.is_async {
        // For async, use tokio::runtime::Handle::current().block_on() to drive the future.
        // The owned `owner` is moved into the async function if the method takes `self`.
        format!(
            "    tokio::runtime::Handle::current()\n        \
             .block_on(owner.{ep_method}({args_str}))\n        \
             .map_err(|e| magnus::Error::new(magnus::exception::runtime_error(), e.to_string()))?;\n"
        )
    } else {
        if ep.error_type.is_some() {
            format!(
                "    owner.{ep_method}({args_str})\n        \
                 .map_err(|e| magnus::Error::new(magnus::exception::runtime_error(), e.to_string()))?;\n"
            )
        } else {
            format!("    owner.{ep_method}({args_str});\n")
        }
    }
}

/// Map a `TypeRef` to a Rust type string for use in generated function signatures.
fn typeref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
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
        TypeRef::Bytes => "Vec<u8>".to_owned(),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_rust_type(k, core_import),
            typeref_to_rust_type(v, core_import)
        ),
        TypeRef::Unit => "()".to_owned(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
        TypeRef::Json => "serde_json::Value".to_owned(),
        TypeRef::Path => "std::path::PathBuf".to_owned(),
        TypeRef::Duration => "std::time::Duration".to_owned(),
    }
}

// ──────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the Magnus backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Magnus Rust glue
/// - `{ruby_pkg}/service.rb`     — idiomatic Ruby class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("ruby"), &config.name, "crates/{name}-rb/src/");
    // Convert crate name to PascalCase module name (same as in mod.rs)
    let native_module_name = api.crate_name.to_upper_camel_case();

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // Ruby wrapper
    let service_rb = gen_service_rb(api, &native_module_name);

    // Ruby package output base (same logic as public_api)
    let gem_name = config.ruby_gem_name();
    let output_base = config
        .ruby
        .as_ref()
        .and_then(|r| r.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let gem_name_snake = gem_name.replace('-', "_");
            PathBuf::from(format!("packages/ruby/{}", gem_name_snake))
        });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("service.rb"),
            content: service_rb,
            generated_header: true,
        },
    ])
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
        ServiceDef, TypeRef,
    };

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one configurator, one registration
    ///   (bound to an async handler contract), and Run + Finalize entrypoints.
    /// - One [`HandlerContractDef`] with wire request/response DTO names.
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

        let configurator = MethodDef {
            name: "with_timeout".to_owned(),
            params: vec![ParamDef {
                name: "timeout_ms".to_owned(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("TestService".to_owned()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Set request timeout.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
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
            metadata_params: vec![
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Register a request handler for a path and method.".to_owned(),
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

        let finalize_ep = EntrypointDef {
            method: "into_router".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Named("Router".to_owned()),
            error_type: None,
            doc: "Consume and convert into a router.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![configurator],
            registrations: vec![registration],
            entrypoints: vec![run_ep, finalize_ep],
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

    /// `gen_service_rb` emits a class named after the service owner.
    #[test]
    fn ruby_output_contains_service_class() {
        let surface = make_fixture_surface();
        let output = gen_service_rb(&surface, "MyCrate");
        assert!(
            output.contains("class TestService"),
            "expected `class TestService` in output:\n{output}"
        );
    }

    /// `gen_service_rb` emits `initialize` with registrations state init.
    #[test]
    fn ruby_output_contains_initialize_with_registrations() {
        let surface = make_fixture_surface();
        let output = gen_service_rb(&surface, "MyCrate");
        assert!(
            output.contains("def initialize"),
            "expected `def initialize` in output:\n{output}"
        );
        assert!(
            output.contains("@registrations = []"),
            "expected `@registrations = []` in output:\n{output}"
        );
    }

    /// `gen_service_rb` emits configurator methods that return `self`.
    #[test]
    fn ruby_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_rb(&surface, "MyCrate");
        assert!(
            output.contains("def with_timeout(timeout_ms: Integer)"),
            "expected `with_timeout` configurator:\n{output}"
        );
        assert!(
            output.contains("self"),
            "expected `self` return in configurator:\n{output}"
        );
    }

    /// `gen_service_rb` emits a registration method accepting a block.
    #[test]
    fn ruby_output_contains_registration_block_param() {
        let surface = make_fixture_surface();
        let output = gen_service_rb(&surface, "MyCrate");
        assert!(
            output.contains("def add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
        assert!(
            output.contains("&block"),
            "expected `&block` parameter in registration:\n{output}"
        );
        assert!(
            output.contains("@registrations.push"),
            "expected `@registrations.push` in registration:\n{output}"
        );
    }

    /// `gen_service_rb` emits the `run` entrypoint.
    #[test]
    fn ruby_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_rb(&surface, "MyCrate");
        assert!(output.contains("def run("), "expected `def run(` entrypoint:\n{output}");
        assert!(
            output.contains(".test_service_run("),
            "expected native call `.test_service_run(` in run:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct RbRequestHandlerBridge"),
            "expected `RbRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for RbRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits GVL handling via Ruby::with_gvl.
    #[test]
    fn rust_output_contains_gvl_handling() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("Ruby::with_gvl"),
            "expected `Ruby::with_gvl` for GVL handling:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[magnus::function]` run entry point.
    #[test]
    fn rust_output_contains_magnus_function_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[magnus::function]"),
            "expected `#[magnus::function]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
        );
    }

    /// `gen_service_rs` emits registration dispatch via `match method_name`.
    #[test]
    fn rust_output_contains_registration_dispatch() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("\"add_handler\""),
            "expected `\"add_handler\"` match arm:\n{output}"
        );
        assert!(
            output.contains("Arc<dyn my_crate::RequestHandler>"),
            "expected Arc wrapping of handler:\n{output}"
        );
    }

    /// Full `generate()` call returns two files when services are non-empty.
    #[test]
    fn generate_returns_two_files_for_non_empty_services() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert_eq!(files.len(), 2, "expected 2 generated files, got {}", files.len());
        let paths: Vec<&str> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"service.rs"), "expected service.rs in output");
        assert!(paths.contains(&"service.rb"), "expected service.rb in output");
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
