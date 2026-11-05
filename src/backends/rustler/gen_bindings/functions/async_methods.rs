use super::args::gen_rustler_method_call_args;
use super::default_deserialization::{build_default_deser_preamble, render_json_string_param};
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
        if matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name)))
        {
            params.push(render_json_string_param(&p.name));
            continue;
        }
        if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                params.push(format!("{}: rustler::ResourceArc<{}>", p.name, n));
                continue;
            }
            if default_types.contains(n) {
                params.push(format!("{}: Option<String>", p.name));
                continue;
            }
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
    let return_annotation = mapper.wrap_nif_return(&return_type, true);

    let has_default_params = method
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));
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

    let mut deser_preamble = build_default_deser_preamble(
        &method.params,
        opaque_types,
        default_types,
        core_import,
        &method_fn_name,
        types_by_name,
    );

    let mut guard_held_across_await = false;
    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types, default_types);
        let core_call = if let (true, Some(receiver)) = (is_opaque, method.receiver.as_ref()) {
            let cloneable = types_by_name.get(struct_name).is_some_and(|typ| typ.is_clone);
            match receiver {
                // Clone the handle out of the guard before `block_on` runs the future. Holding a
                // `std::sync::RwLock` read guard across `.await` trips `clippy::await_holding_lock`
                // in every consumer that lints the generated NIF, and blocks a writer for the whole
                // request. The core type must be `Clone`; when it is not there is no way to keep a
                // `&self` future alive without the guard, so the old shape stays and the lint is
                // allowed on that function alone. ~keep
                ReceiverKind::Ref if cloneable => {
                    deser_preamble.push_str(
                        "let __alef_inner = resource.inner.read().unwrap_or_else(|e| e.into_inner()).clone();\n",
                    );
                    format!("__alef_inner.{}({})", method.name, call_args)
                }
                ReceiverKind::Ref => {
                    guard_held_across_await = true;
                    format!(
                        "resource.inner.read().unwrap_or_else(|e| e.into_inner()).{}({})",
                        method.name, call_args
                    )
                }
                // A `&mut self` future mutates through the write guard; cloning the value out
                // would silently discard those mutations, so the guard is held and the lint is
                // allowed on this function alone. ~keep
                ReceiverKind::RefMut => {
                    guard_held_across_await = true;
                    format!(
                        "resource.inner.write().unwrap_or_else(|e| e.into_inner()).{}({})",
                        method.name, call_args
                    )
                }
                ReceiverKind::Owned if cloneable => {
                    deser_preamble.push_str(
                        "let __alef_inner = resource.inner.read().unwrap_or_else(|e| e.into_inner()).clone();\n",
                    );
                    format!("__alef_inner.{}({})", method.name, call_args)
                }
                ReceiverKind::Owned => {
                    guard_held_across_await = true;
                    format!(
                        "resource.inner.read().unwrap_or_else(|e| e.into_inner()).clone().{}({})",
                        method.name, call_args
                    )
                }
            }
        } else if is_opaque {
            render_method_call("rust_method_static_call.rs.jinja", core_path, &method.name, &call_args)
        } else if method.receiver.is_some() {
            render_method_call(
                "rust_method_instance_call.rs.jinja",
                core_path,
                &method.name,
                &call_args,
            )
        } else {
            render_method_call("rust_method_static_call.rs.jinja", core_path, &method.name, &call_args)
        };
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
    let function_template = if guard_held_across_await {
        tracing::warn!(
            "[{}::{}] holds a std::sync lock across .await: the core type is not Clone or the method needs &mut self, \
             so the lock cannot be released before the future runs (clippy::await_holding_lock is allowed here)",
            struct_name,
            method.name
        );
        "dirty_cpu_nif_function_allow_lock.rs.jinja"
    } else {
        "dirty_cpu_nif_function.rs.jinja"
    };
    out.push_str(&template_env::render(
        function_template,
        crate::alef_context! {
            func_name => &method_fn_name,
            params_str => &params.join(", "),
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}
