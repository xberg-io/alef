use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::mut_writeback;
use crate::codegen::naming::to_java_name;
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

use super::super::helpers::{emit_javadoc_with_throws, is_bridge_param_java, render_nullable_type};
use super::super::marshal::{
    ffi_param_args, is_bytes_result, is_ffi_string_return, java_ffi_return_cast, java_ffi_return_expr,
    marshal_param_to_ffi, opaque_lease_resource,
};
use super::params_returns::public_arg_names;
use super::visitor_bridge::VisitorFunctionBridge;

mod returns;

struct SyncInvocation<'a> {
    func: &'a FunctionDef,
    prefix: &'a str,
    class_name: &'a str,
    opaque_types: &'a AHashSet<String>,
    ffi_handle: String,
    call_args: Vec<String>,
    is_optional_return: bool,
    dispatch_return_type: TypeRef,
    is_clear_fn: bool,
}

struct SyncInvocationParams<'a> {
    func: &'a FunctionDef,
    prefix: &'a str,
    class_name: &'a str,
    opaque_types: &'a AHashSet<String>,
    bridge_param_names: &'a HashSet<String>,
    bridge_type_aliases: &'a HashSet<String>,
    clear_fn_handles: &'a AHashMap<String, String>,
}

impl<'a> SyncInvocation<'a> {
    fn new(params: SyncInvocationParams<'a>) -> Self {
        let SyncInvocationParams {
            func,
            prefix,
            class_name,
            opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            clear_fn_handles,
        } = params;
        let (is_optional_return, dispatch_return_type) = match &func.return_type {
            TypeRef::Optional(inner) => (true, (**inner).clone()),
            other => (false, other.clone()),
        };
        Self {
            func,
            prefix,
            class_name,
            opaque_types,
            ffi_handle: ffi_handle(func, prefix, clear_fn_handles),
            call_args: ffi_call_args(func, opaque_types, bridge_param_names, bridge_type_aliases),
            is_optional_return,
            dispatch_return_type,
            is_clear_fn: clear_fn_handles.contains_key(&func.name),
        }
    }
}

fn effective_param_type(param: &ParamDef) -> TypeRef {
    if param.optional && !matches!(param.ty, TypeRef::Optional(_)) {
        TypeRef::Optional(Box::new(param.ty.clone()))
    } else {
        param.ty.clone()
    }
}

fn public_params(
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> Vec<String> {
    func.params
        .iter()
        .filter(|param| !is_bridge_param_java(param, bridge_param_names, bridge_type_aliases))
        .map(|param| {
            let param_type = if param.optional {
                java_boxed_type(&param.ty)
            } else {
                java_type(&param.ty)
            };
            let annotated = render_nullable_type(&param_type, param.optional);
            format!("final {annotated} {}", to_java_name(&param.name))
        })
        .collect()
}

fn emit_method_header(out: &mut String, func: &FunctionDef, class_name: &str, return_type: &str, params: &[String]) {
    let exception_class_name = format!("{}Exception", class_name);
    emit_javadoc_with_throws(out, &func.doc, "    ", &exception_class_name);
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_method_signature.jinja",
        crate::alef_context! {
            return_type,
            method_name => to_java_name(&func.name),
            params => params.join(", "),
            exception_class => exception_class_name,
        },
    ));
}

fn emit_visitor_dispatch(
    out: &mut String,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    visitor_bridge: Option<&VisitorFunctionBridge>,
) {
    let Some(visitor_bridge) = visitor_bridge else {
        return;
    };
    out.push_str("        if (");
    out.push_str(&visitor_bridge.options_param_java);
    out.push_str(" != null && ");
    out.push_str(&visitor_bridge.options_param_java);
    out.push('.');
    out.push_str(&visitor_bridge.options_field_java);
    out.push_str("() != null) {\n");
    out.push_str("            return ");
    out.push_str(&visitor_bridge.internal_method_name);
    out.push('(');
    out.push_str(&public_arg_names(func, bridge_param_names, bridge_type_aliases).join(", "));
    out.push_str(");\n        }\n\n");
}

fn emit_try_and_marshalling(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    let lease_resources = func
        .params
        .iter()
        .filter(|param| !is_bridge_param_java(param, bridge_param_names, bridge_type_aliases))
        .filter_map(|param| {
            opaque_lease_resource(&to_java_name(&param.name), &effective_param_type(param), opaque_types)
        })
        .collect::<Vec<_>>()
        .join(";\n");
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_try_finally_block_start.jinja",
        crate::alef_context! { lease_resources },
    ));
    emit_param_marshalling(out, func, prefix, opaque_types, bridge_param_names, bridge_type_aliases);
}

fn emit_param_marshalling(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        marshal_param_to_ffi(
            out,
            &to_java_name(&param.name),
            &effective_param_type(param),
            opaque_types,
            prefix,
            "nativeResources",
        );
    }
}

fn ffi_call_args(
    func: &FunctionDef,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> Vec<String> {
    func.params
        .iter()
        .flat_map(|param| {
            if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
                vec!["MemorySegment.NULL".to_string()]
            } else {
                ffi_param_args(&to_java_name(&param.name), &effective_param_type(param), opaque_types)
            }
        })
        .collect()
}

fn ffi_handle(func: &FunctionDef, prefix: &str, clear_fn_handles: &AHashMap<String, String>) -> String {
    clear_fn_handles.get(&func.name).map_or_else(
        || format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase()),
        |handle| format!("NativeLib.{}", handle),
    )
}

pub(super) struct SyncFunctionMethodParams<'a> {
    pub(super) func: &'a FunctionDef,
    pub(super) prefix: &'a str,
    pub(super) class_name: &'a str,
    pub(super) opaque_types: &'a AHashSet<String>,
    pub(super) bridge_param_names: &'a HashSet<String>,
    pub(super) bridge_type_aliases: &'a HashSet<String>,
    pub(super) has_visitor_bridge: bool,
    pub(super) clear_fn_handles: &'a AHashMap<String, String>,
    pub(super) capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
}

#[allow(dead_code)]
pub(super) fn gen_sync_function_method(out: &mut String, params: SyncFunctionMethodParams<'_>) {
    let SyncFunctionMethodParams {
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        has_visitor_bridge,
        clear_fn_handles,
        capsule_types,
    } = params;
    gen_sync_function_method_with_visitor(
        out,
        SyncFunctionMethodWithVisitorParams {
            func,
            prefix,
            class_name,
            opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            has_visitor_bridge,
            clear_fn_handles,
            visitor_bridge: None,
            capsule_types,
        },
    );
}

pub(super) struct SyncFunctionMethodWithVisitorParams<'a> {
    pub(super) func: &'a FunctionDef,
    pub(super) prefix: &'a str,
    pub(super) class_name: &'a str,
    pub(super) opaque_types: &'a AHashSet<String>,
    pub(super) bridge_param_names: &'a HashSet<String>,
    pub(super) bridge_type_aliases: &'a HashSet<String>,
    pub(super) has_visitor_bridge: bool,
    pub(super) clear_fn_handles: &'a AHashMap<String, String>,
    pub(super) visitor_bridge: Option<&'a VisitorFunctionBridge>,
    pub(super) capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
}

pub(super) fn gen_sync_function_method_with_visitor(out: &mut String, params: SyncFunctionMethodWithVisitorParams<'_>) {
    let SyncFunctionMethodWithVisitorParams {
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        has_visitor_bridge,
        clear_fn_handles,
        visitor_bridge,
        capsule_types,
    } = params;

    if let Some(capsule_config) = capsule_return_config(func, capsule_types) {
        return gen_capsule_function_method(
            out,
            CapsuleFunctionMethodParams {
                func,
                prefix,
                class_name,
                opaque_types,
                bridge_param_names,
                bridge_type_aliases,
                config: capsule_config,
            },
        );
    }

    emit_regular_sync_method(
        out,
        RegularSyncMethodParams {
            func,
            prefix,
            class_name,
            opaque_types,
            bridge_param_names,
            bridge_type_aliases,
            has_visitor_bridge,
            clear_fn_handles,
            visitor_bridge,
        },
    );
}

struct RegularSyncMethodParams<'a> {
    func: &'a FunctionDef,
    prefix: &'a str,
    class_name: &'a str,
    opaque_types: &'a AHashSet<String>,
    bridge_param_names: &'a HashSet<String>,
    bridge_type_aliases: &'a HashSet<String>,
    has_visitor_bridge: bool,
    clear_fn_handles: &'a AHashMap<String, String>,
    visitor_bridge: Option<&'a VisitorFunctionBridge>,
}

fn emit_regular_sync_method(out: &mut String, params: RegularSyncMethodParams<'_>) {
    let RegularSyncMethodParams {
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        has_visitor_bridge,
        clear_fn_handles,
        visitor_bridge,
    } = params;
    // A `&mut T` DTO parameter on a unit-returning function cannot be bound as an owned
    // by-value parameter returning void: see `emit_writeback_return`'s doc comment (issue
    // #380). When this narrow shape applies, the method returns the updated `T` instead.
    // `reject_unsupported_writeback` (called by `generate_bindings` before any file is
    // emitted) has already ruled out every `&mut` DTO shape this can't express. ~keep
    let writeback = mut_writeback::writeback_param(&func.params, &func.return_type, opaque_types);
    let return_type_str = match writeback {
        Some(wb) => java_return_type(&wb.ty).into_owned(),
        None => java_return_type(&func.return_type).into_owned(),
    };
    emit_method_header(
        out,
        func,
        class_name,
        &return_type_str,
        &public_params(func, bridge_param_names, bridge_type_aliases),
    );
    if has_visitor_bridge {
        emit_visitor_dispatch(out, func, bridge_param_names, bridge_type_aliases, visitor_bridge);
    }
    emit_try_and_marshalling(out, func, prefix, opaque_types, bridge_param_names, bridge_type_aliases);
    let invocation = SyncInvocation::new(SyncInvocationParams {
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        clear_fn_handles,
    });
    if let Some(wb) = writeback {
        let handle_var = format!("c{}", to_java_name(&wb.name));
        let return_type_name = mut_writeback::writeback_type_name(wb).unwrap_or_default();
        returns::emit_writeback_return(out, &invocation, &handle_var, return_type_name);
    } else {
        returns::emit_sync_return(out, &invocation);
    }
    out.push_str("    }\n");
}

/// Returns the capsule config for a function's return type if it is a capsule type,
/// otherwise returns None.
fn capsule_return_config<'a>(
    func: &FunctionDef,
    capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
) -> Option<&'a HostCapsuleTypeConfig> {
    if let TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

pub(super) struct CapsuleFunctionMethodParams<'a> {
    pub(super) func: &'a FunctionDef,
    pub(super) prefix: &'a str,
    pub(super) class_name: &'a str,
    pub(super) opaque_types: &'a AHashSet<String>,
    pub(super) bridge_param_names: &'a HashSet<String>,
    pub(super) bridge_type_aliases: &'a HashSet<String>,
    pub(super) config: &'a HostCapsuleTypeConfig,
}

/// Generate a Java wrapper for a function returning a host-native capsule (Language) type.
///
/// The exported C symbol returns the host runtime's raw grammar pointer.
/// The wrapper converts parameters, calls the C function, and constructs the host `Language`
/// from the raw pointer — never an opaque alef handle.
pub(super) fn gen_capsule_function_method(out: &mut String, params: CapsuleFunctionMethodParams<'_>) {
    let CapsuleFunctionMethodParams {
        func,
        prefix,
        class_name,
        opaque_types,
        bridge_param_names,
        bridge_type_aliases,
        config,
    } = params;
    let Some(return_type) = required_capsule_return_type(out, config) else {
        return;
    };
    emit_method_header(
        out,
        func,
        class_name,
        &return_type,
        &public_params(func, bridge_param_names, bridge_type_aliases),
    );
    emit_try_and_marshalling(out, func, prefix, opaque_types, bridge_param_names, bridge_type_aliases);
    let call_args = ffi_call_args(func, opaque_types, bridge_param_names, bridge_type_aliases);
    emit_capsule_result_call(out, func, prefix, &call_args);
    emit_capsule_construct(out, config, class_name);
}

fn required_capsule_return_type(out: &mut String, config: &HostCapsuleTypeConfig) -> Option<String> {
    match config.required_host_type("Language", "java") {
        Ok(return_type) => Some(return_type.to_string()),
        Err(error) => {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_alef_error_comment.jinja",
                crate::alef_context! {
                    indent => "    ",
                    error => error.to_string(),
                },
            ));
            None
        }
    }
}

fn emit_capsule_result_call(out: &mut String, func: &FunctionDef, prefix: &str, call_args: &[String]) {
    let ffi_handle = format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase());
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_result_ptr_call.jinja",
        crate::alef_context! {
            ffi_handle,
            args => call_args.join(", "),
        },
    ));
    returns::emit_null_check(out, false);
}

fn emit_capsule_construct(out: &mut String, config: &HostCapsuleTypeConfig, class_name: &str) {
    match config.construct_required("resultPtr", "Language", "java") {
        Ok(construct) => out.push_str(&crate::backends::java::template_env::render(
            "ffi_return_expr.jinja",
            crate::alef_context! { expr => construct },
        )),
        Err(error) => {
            out.push_str(&crate::backends::java::template_env::render(
                "ffi_alef_error_comment.jinja",
                crate::alef_context! {
                    indent => "            ",
                    error => error.to_string(),
                },
            ));
            emit_catch_and_close(out, class_name);
            return;
        }
    }
    emit_catch_and_close(out, class_name);
}

fn emit_catch_and_close(out: &mut String, class_name: &str) {
    super::error_catch::emit_method_catch_chain(out, &format!("{}Exception", class_name));
    out.push_str("    }\n");
}

#[cfg(test)]
mod capsule_tests {
    use super::*;
    use crate::core::ir::ParamDef;

    fn get_language_fn() -> FunctionDef {
        FunctionDef {
            name: "get_language".to_string(),
            rust_path: "sample::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_cfg(host_type: &str, construct_expr: &str) -> HostCapsuleTypeConfig {
        HostCapsuleTypeConfig {
            host_type: host_type.to_string(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: construct_expr.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn capsule_method_emits_configured_host_type_and_construct_expr() {
        let func = get_language_fn();
        let cfg = make_cfg(
            "io.github.example.jtreesitter.Language",
            "new io.github.example.jtreesitter.Language({ptr})",
        );
        let mut out = String::new();
        gen_capsule_function_method(
            &mut out,
            CapsuleFunctionMethodParams {
                func: &func,
                prefix: "tsp",
                class_name: "LanguagePack",
                opaque_types: &AHashSet::new(),
                bridge_param_names: &HashSet::new(),
                bridge_type_aliases: &HashSet::new(),
                config: &cfg,
            },
        );
        assert!(
            out.contains("io.github.example.jtreesitter.Language"),
            "must use configured host_type. Got:\n{out}"
        );
        assert!(
            out.contains("new io.github.example.jtreesitter.Language(resultPtr)"),
            "must use configured construct_expr with ptr substituted. Got:\n{out}"
        );
    }

    #[test]
    fn capsule_method_errors_when_host_type_empty() {
        let func = get_language_fn();
        let cfg = make_cfg("", "new MyLanguage({ptr})");
        let mut out = String::new();
        gen_capsule_function_method(
            &mut out,
            CapsuleFunctionMethodParams {
                func: &func,
                prefix: "tsp",
                class_name: "LanguagePack",
                opaque_types: &AHashSet::new(),
                bridge_param_names: &HashSet::new(),
                bridge_type_aliases: &HashSet::new(),
                config: &cfg,
            },
        );
        assert!(
            out.contains("ALEF ERROR"),
            "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
        );
        assert!(
            out.contains("host_type"),
            "error must mention the missing field. Got:\n{out}"
        );
    }

    #[test]
    fn capsule_method_errors_when_construct_expr_empty() {
        let func = get_language_fn();
        let cfg = make_cfg("io.github.example.jtreesitter.Language", "");
        let mut out = String::new();
        gen_capsule_function_method(
            &mut out,
            CapsuleFunctionMethodParams {
                func: &func,
                prefix: "tsp",
                class_name: "LanguagePack",
                opaque_types: &AHashSet::new(),
                bridge_param_names: &HashSet::new(),
                bridge_type_aliases: &HashSet::new(),
                config: &cfg,
            },
        );
        assert!(
            out.contains("ALEF ERROR"),
            "empty construct_expr must produce an ALEF ERROR comment. Got:\n{out}"
        );
        assert!(
            out.contains("construct_expr"),
            "error must mention the missing field. Got:\n{out}"
        );
    }
}

#[cfg(test)]
mod tests;
