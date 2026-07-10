//! Options-field bridge code generation for the C FFI backend.
//!
//! When a `[[trait_bridges]]` entry is configured with `bind_via = "options_field"`,
//! the visitor handle lives as a field on the options struct rather than as a positional
//! argument. This module generates:
//!
//! 1. `{prefix}_options_set_{field}` — a setter that wraps the vtable bridge in a
//!    `Rc<RefCell<dyn Trait>>` and stores it on `options.{field}`.  Callers (Go, Java, C#)
//!    invoke this before calling the generated FFI wrapper for the configured function.
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

/// Generate the `{prefix}_options_set_{field}` setter.
///
/// The setter wraps the vtable bridge handle in a thin `Rc<RefCell<VtableRef>>` delegating
/// wrapper and stores it in the options struct's visitor field.  Callers must invoke this
/// before passing options to the generated options-field bridge wrapper.
///
/// # Parameters
///
/// - `prefix`: the FFI symbol prefix.
/// - `core_import`: the Rust crate name for the core library.
/// - `trait_def`: the IR definition of the trait whose bridge is being attached.
/// - `field_name`: the field on the options struct.
/// - `options_type_name`: the IR type name of the options struct.
/// - `type_paths`: map of IR type name → fully-qualified Rust path for signature generation.
/// - `use_callbacks_visitor`: when true, accept the `{prefix}Visitor` handle produced by the
///   visitor-callbacks path (gen_visitor) instead of the `{prefix}{trait}Bridge` produced by
///   the trait-bridge path. Required so that strict-typed C consumers (Zig) can pass the
///   handle returned by `{prefix}_visitor_create` directly. Both types implement the same
///   trait so the wrapper body is identical.
pub fn gen_options_set_bridge(
    prefix: &str,
    core_import: &str,
    trait_def: &TypeDef,
    field_name: &str,
    options_type_name: &str,
    type_paths: &HashMap<String, String>,
    use_callbacks_visitor: bool,
) -> String {
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

    let delegation_methods = gen_vtable_ref_delegation(trait_def, core_import, type_paths);

    format!(
        r#"/// Attach a vtable visitor bridge to a `{options_type_name}` options struct.
///
/// The `{handle_type}` encapsulates a set of C function pointers that receive visit
/// callbacks during generated conversion. Call this setter before invoking the generated
/// options-field bridge wrapper to activate visitor callbacks. Pass `visitor = null` to clear
/// a previously attached visitor.
///
/// Neither pointer is consumed: the caller retains ownership of both `options` and `visitor`
/// and must free them independently after conversion completes.
///
/// # Safety
///
/// `options` must be a non-null pointer returned by `{prefix}_{options_type_snake}_new` (or
/// equivalent), valid for write access.  `visitor` must be a non-null pointer returned by
/// `{handle_constructor}`, or null.  Both must remain valid for the duration of any
/// subsequent options-field bridge wrapper call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_name}(
    options: *mut {core_import}::{options_type_name},
    visitor: *mut {handle_type},
) {{
    if options.is_null() {{
        return;
    }}
    // SAFETY: null check above guarantees options is a valid, aligned, initialised pointer.
    let opts = unsafe {{ &mut *options }};

    if visitor.is_null() {{
        opts.{field_name} = None;
        return;
    }}

    // Wrap the raw bridge pointer in a thin delegating type that implements the trait.
    // `VtableRef` borrows the bridge by raw pointer and must not outlive the bridge handle.
    struct VtableRef(*mut {handle_type});

    impl std::fmt::Debug for VtableRef {{
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
            f.debug_tuple("VtableRef").finish()
        }}
    }}

    // SAFETY: {handle_type} is `Send + Sync` (unsafe impl generated by gen_trait_bridge).
    // The caller guarantees the pointer remains valid while options is in use.
    unsafe impl Send for VtableRef {{}}
    // SAFETY: see Send impl above; VtableRef is a transparent wrapper around a raw pointer
    // to a type that is itself `Send + Sync`. The outer Arc<Mutex> serialises access.
    unsafe impl Sync for VtableRef {{}}

    impl {trait_path} for VtableRef {{
{delegation_methods}    }}

    // SAFETY: visitor is non-null; Arc<Mutex<_>> satisfies the configured bridge handle type.
    opts.{field_name} = Some(std::sync::Arc::new(std::sync::Mutex::new(VtableRef(visitor))));
}}"#,
        prefix = prefix,
        handle_type = handle_type,
        handle_constructor = handle_constructor,
        fn_name = fn_name,
        core_import = core_import,
        options_type_name = options_type_name,
        options_type_snake = options_type_snake,
        field_name = field_name,
        trait_path = trait_path,
        delegation_methods = delegation_methods,
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
    let params = render_bridge_params(&non_options_params, options_param, core_import);
    let null_checks = render_bridge_null_checks(&non_options_params);
    let conversions = render_bridge_param_conversions(&non_options_params);
    let call_args = render_bridge_call_args(&func.params, options_param_name);
    let return_type_snake = ffi_symbol_component(return_type_name);

    Some(format!(
        r#"/// Run `{func_name}` with configured options-field bridge support.
///
/// Returns a heap-allocated [`{return_type_name}`] on success, or null on failure.
/// Check `{prefix}_last_error_code` / `{prefix}_last_error_context` for error details.
/// The returned pointer must be freed with `{prefix}_{return_type_snake}_free`.
///
/// If a bridge was attached to `{options_param_name}`, it will be passed through to the core call.
///
/// # Safety
///
/// Pointer arguments must be valid for the duration of this call. Returned pointer must be
/// freed with `{prefix}_{return_type_snake}_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {ffi_function_name}(
{params}
) -> *mut {core_import}::{return_type_name} {{
    clear_last_error();
{null_checks}
{conversions}

    // Clone options out of the pointer. Any bridge attached via the generated setter is
    // embedded in the options value and will be picked up automatically by the core call.
    let {options_param_name}_rs: Option<{core_import}::{options_type_name}> = if {options_param_name}.is_null() {{
        None
    }} else {{
        // SAFETY: null check above guarantees {options_param_name} is a valid pointer.
        Some(unsafe {{ &*{options_param_name} }}.clone())
    }};

    match {core_fn_path}({call_args}) {{
        Ok(result) => Box::into_raw(Box::new(result)),
        Err(e) => {{
            set_last_error(2, &e.to_string());
            std::ptr::null_mut()
        }}
    }}
}}"#,
        prefix = prefix,
        func_name = func.name,
        ffi_function_name = ffi_function_name,
        core_fn_path = core_fn_path,
        core_import = core_import,
        params = params,
        null_checks = null_checks,
        conversions = conversions,
        call_args = call_args,
        options_param_name = options_param_name,
        options_type_name = options_type_name,
        return_type_name = return_type_name,
        return_type_snake = return_type_snake,
    ))
}

/// Generate `impl {trait_path} for VtableRef` method bodies.
///
/// Each method delegates to `unsafe { (*self.0).method_name(args...) }`, reusing
/// the trait impl already generated on the bridge struct by `gen_trait_bridge`.
fn gen_vtable_ref_delegation(trait_def: &TypeDef, core_import: &str, type_paths: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(4096);

    let own_methods: Vec<&MethodDef> = trait_def.methods.iter().filter(|m| m.trait_source.is_none()).collect();

    for method in &own_methods {
        let receiver_str = match &method.receiver {
            Some(ReceiverKind::Ref) => "&self",
            Some(ReceiverKind::RefMut) => "&mut self",
            Some(ReceiverKind::Owned) => "self",
            None => "",
        };

        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, format_param_type(p, type_paths)))
            .collect();

        let all_params = if receiver_str.is_empty() {
            params.join(", ")
        } else if params.is_empty() {
            receiver_str.to_string()
        } else {
            format!("{}, {}", receiver_str, params.join(", "))
        };

        let error_override = method
            .error_type
            .as_ref()
            .map(|_| "Box<dyn std::error::Error + Send + Sync>".to_string());
        let ret = crate::codegen::generators::trait_bridge::format_return_type(
            &method.return_type,
            error_override.as_deref(),
            type_paths,
            method.returns_ref,
        );

        let arg_list = build_arg_list(method, core_import, type_paths);
        let method_name = &method.name;

        out.push_str(&crate::backends::ffi::template_env::render(
            "vtable_ref_delegation_method.jinja",
            minijinja::context! {
                method_name => method_name,
                all_params => &all_params,
                ret => &ret,
                arg_list => &arg_list,
            },
        ));
    }

    out
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

fn render_bridge_params(non_options_params: &[&ParamDef], options_param: &ParamDef, core_import: &str) -> String {
    let mut params: Vec<String> = non_options_params
        .iter()
        .filter_map(|param| match param.ty {
            TypeRef::String | TypeRef::Char => Some(format!("    {}: *const std::ffi::c_char", param.name)),
            _ => None,
        })
        .collect();
    let options_type = named_type_name(&options_param.ty).unwrap_or_default();
    params.push(format!(
        "    {}: *const {core_import}::{options_type}",
        options_param.name
    ));
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
