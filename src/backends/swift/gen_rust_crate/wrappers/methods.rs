//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub SourceT)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_type_enum_aware_ref, needs_json_bridge, needs_json_bridge_with_handles, swift_bridge_rust_type,
};
use crate::core::ir::{ReceiverKind, TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Emit free function shims for each method on `ty`.
///
/// Each method `fn method_name(&self, param: T) -> Result<R, E>` becomes
/// `pub fn type_name_method_name(client: &TypeName, param: BridgeT) -> Result<BridgeR, String>`.
/// Async methods are blocked on a Tokio current-thread runtime (same pattern as function shims).
pub(crate) fn emit_type_method_shims(
    ty: &TypeDef,
    _source_crate: &str,
    type_paths: &HashMap<String, String>,
    handle_returned_types: &std::collections::HashSet<String>,
    unit_enum_names: &HashSet<&str>,
) -> String {
    let type_snake = ty.name.to_snake_case();
    let type_name = &ty.name;

    let mut out = String::new();

    let mut trait_uses: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for method in &ty.methods {
        if method.sanitized {
            continue;
        }
        if let Some(path) = method.trait_source.as_deref() {
            trait_uses.insert(path.to_string());
        }
    }
    for path in &trait_uses {
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_trait_use.rs.jinja",
            minijinja::context! {
                path => path,
            },
        ));
    }
    if !trait_uses.is_empty() {
        out.push('\n');
    }

    for method in &ty.methods {
        if method.sanitized {
            continue;
        }
        if method.is_static {
            continue;
        }
        let method_snake = method.name.to_snake_case();
        let fn_name = format!("{type_snake}_{method_snake}");

        let client_receiver = if matches!(method.receiver, Some(ReceiverKind::RefMut)) {
            format!("client: &mut {type_name}")
        } else {
            format!("client: &{type_name}")
        };
        let mut params_vec: Vec<String> = vec![client_receiver];
        for p in &method.params {
            let bridge_ty = bridge_type_enum_aware_ref(&p.ty, unit_enum_names);
            let bridge_ty = if p.optional && !needs_json_bridge(&p.ty) {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            params_vec.push(format!("{name}: {bridge_ty}"));
        }
        let params_str = params_vec.join(", ");

        let return_ty = if method.error_type.is_some() {
            let ok_ty = crate::backends::swift::gen_rust_crate::type_bridge::bridge_type_with_handles(
                &method.return_type,
                handle_returned_types,
            );
            if matches!(method.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            crate::backends::swift::gen_rust_crate::type_bridge::bridge_type_with_handles(
                &method.return_type,
                handle_returned_types,
            )
        };

        let call_args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                if matches!(&p.ty, TypeRef::Json) {
                    return format!(
                        "serde_json::from_str::<serde_json::Value>(&{name}).unwrap_or(serde_json::Value::Null)"
                    );
                }
                if let TypeRef::Vec(vec_inner) = &p.ty {
                    if let TypeRef::Named(n) = vec_inner.as_ref() {
                        if unit_enum_names.contains(n.as_str()) {
                            let source_enum_ty = type_paths
                                .get(n.as_str())
                                .map(|p| p.replace('-', "_"))
                                .unwrap_or_else(|| n.clone());
                            let map_expr = format!(
                                concat!(
                                    "{name}.into_iter().map(|s| ",
                                    "<{source_enum_ty} as ::std::convert::From<String>>::from(s))",
                                    ".collect::<Vec<_>>()"
                                ),
                                name = name,
                                source_enum_ty = source_enum_ty,
                            );
                            if p.is_ref {
                                return format!("&{map_expr}");
                            }
                            if p.optional {
                                let opt_map = format!(
                                    concat!(
                                        "{name}.map(|values| values.into_iter().map(|s| ",
                                        "<{source_enum_ty} as ::std::convert::From<String>>::from(s))",
                                        ".collect::<Vec<_>>())"
                                    ),
                                    name = name,
                                    source_enum_ty = source_enum_ty,
                                );
                                return opt_map;
                            }
                            return map_expr;
                        }
                    }
                }
                if let TypeRef::Named(n) = &p.ty {
                    if unit_enum_names.contains(n.as_str()) {
                        let source_enum_ty = type_paths
                            .get(n.as_str())
                            .map(|p| p.replace('-', "_"))
                            .unwrap_or_else(|| n.clone());
                        let from_expr = format!("<{source_enum_ty} as ::std::convert::From<String>>::from({name})");
                        if p.optional {
                            return format!(
                                "{name}.map(|s| <{source_enum_ty} as ::std::convert::From<String>>::from(s))"
                            );
                        }
                        if p.is_ref {
                            return format!("&{from_expr}");
                        }
                        return from_expr;
                    }
                }
                if needs_json_bridge(&p.ty) {
                    let native_ty = swift_bridge_rust_type(&p.ty);
                    return format!("serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
                }
                if p.optional {
                    if let TypeRef::Named(n) = &p.ty {
                        if !unit_enum_names.contains(n.as_str()) {
                            return format!("{name}.map(|v| v.0)");
                        }
                    }
                }
                match &p.ty {
                    TypeRef::Named(n) if p.is_ref && !unit_enum_names.contains(n.as_str()) => format!("&{name}.0"),
                    TypeRef::Named(n) if p.is_ref && unit_enum_names.contains(n.as_str()) => format!("&{name}"),
                    TypeRef::Named(n) if !unit_enum_names.contains(n.as_str()) => format!("{name}.0"),
                    TypeRef::Named(n) if unit_enum_names.contains(n.as_str()) => name,
                    TypeRef::String => format!("&{name}"),
                    TypeRef::Path if p.optional && p.is_ref => {
                        format!("{name}.as_ref().map(::std::path::Path::new)")
                    }
                    TypeRef::Path if p.optional => format!("{name}.map(::std::path::PathBuf::from)"),
                    TypeRef::Path if p.is_ref => format!("::std::path::Path::new(&{name})"),
                    TypeRef::Path => format!("::std::path::PathBuf::from({name})"),
                    TypeRef::Bytes if p.is_ref => format!("&{name}"),
                    TypeRef::Vec(_)
                        if p.is_ref
                            && p.vec_inner_is_ref
                            && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String)) =>
                    {
                        format!("&{name}.iter().map(|s| s.as_str()).collect::<Vec<_>>()")
                    }
                    TypeRef::Vec(_) if p.is_ref => {
                        format!("&{name}")
                    }
                    _ => name,
                }
            })
            .collect();
        let call_args_str = call_args.join(", ");

        let is_owned_receiver = matches!(method.receiver.as_ref(), Some(ReceiverKind::Owned));
        let inner_access = if is_owned_receiver {
            "client.0.clone()"
        } else {
            "client.0"
        };
        let method_call = format!("{inner_access}.{method_snake}({call_args_str})");

        let json_wrap_ok = needs_json_bridge_with_handles(&method.return_type, handle_returned_types);
        let wrap_return = |source: String| -> String {
            if json_wrap_ok {
                return format!("serde_json::to_string(&({source})).expect(\"serializable return\")");
            }
            match &method.return_type {
                TypeRef::Named(t) => format!("{t}({source})"),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(t) = inner.as_ref() {
                        if method.returns_ref {
                            format!("({source}).map(|v| {t}(v.clone()))")
                        } else {
                            format!("({source}).map({t})")
                        }
                    } else {
                        source
                    }
                }
                TypeRef::Vec(inner) if method.returns_ref && matches!(inner.as_ref(), TypeRef::String) => {
                    format!("{source}.iter().map(|s| s.to_string()).collect()")
                }
                TypeRef::Vec(inner) => {
                    if let TypeRef::Named(t) = inner.as_ref() {
                        if method.returns_ref {
                            format!("({source}).iter().map(|v| {t}(v.clone())).collect()")
                        } else {
                            format!("({source}).into_iter().map({t}).collect()")
                        }
                    } else {
                        source
                    }
                }
                TypeRef::String => format!("{source}.to_string()"),
                TypeRef::Path => format!("{source}.to_string_lossy().into_owned()"),
                _ => source,
            }
        };

        let body = if method.is_async {
            let chain = if method.error_type.is_some() {
                let ok_wrap = if json_wrap_ok {
                    ".map(|v| serde_json::to_string(&v).expect(\"serializable return\"))".to_string()
                } else {
                    match &method.return_type {
                        TypeRef::Named(t) => format!(".map({t})"),
                        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                            if let TypeRef::Named(t) = inner.as_ref() {
                                format!(".map(|vec| vec.into_iter().map({t}).collect())")
                            } else {
                                String::new()
                            }
                        }
                        TypeRef::String | TypeRef::Path => ".map(|s| s.to_string())".to_string(),
                        TypeRef::Bytes => ".map(|b| b.to_vec())".to_string(),
                        _ => String::new(),
                    }
                };
                format!("{method_call}.await.map_err(|e| e.to_string()){ok_wrap}")
            } else {
                wrap_return(format!("{method_call}.await"))
            };
            format!("    crate::__alef_tokio_runtime().block_on(async {{ {chain} }})")
        } else if method.error_type.is_some() {
            let ok_wrap = if json_wrap_ok {
                ".map(|v| serde_json::to_string(&v).expect(\"serializable return\"))".to_string()
            } else {
                match &method.return_type {
                    TypeRef::Named(t) => format!(".map({t})"),
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                        if let TypeRef::Named(t) = inner.as_ref() {
                            format!(".map(|vec| vec.into_iter().map({t}).collect())")
                        } else {
                            String::new()
                        }
                    }
                    TypeRef::String | TypeRef::Path => ".map(|s| s.to_string())".to_string(),
                    TypeRef::Bytes => ".map(|b| b.to_vec())".to_string(),
                    _ => String::new(),
                }
            };
            format!("    {method_call}.map_err(|e| e.to_string()){ok_wrap}")
        } else {
            format!("    {}", wrap_return(method_call))
        };

        let return_clause = if return_ty == "()" {
            String::new()
        } else {
            format!(" -> {return_ty}")
        };
        if let Some(cfg) = ty.cfg.as_deref() {
            out.push_str(&format!("#[cfg({cfg})]\n"));
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_wrapper_free_fn.rs.jinja",
            minijinja::context! {
                fn_name => fn_name,
                params => params_str,
                return_clause => return_clause,
                body => body,
            },
        ));
    }

    out
}

/// Emit wrapper functions for instance methods on first-class (non-opaque) DTOs.
///
/// These wrappers handle JSON marshaling since swift-bridge cannot directly bridge
/// instance methods on value types. Each wrapper:
/// 1. Deserializes the JSON string of `self`
/// 2. Calls the actual method on the deserialized value
/// 3. Serializes the result back to JSON
/// 4. Returns Result<String, String> (JSON result or error)
pub(crate) fn emit_first_class_dto_method_wrappers(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    _unit_enum_names: &HashSet<&str>,
) -> String {
    if ty.is_opaque {
        return String::new();
    }

    let instance_methods: Vec<_> = ty.methods.iter().filter(|m| !m.sanitized && !m.is_static).collect();
    if instance_methods.is_empty() {
        return String::new();
    }

    let type_name = &ty.name;
    let type_snake = type_name.to_snake_case();
    let core_ty = type_paths
        .get(type_name.as_str())
        .map(|p| p.replace('-', "_"))
        .unwrap_or_else(|| format!("{source_crate}::{type_name}"));
    let mut out = String::new();

    for method in instance_methods {
        let method_snake = method.name.to_snake_case();
        let fn_name = format!("{type_snake}_{method_snake}_from_json");

        let mut params = vec!["json: String".to_string()];
        for p in &method.params {
            let base_ty = match &p.ty {
                TypeRef::Primitive(prim) => format!("{:?}", prim).to_lowercase(),
                TypeRef::String => "String".to_string(),
                TypeRef::Named(n) => n.clone(),
                _ => "String".to_string(),
            };
            // `#[swift_bridge::bridge]` declaration and this impl stay in agreement.
            let ty_str = if p.optional && !needs_json_bridge(&p.ty) {
                format!("Option<{base_ty}>")
            } else {
                base_ty
            };
            let name = p.name.to_snake_case();
            params.push(format!("{name}: {ty_str}"));
        }

        out.push_str(&format!("pub fn {fn_name}("));
        out.push_str(&params.join(", "));
        out.push_str(") -> Result<String, String> {\n");

        let self_binding = if matches!(method.receiver, Some(ReceiverKind::RefMut)) {
            "let mut __self"
        } else {
            "let __self"
        };
        out.push_str(&format!(
            "    {self_binding}: {core_ty} = serde_json::from_str(&json)\n"
        ));
        out.push_str(&format!(
            "        .map_err(|e| format!(\"Failed to deserialize {type_name}: {{}}\", e))?;\n"
        ));

        let method_call_args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                match &p.ty {
                    TypeRef::Path if p.optional && p.is_ref => {
                        format!("{name}.as_ref().map(::std::path::Path::new)")
                    }
                    TypeRef::Path if p.optional => format!("{name}.map(::std::path::PathBuf::from)"),
                    TypeRef::Path if p.is_ref => format!("::std::path::Path::new(&{name})"),
                    TypeRef::Path => format!("::std::path::PathBuf::from({name})"),
                    TypeRef::String if p.optional && p.is_ref => format!("{name}.as_deref()"),
                    TypeRef::String if p.is_ref => format!("&{name}"),
                    TypeRef::Named(_) if p.optional && p.is_ref => format!("{name}.as_ref()"),
                    TypeRef::Named(_) if p.is_ref => format!("&{name}"),
                    _ => name,
                }
            })
            .collect();
        let __call = format!("__self.{}({})", method.name, method_call_args.join(", "));

        if method.error_type.is_some() {
            out.push_str(&format!("    let __result = {__call};\n"));
            if matches!(method.return_type, TypeRef::Unit) {
                // (`let __value = ...` would trip clippy::let_unit_value).
                out.push_str("    __result.map_err(|e| e.to_string())?;\n");
                out.push_str("    Ok(\"{}\".to_string())\n");
            } else {
                out.push_str("    let __value = __result.map_err(|e| e.to_string())?;\n");
                out.push_str("    serde_json::to_string(&__value)\n");
                out.push_str("        .map_err(|e| format!(\"Failed to serialize result: {}\", e))\n");
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            // value to `let __result` would trip clippy::let_unit_value).
            out.push_str(&format!("    {__call};\n"));
            out.push_str("    Ok(\"{}\".to_string())\n");
        } else {
            out.push_str(&format!("    let __result = {__call};\n"));
            out.push_str("    serde_json::to_string(&__result)\n");
            out.push_str("        .map_err(|e| format!(\"Failed to serialize result: {}\", e))\n");
        }

        out.push_str("}\n\n");
    }

    out
}
