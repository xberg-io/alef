use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, visitor_param_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// The IR-derived inputs `gen_visitor_bridge` needs beyond the trait/bridge-config pair,
/// grouped only to keep its own parameter count under the lint limit at its single call site
/// (`trait_bridge.rs`). `pub(super)` (rather than file-private, unlike this module's other
/// grouping structs) because that one call site lives in the parent module and must be able to
/// construct it.
pub(super) struct VisitorBridgeInputs<'a> {
    pub(super) core_crate: &'a str,
    pub(super) type_paths: &'a HashMap<String, String>,
    pub(super) api: &'a ApiSurface,
    pub(super) pyclass_absent_types: &'a ahash::AHashSet<String>,
    pub(super) core_to_binding_convertible_types: &'a ahash::AHashSet<String>,
}

pub(super) fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    inputs: &VisitorBridgeInputs<'_>,
) -> anyhow::Result<String> {
    let core_crate = inputs.core_crate;
    let type_paths = inputs.type_paths;
    let api = inputs.api;
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Pyo3,
    )?;

    let binding_class = context_binding_class(
        api,
        bridge_cfg,
        inputs.pyclass_absent_types,
        inputs.core_to_binding_convertible_types,
    );
    let helper_fn = crate::backends::pyo3::template_env::render(
        "trait_bridge/nodecontext_to_py_object.jinja",
        minijinja::context! {
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
            binding_type_name => binding_class.map(|typ| typ.name.clone()),
            build_binding_type => binding_class.is_some(),
        },
    );

    let struct_def = crate::backends::pyo3::template_env::render(
        "trait_bridge/visitor_struct.jinja",
        minijinja::context! {
            struct_name => struct_name,
        },
    );

    let mut methods_code = String::new();
    let method_context = VisitorMethodContext {
        bridge_cfg,
        type_paths,
        struct_name,
        result_metadata: &result_metadata,
    };
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method(&mut methods_code, method, &method_context);
    }

    let mut out = String::with_capacity(4096);
    out.push_str(&helper_fn);
    out.push_str(&struct_def);
    out.push_str(&crate::backends::pyo3::template_env::render(
        "trait_bridge/impl_header.jinja",
        minijinja::context! { trait_path => trait_path, struct_name => struct_name },
    ));
    out.push_str(&methods_code);
    out.push_str("}\n");
    Ok(out)
}

/// The context type a visitor callback can hand the host as the generated `#[pyclass]` the `.pyi`
/// stub advertises for it, rather than as an untyped `dict`. `None` keeps the dict shape.
///
/// Four independently necessary conditions, none of them a fresh rule invented here:
/// - the `.pyi` stub declares the class at all — `gen_stubs::classes` filters `binding_excluded`
///   and `gen_stubs::protocol` additionally treats `api.excluded_type_paths` as absent, so a
///   context reached through either exclusion has no class for the bridge to construct;
/// - `impl From<core::T> for T` is actually emitted, asked through the same shared predicate the
///   type emitter itself gates on ([`crate::codegen::conversions::core_to_binding_from_impl_emitted`]
///   over the caller-supplied `core_to_binding_convertible_types` -- the same
///   `core_to_binding_convertible_types(api, &[])` set `generate_bindings` computes, now
///   precomputed once per generation loop and passed in rather than rebuilt on every bridge, since
///   the fixpoint is transitive over every type and field in `api`). Re-deriving eligibility here
///   instead would let the bridge emit `.into()` against a `From` impl that was never generated;
/// - the conversion is reachable from what the bridge holds: the generated `From` takes the core
///   value **by value** and the bridge only has `&core::T`, so `T: Clone` is required to get there;
/// - the module emits the `#[pyclass]` at all: `pyclass_absent_types` is
///   [`crate::backends::pyo3::gen_bindings::binding_exclusions::pyclass_absent_type_names`], the
///   same set the `#[pyclass]` loop filters on, so a context removed by `[crates.python]
///   exclude_types` or routed through `capsule_types` cannot be constructed here. Deriving this
///   from the IR alone is what let a config-only exclusion through: no IR flag records it, so the
///   bridge emitted `Py<Ctx>::from(..)` against a struct the emitter had skipped and the generated
///   crate did not compile.
///
/// The `.pyi` protocol stub calls this same function, so the class the stub names on a visitor
/// callback's context parameter and the object the bridge actually passes cannot diverge. ~keep
pub(crate) fn context_binding_class<'a>(
    api: &'a ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
    pyclass_absent_types: &ahash::AHashSet<String>,
    core_to_binding_convertible_types: &ahash::AHashSet<String>,
) -> Option<&'a TypeDef> {
    let context_type = bridge_cfg.context_type.as_deref()?;
    let context_def = api.types.iter().find(|type_def| type_def.name == *context_type)?;
    if context_def.binding_excluded || api.excluded_type_paths.contains_key(context_type) {
        return None;
    }
    if pyclass_absent_types.contains(context_type) {
        return None;
    }
    if !context_def.is_clone {
        return None;
    }
    crate::codegen::conversions::core_to_binding_from_impl_emitted(context_def, core_to_binding_convertible_types)
        .then_some(context_def)
}

/// The trait-bridge context shared by every generated visitor callback method: everything that
/// stays constant across the loop in [`gen_visitor_bridge`], as opposed to `out` and `method`,
/// which change on every call.
struct VisitorMethodContext<'a> {
    bridge_cfg: &'a TraitBridgeConfig,
    type_paths: &'a HashMap<String, String>,
    struct_name: &'a str,
    result_metadata: &'a crate::codegen::visitor_result::VisitorResultMetadata,
}

/// Generate a single visitor-style trait method that tries Python dispatch, falls back to default.
///
/// For each method the generated code:
/// 1. Checks if the Python object has an attribute with this method's name.
/// 2. If yes, calls the method with converted arguments and converts the Python return value
///    to the appropriate Rust return type.
/// 3. If no (attribute absent), returns the configured default result variant.
fn gen_visitor_method(out: &mut String, method: &MethodDef, context: &VisitorMethodContext<'_>) {
    use crate::core::ir::TypeRef;

    let type_paths = context.type_paths;
    let struct_name = context.struct_name;
    let result_metadata = context.result_metadata;
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths.get(n).cloned().unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let default_result_expr = crate::codegen::visitor_result::default_result_expr(&ret_ty, result_metadata);
    let VisitorPyArgs { setup, args: py_args } =
        build_visitor_py_args(method, context.bridge_cfg, struct_name, name, &default_result_expr);

    let py_call = if py_args.is_empty() {
        format!("obj.call_method0(\"{name}\")")
    } else {
        format!("obj.call_method1(\"{name}\", ({py_args}))")
    };

    let method_code = crate::backends::pyo3::template_env::render(
        "trait_bridge/visitor_method.jinja",
        minijinja::context! {
            wrapper => struct_name,
            method_name => name,
            sig => sig,
            ret_ty => ret_ty,
            arg_setup => setup,
            default_result_expr => default_result_expr,
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &ret_ty,
                result_metadata,
                "s",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            py_call => py_call,
        },
    );

    out.push_str(&method_code);
}

/// The rendered pieces of a visitor method's Python call: statements that must run before the
/// call, and the comma-joined argument expressions themselves.
struct VisitorPyArgs {
    setup: String,
    args: String,
}

/// Build Python call argument expressions for a visitor method.
///
/// - configured context params: built by `nodecontext_to_py_object` in a `setup` statement, which
///   hands the host an instance of the generated `#[pyclass]` the `.pyi` stub advertises for that
///   parameter (a plain dict only when the context type cannot be built from a borrowed core
///   value). The statement form exists so the construction's `PyErr` has somewhere to go: this
///   trait method is infallible on the Rust side, so the error is logged with its message and the
///   configured default action is returned. Building it inline in the argument tuple is what
///   forced the previous `Err(_) => py.None()`, which handed the callback a `None` typed as the
///   context class and lost the error entirely. ~keep
/// - `&str` params: passed directly (PyO3 handles `&str` → Python str coercion)
/// - `Option<&str>` params: passed as `Option<&str>` (PyO3 maps `None` → Python `None`)
/// - `bool` and integer params: passed directly
/// - `&[String]` / `Vec<String>` params: passed as Python lists
fn build_visitor_py_args(
    method: &MethodDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    method_name: &str,
    default_result_expr: &str,
) -> VisitorPyArgs {
    use crate::core::ir::TypeRef;
    let mut setup = String::new();
    let mut reserved_names: std::collections::HashSet<String> =
        method.params.iter().map(|param| param.name.clone()).collect();
    reserved_names.extend(
        ["py", "obj", "result", "s", "py_dict", "d", "action", "v", "e"]
            .into_iter()
            .map(str::to_string),
    );
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Named(n) = &p.ty
                && Some(n.as_str()) == bridge_cfg.context_type.as_deref()
            {
                let borrow_expr = if p.is_ref {
                    p.name.clone()
                } else {
                    format!("&{}", p.name)
                };
                let arg_name = collision_free_local_name(&format!("{}_py", p.name), &mut reserved_names);
                setup.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/visitor_context_arg.jinja",
                    minijinja::context! {
                        arg_name => arg_name,
                        borrow_expr => borrow_expr,
                        wrapper => struct_name,
                        method_name => method_name,
                        default_result_expr => default_result_expr,
                    },
                ));
                return arg_name;
            }
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                return p.name.clone();
            }
            if p.is_ref
                && let TypeRef::Vec(inner) = &p.ty
                && matches!(inner.as_ref(), TypeRef::String)
            {
                return p.name.clone();
            }
            if let TypeRef::Vec(inner) = &p.ty
                && matches!(inner.as_ref(), TypeRef::String)
            {
                return format!("{}.to_vec()", p.name);
            }
            if let TypeRef::Optional(inner) = &p.ty
                && matches!(inner.as_ref(), TypeRef::String)
            {
                return p.name.clone();
            }
            if matches!(&p.ty, TypeRef::String) && p.is_ref {
                return p.name.clone();
            }
            if matches!(&p.ty, TypeRef::String) {
                return format!("{}.as_str()", p.name);
            }
            p.name.clone()
        })
        .collect();
    let args = if args.len() == 1 {
        format!("{},", args[0])
    } else {
        args.join(", ")
    };
    VisitorPyArgs { setup, args }
}

fn collision_free_local_name(base: &str, reserved_names: &mut std::collections::HashSet<String>) -> String {
    if reserved_names.insert(base.to_string()) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base}_{suffix}");
        if reserved_names.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}
