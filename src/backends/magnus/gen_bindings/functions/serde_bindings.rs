use crate::backends::magnus::type_map::MagnusMapper;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use ahash::AHashSet;

/// Build pre-call `let` bindings for AHashMap<Cow, Value> params.
///
/// When a Rust core function takes `Option<&AHashMap<Cow<'static, str>, Value>>`,
/// the Magnus wrapper receives `Option<HashMap<String, String>>` (Ruby Hash decoded
/// to Rust). A two-step conversion is needed:
/// (1) Bind an owned AHashMap to a named `let __<name>_ahash` before the call so
///     the borrow in the call arg lives long enough.
/// (2) Return the bound variable name for use in the call arg.
pub(in crate::backends::magnus::gen_bindings::functions) fn magnus_ahash_pre_call_bindings(
    params: &[ParamDef],
) -> Vec<String> {
    let mut bindings = Vec::new();
    for p in params {
        if let TypeRef::Map(_, _) = &p.ty {
            if p.map_is_ahash && p.map_key_is_cow {
                let bound_name = format!("__{}_ahash", p.name);
                bindings.push(format!(
                    "    let {bound_name} = {}.map(|m| m.into_iter().map(|(k, v)| (std::borrow::Cow::Owned(k), serde_json::Value::String(v))).collect::<ahash::AHashMap<std::borrow::Cow<'static, str>, serde_json::Value>>()); ",
                    p.name
                ));
            }
        }
    }
    bindings
}

/// Build call argument string, substituting AHashMap pre-bound variables for
/// any AHashMap<Cow, Value> params (which were re-bound in magnus_ahash_pre_call_bindings).
pub(in crate::backends::magnus::gen_bindings::functions) fn magnus_call_args_with_ahash(
    params: &[ParamDef],
    _opaque_types: &AHashSet<String>,
    base_call_args: &str,
) -> String {
    if !params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Map(_, _)) && p.map_is_ahash && p.map_key_is_cow)
    {
        return base_call_args.to_string();
    }
    let terms: Vec<&str> = base_call_args.split(", ").collect();
    let result: Vec<String> = terms
        .into_iter()
        .zip(params.iter())
        .map(|(term, p)| {
            if let TypeRef::Map(_, _) = &p.ty {
                if p.map_is_ahash && p.map_key_is_cow {
                    let bound_name = format!("__{}_ahash", p.name);
                    return if p.optional && p.is_ref {
                        format!("{bound_name}.as_ref()")
                    } else if p.is_ref {
                        format!("{bound_name}.as_ref().unwrap()")
                    } else {
                        bound_name
                    };
                }
            }
            term.to_string()
        })
        .collect();
    result.join(", ")
}

/// Returns true if a non-delegatable Magnus function/method can be recovered via serde
/// JSON-roundtrip on its params: every Named non-opaque param can be deserialized from a
/// String, and every sanitized Vec<String> param has `original_type` set.  Requires the
/// wrapper to return Result so the generated `?` operator works — `has_error` captures that
/// (the core fn returns Result, is async, is variadic, or was force-wrapped in Result because
/// it takes fallibly-deserialized params; see `params_need_fallible_deser`).
pub(in crate::backends::magnus::gen_bindings::functions) fn magnus_serde_recoverable(
    func: &FunctionDef,
    opaque_types: &AHashSet<String>,
    has_error: bool,
) -> bool {
    if !has_error {
        return false;
    }
    if !crate::codegen::shared::is_delegatable_return(&func.return_type) {
        return false;
    }
    func.params.iter().all(|p| {
        if p.sanitized {
            return p.original_type.is_some()
                && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));
        }
        match &p.ty {
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => true,
            _ => crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types),
        }
    })
}

/// Returns true if any param's deserialization preamble uses the fallible `?` operator —
/// i.e. a non-opaque Named param (serde JSON-roundtrip), a `Vec<Named>` batch param, or a
/// sanitized `Vec<String>` param. Such functions MUST return `Result` even when the core fn
/// is infallible (e.g. `max_sim_score -> f64`), otherwise the generated `?` fails to compile
/// (`E0277`). Callers OR this into `has_error` to force a `Result` return + `Ok(...)` wrap.
pub(in crate::backends::magnus::gen_bindings::functions) fn params_need_fallible_deser(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
) -> bool {
    params.iter().any(|p| match &p.ty {
        TypeRef::Named(n) => !opaque_types.contains(n.as_str()),
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())),
        _ => p.sanitized && p.original_type.is_some(),
    })
}

/// Generate Magnus serde let-bindings that produce `{name}_core: core::Type` so the shared
/// `gen_call_args_with_let_bindings` can emit `&{name}_core` for is_ref Named params.
///
/// Handles four cases for Named non-opaque params:
/// 1. `optional=true`: parameter is `Option<magnus::Value>` — bind as `Option<CoreType>`.
/// 2. `optional=false` but promoted (follows an optional param): parameter is also
///    `Option<magnus::Value>` due to `function_params` promotion — bind as `CoreType`,
///    falling back to `Default::default()` when the caller passes `nil`/omits the arg.
/// 3. `optional=false` and not promoted, but last param is default config: parameter is
///    `Option<magnus::Value>` from scan_args — bind as `CoreType`, falling back to
///    `Default::default()` when the caller omits the arg.
/// 4. `optional=false` and not promoted: parameter is `magnus::Value` — bind as `CoreType`.
pub(in crate::backends::magnus::gen_bindings::functions) fn magnus_serde_let_bindings(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
    _mapper: &MagnusMapper,
    is_default_config_func: bool,
) -> Vec<String> {
    let err = "magnus::Error::new(unsafe { Ruby::get_unchecked() }.exception_runtime_error(), e.to_string())";
    let mut out = Vec::new();
    for (idx, p) in params.iter().enumerate() {
        let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
        let is_last = idx == params.len() - 1;
        let is_last_config = is_last && is_default_config_func;
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                if p.optional {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_serde_named_binding.rs.jinja",
                        minijinja::context! {
                            mode => "optional",
                            name => &p.name,
                            core_import => core_import,
                            type_name => name,
                            error_expr => err,
                            is_mut => p.is_mut,
                        },
                    ));
                } else if promoted || is_last_config {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_serde_named_binding.rs.jinja",
                        minijinja::context! {
                            mode => "default",
                            name => &p.name,
                            core_import => core_import,
                            type_name => name,
                            error_expr => err,
                            is_mut => p.is_mut,
                        },
                    ));
                } else {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_serde_named_binding.rs.jinja",
                        minijinja::context! {
                            mode => "required",
                            name => &p.name,
                            core_import => core_import,
                            type_name => name,
                            error_expr => err,
                            is_mut => p.is_mut,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref && !p.sanitized =>
            {
                if p.optional {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_vec_refs_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => true,
                        },
                    ));
                } else {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_vec_refs_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => false,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() =>
            {
                if p.optional {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_sanitized_vec_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => true,
                            error_expr => err,
                        },
                    ));
                } else {
                    out.push(crate::backends::magnus::template_env::render(
                        "function_sanitized_vec_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => false,
                            error_expr => err,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    let core_inner_ty = format!("{core_import}::{name}");
                    let vec_ty = format!("Vec<{core_inner_ty}>");
                    if p.optional {
                        out.push(crate::backends::magnus::template_env::render(
                            "function_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                name => &p.name,
                                vec_ty => &vec_ty,
                                optional => true,
                            },
                        ));
                    } else {
                        out.push(crate::backends::magnus::template_env::render(
                            "function_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                name => &p.name,
                                vec_ty => &vec_ty,
                                optional => false,
                            },
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    out
}
