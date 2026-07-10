use crate::backends::ffi::gen_visitor::callback_specs::{CallbackSpec, ParamKind};
use crate::backends::ffi::gen_visitor::protocol::VisitorProtocol;
use crate::backends::ffi::template_env::render;

pub(super) fn rust_param_list(spec: &CallbackSpec, protocol: &VisitorProtocol) -> String {
    let mut parts = vec!["&mut self".to_string(), format!("ctx: &{}", protocol.context_path)];
    for p in &spec.params {
        match p {
            ParamKind::Str(n) => parts.push(format!("{n}: &str")),
            ParamKind::OptStr(n) => parts.push(format!("{n}: Option<&str>")),
            ParamKind::Bool(n) => parts.push(format!("{n}: bool")),
            ParamKind::U32(n) => parts.push(format!("{n}: u32")),
            ParamKind::Usize(n) => parts.push(format!("{n}: usize")),
            ParamKind::CellSlice(n) => parts.push(format!("{n}: &[String]")),
        }
    }
    parts.join(", ")
}

/// Generate the body of one visitor trait impl method.
///
/// Produces local CString bindings, the `call_with_ctx` invocation, and the
/// callback argument forwarding.
fn gen_impl_body(spec: &CallbackSpec, _core_import: &str, protocol: &VisitorProtocol, default_result: &str) -> String {
    let mut bindings = String::new();
    let mut cb_args = Vec::new();
    let _ = protocol;

    for p in &spec.params {
        match p {
            ParamKind::Str(n) => {
                bindings.push_str(&render(
                    "ffi_visitor_cstring_param_setup.jinja",
                    minijinja::context! {
                        name => n.as_str(),
                        default_result,
                    },
                ));
                cb_args.push(format!("{n}_cs.as_ptr()"));
            }
            ParamKind::OptStr(n) => {
                bindings.push_str(&render(
                    "ffi_visitor_optional_string_param_setup.jinja",
                    minijinja::context! { name => n.as_str() },
                ));
                cb_args.push(format!("{n}_ptr"));
            }
            ParamKind::Bool(n) => {
                bindings.push_str(&render(
                    "ffi_visitor_bool_param_setup.jinja",
                    minijinja::context! { name => n.as_str() },
                ));
                cb_args.push(format!("{n}_i"));
            }
            ParamKind::U32(n) | ParamKind::Usize(n) => {
                cb_args.push(n.clone());
            }
            ParamKind::CellSlice(n) => {
                bindings.push_str(&render(
                    "ffi_visitor_string_list_param_setup.jinja",
                    minijinja::context! { name => n.as_str() },
                ));
                cb_args.push(format!("{n}_ptrs.as_ptr()"));
                cb_args.push("cell_count".to_string());
            }
        }
    }

    let args_str = if cb_args.is_empty() {
        "out_custom, out_len".to_string()
    } else {
        format!("{}, out_custom, out_len", cb_args.join(", "))
    };

    render(
        "ffi_visitor_impl_body.jinja",
        minijinja::context! {
            name => spec.name.as_str(),
            default_result,
            bindings,
            args_str,
        },
    )
}

/// Generate all visitor trait impl methods.
pub(super) fn gen_impl_methods(
    specs: &[CallbackSpec],
    pascal_prefix: &str,
    core_import: &str,
    protocol: &VisitorProtocol,
    default_result: &str,
) -> String {
    let mut out = String::new();
    let result_path = protocol.result_path.clone();
    for spec in specs {
        out.push_str(&render(
            "ffi_visitor_impl_method.jinja",
            minijinja::context! {
                name => spec.name.as_str(),
                params => rust_param_list(spec, protocol),
                result_path => result_path.as_str(),
                body => gen_impl_body(spec, core_import, protocol, default_result),
            },
        ));
    }
    let _ = pascal_prefix;
    out
}

/// Build the forwarding argument list for `VisitorRef` delegation.
fn visitor_ref_args(spec: &CallbackSpec) -> String {
    let mut args = vec!["ctx".to_string()];
    for p in &spec.params {
        match p {
            ParamKind::Str(n)
            | ParamKind::OptStr(n)
            | ParamKind::Bool(n)
            | ParamKind::U32(n)
            | ParamKind::Usize(n)
            | ParamKind::CellSlice(n) => args.push(n.clone()),
        }
    }
    args.join(", ")
}

/// Generate all `VisitorRef` forwarding methods.
pub(super) fn gen_visitor_ref_methods(
    specs: &[CallbackSpec],
    _core_import: &str,
    protocol: &VisitorProtocol,
) -> String {
    let mut out = String::new();
    let result_path = protocol.result_path.clone();
    for spec in specs {
        let params = rust_param_list(spec, protocol);
        let args = visitor_ref_args(spec);
        out.push_str(&render(
            "vtable_delegation_method.jinja",
            minijinja::context! {
                method_name => spec.name.as_str(),
                all_params => params,
                ret => result_path.as_str(),
                arg_list => args,
            },
        ));
    }
    out
}
