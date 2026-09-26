//! Options-field bridge code generation for the C FFI backend.
//!
//! When a `[[trait_bridges]]` entry is configured with `bind_via = "options_field"`,
//! the visitor handle lives as a field on the options struct rather than as a positional
//! argument. This module generates:
//!
//! 1. `{prefix}_options_set_{field}` — a setter that resolves managed options and visitor
//!    handles, then stores a delegating visitor in `options.{field}`. Callers invoke this
//!    before calling the generated FFI wrapper for the configured function.
//! 2. `{prefix}_{function}` — a function wrapper that passes options (with the embedded
//!    visitor) directly to the core call. Replaces sanitized stubs that the normal
//!    free-function path would emit for functions whose signature the IR sanitizer marks
//!    unimplementable due to the trait-object field.
//!
//! The `{prefix}_convert_with_visitor` export from the legacy `visitor_callbacks` path is
//! NOT emitted in this mode; the IR/config-derived wrapper is the public FFI entrypoint.

use crate::codegen::naming::{pascal_to_snake, to_class_name};
use crate::core::ir::{FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::HashMap;

use crate::codegen::generators::trait_bridge::format_param_type;
use crate::core::ir::{MethodDef, ReceiverKind};

/// The options field being bridged, and how its handle is obtained.
///
/// Grouped only to keep [`gen_options_set_bridge`]'s parameter list short -- each field is
/// independent and describes one configured `options_field` binding. ~keep
pub struct OptionsFieldBridgeTarget<'a> {
    /// The field on the options struct.
    pub field_name: &'a str,
    /// The IR type name of the options struct.
    pub options_type_name: &'a str,
    /// When true, accept the `{prefix}Visitor` handle produced by the visitor-callbacks path
    /// (gen_visitor) instead of the `{prefix}{trait}Bridge` produced by the trait-bridge path.
    /// Required so that strict-typed C consumers (Zig) can pass the handle returned by
    /// `{prefix}_visitor_create` directly. Both types implement the same trait so the wrapper
    /// body is identical.
    pub use_callbacks_visitor: bool,
}

/// Generate the `{prefix}_options_set_{field}` setter.
///
/// The setter resolves the vtable bridge handle, wraps it in a thin delegating visitor,
/// and stores it in the options struct's visitor field. Callers must invoke this before
/// passing options to the generated options-field bridge wrapper.
///
/// # Parameters
///
/// - `prefix`: the FFI symbol prefix.
/// - `core_import`: the Rust crate name for the core library.
/// - `trait_def`: the IR definition of the trait whose bridge is being attached.
/// - `field_name`: the field on the options struct.
/// - `options_type_name`: the IR type name of the options struct.
/// - `type_paths`: map of IR type name → fully-qualified Rust path for signature generation.
/// - `target`: the specific options field being bridged and how its handle is obtained. See
///   [`OptionsFieldBridgeTarget`].
pub fn gen_options_set_bridge(
    prefix: &str,
    core_import: &str,
    trait_def: &TypeDef,
    type_paths: &HashMap<String, String>,
    target: OptionsFieldBridgeTarget<'_>,
) -> String {
    let OptionsFieldBridgeTarget {
        field_name,
        options_type_name,
        use_callbacks_visitor,
    } = target;
    let pascal_prefix = to_class_name(prefix);
    let trait_name = &trait_def.name;
    let handle_type = if use_callbacks_visitor {
        format!("{pascal_prefix}Visitor")
    } else {
        format!("{pascal_prefix}{trait_name}Bridge")
    };
    let options_type_snake = ffi_symbol_component(options_type_name);
    let handle_constructor = if use_callbacks_visitor {
        format!("{prefix}_visitor_create")
    } else {
        format!("{prefix}_{}_new", ffi_symbol_component(&handle_type))
    };
    let fn_name = format!("{prefix}_options_set_{field_name}");
    let trait_path = trait_def.rust_path.replace('-', "_");

    let delegation_methods = gen_vtable_ref_delegation(trait_def, core_import, type_paths, &handle_type);

    crate::backends::ffi::template_env::render(
        "options_field_bridge_setter.rs.jinja",
        minijinja::context! {
            prefix,
            handle_type,
            handle_constructor,
            function_name => fn_name,
            core_import,
            options_type_name,
            options_type_snake,
            field_name,
            trait_path,
            delegation_methods,
        },
    )
}

/// Generate a function wrapper for the `options_field` bridge mode.
///
/// In this mode the bridge is embedded in the configured options field via the setter.
/// The generated wrapper passes options, including the embedded bridge, directly to the
/// core function. This derives names and types from IR/config instead of hardcoded
/// downstream-shaped conversion names.
pub fn gen_function_with_options_field_bridge(
    prefix: &str,
    core_import: &str,
    func: &FunctionDef,
    options_param: &ParamDef,
    options_type_name: &str,
) -> Option<String> {
    let ffi_function_name = format!("{prefix}_{}", ffi_symbol_component(&func.name));
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let return_type_name = named_type_name(&func.return_type)?;
    let options_param_name = &options_param.name;
    let non_options_params: Vec<&ParamDef> = func.params.iter().filter(|p| p.name != options_param.name).collect();
    if !non_options_params
        .iter()
        .all(|param| matches!(param.ty, TypeRef::String | TypeRef::Char))
    {
        return None;
    }
    let params = render_bridge_params(&non_options_params, options_param);
    let null_checks = render_bridge_null_checks(&non_options_params);
    let conversions = render_bridge_param_conversions(&non_options_params);
    let call_args = render_bridge_call_args(&func.params, options_param_name);
    let return_type_snake = ffi_symbol_component(return_type_name);

    Some(crate::backends::ffi::template_env::render(
        "options_field_bridge_function.rs.jinja",
        minijinja::context! {
            prefix,
            function_name => func.name.clone(),
            ffi_function_name,
            core_function_path => core_fn_path,
            core_import,
            params,
            null_checks,
            conversions,
            call_args,
            options_param_name,
            options_type_name,
            return_type_name,
            return_type_snake,
        },
    ))
}

/// Generate `impl {trait_path} for VtableRef` method bodies.
///
/// Each method resolves the scalar handle and delegates through the trait implementation
/// already generated on the bridge struct by `gen_trait_bridge`.
fn gen_vtable_ref_delegation(
    trait_def: &TypeDef,
    core_import: &str,
    type_paths: &HashMap<String, String>,
    handle_type: &str,
) -> String {
    let mut out = String::with_capacity(4096);
    for method in trait_def.methods.iter().filter(|method| method.trait_source.is_none()) {
        out.push_str(&render_vtable_ref_method(method, core_import, type_paths, handle_type));
    }
    out
}

fn render_vtable_ref_method(
    method: &MethodDef,
    core_import: &str,
    type_paths: &HashMap<String, String>,
    handle_type: &str,
) -> String {
    let all_params = format_delegation_params(method, type_paths);
    let error_override = method
        .error_type
        .as_ref()
        .map(|_| "Box<dyn std::error::Error + Send + Sync>".to_string());
    let return_type = crate::codegen::generators::trait_bridge::format_return_type(
        &method.return_type,
        error_override.as_deref(),
        type_paths,
        method.returns_ref,
    );
    let arg_list = build_arg_list(method, core_import, type_paths);
    crate::backends::ffi::template_env::render(
        "vtable_ref_delegation_method.jinja",
        minijinja::context! {
            method_name => &method.name,
            all_params,
            ret => return_type,
            arg_list,
            handle_type,
        },
    )
}

fn format_delegation_params(method: &MethodDef, type_paths: &HashMap<String, String>) -> String {
    let receiver = match &method.receiver {
        Some(ReceiverKind::Ref) => "&self",
        Some(ReceiverKind::RefMut) => "&mut self",
        Some(ReceiverKind::Owned) => "self",
        None => "",
    };
    let params = method
        .params
        .iter()
        .map(|param| format!("{}: {}", param.name, format_param_type(param, type_paths)))
        .collect::<Vec<_>>()
        .join(", ");
    match (receiver.is_empty(), params.is_empty()) {
        (true, _) => params,
        (_, true) => receiver.to_string(),
        _ => format!("{receiver}, {params}"),
    }
}

/// Build the argument expression list for a method call.
fn build_arg_list(method: &MethodDef, _core_import: &str, _type_paths: &HashMap<String, String>) -> String {
    method
        .params
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Convert a PascalCase identifier to snake_case.
pub(crate) fn ffi_symbol_component(s: &str) -> String {
    pascal_to_snake(s)
}

fn named_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => named_type_name(inner),
        _ => None,
    }
}

fn render_bridge_params(non_options_params: &[&ParamDef], options_param: &ParamDef) -> String {
    let mut params: Vec<String> = non_options_params
        .iter()
        .filter_map(|param| match param.ty {
            TypeRef::String | TypeRef::Char => Some(format!("    {}: *const std::ffi::c_char", param.name)),
            _ => None,
        })
        .collect();
    params.push(format!("    {}: AlefHandle", options_param.name));
    params.join(",\n")
}

fn render_bridge_null_checks(non_options_params: &[&ParamDef]) -> String {
    let mut out = String::new();
    for param in non_options_params {
        if matches!(param.ty, TypeRef::String | TypeRef::Char) {
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_string_bridge_null_check.jinja",
                minijinja::context! { name => param.name.clone() },
            ));
        }
    }
    out
}

fn render_bridge_param_conversions(non_options_params: &[&ParamDef]) -> String {
    let mut out = String::new();
    for param in non_options_params {
        if matches!(param.ty, TypeRef::String | TypeRef::Char) {
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_string_bridge_param_conversion.jinja",
                minijinja::context! { name => param.name.clone() },
            ));
        }
    }
    out
}

fn render_bridge_call_args(params: &[ParamDef], options_param_name: &str) -> String {
    params
        .iter()
        .map(|param| {
            let is_string_or_char = matches!(param.ty, TypeRef::String | TypeRef::Char);
            if param.name == options_param_name || (is_string_or_char && param.is_ref) {
                format!("{}_rs", param.name)
            } else if is_string_or_char {
                format!("{}_rs.to_string()", param.name)
            } else {
                param.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
