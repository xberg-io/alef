use crate::core::ir::{MethodDef, ReceiverKind, TypeRef};

use super::excluded::{excluded_carrier_name, excluded_type_core_path};
use crate::backends::dart::gen_rust_crate::trait_types::{
    trait_impl_param_conversion, trait_impl_param_type, trait_impl_return_conversion, trait_impl_return_type,
};

/// Emit one method implementation on the bridge struct.
///
/// The method signature must match the **original** trait signature (ref-aware,
/// original primitive widths). The closures stored in the struct hold
/// FRB-friendly widened types (e.g. `i64` for `u64`, `f64` for `f32`). The
/// impl body converts between the two representations.
///
/// For methods with an `error_type`, the return type is
/// `{source_crate}::Result<T>` — the Dart callback never fails, so the body
/// wraps the awaited value in `Ok(...)`.
pub(super) fn emit_trait_bridge_method(
    out: &mut String,
    method: &MethodDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
    excluded_type_paths: &std::collections::HashMap<String, String>,
    lifetime_type_names: &std::collections::HashSet<String>,
) {
    let method_name = &method.name;

    // Build the method signature matching the actual trait.
    // - Reference params use `&` / `&mut` prefix.
    // - Primitive params use their original width (not FRB-widened).
    // Emit the self receiver matching the trait definition so rustc's E0053
    // ("method has an incompatible type for trait") is not triggered for
    // traits that use `&mut self` (e.g. `HtmlVisitor`).
    let self_receiver = match method.receiver {
        Some(ReceiverKind::RefMut) => "&mut self",
        Some(ReceiverKind::Owned) => "self",
        // Default: `&self` (covers `Some(ReceiverKind::Ref)` and `None`).
        _ => "&self",
    };
    let params_sig: Vec<String> = std::iter::once(self_receiver.to_string())
        .chain(method.params.iter().map(|p| {
            let orig_ty = trait_impl_param_type(p, source_crate_name, type_paths, lifetime_type_names);
            format!("{}: {orig_ty}", p.name)
        }))
        .collect();

    // Detect the `&[&str]` (Vec<String> + returns_ref) special case — the trait method
    // expects a borrowed static slice but the Dart-side closure produces owned
    // `Vec<String>`. We materialise that into `&'static [&'static str]` via Box::leak
    // (same pattern as the napi/pyo3 trait-bridges, see
    // `alef-codegen::trait_bridge::gen_method`). The owned vector is leaked once per
    // method invocation: acceptable for plugin metadata that's typically read at
    // registration time.
    let is_ref_slice_of_str = method.returns_ref
        && matches!(
            &method.return_type,
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String)
        );
    // Return type: use original primitive/named type; wrap in source-crate Result when error_type set.
    let ret = if is_ref_slice_of_str {
        "&[&str]".to_string()
    } else {
        trait_impl_return_type(&method.return_type, source_crate_name, type_paths)
    };
    let return_sig = if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            format!("{source_crate_name}::Result<()>")
        } else {
            format!("{source_crate_name}::Result<{ret}>")
        }
    } else {
        ret.clone()
    };

    let async_kw = if method.is_async { "async " } else { "" };
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_method_signature.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_name => method_name.as_str(),
            params => params_sig.join(", "),
            return_sig => return_sig.as_str(),
        },
    ));

    // Emit owned-conversion let-bindings for each parameter before calling the closure.
    // References become owned; primitives may be widened; mut refs are copied for the callback.
    for p in &method.params {
        let conv = trait_impl_param_conversion(p, excluded_type_paths);
        if !conv.is_empty() {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_param_conversion.jinja",
                minijinja::context! {
                    conversion => conv,
                },
            ));
        }
    }

    // Build call-site arg list (use the local owned var names).
    //
    // For params whose original type was excluded from public bindings, the Dart-facing
    // closure receives an opaque JSON carrier. The Rust trait method itself still
    // receives the source-crate type, so serialize at the bridge edge explicitly.
    let mut pre_bindings = String::new();
    let call_args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let carrier_type = match &p.ty {
                TypeRef::Named(name) if excluded_type_paths.contains_key(name) => Some(excluded_carrier_name(name)),
                _ => None,
            };
            if let Some(carrier_type) = carrier_type {
                let local = format!("__{}_local", p.name);
                let expr = if p.optional {
                    if method.error_type.is_some() {
                        format!(
                            "{name}.map(|v| serde_json::to_string(&v).map(|json| {carrier_type} {{ json }})).transpose()?",
                            name = p.name,
                            carrier_type = carrier_type,
                        )
                    } else {
                        format!(
                            "{name}.map(|v| {carrier_type} {{ json: serde_json::to_string(&v).expect(\"serialize excluded Dart trait bridge value\") }})",
                            name = p.name,
                            carrier_type = carrier_type,
                        )
                    }
                } else if method.error_type.is_some() {
                    format!(
                        "{carrier_type} {{ json: serde_json::to_string(&{name})? }}",
                        name = p.name,
                        carrier_type = carrier_type,
                    )
                } else {
                    format!(
                        "{carrier_type} {{ json: serde_json::to_string(&{name}).expect(\"serialize excluded Dart trait bridge value\") }}",
                        name = p.name,
                        carrier_type = carrier_type,
                    )
                };
                let _ = std::fmt::Write::write_fmt(
                    &mut pre_bindings,
                    format_args!("        let {local} = {expr};\n", local = local, expr = expr),
                );
                local
            } else {
                p.name.clone()
            }
        })
        .collect();
    if !pre_bindings.is_empty() {
        out.push_str(&pre_bindings);
    }
    let call_expr = format!("(self.{method_name})({})", call_args.join(", "));

    // Emit the body, adapting the return value from FRB-widened to original type.
    let ret_conv = trait_impl_return_conversion(&method.return_type, source_crate_name);

    // Special case: Named return type — the mirror type cannot be trivially converted
    // back to the core type. Drop the result and return Default::default().
    let named_return_default = ret_conv == "__NAMED_RETURN_DEFAULT__";

    // Special case: the return type was excluded from public bindings, substituted
    // to a JSON-backed carrier in the closure signature. Deserialize explicitly
    // to the source trait's exact return type.
    let excluded_return_name = match &method.return_type {
        TypeRef::Named(name) if excluded_type_paths.contains_key(name) => Some(name.as_str()),
        _ => None,
    };
    if let Some(excluded_return_name) = excluded_return_name {
        let core_path = excluded_type_core_path(excluded_return_name, source_crate_name, excluded_type_paths);
        let carrier_type = excluded_carrier_name(excluded_return_name);
        if method.is_async {
            if method.error_type.is_some() {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_excluded_async_result_return.rs.jinja",
                    minijinja::context! {
                        carrier_type => carrier_type.as_str(),
                        call_expr => call_expr.as_str(),
                        core_path => core_path.as_str(),
                    },
                ));
            } else {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_excluded_async_plain_return.rs.jinja",
                    minijinja::context! {
                        carrier_type => carrier_type.as_str(),
                        call_expr => call_expr.as_str(),
                        core_path => core_path.as_str(),
                    },
                ));
            }
        } else {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_block_on.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                    result_var => "__ret_bridge",
                },
            ));
            if method.error_type.is_some() {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_excluded_block_on_result_return.rs.jinja",
                    minijinja::context! {
                        core_path => core_path.as_str(),
                    },
                ));
            } else {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_excluded_block_on_plain_return.rs.jinja",
                    minijinja::context! {
                        core_path => core_path.as_str(),
                    },
                ));
            }
        }
        if method.error_type.is_some() {
            out.push_str("        Ok(__ret)\n");
        } else {
            out.push_str("        __ret\n");
        }
        out.push_str("    }\n");
        return;
    }

    if method.error_type.is_some() {
        // DartFnFuture never fails: wrap the awaited value in Ok(...).
        if method.is_async {
            if named_return_default {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_method_default_await.jinja",
                    minijinja::context! {
                        call_expr => call_expr.as_str(),
                        return_expr => "Ok(Default::default())",
                    },
                ));
            } else if ret_conv.is_empty() {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_method_ok_await.jinja",
                    minijinja::context! {
                        call_expr => call_expr.as_str(),
                    },
                ));
            } else {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_method_await_result.jinja",
                    minijinja::context! {
                        call_expr => call_expr.as_str(),
                        ret_conv => ret_conv.as_str(),
                    },
                ));
            }
        } else {
            // FRB workers run inside flutter_rust_bridge's active tokio runtime.
            // Spawn a dedicated OS thread with its own tokio runtime to avoid
            // "Cannot start a runtime from within a runtime" panic.
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_block_on.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                    result_var => "__result",
                },
            ));
            if named_return_default {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_method_default_from_result.jinja",
                    minijinja::context! {
                        return_expr => "Ok(Default::default())",
                    },
                ));
            } else {
                // error_type present: the Dart callback never fails, so wrap in Ok(...).
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_trait_method_ok_block_on.jinja",
                    minijinja::context! {
                        ret_conv => ret_conv.as_str(),
                    },
                ));
            }
        }
    } else if method.is_async {
        if named_return_default {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_default_await.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                    return_expr => "Default::default()",
                },
            ));
        } else if ret_conv.is_empty() {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_await_plain.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                },
            ));
        } else {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_await_result.jinja",
                minijinja::context! {
                    call_expr => call_expr.as_str(),
                    ret_conv => ret_conv.as_str(),
                },
            ));
        }
    } else {
        // FRB workers run inside flutter_rust_bridge's active tokio runtime.
        // Spawn a dedicated OS thread with its own tokio runtime to avoid
        // "Cannot start a runtime from within a runtime" panic.
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_trait_method_block_on.jinja",
            minijinja::context! {
                call_expr => call_expr.as_str(),
                result_var => "__result",
            },
        ));
        if named_return_default {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_default_from_result.jinja",
                minijinja::context! {
                    return_expr => "Default::default()",
                },
            ));
        } else if is_ref_slice_of_str {
            // Materialise `Vec<String>` into `&'static [&'static str]` so the trait
            // method's `&[&str]` return type is satisfied. Each closure invocation
            // leaks its strings — acceptable for plugin-metadata callsites.
            out.push_str(
                "            ;\n        \
                 let __strs: Vec<&'static str> = __result\n            \
                 .into_iter()\n            \
                 .map(|s| -> &'static str { Box::leak(s.into_boxed_str()) })\n            \
                 .collect();\n        \
                 Box::leak(__strs.into_boxed_slice())\n",
            );
        } else {
            // No error_type: return the plain value (no Ok() wrapping).
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_trait_method_plain_block_on_result.jinja",
                minijinja::context! {
                    ret_conv => ret_conv.as_str(),
                },
            ));
        }
    }
    out.push_str("    }\n");
}
