use super::args::gen_rustler_method_call_args;
use super::default_deserialization::build_default_deser_preamble;
use super::shared::{render_async_body, render_method_call};
use crate::backends::rustler::gen_bindings::types::gen_rustler_wrap_return;
use crate::backends::rustler::template_env;
use crate::backends::rustler::type_map::RustlerMapper;
use crate::codegen::doc_emission;
use crate::codegen::shared;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Generate a Rustler NIF async method for a struct (sync wrapper scheduled on DirtyCpu).
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::rustler::gen_bindings) fn gen_nif_async_method(
    struct_name: &str,
    core_path: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> String {
    let method_fn_name = format!("{}_{}_async", struct_name.to_lowercase(), method.name);

    let mut params = if method.receiver.is_some() {
        if is_opaque {
            vec![format!("resource: rustler::ResourceArc<{}>", struct_name)]
        } else {
            vec![format!("obj: {}", struct_name)]
        }
    } else {
        vec![]
    };

    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                params.push(format!("{}: rustler::ResourceArc<{}>", p.name, n));
                continue;
            }
            // Default (has_default) types are passed as JSON strings so partial maps work.
            if default_types.contains(n) {
                params.push(format!("{}: Option<String>", p.name));
                continue;
            }
            // Optional Named non-opaque params must be Option<T> so callers can
            // pass nil (Elixir) and the NIF receives None rather than a decode error.
            if p.optional {
                params.push(format!("{}: Option<{}>", p.name, n));
                continue;
            }
        }
        let param_type = mapper.map_type(&p.ty);
        if p.optional {
            params.push(format!("{}: Option<{}>", p.name, param_type));
        } else {
            params.push(format!("{}: {}", p.name, param_type));
        }
    }

    let return_type =
        crate::backends::rustler::gen_bindings::helpers::map_return_type(&method.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // method itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let has_default_params = method
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));
    // Rustler stores opaque resources behind `Arc<RwLock<T>>`, so `&mut self` methods
    // mutate in place through a write lock (see the receiver match below). The shared
    // `can_auto_delegate` conservatively rejects RefMut opaque methods (correct for the
    // Arc<T>-only backends), so re-permit them here when their params/return are delegatable.
    let can_delegate_refmut_opaque = is_opaque
        && matches!(method.receiver, Some(ReceiverKind::RefMut))
        && method.trait_source.is_none()
        && !method.sanitized
        && method.params.iter().all(|p| {
            !p.sanitized
                && shared::is_delegatable_param(&p.ty, opaque_types)
                && !shared::is_named_ref_param_pub(p, opaque_types)
        })
        && shared::is_delegatable_return(&method.return_type);
    let can_delegate =
        shared::can_auto_delegate(method, opaque_types) || has_default_params || can_delegate_refmut_opaque;

    // Build deserialization preamble for default-typed (JSON-string) params.
    let deser_preamble = build_default_deser_preamble(
        &method.params,
        default_types,
        core_import,
        method.error_type.is_some(),
        types_by_name,
    );

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types, default_types);
        let core_call = if let (true, Some(receiver)) = (is_opaque, method.receiver.as_ref()) {
            // For &self: Arc<T> derefs to T, no clone needed (and avoids the
            // noop_method_call lint that the previous as_ref().clone() tripped).
            // For &mut self / self: clone the inner T to get an owned value the
            // method can consume — requires T: Clone (callers needing non-Clone
            // opaque types with mutating methods should configure exclude_methods).
            match receiver {
                ReceiverKind::Ref => format!(
                    "resource.inner.read().unwrap_or_else(|e| e.into_inner()).{}({})",
                    method.name, call_args
                ),
                ReceiverKind::RefMut => {
                    format!(
                        "resource.inner.write().unwrap_or_else(|e| e.into_inner()).{}({})",
                        method.name, call_args
                    )
                }
                ReceiverKind::Owned => {
                    format!(
                        "resource.inner.read().unwrap_or_else(|e| e.into_inner()).clone().{}({})",
                        method.name, call_args
                    )
                }
            }
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            render_method_call("rust_method_static_call.rs.jinja", core_path, &method.name, &call_args)
        } else if method.receiver.is_some() {
            render_method_call(
                "rust_method_instance_call.rs.jinja",
                core_path,
                &method.name,
                &call_args,
            )
        } else {
            // Static method on non-opaque: call directly on core type
            render_method_call("rust_method_static_call.rs.jinja", core_path, &method.name, &call_args)
        };
        // When the IR's return type was sanitized from a Named type to TypeRef::String
        // (because the original type is excluded from the binding API), the core call
        // still returns the original Named type — JSON-serialize it to satisfy the
        // String return type declared by the NIF.
        let return_was_sanitized = method.sanitized && matches!(&method.return_type, TypeRef::String);
        let result_wrap = if return_was_sanitized {
            "serde_json::to_string(&result).map_err(|e| e.to_string())?".to_string()
        } else {
            gen_rustler_wrap_return(
                "result",
                &method.return_type,
                struct_name,
                opaque_types,
                method.returns_ref,
            )
        };
        if method.error_type.is_some() {
            render_async_body("async_result_body.rs.jinja", &deser_preamble, &core_call, &result_wrap)
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            render_async_body(
                "async_infallible_body.rs.jinja",
                &deser_preamble,
                &core_call,
                &result_wrap,
            )
        }
    } else {
        let adapter_key = format!("{struct_name}.{}", method.name);
        if let Some(body) = adapter_bodies.get(&adapter_key) {
            body.clone()
        } else {
            crate::backends::rustler::gen_bindings::helpers::gen_rustler_unimplemented_body(
                &method.return_type,
                &method_fn_name,
                true,
            )
        }
    };
    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &method.doc, "");
    out.push_str(&template_env::render(
        "dirty_cpu_nif_function.rs.jinja",
        minijinja::context! {
            func_name => &method_fn_name,
            params_str => &params.join(", "),
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}
