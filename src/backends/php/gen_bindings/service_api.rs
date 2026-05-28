//! Service-API codegen for the PHP (ext-php-rs) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust ext-php-rs glue that wraps each registered PHP
//!    callable as `Arc<dyn <HandlerContractDef::trait_name>>` via a blocking
//!    callback bridge (PHP is single-threaded per request), builds the core
//!    service via the owner type's registration and run entrypoints, and exposes
//!    a `#[php_function]` entry point.
//!
//! 2. **`service.php`** — An idiomatic PHP class mirroring the service's
//!    constructor, configurator methods, and registration methods, with a
//!    `run(...)` method that delegates to the native extension.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a simple PHP type annotation string.
fn php_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float".to_owned(),
                _ => "int".to_owned(),
            }
        }
        TypeRef::Bytes => "string".to_owned(), // PHP doesn't distinguish; use string
        TypeRef::Optional(inner) => format!("?{}", php_type_annotation(inner)),
        TypeRef::Vec(_) => "array".to_owned(), // Omit inner type in annotation
        TypeRef::Map(_, _) => "array".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "mixed".to_owned(),
        TypeRef::Path => "string".to_owned(),
        TypeRef::Duration => "float".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ─────────────────────────────────────────────────────────────── PHP output ──

/// Generate the idiomatic PHP service class (`service.php`).
///
/// Produces a PHP file containing one class per service. Each class exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration methods from [`ServiceDef::registrations`].
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`]
///   entrypoint.
pub(super) fn gen_service_php(api: &ApiSurface, extension_name: &str) -> String {
    let mut out = String::new();

    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");

    // Emit one class per service
    for service in &api.services {
        gen_service_class(&mut out, service, api, extension_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, extension_name: &str) {
    let class_name = &service.name;

    // Class declaration with docblock
    if !service.doc.is_empty() {
        out.push_str(&format!("/**\n * {}\n */\n", service.doc.trim().replace('\n', "\n * ")));
    }
    out.push_str(&format!("class {class_name} {{\n"));

    // Private registrations storage
    out.push_str("    private array $registrations = [];\n\n");

    // __construct
    {
        let ctor = &service.constructor;
        let mut ctor_params = Vec::new();
        let mut ctor_assigns = Vec::new();

        for p in &ctor.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                ctor_params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                ctor_params.push(format!("{} ${}", annotation, p.name));
            }
            // Store constructor param as private property for use in run()
            ctor_assigns.push(p.name.clone());
        }

        let param_sig = ctor_params.join(", ");
        out.push_str(&format!("    public function __construct({param_sig}): void {{\n"));
        if !ctor.doc.is_empty() {
            out.push_str(&format!("        // {}\n", ctor.doc.trim()));
        }

        // Store constructor args as instance properties
        for arg in &ctor_assigns {
            out.push_str(&format!("        $this->_{arg} = ${arg};\n"));
        }
        out.push_str("    }\n\n");
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let method_name = &method.name;
        out.push_str(&format!("    public function {method_name}({param_sig}): self {{\n"));
        if !method.doc.is_empty() {
            out.push_str(&format!("        // {}\n", method.doc.trim()));
        }

        // Store each configurator param as instance property
        for p in &method.params {
            out.push_str(&format!("        $this->_{} = ${}\n", p.name, p.name));
        }
        out.push_str("        return $this;\n");
        out.push_str("    }\n\n");
    }

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, extension_name);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let mut params = Vec::new();
        for p in &ep.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&format!("    public function {ep_name}({param_sig}): void {{\n"));
                if !ep.doc.is_empty() {
                    out.push_str(&format!("        // {}\n", ep.doc.trim()));
                }

                // Build the call to the native run function
                // Convention: native fn is `{snake_service_name}_{entrypoint_name}`
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                out.push_str(&format!("        {native_fn}($this->registrations"));

                for p in &ep.params {
                    out.push_str(&format!(", ${}", p.name));
                }
                out.push_str(");\n");
                out.push_str("    }\n\n");
            }
            EntrypointKind::Finalize => {
                let return_annotation = php_type_annotation(&ep.return_type);
                out.push_str(&format!(
                    "    public function {ep_name}({param_sig}): {return_annotation} {{\n"
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format!("        // {}\n", ep.doc.trim()));
                }

                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                out.push_str(&format!("        return {native_fn}($this->registrations"));

                for p in &ep.params {
                    out.push_str(&format!(", ${}", p.name));
                }
                out.push_str(");\n");
                out.push_str("    }\n\n");
            }
        }
    }

    out.push_str("}\n\n");
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    _extension_name: &str,
) {
    let method_name = &reg.method;

    // Build metadata param signature (excluding the callback param)
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                format!("?{} ${} = null", annotation, p.name)
            } else {
                format!("{} ${}", annotation, p.name)
            }
        })
        .collect();

    // For direct registration (non-decorator), also add the callback param
    let mut direct_params = meta_params.clone();
    direct_params.push(format!("callable ${}", reg.callback_param));

    let meta_sig = meta_params.join(", ");
    let direct_sig = direct_params.join(", ");

    // Decorator factory form: returns a closure
    out.push_str(&format!("    public function {method_name}({meta_sig}): callable {{\n"));
    if !reg.doc.is_empty() {
        out.push_str(&format!("        // {}\n", reg.doc.trim()));
    }

    // Build the metadata tuple for storage
    let meta_tuple = if reg.metadata_params.is_empty() {
        "[]".to_owned()
    } else {
        let names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
        format!(
            "[{}]",
            names.iter().map(|n| format!("${}", n)).collect::<Vec<_>>().join(", ")
        )
    };

    out.push_str(&format!(
        "        return function (callable ${callback_param}) {{\n            \
         $this->registrations[] = [\"{method_name}\", {meta_tuple}, ${callback_param}];\n            \
         return ${callback_param};\n        \
         }};\n",
        callback_param = reg.callback_param,
    ));
    out.push_str("    }\n\n");

    // Also expose a direct (non-decorator) variant: `register_{method_name}`
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        out.push_str(&format!("    public function {direct_name}({direct_sig}): self {{\n"));
        out.push_str(&format!(
            "        $this->registrations[] = [\"{method_name}\", {meta_tuple}, ${callback_param}];\n",
            callback_param = reg.callback_param,
        ));
        out.push_str("        return $this;\n");
        out.push_str("    }\n\n");
    }
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust ext-php-rs glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Php{ContractName}Bridge` struct that wraps a PHP callable (stored as
///   an index into a thread-local registry) and `impl`s the handler contract trait.
///   Since PHP is single-threaded per request, the async dispatch blocks on
///   the Tokio runtime.
/// - A `#[php_function]` `{snake_service}_{entrypoint}` that accepts the
///   collected registrations list and any entrypoint params, builds the native
///   service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    // File-level allow attributes to keep clippy happy in generated code
    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async)]\n\n");
    out.push_str("use ext_php_rs::prelude::*;\n");
    out.push_str("use ext_php_rs::types::{ZendCallable, Zval};\n");
    out.push_str("use std::panic::AssertUnwindSafe;\n");
    out.push_str("use std::sync::Arc;\n\n");

    // Global handler registry (thread-local since Zval is not Send/Sync)
    out.push_str("thread_local! {\n");
    out.push_str("    static PHP_HANDLER_REGISTRY: std::cell::RefCell<Vec<ZendCallable<'static>>> =\n");
    out.push_str("        const { std::cell::RefCell::new(Vec::new()) };\n");
    out.push_str("}\n\n");

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

    // Emit one php_function per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_php_function(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Php{ContractName}Bridge` struct + trait impl.
///
/// Stores the handler callable as an index into the thread-local registry
/// (since ZendCallable is not Send/Sync). When dispatched, retrieves the
/// callable, invokes it synchronously via the PHP FFI, serializes the result,
/// and blocks the Tokio executor on the response deserialization.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Php{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Build req/resp paths: if wire type includes "::", strip it; otherwise prefix with core_import
    let req_path = if req_type.contains("::") {
        req_type.split("::").last().unwrap_or(req_type).to_string()
    } else if req_type == "Value" || req_type == "serde_json::Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type.contains("::") {
        resp_type.split("::").last().unwrap_or(resp_type).to_string()
    } else if resp_type == "Value" || resp_type == "serde_json::Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{resp_type}")
    };

    // Extra dispatch parameters the bridge ignores (leading verbatim params)
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    // The future's `Output` is the contract dispatch's real return type when the library
    // supplies one (`dispatch_return_type`); otherwise the bridge yields the wire response
    // wrapped in a boxed-error `Result`. When a `response_adapter` is configured, the inner
    // fallible computation produces the wire `Result` and the adapter converts it into the
    // dispatch return type.
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

    out.push_str(&format!(
        "/// Generated ext-php-rs bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps a PHP callable (stored as an index in a thread-local registry)\n\
         /// so it can be used as `Arc<dyn {trait_name}>` from Rust async code.\n\
         /// Dispatch blocks on the Tokio runtime (PHP is single-threaded per request).\n\
         pub struct {bridge_name} {{\n    \
             handler_index: usize,\n\
         }}\n\n"
    ));

    out.push_str(&format!(
        "impl {bridge_name} {{\n    \
             /// Create a bridge from a handler index.\n    \
             pub fn new(handler_index: usize) -> Self {{\n        \
                 Self {{ handler_index }}\n    \
             }}\n\
         }}\n\n"
    ));

    // Safety: The bridge holds a usize (immutable). No unsafe.
    out.push_str(&format!(
        "// SAFETY: The bridge holds only a usize (immutable, Copy).\n\
         // PHP handler registry lookup is thread-safe via thread-local RefCell.\n\
         impl Send for {bridge_name} {{}}\n\
         impl Sync for {bridge_name} {{}}\n\n"
    ));

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
                 Box::pin(async move {{\n            \
                     // Invoke the PHP callable synchronously (blocking)\n            \
                     let outcome: {wire_output} = (async {{\n                \
                         // Serialize the request to JSON for PHP roundtrip\n                \
                         let req_json = serde_json::to_string(&{wire_name})\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                         let raw_result = std::panic::catch_unwind(AssertUnwindSafe(|| {{\n                    \
                             PHP_HANDLER_REGISTRY.with(|registry| -> Result<String, String> {{\n                        \
                                 let registry = registry.borrow();\n                        \
                                 let Some(callable) = registry.get(self.handler_index) else {{\n                            \
                                     return Err(format!(\"Handler not found at index {{}}\", self.handler_index));\n                            \
                                 }};\n\n                        \
                                 // Deserialize JSON request into PHP object\n                        \
                                 let req_obj = serde_json::from_str::<serde_json::Value>(&req_json)\n                            \
                                     .map_err(|e| e.to_string())?;\n                        \
                                 let req_zval = serde_json::json!(req_obj).into();\n\n                        \
                                 // Invoke the callable\n                        \
                                 let resp_zval = callable.try_call(vec![&req_zval])\n                            \
                                     .map_err(|e| format!(\"PHP callable invocation failed: {{:?}}\", e))?;\n\n                        \
                                 // Serialize response back to JSON\n                        \
                                 Ok(serde_json::to_string(&resp_zval).unwrap_or_else(|_| \"{{}}\".to_string()))\n                    \
                             }})\n                \
                         }}))\n                    \
                         .map_err(|_| Box::new(std::io::Error::new(\n                        \
                             std::io::ErrorKind::Other,\n                        \
                             \"PHP handler panicked\",\n                \
                         )) as {box_err})?\n                    \
                         .map_err(|e| Box::new(std::io::Error::new(\n                        \
                             std::io::ErrorKind::Other,\n                        \
                             e,\n                \
                         )) as {box_err})?;\n\n                    \
                         // Deserialize the JSON result back into the wire response DTO.\n                    \
                         let response: {resp_path} = serde_json::from_str(&raw_result)\n                        \
                             .map_err(|e| Box::new(e) as {box_err})?;\n                    \
                         Ok(response)\n            \
                     }}).await;\n\n            \
                     {tail}\n        \
                 }})\n    \
             }}\n\
         }}\n\n"
    ));
}

/// Emit the `#[php_function]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list (`array<array{string, array, callable}>`).
/// 2. Constructs the native service owner via its constructor.
/// 3. Iterates registrations, wraps each callable in the appropriate bridge,
///    and calls the owner's registration method.
/// 4. Calls the owner's entrypoint (blocking if Run, synchronous if Finalize).
fn gen_run_php_function(
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
    let mut rust_params = vec!["registrations: &Bound<'_, Zval>".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    out.push_str(&format!(
        "/// Drive `{owner_path}::{ep_method}` from PHP.\n\
         ///\n\
         /// Each entry in `registrations` is an array of `[method_name, metadata_array, callable]`\n\
         /// produced by the PHP service class.\n\
         #[php_function]\n\
         pub fn {fn_name}({param_sig}) -> PhpResult<()> {{\n"
    ));

    // Build the owner instance via its constructor
    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&format!("    let mut owner = {ctor_call};\n\n"));

    // Iterate registrations and dispatch
    out.push_str("    // Register all handlers with the owner\n");
    out.push_str("    if let Ok(reg_arr) = registrations.try_into::<Vec<Zval>>() {\n");
    out.push_str("        for entry in reg_arr {\n");
    out.push_str("            if let Ok(tuple) = entry.try_into::<Vec<Zval>>() {\n");
    out.push_str("                if tuple.len() < 3 {\n");
    out.push_str(
        "                    return Err(PhpException::default(\"Invalid registration tuple length\".into()));\n",
    );
    out.push_str("                }\n");
    out.push_str("                let method_name: String = tuple[0].try_into()?;\n");
    out.push_str("                let callable = tuple[2].clone();\n\n");

    // Dispatch on method name
    out.push_str("                match method_name.as_str() {\n");
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Php{}Bridge", contract.trait_name.to_upper_camel_case());
            let meta_count = reg.metadata_params.len();

            out.push_str(&format!("                    \"{reg_method}\" => {{\n"));

            // Store the callable in the registry and get its index
            out.push_str("                        let handler_index = PHP_HANDLER_REGISTRY.with(|registry| {\n");
            out.push_str("                            let mut registry = registry.borrow_mut();\n");
            out.push_str("                            let idx = registry.len();\n");
            out.push_str("                            // Convert Zval to ZendCallable\n");
            out.push_str(
                "                            if let Ok(zen_callable) = ZendCallable::new_owned(callable.clone()) {\n",
            );
            out.push_str("                                registry.push(zen_callable);\n");
            out.push_str("                                idx\n");
            out.push_str("                            } else {\n");
            out.push_str("                                usize::MAX\n");
            out.push_str("                            }\n");
            out.push_str("                        });\n");
            out.push_str("                        if handler_index == usize::MAX {\n");
            out.push_str("                            return Err(PhpException::default(\"Failed to register callable\".into()));\n");
            out.push_str("                        }\n\n");

            out.push_str(&format!(
                "                        let bridge = {bridge_name}::new(handler_index);\n"
            ));
            out.push_str(&format!(
                "                        let handler: Arc<dyn {core_import}::{contract_name}> = Arc::new(bridge);\n"
            ));

            if meta_count > 0 {
                out.push_str("                        let meta: Vec<Zval> = tuple[1].clone().try_into()?;\n");
                for (i, meta_param) in reg.metadata_params.iter().enumerate() {
                    let rust_ty = typeref_to_rust_type(&meta_param.ty, core_import);
                    out.push_str(&format!(
                        "                        let {}: {} = meta.get({i}).ok_or_else(|| PhpException::default(\"Missing metadata at index {i}\".into()))?.try_into()?;\n",
                        meta_param.name, rust_ty,
                    ));
                }
                let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
                out.push_str(&format!(
                    "                        owner.{reg_method}({}, handler)\n",
                    meta_args.join(", ")
                ));
            } else {
                out.push_str(&format!("                        owner.{reg_method}(handler)\n"));
            }

            // Handle error if the registration is fallible
            if reg.error_type.is_some() {
                out.push_str("                            .map_err(|e| PhpException::default(e.to_string()))?;\n");
            } else {
                out.push_str("                            ;\n");
            }
            out.push_str("                    }\n");
        }
    }
    out.push_str("                    _ => {\n");
    out.push_str(
        "                        return Err(PhpException::default(\n                            \
         format!(\"unknown registration method: {method_name}\"),\n                        ));\n",
    );
    out.push_str("                    }\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    // Call the entrypoint
    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str("    Ok(())\n}\n\n");
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        // For a first-pass implementation where constructor params are not
        // yet threaded through, fall back to Default if available; otherwise
        // use new() with zero-value placeholders.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");

    if ep.is_async {
        // Use tokio::runtime::Handle::current().block_on for async entrypoints.
        // This assumes a Tokio runtime is already active (as in the PHP bridge invocations).
        if args_str.is_empty() {
            format!(
                "    tokio::runtime::Handle::current()\n        \
                 .block_on(owner.{ep_method}())\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        } else {
            format!(
                "    tokio::runtime::Handle::current()\n        \
                 .block_on(owner.{ep_method}({args_str}))\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        }
    } else {
        if ep.error_type.is_some() {
            if args_str.is_empty() {
                format!(
                    "    owner.{ep_method}()\n        \
                     .map_err(|e| PhpException::default(e.to_string()))?;\n"
                )
            } else {
                format!(
                    "    owner.{ep_method}({args_str})\n        \
                     .map_err(|e| PhpException::default(e.to_string()))?;\n"
                )
            }
        } else {
            if args_str.is_empty() {
                format!("    owner.{ep_method}();\n")
            } else {
                format!("    owner.{ep_method}({args_str});\n")
            }
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

/// Generate all service-API files for the PHP backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust ext-php-rs glue
/// - `{php_pkg}/Service.php`     — idiomatic PHP class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("php"), &config.name, "crates/{name}-php/src/");

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // PHP wrapper
    // Extension name matches the Rust crate name with hyphens replaced by underscores.
    let extension_name = config.name.replace('-', "_");
    let service_php = gen_service_php(api, &extension_name);

    // PHP package output base (same logic as generate_public_api)
    let output_base = config
        .php
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let package_name = config.name.replace('-', "_");
            PathBuf::from(format!("packages/php/{}", package_name))
        });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("Service.php"),
            content: service_php,
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

    /// `gen_service_php` emits a class named after the service owner.
    #[test]
    fn php_output_contains_service_class() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("class TestService"),
            "expected `class TestService` in output:\n{output}"
        );
    }

    /// `gen_service_php` emits `__construct` with registrations initialization.
    #[test]
    fn php_output_contains_construct_with_registrations() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function __construct()"),
            "expected `public function __construct()` in output:\n{output}"
        );
        assert!(
            output.contains("private array $registrations"),
            "expected `private array $registrations` in output:\n{output}"
        );
    }

    /// `gen_service_php` emits configurator methods that return `self`.
    #[test]
    fn php_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function with_timeout"),
            "expected `with_timeout` configurator:\n{output}"
        );
        assert!(
            output.contains("return $this"),
            "expected `return $this` in configurator:\n{output}"
        );
    }

    /// `gen_service_php` emits a registration method returning a closure.
    #[test]
    fn php_output_contains_registration_method() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
        assert!(
            output.contains("return function"),
            "expected inner `return function` closure:\n{output}"
        );
        assert!(
            output.contains("$this->registrations[]"),
            "expected `$this->registrations[]` append in registration:\n{output}"
        );
    }

    /// `gen_service_php` emits the `run` entrypoint.
    #[test]
    fn php_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function run("),
            "expected `public function run(` entrypoint:\n{output}"
        );
        assert!(
            output.contains("test_service_run("),
            "expected native call `test_service_run(` in run:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct PhpRequestHandlerBridge"),
            "expected `PhpRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for PhpRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[php_function]` run entry point.
    #[test]
    fn rust_output_contains_php_function_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[php_function]"),
            "expected `#[php_function]` attribute:\n{output}"
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
        assert!(paths.contains(&"Service.php"), "expected Service.php in output");
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
