//! Presence companions for free functions and methods returning `Optional<T>` directly.
//!
//! `null_return_value` collapses a bare `Option<T>` return to the same scalar/`0` sentinel a
//! legitimate `Some(0)` (or `Some(false)`, `Some(Duration::ZERO)`, ...) would also produce --
//! the exact defect the struct-field `has_<field>` companion
//! (`crate::backends::ffi::gen_bindings::types::gen_field_presence_accessor`) already fixes for
//! fields. A function or method has no struct to hang that companion off, so this module emits
//! the same convention -- an additive `{fn}_has_result` sibling export -- for the return path.
//!
//! # Why a second call, not an out-parameter
//!
//! Three shapes were on the table: a paired `_has_result` companion, an out-parameter alongside
//! the existing return, or an encoding with no sentinel at all. An out-parameter would change an
//! already-exported symbol's signature -- a breaking ABI change outside a MAJOR bump. An
//! encoding with no sentinel (e.g. always returning a pointer) would change the primary
//! function's return type for the same reason. A paired companion is strictly additive: it
//! declares a brand new symbol and leaves the primary export untouched, so it ships within a
//! MINOR release exactly like the field companion did. The cost is that the companion calls the
//! underlying function/method a second time -- acceptable for the accessor-shaped functions this
//! defect was reported against (and no worse than the field companion's serialized-handle path,
//! which already re-runs a full JSON deserialization to answer the same question). It is NOT
//! generated for an owned receiver (`self` by value): the underlying call there removes the
//! handle from the registry, so a second invocation would not just risk re-running a side
//! effect, it would fail outright because the handle is already gone. ~keep

use ahash::{AHashMap, AHashSet};
use minijinja::context;

use crate::codegen::c_consumer;
use crate::codegen::conversions::core_type_path;
use crate::core::ir::{CoreWrapper, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

use super::orchestration::{named_handle_type, named_type_path};
use super::params::{ParamConversionContext, gen_param_conversion_with_enums};
use super::support::{ffi_doxygen_block, method_sanitized_recoverable, sanitized_recoverable};
use crate::backends::ffi::type_map::result_presence_companion_exists;

/// Every presence companion fails the same way regardless of the primary function's return
/// shape: it always returns `i32`, so a param-conversion failure, a bad handle, or a caught
/// panic all report `-1` (see `{prefix}_last_error_code` for the reason). `ffi_null_return_value`
/// exists to answer "what does the PRIMARY return type's sentinel look like"; the companion does
/// not have that question, so this is a fixed constant, not a re-derivation. ~keep
const PRESENCE_FAIL_RET: &str = "return -1;";
const PRESENCE_PANIC_RETURN: &str = "-1";

/// The Rust call-site argument expression for one already-converted parameter local
/// (`{name}_rs`). Identical in shape to the primary wrapper's own arg-building match (see
/// `gen_method_wrapper` / `gen_free_function` in `orchestration.rs`) with one simplification:
/// the primary method version also special-cases an owned receiver for `Named` params, which
/// never applies here because [`gen_method_result_presence_wrapper`] refuses to generate a
/// companion for an owned receiver in the first place -- see the module doc. That reduces the
/// method and free-function cases to the exact same match, so this one helper serves both. ~keep
fn presence_call_arg(p: &ParamDef) -> String {
    let rs = format!("{}_rs", p.name);
    match &p.ty {
        TypeRef::Path if !p.optional => {
            if p.is_ref {
                format!("{rs}.as_path()")
            } else {
                rs
            }
        }
        TypeRef::String | TypeRef::Char if !p.optional => {
            if p.is_ref {
                format!("&{rs}")
            } else if p.core_wrapper == CoreWrapper::Cow {
                format!("{rs}.into()")
            } else {
                rs
            }
        }
        TypeRef::Bytes if !p.optional => {
            if p.is_ref {
                format!("&{rs}")
            } else {
                rs
            }
        }
        TypeRef::Named(_) if !p.optional => {
            if p.is_mut || !p.is_ref {
                rs
            } else {
                format!("&{rs}")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
            if p.is_ref {
                format!("{rs}.as_deref()")
            } else if p.core_wrapper == CoreWrapper::Cow {
                format!("{rs}.map(std::borrow::Cow::Owned)")
            } else {
                rs
            }
        }
        TypeRef::Path if p.optional => {
            if p.is_ref {
                format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
            } else {
                rs
            }
        }
        TypeRef::Named(_) if p.optional => {
            if p.is_ref {
                format!("{rs}.as_ref()")
            } else {
                rs
            }
        }
        TypeRef::Json if !p.optional => {
            if p.is_ref {
                format!("&{rs}")
            } else {
                rs
            }
        }
        TypeRef::Json if p.optional => {
            if p.is_ref {
                format!("{rs}.as_ref()")
            } else {
                rs
            }
        }
        TypeRef::Vec(_inner) if !p.optional => {
            if p.is_mut {
                format!("&mut {rs}")
            } else if p.is_ref && p.vec_inner_is_ref {
                format!("&{rs}.iter().map(|s| s.as_str()).collect::<Vec<&str>>()")
            } else if p.is_ref {
                format!("&{rs}")
            } else {
                rs
            }
        }
        TypeRef::Map(_, _) if !p.optional => {
            if p.is_mut {
                format!("&mut {rs}")
            } else if p.is_ref && p.map_is_btree {
                format!("&{}_btree", p.name)
            } else if p.is_ref {
                format!("&{rs}")
            } else if p.map_is_btree {
                format!("{rs}.into_iter().collect::<std::collections::BTreeMap<_, _>>()")
            } else {
                rs
            }
        }
        TypeRef::Vec(_) if p.optional => {
            if p.is_mut {
                format!("{rs}.as_deref_mut()")
            } else if p.is_ref {
                format!("{rs}.as_deref()")
            } else {
                rs
            }
        }
        TypeRef::Map(_, _) if p.optional => {
            if p.is_mut {
                format!("{rs}.as_deref_mut()")
            } else if p.is_ref {
                format!("{rs}.as_ref()")
            } else {
                rs
            }
        }
        _ => rs,
    }
}

/// Build the C parameter declarations for the presence-companion header: an optional leading
/// `this: AlefHandle` plus one entry per source parameter (and its `_len` sibling for `Bytes`).
/// Names get an `_` prefix when `will_be_unimplemented`, mirroring the primary wrapper's own
/// convention for an unused, stubbed-out parameter list.
fn presence_param_list(
    params: &[ParamDef],
    self_param: Option<&str>,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    will_be_unimplemented: bool,
) -> Vec<String> {
    let mut c_params = Vec::new();
    if let Some(name) = self_param {
        c_params.push(format!("    {name}: AlefHandle"));
    }
    for p in params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        c_params.push(format!(
            "    {}: {}",
            param_name,
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                core_import,
                path_map,
                enum_names,
                p.is_mut,
            )
        ));
        if matches!(p.ty, TypeRef::Bytes) {
            let len_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            c_params.push(format!("    {len_name}: usize"));
        }
    }
    c_params
}

/// Emit the per-parameter conversion statements (`let {name}_rs = ...;`) plus any BTree-map
/// rebinding, exactly as the primary wrapper does -- only called once eligibility is confirmed
/// and the primary is not a stub, so real parameter names are always in scope here.
fn render_presence_param_conversions(
    out: &mut String,
    params: &[ParamDef],
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    has_error: bool,
) {
    for p in params {
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_param_conversion_with_enums(p, &ParamConversionContext {
                    has_error,
                    // Forces the param-conversion failure path to `PRESENCE_FAIL_RET` regardless
                    // of the primary function's return shape: this companion's C return type is
                    // always `i32`, never the primary's real byte-buffer out-params. ~keep
                    is_bytes_result: true,
                    return_type: &TypeRef::Unit,
                    ffi_return_type: Some("i32"),
                    core_import,
                    path_map,
                    enum_names,
                }),
            },
        ));
    }
    for p in params {
        if matches!(p.ty, TypeRef::Map(_, _)) && !p.optional && p.is_ref && p.map_is_btree {
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_btree_binding.jinja",
                context! { btree => format!("{}_btree", p.name), rs => format!("{}_rs", p.name) },
            ));
        }
    }
}

fn presence_header(fn_name: &str, doc_comment: &str, params: Vec<String>, source_cfg: &str) -> String {
    crate::backends::ffi::template_env::render(
        "method_wrapper_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => Option::<String>::None,
            fn_name => fn_name,
            params => params,
            return_type => Some("i32"),
            source_cfg => source_cfg,
            inline_callee => Option::<String>::None,
        },
    )
}

fn presence_footer() -> String {
    crate::backends::ffi::template_env::render(
        "function_wrapper_footer.jinja",
        context! { panic_return => PRESENCE_PANIC_RETURN, trivial_call => false },
    )
}

fn push_presence_unimplemented_body(out: &mut String, qualified_name: &str) {
    out.push_str(&format!(
        "    set_last_error(99, \"Not implemented: {qualified_name}\");\n    {PRESENCE_PANIC_RETURN}"
    ));
    out.push_str(&presence_footer());
}

/// Emit `handle_acquisition.rs.jinja` for `this` (when present, as `self_request`) plus any
/// `Named` parameters, always failing with `PRESENCE_FAIL_RET`.
fn render_handle_acquisition(
    out: &mut String,
    self_request: Option<String>,
    params: &[ParamDef],
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
) {
    let mut requests: Vec<String> = self_request.into_iter().collect();
    for parameter in params {
        let Some(type_name) = named_handle_type(&parameter.ty) else {
            continue;
        };
        if enum_names.contains(type_name) {
            continue;
        }
        let request = format!(
            "HandleRequest {{ handle: {}, expected_type: std::any::TypeId::of::<{}>() }}",
            parameter.name,
            named_type_path(type_name, core_import, path_map)
        );
        if parameter.optional {
            requests.push(format!(
                "if {} != 0 {{ Some({request}) }} else {{ None }}",
                parameter.name
            ));
        } else {
            requests.push(format!("Some({request})"));
        }
    }
    if requests.is_empty() {
        return;
    }
    out.push_str(&crate::backends::ffi::template_env::render(
        "handle_acquisition.rs.jinja",
        context! {
            has_requests => true,
            requests => requests.join(",\n"),
            fail_ret => PRESENCE_FAIL_RET,
            owned_handle => Option::<&str>::None,
        },
    ));
}

/// Emit the tail: check `result.is_some()` (bare) or match `Ok(val)`/`Err(e)` and check
/// `val.is_some()` (fallible), then the shared panic footer.
fn render_presence_tail(out: &mut String, has_error: bool) {
    if has_error {
        out.push_str(&crate::backends::ffi::template_env::render(
            "error_match_non_void.jinja",
            context! {
                ok_body => "            i32::from(val.is_some())\n",
                null_ret => PRESENCE_PANIC_RETURN,
            },
        ));
    } else {
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! { content => "    i32::from(result.is_some())\n" },
        ));
    }
    out.push_str(&presence_footer());
}

/// Generate a `{prefix}_{type}_{method}_has_result` companion for a method returning
/// `Optional<T>` where `T`'s leaf collapses `None` and a zero-valued `Some` to the same FFI
/// sentinel. Returns `None` when no companion is needed -- the return type isn't an ambiguous
/// `Optional`, or the receiver is owned (see the module doc for why owned receivers are
/// excluded).
pub(in crate::backends::ffi::gen_bindings) fn gen_method_result_presence_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
) -> Option<String> {
    if !result_presence_companion_exists(&method.return_type, method.receiver.as_ref()) {
        return None;
    }

    let has_error = method.error_type.is_some();
    let will_be_unimplemented = method.sanitized && !method_sanitized_recoverable(method);

    let type_name = &typ.name;
    let method_name = &method.name;
    let base_fn_name = c_consumer::method_symbol(prefix, &typ.name, &method.name);
    let fn_name = c_consumer::result_presence_symbol(&base_fn_name);
    let doc_comment = ffi_doxygen_block(&format!(
        "Report whether `{type_name}::{method_name}` returned `Some`.\n\n`{base_fn_name}` cannot \
         distinguish a `None` result from a legitimate zero-valued `Some` at the C ABI boundary. \
         Call this function first: `1` means the sibling getter's return value is meaningful, `0` \
         means the result was absent and the getter's sentinel must be ignored, `-1` reports an \
         invalid handle or a call error (see `{prefix}_last_error_code`)."
    ));
    let source_cfg = method.cfg_within(typ.cfg.as_deref()).unwrap_or_default();

    let self_param = (!method.is_static).then_some(if will_be_unimplemented { "_this" } else { "this" });
    let c_params = presence_param_list(
        &method.params,
        self_param,
        core_import,
        path_map,
        enum_names,
        will_be_unimplemented,
    );
    let mut out = presence_header(&fn_name, &doc_comment, c_params, &source_cfg);

    if will_be_unimplemented {
        push_presence_unimplemented_body(&mut out, &format!("{type_name}::{method_name}"));
        return Some(out);
    }

    let qualified = core_type_path(typ, core_import);
    let qualified_with_lifetime = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };
    let handle_qualified = if typ.has_lifetime_params {
        format!("SerializedHandle<{qualified_with_lifetime}>")
    } else {
        qualified.clone()
    };

    if !method.is_static {
        let self_request = Some(format!(
            "Some(HandleRequest {{ handle: this, expected_type: std::any::TypeId::of::<{handle_qualified}>() }})"
        ));
        render_handle_acquisition(
            &mut out,
            self_request,
            &method.params,
            core_import,
            path_map,
            enum_names,
        );

        let null_check = if typ.has_lifetime_params {
            crate::backends::ffi::template_env::render(
                "snapshot_handle_self_ref.jinja",
                context! {
                    fail_ret => PRESENCE_FAIL_RET,
                    qualified => qualified_with_lifetime.clone(),
                    handle_qualified => handle_qualified.clone(),
                },
            )
        } else {
            match method.receiver.as_ref().unwrap_or(&ReceiverKind::Ref) {
                ReceiverKind::RefMut => crate::backends::ffi::template_env::render(
                    "null_check_self_mut.jinja",
                    context! { fail_ret => PRESENCE_FAIL_RET, qualified => qualified.clone() },
                ),
                _ => crate::backends::ffi::template_env::render(
                    "null_check_self_ref.jinja",
                    context! { fail_ret => PRESENCE_FAIL_RET, qualified => qualified.clone() },
                ),
            }
        };
        out.push_str(&crate::backends::ffi::template_env::render(
            "code_line.jinja",
            context! { content => null_check },
        ));
    } else {
        render_handle_acquisition(&mut out, None, &method.params, core_import, path_map, enum_names);
    }

    render_presence_param_conversions(&mut out, &method.params, core_import, path_map, enum_names, has_error);

    let call_args = method
        .params
        .iter()
        .map(presence_call_arg)
        .collect::<Vec<_>>()
        .join(", ");
    if method.is_static {
        out.push_str(&crate::backends::ffi::template_env::render(
            "static_method_call_result.jinja",
            context! { qualified => qualified, method_name => method_name.clone(), call_args => call_args },
        ));
    } else if method.is_async {
        let call = format!("get_ffi_runtime().block_on(async {{ obj.{method_name}({call_args}).await }})");
        out.push_str(&crate::backends::ffi::template_env::render(
            "call_with_result.jinja",
            context! { call => call },
        ));
    } else {
        out.push_str(&crate::backends::ffi::template_env::render(
            "instance_method_call_result.jinja",
            context! { method_name => method_name.clone(), call_args => call_args },
        ));
    }

    render_presence_tail(&mut out, has_error);
    Some(out)
}

/// Generate a `{prefix}_{function}_has_result` companion for a free function returning
/// `Optional<T>` where `T`'s leaf collapses `None` and a zero-valued `Some` to the same FFI
/// sentinel. Returns `None` when no companion is needed.
pub(in crate::backends::ffi::gen_bindings) fn gen_free_function_result_presence_wrapper(
    func: &FunctionDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
) -> Option<String> {
    if !result_presence_companion_exists(&func.return_type, None) {
        return None;
    }

    let has_error = func.error_type.is_some();
    let will_be_unimplemented = func.sanitized && !sanitized_recoverable(func);

    let func_name = &func.name;
    let base_fn_name = c_consumer::free_function_symbol(prefix, &func.name);
    let fn_name = c_consumer::result_presence_symbol(&base_fn_name);
    let doc_comment = ffi_doxygen_block(&format!(
        "Report whether `{func_name}` returned `Some`.\n\n`{base_fn_name}` cannot distinguish a \
         `None` result from a legitimate zero-valued `Some` at the C ABI boundary. Call this \
         function first: `1` means the sibling getter's return value is meaningful, `0` means \
         the result was absent and the getter's sentinel must be ignored, `-1` reports an \
         invalid handle or a call error (see `{prefix}_last_error_code`)."
    ));
    let source_cfg = func.cfg.as_deref().unwrap_or("").to_string();

    let c_params = presence_param_list(
        &func.params,
        None,
        core_import,
        path_map,
        enum_names,
        will_be_unimplemented,
    );
    let mut out = presence_header(&fn_name, &doc_comment, c_params, &source_cfg);

    if will_be_unimplemented {
        push_presence_unimplemented_body(&mut out, func_name);
        return Some(out);
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    render_handle_acquisition(&mut out, None, &func.params, core_import, path_map, enum_names);
    render_presence_param_conversions(&mut out, &func.params, core_import, path_map, enum_names, has_error);

    let call_args = func.params.iter().map(presence_call_arg).collect::<Vec<_>>().join(", ");
    let call = if func.is_async {
        format!("get_ffi_runtime().block_on(async {{ {core_fn_path}({call_args}).await }})")
    } else {
        format!("{core_fn_path}({call_args})")
    };
    out.push_str(&crate::backends::ffi::template_env::render(
        "call_with_result.jinja",
        context! { call => call },
    ));

    render_presence_tail(&mut out, has_error);
    Some(out)
}
