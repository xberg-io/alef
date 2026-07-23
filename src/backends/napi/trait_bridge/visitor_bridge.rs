use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, to_camel_case, visitor_param_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

pub(super) fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> anyhow::Result<String> {
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Napi,
    )?;
    let mut method_impls = String::with_capacity(4096);
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method_napi(
            &mut method_impls,
            method,
            trait_path,
            core_crate,
            bridge_cfg,
            type_paths,
            &result_metadata,
        );
    }

    Ok(crate::backends::napi::template_env::render(
        "visitor_bridge.jinja",
        minijinja::context! {
            core_crate => core_crate,
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
            struct_name => struct_name,
            trait_path => trait_path,
            method_impls => method_impls,
        },
    ))
}

/// Build the Function args tuple type string for a given number of Unknown args.
pub(super) fn unknown_tuple_type(count: usize) -> String {
    if count == 0 {
        return "()".to_string();
    }
    let parts = vec!["napi::bindgen_prelude::Unknown"; count];
    format!("({}{})", parts.join(", "), if count == 1 { "," } else { "" })
}

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
fn gen_visitor_method_napi(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    _core_crate: &str,
    bridge_cfg: &TraitBridgeConfig,
    type_paths: &HashMap<String, String>,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) {
    let name = &method.name;
    let js_method_name = to_camel_case(name);

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let signature = sig_parts.join(", ");

    let return_type = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let arg_count = method.params.len();
    let empty_args = arg_count == 0;
    let inner_tuple_ty = unknown_tuple_type(arg_count);
    let args_tuple_ty = if empty_args {
        inner_tuple_ty
    } else {
        format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
    };

    let js_args_exprs = build_napi_args(method, bridge_cfg, &std::collections::HashSet::new(), "Js");
    let arg_exprs: Vec<String> = js_args_exprs
        .iter()
        .map(|expr| expr.replace("self.env()", "__env"))
        .collect();

    let tuple_args = if arg_count == 1 {
        "(arg_0,)".to_string()
    } else if arg_count > 0 {
        let arg_names: Vec<String> = (0..arg_count).map(|i| format!("arg_{i}")).collect();
        format!("({})", arg_names.join(", "))
    } else {
        String::new()
    };

    out.push_str(&crate::backends::napi::template_env::render(
        "visitor_method.jinja",
        minijinja::context! {
            method_name => name,
            js_method_name => js_method_name,
            signature => signature,
            return_type => return_type,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&return_type, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &return_type,
                result_metadata,
                "s",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            empty_args => empty_args,
            arg_exprs => arg_exprs,
            tuple_args => tuple_args,
            args_tuple_ty => args_tuple_ty,
        },
    ));
}

/// Returns true if a napi value for `ty` can be produced directly via `ToNapiValue`
/// (Rust -> JS), recursing into `Vec`. Used to decide when a trait-bridge callback
/// argument can be passed as a native JS value instead of a Debug-string encoded one.
pub(super) fn is_napi_encodable(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String => true,
        TypeRef::Primitive(_) => true,
        TypeRef::Vec(inner) => is_napi_encodable(inner),
        _ => false,
    }
}

/// Returns true if a napi value for `ty` can be decoded directly via `FromNapiValue`
/// (JS -> Rust), recursing into `Vec`. Unlike [`is_napi_encodable`], `f32` is excluded:
/// napi-rs implements `ToNapiValue` for `f32` but not `FromNapiValue` (JS numbers decode
/// as `f64`), so a `Vec<f32>` / `Vec<Vec<f32>>` return type cannot be decoded natively.
pub(super) fn is_napi_decodable(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String => true,
        TypeRef::Primitive(p) => !matches!(p, crate::core::ir::PrimitiveType::F32),
        TypeRef::Vec(inner) => is_napi_decodable(inner),
        _ => false,
    }
}

/// Returns the "f64 analog" of `ty` if `ty` is a (possibly `Vec`-nested) `f32` — i.e. the
/// only reason [`is_napi_decodable`] rejects it is the `f32` leaf. napi-rs has no
/// `FromNapiValue for f32`, but every JS number already round-trips losslessly through
/// `f64` (which does implement `FromNapiValue`), so such a return type can still decode
/// natively: decode into this f64 analog, then cast element-wise back to `f32` (see
/// [`f32_bridge_cast_expr`]).
///
/// Returns `None` for anything else: already-decodable types (no bridging needed) and
/// genuinely non-native leaves (`Named`/`Bytes`/`Map`/...), which keep the JSON fallback.
pub(super) fn f32_bridge_target(ty: &TypeRef) -> Option<TypeRef> {
    match ty {
        TypeRef::Primitive(crate::core::ir::PrimitiveType::F32) => {
            Some(TypeRef::Primitive(crate::core::ir::PrimitiveType::F64))
        }
        TypeRef::Vec(inner) => f32_bridge_target(inner).map(|t| TypeRef::Vec(Box::new(t))),
        _ => None,
    }
}

/// Build the Rust expression that element-wise casts a decoded f64-analog value bound to
/// `var` back into the original `f32`-leaved shape described by `ty`. Mirrors `ty`'s `Vec`
/// nesting with `.into_iter().map(|v| ...).collect()`, bottoming out in `as f32`.
pub(super) fn f32_bridge_cast_expr(ty: &TypeRef, var: &str) -> String {
    match ty {
        TypeRef::Vec(inner) => {
            let inner_expr = f32_bridge_cast_expr(inner, "v");
            format!("{var}.into_iter().map(|v| {inner_expr}).collect()")
        }
        _ => format!("{var} as f32"),
    }
}

/// Build NAPI argument expressions for a visitor method.
///
/// Returns one expression per parameter, each producing a `napi::bindgen_prelude::Unknown`.
pub(super) fn build_napi_args(
    method: &MethodDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_param_types: &std::collections::HashSet<String>,
    type_prefix: &str,
) -> Vec<String> {
    method
        .params
        .iter()

        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
                    return crate::backends::napi::template_env::render(
                        "visitor_context_arg_expr.jinja",
                        minijinja::context! { ref_prefix => if p.is_ref { "" } else { "&" }, name => p.name.as_str() },
                    )
                    .trim_end()
                    .to_string();
                }
                if struct_param_types.contains(n.as_str()) {
                    let owned = if p.is_ref {
                        format!("(*{}).clone()", p.name)
                    } else {
                        p.name.clone()
                    };
                    return format!(
                        "unsafe {{ \
                         let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {prefix}{ty}::from({owned})).unwrap_or(std::ptr::null_mut()); \
                         napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}",
                        prefix = type_prefix,
                        ty = n,
                    );
                }
            }
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                return format!(
                    "match {name} {{ \
                     Some(s) => match self.env().create_string(s) {{ \
                       Ok(v) => v.to_unknown(), \
                       Err(_) => unsafe {{ \
                       let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                       napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                     }}, \
                     None => unsafe {{ \
                       let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                       napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::String) && p.is_ref {
                return format!(
                    "match self.env().create_string({name}) {{ \
                     Ok(s) => s.to_unknown(), \
                     Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::String) {
                return format!(
                    "match self.env().create_string({name}.as_str()) {{ \
                     Ok(s) => s.to_unknown(), \
                     Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
                return format!(
                    "unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {name}).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)) {
                return format!(
                    "match self.env().create_uint32({name}) {{ Ok(n) => n.to_unknown(), Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize)) {
                return format!(
                    "match self.env().create_uint32({name} as u32) {{ Ok(n) => n.to_unknown(), Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if let TypeRef::Vec(inner) = &p.ty {
                if is_napi_encodable(inner) {
                    let owned = if p.is_ref { format!("{}.to_vec()", p.name) } else { format!("{}.clone()", p.name) };
                    return format!(
                        "unsafe {{ \
                         let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {owned}).unwrap_or(std::ptr::null_mut()); \
                         napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}",
                    );
                }
            }

            format!(
                "match self.env().create_string(&format!(\"{{:?}}\", {name})) {{ Ok(s) => s.to_unknown(), Err(_) => unsafe {{ \
                 let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                 napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                }}",
                name = p.name
            )
        })
        .collect()
}
