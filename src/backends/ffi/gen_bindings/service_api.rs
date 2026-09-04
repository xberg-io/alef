//! Service-API codegen for the FFI backend.
//!
//! Generates a single output per `ApiSurface` with non-empty services:
//!
//! **`service.rs`** — C ABI contract for service ownership and handler registration.
//!
//! Exports:
//! - For each [`ServiceDef`]: opaque `*mut <service_name>` handle + constructor/destructor.
//! - For each [`RegistrationDef`]: a registration function accepting a callback, its paired
//!   response deallocator, and metadata.
//! - For each [`EntrypointDef`]: a run/finalize function that builds the service, registers
//!   callbacks via a Rust bridge, and invokes the entrypoint.
//! - A callback typedef shared across all handler contracts.
//!
//! Ownership: Every `*mut T` is caller-owned; each service type has a matching `_free` function.
//! Error handling: C callbacks return null-terminated JSON strings; parsing errors are
//! logged and cause the handler dispatch to return an error JSON response.

use crate::codegen::c_consumer;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, RegistrationVariant, ServiceDef, TypeRef,
    WrapperConstructorArg,
};
use heck::ToUpperCamelCase;
use std::path::PathBuf;

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::ffi::template_env::render(template_name, ctx)
}

fn render_inline(template_name: &str, ctx: minijinja::Value) -> String {
    render(template_name, ctx).trim_end_matches('\n').to_owned()
}

fn render_service_h_param_decl(c_type: String, param_name: &str) -> String {
    render_inline(
        "service_api_h_param_decl.h.jinja",
        minijinja::context! {
            c_type,
            param_name => param_name.to_owned(),
        },
    )
}

fn render_service_api_arg(value: &str) -> String {
    render_inline(
        "service_api_arg.rs.jinja",
        minijinja::context! {
            value => value.to_owned(),
        },
    )
}

fn trim_pending_service_h_decl_newline(out: &mut String) {
    if out.ends_with('\n') {
        out.pop();
    }
}

/// Generate the C FFI header that declares the callback typedef and service API.
///
/// This header is an input to cbindgen for human-readable API documentation,
/// but the actual exported Rust functions below (`extern "C"`) are the binding contract.
#[allow(dead_code)]
fn gen_service_h(api: &ApiSurface, crate_name: &str) -> String {
    let mut out = String::new();
    let header_guard = format!("{}_SERVICE_H", crate_name.to_uppercase().replace('-', "_"));

    out.push_str(&render(
        "service_api_h_header_start.h.jinja",
        minijinja::context! { header_guard },
    ));
    out.push_str(&render(
        "service_api_h_callback_typedef.h.jinja",
        minijinja::context! {},
    ));
    out.push('\n');

    for service in &api.services {
        let opaque_name = format!("{}Opaque", service.name);
        out.push_str(&render(
            "service_api_h_opaque_typedef.h.jinja",
            minijinja::context! { opaque_name },
        ));
    }
    out.push('\n');

    for service in &api.services {
        gen_service_h_decls(&mut out, service, api, crate_name);
    }

    out.push_str(&render(
        "service_api_h_header_end.h.jinja",
        minijinja::context! { header_guard },
    ));
    out
}

#[allow(dead_code)]
fn gen_service_h_decls(out: &mut String, service: &ServiceDef, api: &ApiSurface, prefix: &str) {
    let opaque_name = format!("{}Opaque", service.name);

    out.push_str(&render(
        "service_api_h_constructor_decl.h.jinja",
        minijinja::context! {
            service_name => service.name.clone(),
            symbol => c_consumer::service_new_symbol(prefix, &service.name),
            opaque_name => opaque_name.clone(),
        },
    ));

    out.push_str(&render(
        "service_api_h_destructor_decl.h.jinja",
        minijinja::context! {
            service_name => service.name.clone(),
            symbol => c_consumer::service_free_symbol(prefix, &service.name),
            opaque_name => opaque_name.clone(),
        },
    ));

    for reg in &service.registrations {
        out.push_str(&render_inline(
            "service_api_h_registration_decl_start.h.jinja",
            minijinja::context! {
                method_name => reg.method.clone(),
                symbol => c_consumer::service_register_symbol(prefix, &service.name, &reg.method),
                opaque_name => opaque_name.clone(),
            },
        ));

        if !reg.metadata_params.is_empty() {
            trim_pending_service_h_decl_newline(out);
        }
        for meta_param in &reg.metadata_params {
            let c_type = typeref_to_c_type(&meta_param.ty);
            out.push_str(&render_service_h_param_decl(c_type, &meta_param.name));
        }
        out.push_str("\n);\n\n");
    }

    for ep in &service.entrypoints {
        if matches!(ep.kind, EntrypointKind::Finalize) && !entrypoint_return_representable(ep, api) {
            continue;
        }
        let return_type = match &ep.return_type {
            TypeRef::Named(name) if api.types.iter().any(|typ| typ.name == *name) => "uint64_t".to_owned(),
            _ => typeref_to_c_type(&ep.return_type),
        };

        let kind = if ep.kind == EntrypointKind::Run {
            "Run"
        } else {
            "Finalize"
        };
        out.push_str(&render_inline(
            "service_api_h_entrypoint_decl_start.h.jinja",
            minijinja::context! {
                kind,
                return_type,
                symbol => c_consumer::service_entrypoint_symbol(prefix, &service.name, &ep.method),
                opaque_name => opaque_name.clone(),
            },
        ));

        if !ep.params.is_empty() {
            trim_pending_service_h_decl_newline(out);
        }
        for ep_param in &ep.params {
            let c_type = typeref_to_c_type(&ep_param.ty);
            out.push_str(&render_service_h_param_decl(c_type, &ep_param.name));
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
        TypeRef::Named(_) => "int32_t".to_owned(),
        _ => "void*".to_owned(),
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

/// A C-ABI binding for one non-callback parameter (registration metadata or entrypoint param).
struct FfiParamBinding {
    /// The Rust `extern "C"` parameter declaration (`name: type`).
    decl: String,
    /// A statement (possibly empty) that rebinds the raw value to a usable owned Rust value.
    conversion: String,
    /// The expression to pass at the call site.
    arg: String,
    /// Whether the raw parameter is a pointer that must be null-checked before use.
    pointer: bool,
}

fn param_decl_suffix(bindings: &[FfiParamBinding]) -> String {
    bindings
        .iter()
        .map(|binding| format!(",\n    {}", binding.decl))
        .collect()
}

fn pointer_null_checks<'a>(
    params: impl Iterator<Item = &'a crate::core::ir::ParamDef>,
    bindings: &[FfiParamBinding],
    null_return: &str,
    include_comment: bool,
) -> String {
    params
        .zip(bindings)
        .filter_map(|(param, binding)| {
            if !binding.pointer {
                return None;
            }
            let comment = if include_comment { " // Error: null pointer" } else { "" };
            Some(format!(
                "    if {}.is_null() {{\n        return {null_return}{comment};\n    }}\n",
                param.name
            ))
        })
        .collect()
}

fn conversion_body(bindings: &[FfiParamBinding], add_trailing_blank: bool) -> String {
    let mut body: String = bindings.iter().map(|binding| binding.conversion.as_str()).collect();
    if add_trailing_blank && bindings.iter().any(|binding| !binding.conversion.is_empty()) {
        body.push('\n');
    }
    body
}

/// Bind a non-callback parameter to its C-ABI form.
///
/// - `String` crosses as `*const c_char` and is rebound to an owned `String`.
/// - An enum crosses as `i32` discriminant and is reconstructed via `from_i32`.
/// - A `Named` type this surface wraps crosses as a `*mut {core}::{name}` opaque pointer and is
///   reconstructed (consumed) via `Box::from_raw`.
/// - Everything else crosses by value via [`typeref_to_rust_ffi_type`].
fn ffi_param_binding(
    p: &crate::core::ir::ParamDef,
    core_import: &str,
    api: &ApiSurface,
    failure_return: &str,
) -> FfiParamBinding {
    match &p.ty {
        TypeRef::String => FfiParamBinding {
            decl: format!("{}: *const c_char", p.name),
            conversion: format!(
                "    let {0} = if {0}.is_null() {{\n        \
                     String::new()\n    \
                 }} else {{\n        \
                     // SAFETY: caller guarantees a valid null-terminated C string.\n        \
                     unsafe {{ CStr::from_ptr({0}) }}.to_string_lossy().into_owned()\n    \
                 }};\n",
                p.name
            ),
            arg: p.name.clone(),
            pointer: true,
        },
        TypeRef::Named(n) if api.enums.iter().any(|e| e.name == *n) => {
            let enum_snake = heck::ToSnakeCase::to_snake_case(n.as_str());
            FfiParamBinding {
                decl: format!("{}: i32", p.name),
                conversion: format!(
                    "    let {0} = {1}::{0}_from_i32({0})\n        \
                     .ok_or_else(|| \"invalid discriminant for {2}\")?;\n",
                    enum_snake, core_import, n
                ),
                arg: enum_snake,
                pointer: false,
            }
        }
        TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => FfiParamBinding {
            decl: format!("{}: AlefHandle", p.name),
            conversion: format!(
                "    let {0} = match with_handle::<{1}::{2}, _>({0}, Clone::clone) {{\n        \
                 Ok(value) => value,\n        \
                 Err(error) => {{ set_handle_error(&error); return {3}; }}\n    \
                 }};\n",
                p.name, core_import, n, failure_return
            ),
            arg: p.name.clone(),
            pointer: false,
        },
        _ => FfiParamBinding {
            decl: format!("{}: {}", p.name, typeref_to_rust_ffi_type(&p.ty, core_import)),
            conversion: String::new(),
            arg: p.name.clone(),
            pointer: false,
        },
    }
}

/// Whether an entrypoint's return type can be represented over the C ABI as a function return.
///
/// Unit/primitive/string/bytes map to a status code or scalar; a `Named` type is representable only
/// when this surface wraps it (so it can cross as a `*mut {core}::{name}` opaque). Anything else
/// (e.g. a foreign framework type a `finalize` converts into) is not representable.
fn entrypoint_return_representable(ep: &crate::core::ir::EntrypointDef, api: &ApiSurface) -> bool {
    match &ep.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n),
        _ => false,
    }
}

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

    out.push_str(&render("service_api_rs_header.rs.jinja", minijinja::context! {}));

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

    for service in &api.services {
        gen_service_opaque(&mut out, service, &core_import, &prefix);
        gen_service_functions(&mut out, service, api, &core_import, &prefix);
    }

    out
}

/// Emit the opaque service type and its constructor/destructor.
fn gen_service_opaque(out: &mut String, service: &ServiceDef, _core_import: &str, prefix: &str) {
    let opaque_name = format!("{}Opaque", service.name);
    let owner_path = &service.rust_path;
    let new_fn_name = c_consumer::service_new_symbol(prefix, &service.name);
    let free_fn_name = c_consumer::service_free_symbol(prefix, &service.name);

    out.push_str(&render(
        "service_api_opaque.rs.jinja",
        minijinja::context! {
            service_name => service.name.clone(),
            new_fn_name,
            free_fn_name,
            opaque_name,
            owner_path => owner_path.clone(),
            constructor_name => service.constructor.name.clone(),
        },
    ));
}

/// Emit the handler bridge struct for one contract.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Ffi{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    out.push_str(&render(
        "service_api_handler_bridge_struct.rs.jinja",
        minijinja::context! {
            trait_name => trait_name.clone(),
            bridge_name => bridge_name.clone(),
        },
    ));

    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

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

    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

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

    out.push_str(&render(
        "service_api_handler_bridge_impl.rs.jinja",
        minijinja::context! {
            core_import => core_import.to_owned(),
            trait_name => trait_name.clone(),
            bridge_name,
            dispatch_name => dispatch_name.clone(),
            extra_param,
            wire_name,
            req_path,
            output_type,
            wire_output,
            box_err,
            resp_path,
            tail,
        },
    ));
}

/// Emit registration and entrypoint functions for one service.
fn gen_service_functions(out: &mut String, service: &ServiceDef, api: &ApiSurface, core_import: &str, prefix: &str) {
    let opaque_name = format!("{}Opaque", service.name);

    for reg in &service.registrations {
        gen_registration_function(out, service, reg, api, core_import, prefix, &opaque_name);
        gen_registration_variants(out, service, reg, api, core_import, prefix, &opaque_name);
    }

    for cfg in &service.configurators {
        gen_configurator_function(out, service, cfg, api, core_import, prefix, &opaque_name);
    }

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
    let fn_name = c_consumer::service_register_symbol(prefix, &service.name, &reg.method);

    let contract = find_contract(api, &reg.callback_contract).expect("contract not found");
    let bridge_name = format!("Ffi{}Bridge", contract.trait_name.to_upper_camel_case());

    let meta_bindings: Vec<FfiParamBinding> = reg
        .metadata_params
        .iter()
        .map(|p| ffi_param_binding(p, core_import, api, "1"))
        .collect();

    let meta_args: String = meta_bindings.iter().map(|b| format!("{}, ", b.arg)).collect();
    let dispatch_body = if reg.error_type.is_some() {
        render(
            "service_api_registration_dispatch_result.rs.jinja",
            minijinja::context! {
                method_name => reg.method.clone(),
                meta_args => meta_args.clone(),
                opaque_name,
            },
        )
    } else {
        render(
            "service_api_registration_dispatch_void.rs.jinja",
            minijinja::context! {
                method_name => reg.method.clone(),
                meta_args => meta_args.clone(),
                opaque_name,
            },
        )
    };

    let pre_bridge_body = format!(
        "{}\n{}",
        pointer_null_checks(reg.metadata_params.iter(), &meta_bindings, "1", true),
        conversion_body(&meta_bindings, true)
    );
    out.push_str(&render(
        "service_api_registration_function.rs.jinja",
        minijinja::context! {
            method_name => reg.method.clone(),
            new_fn_name => c_consumer::service_new_symbol(prefix, &service.name),
            fn_name,
            opaque_name => opaque_name.to_owned(),
            param_decls => param_decl_suffix(&meta_bindings),
            pre_bridge_body,
            bridge_name,
            handler_trait_path => format!("{}::{}", core_import, contract.trait_name),
            dispatch_body,
        },
    ));
}

/// Emit one `#[no_mangle] pub extern "C" fn` per [`RegistrationVariant`] on `reg`.
///
/// Each variant symbol:
/// - Takes the variant's `signature_params` (free constructor args, as C-ABI decls) plus the
///   fixed `owner`/`callback`/`context` triple from the base registration.
/// - Builds the metadata wrapper inline via `wrapper_type_path::constructor_method(args)`,
///   substituting `Fixed.value_expr` verbatim and marshaling `Free` params via
///   [`ffi_param_binding`].
/// - Forwards to the same registration logic as the base `register_*` function.
///
/// Variants without a `wrapper_call` are skipped — they represent direct metadata-param
/// overrides that only make sense for non-FFI backends.
fn gen_registration_variants(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    core_import: &str,
    prefix: &str,
    opaque_name: &str,
) {
    if reg.variants.is_empty() {
        return;
    }

    let base_fn_name = c_consumer::service_register_symbol(prefix, &service.name, &reg.method);
    let new_fn_name = c_consumer::service_new_symbol(prefix, &service.name);

    let contract = find_contract(api, &reg.callback_contract).expect("contract not found");
    let bridge_name = format!("Ffi{}Bridge", contract.trait_name.to_upper_camel_case());

    for variant in &reg.variants {
        let wrapper_call = match &variant.wrapper_call {
            Some(wc) => wc,
            None => continue,
        };

        gen_registration_variant(
            out,
            variant,
            wrapper_call,
            service,
            reg,
            api,
            core_import,
            prefix,
            opaque_name,
            &base_fn_name,
            &new_fn_name,
            &bridge_name,
            contract,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn gen_registration_variant(
    out: &mut String,
    variant: &RegistrationVariant,
    wrapper_call: &crate::core::ir::WrapperConstructorCall,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    core_import: &str,
    prefix: &str,
    opaque_name: &str,
    base_fn_name: &str,
    new_fn_name: &str,
    bridge_name: &str,
    contract: &HandlerContractDef,
) {
    let variant_fn_name = c_consumer::service_method_symbol(prefix, &service.name, &variant.name);

    let sig_bindings: Vec<FfiParamBinding> = variant
        .signature_params
        .iter()
        .map(|p| ffi_param_binding(p, core_import, api, "1"))
        .collect();

    let default_doc = format!("Variant shortcut `{}` over `{}`.", variant.name, base_fn_name);
    let doc = variant.doc.as_deref().unwrap_or(&default_doc);

    let mut ctor_args = String::new();
    for arg in &wrapper_call.args {
        match arg {
            WrapperConstructorArg::Fixed { value_expr, .. } => {
                ctor_args.push_str(&render(
                    "service_api_wrapper_ctor_arg.rs.jinja",
                    minijinja::context! { value => value_expr.clone() },
                ));
            }
            WrapperConstructorArg::Free { param } => {
                let binding = sig_bindings
                    .iter()
                    .find(|b| b.decl.starts_with(&format!("{}: ", param.name)) || b.arg == param.name)
                    .map(|b| b.arg.as_str())
                    .unwrap_or(param.name.as_str());
                ctor_args.push_str(&render(
                    "service_api_wrapper_ctor_arg.rs.jinja",
                    minijinja::context! { value => binding.to_owned() },
                ));
            }
        }
    }

    let meta_args: String = {
        let mut args = render_service_api_arg(&wrapper_call.metadata_param);
        for meta_param in &reg.metadata_params {
            if meta_param.name == wrapper_call.metadata_param {
                continue;
            }
            let is_overridden = variant.overrides.iter().any(|o| o.param_name == meta_param.name);
            if is_overridden {
                let override_expr = variant
                    .overrides
                    .iter()
                    .find(|o| o.param_name == meta_param.name)
                    .map(|o| o.value_expr.as_str())
                    .unwrap_or("");
                args.push_str(&render_service_api_arg(override_expr));
            } else {
                let binding_arg = sig_bindings
                    .iter()
                    .find(|b| b.arg == meta_param.name)
                    .map(|b| b.arg.as_str())
                    .unwrap_or(meta_param.name.as_str());
                args.push_str(&render_service_api_arg(binding_arg));
            }
        }
        args
    };

    let dispatch_body = if reg.error_type.is_some() {
        render(
            "service_api_registration_dispatch_result.rs.jinja",
            minijinja::context! {
                method_name => reg.method.clone(),
                meta_args => meta_args.clone(),
                opaque_name,
            },
        )
    } else {
        render(
            "service_api_registration_dispatch_void.rs.jinja",
            minijinja::context! {
                method_name => reg.method.clone(),
                meta_args => meta_args.clone(),
                opaque_name,
            },
        )
    };

    let pre_wrapper_body = format!(
        "{}\n{}",
        pointer_null_checks(variant.signature_params.iter(), &sig_bindings, "1", true),
        conversion_body(&sig_bindings, true)
    );
    out.push_str(&render(
        "service_api_registration_variant.rs.jinja",
        minijinja::context! {
            doc => doc.to_owned(),
            new_fn_name => new_fn_name.to_owned(),
            variant_fn_name,
            opaque_name => opaque_name.to_owned(),
            param_decls => param_decl_suffix(&sig_bindings),
            pre_wrapper_body,
            metadata_param => wrapper_call.metadata_param.clone(),
            wrapper_type_path => wrapper_call.wrapper_type_path.clone(),
            constructor_method => wrapper_call.constructor_method.clone(),
            ctor_args,
            bridge_name => bridge_name.to_owned(),
            handler_trait_path => format!("{}::{}", core_import, contract.trait_name),
            dispatch_body,
        },
    ));
}

fn gen_configurator_function(
    out: &mut String,
    service: &ServiceDef,
    cfg: &crate::core::ir::MethodDef,
    api: &ApiSurface,
    core_import: &str,
    prefix: &str,
    opaque_name: &str,
) {
    let fn_name = c_consumer::service_method_symbol(prefix, &service.name, &cfg.name);

    let param_bindings: Vec<FfiParamBinding> = cfg
        .params
        .iter()
        .map(|p| ffi_param_binding(p, core_import, api, "0"))
        .collect();

    let call_args: String = param_bindings
        .iter()
        .map(|b| b.arg.clone())
        .collect::<Vec<_>>()
        .join(", ");

    let pre_call_body = format!(
        "{}\n{}",
        pointer_null_checks(cfg.params.iter(), &param_bindings, "std::ptr::null_mut()", false,),
        conversion_body(&param_bindings, true)
    );
    out.push_str(&render(
        "service_api_configurator_function.rs.jinja",
        minijinja::context! {
            method_name => cfg.name.clone(),
            new_fn_name => c_consumer::service_new_symbol(prefix, &service.name),
            fn_name,
            opaque_name => opaque_name.to_owned(),
            param_decls => param_decl_suffix(&param_bindings),
            pre_call_body,
            call_args,
        },
    ));
}

fn gen_entrypoint_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    api: &ApiSurface,
    core_import: &str,
    prefix: &str,
    opaque_name: &str,
) {
    if matches!(ep.kind, EntrypointKind::Finalize) && !entrypoint_return_representable(ep, api) {
        return;
    }

    let fn_name = c_consumer::service_entrypoint_symbol(prefix, &service.name, &ep.method);

    let returns_opaque = matches!(&ep.return_type, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n));
    let return_type = match &ep.return_type {
        TypeRef::Named(_) if returns_opaque => "AlefHandle".to_owned(),
        _ => "i32".to_owned(),
    };
    let null_return = if returns_opaque { "0" } else { "1" };

    let param_bindings: Vec<FfiParamBinding> = ep
        .params
        .iter()
        .map(|p| ffi_param_binding(p, core_import, api, null_return))
        .collect();

    let call_args: String = param_bindings
        .iter()
        .map(|b| b.arg.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let runtime_block = if ep.is_async {
        "    // 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep\n    \
         // extraction future (a nested archive member, a multi-stage OCR pipeline), and a\n    \
         // stack overflow aborts the process with SIGBUS instead of raising a catchable panic.\n    \
         const ENTRYPOINT_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;\n    \
         let rt = tokio::runtime::Builder::new_multi_thread()\n        \
             .enable_all()\n        \
             .thread_stack_size(ENTRYPOINT_RUNTIME_STACK_SIZE_BYTES)\n        \
             .build()\n        \
             .expect(\"failed to create tokio runtime\");\n"
    } else {
        ""
    };
    let call = if ep.is_async {
        format!("rt.block_on(inner.{}({call_args}))", ep.method)
    } else {
        format!("inner.{}({call_args})", ep.method)
    };

    let return_body = if returns_opaque {
        if ep.error_type.is_some() {
            render(
                "service_api_entrypoint_return_opaque_result.rs.jinja",
                minijinja::context! { call => call.clone() },
            )
        } else {
            render(
                "service_api_entrypoint_return_opaque_value.rs.jinja",
                minijinja::context! { call => call.clone() },
            )
        }
    } else if ep.error_type.is_some() {
        render(
            "service_api_entrypoint_return_result_status.rs.jinja",
            minijinja::context! { call => call.clone() },
        )
    } else {
        render(
            "service_api_entrypoint_return_void_status.rs.jinja",
            minijinja::context! { call => call.clone() },
        )
    };

    let pre_call_body = format!(
        "{}\n{}",
        pointer_null_checks(ep.params.iter(), &param_bindings, null_return, false),
        conversion_body(&param_bindings, false)
    );
    out.push_str(&render(
        "service_api_entrypoint_function.rs.jinja",
        minijinja::context! {
            method_name => ep.method.clone(),
            new_fn_name => c_consumer::service_new_symbol(prefix, &service.name),
            fn_name,
            opaque_name => opaque_name.to_owned(),
            param_decls => param_decl_suffix(&param_bindings),
            return_type,
            null_return,
            pre_call_body,
            runtime_block,
            return_body,
        },
    ));
}

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

    let service_rs = gen_service_rs(api, config);

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("service.rs"),
        content: service_rs,
        generated_header: true,
    }])
}

#[cfg(test)]
mod tests;
