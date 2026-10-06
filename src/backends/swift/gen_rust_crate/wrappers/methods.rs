//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub SourceT)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_result_ok_type_with_handles, bridge_type_enum_aware_ref, enum_from_string_fn_name,
    forces_fallible_enum_bridge, needs_json_bridge, needs_json_bridge_with_handles, swift_bridge_rust_type,
};
use crate::codegen::conversions::helpers::{apply_param_newtype_to_core, is_explicit_newtype};
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::core::ir::{PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Emit free function shims for each method on `ty`.
///
/// Each method `fn method_name(&self, param: T) -> Result<R, E>` becomes
/// `pub fn type_name_method_name(client: &TypeName, param: BridgeT) -> Result<BridgeR, String>`.
/// Async methods are emitted as `async fn`s that spawn the work onto the process-wide Tokio
/// runtime (same pattern as function shims).
///
/// The two enum sets are NOT interchangeable and both are required, because the paired
/// `extern "Rust"` declaration (`extern_block::emit_extern_block_for_type_methods`) consults each
/// for a different decision and the two signatures must agree exactly or swift-bridge reports
/// `E0308`: `enum_names` (ALL enums) decides the bridge type — every enum crosses as `String` —
/// while `unit_enum_names` (fieldless variants only) decides both the fallible reverse conversion
/// (`enum_from_string_fn_name`, a helper `enums.rs` emits for unit enums only) and, through
/// `forces_fallible_enum_bridge`, whether the wrapper's return type is forced to `Result`. ~keep
pub(crate) fn emit_type_method_shims(
    ty: &TypeDef,
    _source_crate: &str,
    type_paths: &HashMap<String, String>,
    handle_returned_types: &std::collections::HashSet<String>,
    enum_names: &HashSet<&str>,
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
            let bridge_ty = bridge_type_enum_aware_ref(&p.ty, enum_names);
            let bridge_ty = if p.optional && !needs_json_bridge(&p.ty) {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            params_vec.push(format!("{name}: {bridge_ty}"));
        }
        let params_str = params_vec.join(", ");

        // An unrecognised wire string used to `panic!` inside the enum's `_from_swift_string`
        // helper, unwinding across the FFI boundary. That helper now returns `Result<_, String>`,
        // so this wrapper is forced fallible to give the `?` somewhere to go. The paired extern
        // declaration calls the same helper -- see `forces_fallible_enum_bridge`. ~keep
        let forced_fallible =
            method.is_async || forces_fallible_enum_bridge(&method.params, method.error_type.as_ref(), unit_enum_names);

        let return_ty = if method.error_type.is_some() || forced_fallible {
            let ok_ty = bridge_result_ok_type_with_handles(&method.return_type, handle_returned_types);
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

        let mut pre_call_bindings: Vec<String> = Vec::new();
        let source_type = |type_name: &str| {
            type_paths
                .get(type_name)
                .cloned()
                .unwrap_or_else(|| type_name.to_string())
                .replace('-', "_")
        };
        let call_args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                if p
                    .newtype_wrapper
                    .as_deref()
                    .is_some_and(is_explicit_newtype)
                {
                    let binding_expr = if needs_json_bridge(&p.ty) {
                        let binding_ty = swift_bridge_rust_type(&p.ty);
                        let binding_ty = if p.optional {
                            format!("Option<{binding_ty}>")
                        } else {
                            binding_ty
                        };
                        format!(
                            "::serde_json::from_str::<{binding_ty}>(&{name}).expect(\"valid JSON for {name}\")"
                        )
                    } else {
                        name.clone()
                    };
                    let converted = apply_param_newtype_to_core(&binding_expr, p)
                        .expect("explicit newtype metadata checked above");
                    if p.is_ref {
                        let bound = format!("__{name}_newtype");
                        let mutability = if p.is_mut { "mut " } else { "" };
                        pre_call_bindings.push(format!("    let {mutability}{bound} = {converted};"));
                        if p.optional {
                            return if p.is_mut {
                                format!("{bound}.as_mut()")
                            } else {
                                format!("{bound}.as_ref()")
                            };
                        }
                        return if p.is_mut {
                            format!("&mut {bound}")
                        } else {
                            format!("&{bound}")
                        };
                    }
                    return converted;
                }
                if matches!(&p.ty, TypeRef::Json) {
                    return format!(
                        "serde_json::from_str::<serde_json::Value>(&{name}).unwrap_or(serde_json::Value::Null)"
                    );
                }
                if let TypeRef::Vec(vec_inner) = &p.ty
                    && let TypeRef::Named(n) = vec_inner.as_ref()
                    && unit_enum_names.contains(n.as_str())
                {
                    let fn_name = enum_from_string_fn_name(n);
                    let bound = format!("__{name}_vec_enum");
                    let collect_expr = format!("{name}.into_iter().map(|s| {fn_name}(&s)).collect::<Result<Vec<_>, String>>()");
                    if p.optional {
                        pre_call_bindings.push(format!(
                            "    let {bound} = {name}.map(|values| values.into_iter().map(|s| {fn_name}(&s)).collect::<Result<Vec<_>, String>>()).transpose()?;"
                        ));
                    } else {
                        pre_call_bindings.push(format!("    let {bound} = {collect_expr}?;"));
                    }
                    if p.is_ref {
                        return format!("&{bound}");
                    }
                    return bound;
                }
                // A data-carrying enum has no `__alef_{enum}_from_swift_string` helper to call --
                // `enums.rs` emits that reconstruction only for fieldless enums, since a
                // discriminant-only wire string carries no field data. Such a parameter crosses as
                // JSON instead, exactly as `shims::swift_call_arg` already routes a free
                // function's tagged-enum parameter. ~keep
                if let TypeRef::Vec(vec_inner) = &p.ty
                    && let TypeRef::Named(n) = vec_inner.as_ref()
                    && enum_names.contains(n.as_str())
                {
                    let native_ty = source_type(n);
                    let map_expr = format!(
                        "values.into_iter().map(|s| ::serde_json::from_str::<{native_ty}>(&s).expect(\"valid JSON for {name} element\")).collect::<Vec<_>>()"
                    );
                    if p.optional {
                        return format!("{name}.map(|values| {map_expr})");
                    }
                    if p.is_ref {
                        return format!("&{{ let values = {name}; {map_expr} }}");
                    }
                    return format!("{{ let values = {name}; {map_expr} }}");
                }
                if let TypeRef::Named(n) = &p.ty
                    && enum_names.contains(n.as_str())
                    && !unit_enum_names.contains(n.as_str())
                {
                    let native_ty = source_type(n);
                    let deserialize = |value: &str| {
                        format!("::serde_json::from_str::<{native_ty}>({value}).expect(\"valid JSON for {name}\")")
                    };
                    if p.optional {
                        let converted = format!("{name}.as_ref().map(|value| {})", deserialize("value"));
                        if p.is_ref {
                            return if p.is_mut {
                                format!("{converted}.as_mut()")
                            } else {
                                format!("{converted}.as_ref()")
                            };
                        }
                        return converted;
                    }
                    let converted = deserialize(&format!("&{name}"));
                    if p.is_ref {
                        if p.is_mut {
                            return format!("&mut {converted}");
                        }
                        return format!("&{converted}");
                    }
                    return converted;
                }
                if let TypeRef::Named(n) = &p.ty
                    && unit_enum_names.contains(n.as_str())
                {
                    let fn_name = enum_from_string_fn_name(n);
                    let bound = format!("__{name}_enum");
                    if p.optional {
                        pre_call_bindings.push(format!("    let {bound} = {name}.map(|s| {fn_name}(&s)).transpose()?;"));
                    } else {
                        pre_call_bindings.push(format!("    let {bound} = {fn_name}(&{name})?;"));
                    }
                    if p.is_ref {
                        return format!("&{bound}");
                    }
                    return bound;
                }
                if needs_json_bridge(&p.ty) {
                    let native_ty = swift_bridge_rust_type(&p.ty);
                    return format!("serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
                }
                if p.optional
                    && let TypeRef::Named(n) = &p.ty
                    && !unit_enum_names.contains(n.as_str())
                {
                    return format!("{name}.map(|v| v.0)");
                }
                match &p.ty {
                    TypeRef::Named(n) if p.is_ref && !unit_enum_names.contains(n.as_str()) => format!("&{name}.0"),
                    TypeRef::Named(n) if p.is_ref && unit_enum_names.contains(n.as_str()) => format!("&{name}"),
                    TypeRef::Named(n) if !unit_enum_names.contains(n.as_str()) => format!("{name}.0"),
                    TypeRef::Named(n) if unit_enum_names.contains(n.as_str()) => name,
                    TypeRef::String if p.is_ref => format!("&{name}"),
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

        // See `result_ok_needs_json_bridge_with_handles`'s doc comment: only widen the plain
        // json-bridge check with the u64/i64 Result gap when this method's return really is a
        // `Result<_, String>` (matching the `ok_ty` computation above) -- a bare, non-`Result`
        // `u64`/`i64` getter never reaches swift-bridge-ir's panicking path. ~keep
        let is_result_return = method.error_type.is_some() || forced_fallible;
        let json_wrap_ok = needs_json_bridge_with_handles(&method.return_type, handle_returned_types)
            || (is_result_return
                && matches!(
                    &method.return_type,
                    TypeRef::Primitive(PrimitiveType::U64) | TypeRef::Primitive(PrimitiveType::I64)
                ));
        let wrap_return = |source: String| -> String {
            if method.return_newtype_wrapper.is_some() {
                let converted = apply_return_newtype_unwrap(&source, &method.return_newtype_wrapper);
                return if json_wrap_ok {
                    format!("serde_json::to_string(&({converted})).expect(\"serializable return\")")
                } else {
                    converted
                };
            }
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
                let ok_wrap = if method.return_newtype_wrapper.is_some() {
                    let converted = apply_return_newtype_unwrap("v", &method.return_newtype_wrapper);
                    if json_wrap_ok {
                        format!(".map(|v| serde_json::to_string(&({converted})).expect(\"serializable return\"))")
                    } else {
                        format!(".map(|v| {converted})")
                    }
                } else if json_wrap_ok {
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
            } else if forced_fallible {
                format!("Ok({})", wrap_return(format!("{method_call}.await")))
            } else {
                wrap_return(format!("{method_call}.await"))
            };
            // The receiver is borrowed from the Swift object for the duration of the bridged call,
            // but `spawn` needs a `'static` future. Swift keeps its argument alive until the
            // continuation resumes, and the continuation resumes only after this future -- which
            // awaits the spawned task to completion -- returns, so the task never outlives the
            // borrow. swift-bridge never drops the future early. ~keep
            let extend_receiver = match method.receiver {
                Some(ReceiverKind::RefMut) => format!(
                    "    // SAFETY: Swift keeps the receiver alive until this future's task completes.\n    let client: &'static mut {type_name} = unsafe {{ &mut *(client as *mut {type_name}) }};\n"
                ),
                _ => format!(
                    "    // SAFETY: Swift keeps the receiver alive until this future's task completes.\n    let client: &'static {type_name} = unsafe {{ &*(client as *const {type_name}) }};\n"
                ),
            };
            format!(
                "{extend_receiver}    let __alef_task = crate::__alef_tokio_runtime().spawn(async move {{ {chain} }});\n    __alef_task.await.unwrap_or_else(|__alef_join_error| {{\n        Err(format!(\"alef: spawned async task failed: {{__alef_join_error}}\"))\n    }})"
            )
        } else if method.error_type.is_some() {
            let ok_wrap = if method.return_newtype_wrapper.is_some() {
                let converted = apply_return_newtype_unwrap("v", &method.return_newtype_wrapper);
                if json_wrap_ok {
                    format!(".map(|v| serde_json::to_string(&({converted})).expect(\"serializable return\"))")
                } else {
                    format!(".map(|v| {converted})")
                }
            } else if json_wrap_ok {
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
        } else if forced_fallible {
            format!("    Ok({})", wrap_return(method_call))
        } else {
            format!("    {}", wrap_return(method_call))
        };
        let bindings_str = if pre_call_bindings.is_empty() {
            String::new()
        } else {
            pre_call_bindings.join("\n") + "\n"
        };
        let body = format!("{bindings_str}{body}");

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
                is_async => method.is_async,
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

        let async_uses_transparent_wrapper = method.return_newtype_wrapper.is_some()
            || method.params.iter().any(|param| param.newtype_wrapper.is_some());
        if method.is_async && async_uses_transparent_wrapper {
            out.push_str(&format!(
                "    compile_error!(\"alef cannot safely bridge async first-class DTO method `{}` through Swift; exclude the method from Swift generation\");\n",
                method.name
            ));
            out.push_str("}\n\n");
            continue;
        }

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

        let mut pre_call_bindings = Vec::new();
        let method_call_args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                if p.newtype_wrapper.as_deref().is_some_and(is_explicit_newtype) {
                    let binding_expr = if needs_json_bridge(&p.ty) {
                        let binding_ty = swift_bridge_rust_type(&p.ty);
                        let binding_ty = if p.optional {
                            format!("Option<{binding_ty}>")
                        } else {
                            binding_ty
                        };
                        format!("::serde_json::from_str::<{binding_ty}>(&{name}).expect(\"valid JSON for {name}\")")
                    } else {
                        name.clone()
                    };
                    let converted =
                        apply_param_newtype_to_core(&binding_expr, p).expect("explicit newtype metadata checked above");
                    if p.is_ref {
                        let bound = format!("__{name}_newtype");
                        let mutability = if p.is_mut { "mut " } else { "" };
                        pre_call_bindings.push(format!("    let {mutability}{bound} = {converted};\n"));
                        if p.optional {
                            return if p.is_mut {
                                format!("{bound}.as_mut()")
                            } else {
                                format!("{bound}.as_ref()")
                            };
                        }
                        return if p.is_mut {
                            format!("&mut {bound}")
                        } else {
                            format!("&{bound}")
                        };
                    }
                    return converted;
                }
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
        for binding in pre_call_bindings {
            out.push_str(&binding);
        }
        let __call = format!("__self.{}({})", method.name, method_call_args.join(", "));
        let __call = if method.is_async {
            format!("crate::__alef_tokio_runtime().block_on(async {{ {__call}.await }})")
        } else {
            __call
        };

        if method.error_type.is_some() {
            out.push_str(&format!("    let __result = {__call};\n"));
            if matches!(method.return_type, TypeRef::Unit) {
                // (`let __value = ...` would trip clippy::let_unit_value).
                out.push_str("    __result.map_err(|e| e.to_string())?;\n");
                out.push_str("    Ok(\"{}\".to_string())\n");
            } else {
                out.push_str("    let __value = __result.map_err(|e| e.to_string())?;\n");
                let value = apply_return_newtype_unwrap("__value", &method.return_newtype_wrapper);
                out.push_str(&format!("    serde_json::to_string(&({value}))\n"));
                out.push_str("        .map_err(|e| format!(\"Failed to serialize result: {}\", e))\n");
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            // value to `let __result` would trip clippy::let_unit_value).
            out.push_str(&format!("    {__call};\n"));
            out.push_str("    Ok(\"{}\".to_string())\n");
        } else {
            out.push_str(&format!("    let __result = {__call};\n"));
            let result = apply_return_newtype_unwrap("__result", &method.return_newtype_wrapper);
            out.push_str(&format!("    serde_json::to_string(&({result}))\n"));
            out.push_str("        .map_err(|e| format!(\"Failed to serialize result: {}\", e))\n");
        }

        out.push_str("}\n\n");
    }

    out
}

#[cfg(test)]
mod tests;
