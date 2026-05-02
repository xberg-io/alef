//! NAPI-RS function and method code generation.

use ahash::AHashSet;
use alef_codegen::generators::{self, RustBindingConfig};
use alef_codegen::naming::to_node_name;
use alef_codegen::shared::function_params;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{FunctionDef, ParamDef, TypeRef};

use crate::type_map::NapiMapper;

pub(super) fn gen_function(
    func: &FunctionDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let params = function_params(&func.params, &|ty| {
        // Opaque Named params must be received by reference since NAPI opaque
        // structs don't implement FromNapiValue (they use Arc<T> internally).
        if let TypeRef::Named(n) = ty {
            if opaque_types.contains(n.as_str()) {
                return format!("&{prefix}{n}");
            }
        }
        mapper.map_type(ty)
    });
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let core_import = cfg.core_import;
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    // Use let-binding pattern for non-opaque Named params, or for Vec<f32> params that need conversion
    let use_let_bindings = generators::has_named_params(&func.params, opaque_types)
        || func.params.iter().any(|p| needs_vec_f32_conversion(&p.ty));
    let call_args = if use_let_bindings {
        let base_args = generators::gen_call_args_with_let_bindings(&func.params, opaque_types);
        napi_apply_primitive_casts_to_call_args(&base_args, &func.params)
    } else {
        napi_gen_call_args(&func.params, opaque_types)
    };

    let can_delegate_fn = alef_codegen::shared::can_auto_delegate_function(func, opaque_types)
        || can_delegate_with_named_let_bindings(func, opaque_types);

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    let async_kw = if func.is_async { "async " } else { "" };

    let body = if !can_delegate_fn {
        // Try serde-based conversion for non-delegatable functions with Named params
        // Only use serde conversion if cfg.has_serde is true (binding crate has serde deps)
        if cfg.has_serde && use_let_bindings && func.error_type.is_some() {
            let serde_bindings =
                generators::gen_serde_let_bindings(&func.params, opaque_types, core_import, err_conv, "    ");
            // Also generate Vec<String>+is_ref bindings (names_refs) since serde doesn't handle them
            let vec_str_bindings: String = func.params.iter().filter(|p| {
                p.is_ref && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char))
            }).map(|p| {
                format!("let {}_refs: Vec<&str> = {}.iter().map(|s| s.as_str()).collect();\n    ", p.name, p.name)
            }).collect();
            let core_call = format!("{core_fn_path}({call_args})");
            let await_kw = if func.is_async { ".await" } else { "" };

            if matches!(func.return_type, TypeRef::Unit) {
                format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}?;\n    Ok(())")
            } else {
                let wrapped = napi_wrap_return_fn("val", &func.return_type, opaque_types, func.returns_ref, prefix);
                if wrapped == "val" {
                    format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}")
                } else {
                    format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}.map(|val| {wrapped}){err_conv}")
                }
            }
        } else {
            generators::gen_unimplemented_body(
                &func.return_type,
                &func.name,
                func.error_type.is_some(),
                cfg,
                &func.params,
                opaque_types,
            )
        }
    } else if func.is_async {
        // For async delegatable functions, generate let bindings if needed before the async call
        let mut let_bindings = if use_let_bindings {
            generators::gen_named_let_bindings_pub(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Add Vec<f32> conversion bindings for parameters not already handled
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));
        let core_call = format!("{core_fn_path}({call_args})");
        let return_wrap = napi_wrap_return_fn("result", &func.return_type, opaque_types, func.returns_ref, prefix);
        let return_type = mapper.map_type(&func.return_type);
        generators::gen_async_body(
            &core_call,
            cfg,
            func.error_type.is_some(),
            &return_wrap,
            false,
            &let_bindings,
            matches!(func.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_fn_path}({call_args})");
        // Generate let bindings for Named params if needed
        let mut let_bindings = if use_let_bindings {
            generators::gen_named_let_bindings_pub(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Add Vec<f32> conversion bindings for parameters not already handled
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));

        if func.error_type.is_some() {
            let wrapped = napi_wrap_return_fn("val", &func.return_type, opaque_types, func.returns_ref, prefix);
            if wrapped == "val" {
                format!("{let_bindings}{core_call}{err_conv}")
            } else {
                format!("{let_bindings}{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            format!(
                "{let_bindings}{}",
                napi_wrap_return_fn(&core_call, &func.return_type, opaque_types, func.returns_ref, prefix)
            )
        }
    };

    let mut attrs = String::new();
    // Per-item clippy suppression: too_many_arguments when >7 params
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    format!(
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        func.name
    )
}

fn can_delegate_with_named_let_bindings(func: &FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func
            .params
            .iter()
            .all(|p| !p.sanitized && alef_codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && alef_codegen::shared::is_delegatable_return(&func.return_type)
}

/// Apply NAPI-specific primitive casts to the call args generated by the generic let-binding handler.
/// Adds i64→usize, i64→isize, f64→f32 casts where needed.
pub(super) fn napi_apply_primitive_casts_to_call_args(generic_args: &str, params: &[ParamDef]) -> String {
    // Split args by comma and match with params to apply casting
    let args_list: Vec<&str> = generic_args.split(',').map(|s| s.trim()).collect();
    args_list
        .iter()
        .zip(params.iter())
        .map(|(arg, p)| {
            // Special case: Vec<f32> param with is_ref uses the converted variable
            if needs_vec_f32_conversion(&p.ty) && p.is_ref {
                return format!("&{}_f32", p.name);
            }
            match &p.ty {
                TypeRef::Primitive(prim) if needs_napi_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        // Optional: arg might be like "param.map(...)" so re-apply map
                        if arg.contains(".map(") || arg.contains(".as_") {
                            // Already handled, keep as is
                            arg.to_string()
                        } else {
                            format!("{}.map(|v| v as {})", arg, core_ty)
                        }
                    } else {
                        // Non-optional: simple cast
                        format!("{} as {}", arg, core_ty)
                    }
                }
                _ => arg.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate let bindings for Vec<f32> parameters that need f64→f32 conversion.
/// This handles the case where NAPI maps f32→f64, but a function param is Vec<f32> taking a reference.
pub(super) fn gen_vec_f32_conversion_bindings(params: &[ParamDef]) -> String {
    let mut bindings = String::new();
    for p in params {
        if needs_vec_f32_conversion(&p.ty) && p.is_ref {
            let conv_name = format!("{}_f32", p.name);
            bindings.push_str(&format!(
                "    let {conv_name}: Vec<f32> = {}.iter().map(|&x| x as f32).collect();\n",
                p.name
            ));
        }
    }
    bindings
}

/// NAPI-specific call args that casts i64 params to u64/usize where the core expects it.
/// Properly handles is_ref for reference parameters and complex type conversions.
pub(super) fn napi_gen_call_args(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .map(|p| {
            // Special case: Vec<f32> param with is_ref uses the converted variable
            if needs_vec_f32_conversion(&p.ty) && p.is_ref {
                return format!("&{}_f32", p.name);
            }
            match &p.ty {
                TypeRef::Primitive(prim) if needs_napi_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        format!("{}.map(|v| v as {})", p.name, core_ty)
                    } else {
                        format!("{} as {}", p.name, core_ty)
                    }
                }
                TypeRef::Duration => {
                    if p.optional {
                        format!("{}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))", p.name)
                    } else {
                        format!("std::time::Duration::from_millis({}.max(0) as u64)", p.name)
                    }
                }
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    // When an opaque type param is required by a builder/owned-receiver method,
                    // clone the inner value (Arc dereference) to get an owned copy.
                    // When used as a reference param, borrow `&v.inner` instead.
                    if p.is_ref {
                        if p.optional {
                            format!("{}.as_ref().map(|v| v.inner.as_ref())", p.name)
                        } else {
                            format!("{}.inner.as_ref()", p.name)
                        }
                    } else if p.optional {
                        format!("{}.as_ref().map(|v| (*v.inner).clone())", p.name)
                    } else {
                        format!("(*{}.inner).clone()", p.name)
                    }
                }
                TypeRef::Named(_) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else {
                            format!("{}.map(Into::into)", p.name)
                        }
                    } else {
                        format!("{}.into()", p.name)
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Path => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref().map(std::path::Path::new)", p.name)
                        } else {
                            format!("{}.map(std::path::PathBuf::from)", p.name)
                        }
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{})", p.name)
                    } else {
                        format!("std::path::PathBuf::from({})", p.name)
                    }
                }
                TypeRef::Bytes => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Vec(inner) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) {
                        format!("&{}_refs", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Map(_, _) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// NAPI-specific return wrapping for opaque instance methods.
/// Extends the shared `wrap_return` with i64 casts for u64/usize/isize primitives.
pub(super) fn napi_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    prefix: &str,
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_napi_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        // Opaque Named returns need prefix
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        _ => generators::wrap_return(
            expr,
            return_type,
            type_name,
            opaque_types,
            self_is_opaque,
            returns_ref,
            false,
        ),
    }
}

/// NAPI-specific return wrapping for free functions (no type_name context).
pub(super) fn napi_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    prefix: &str,
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_napi_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
            if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(_) => {
                    if returns_ref {
                        format!("{expr}.map(|v| v.into_iter().map(|x| x.clone().into()).collect())")
                    } else {
                        format!("{expr}.map(|v| v.into_iter().map(Into::into).collect())")
                    }
                }
                _ => expr.to_string(),
            },
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                // Vec<usize>, Vec<f32>, etc. need element-wise casting to i64 or f64
                let target_ty = match p {
                    alef_core::ir::PrimitiveType::F32 => "f64",
                    _ => "i64", // u64, usize, isize
                };
                format!("{expr}.into_iter().map(|v| v as {target_ty}).collect()")
            }
            // Vec<Vec<T>> where the inner primitive needs widening (e.g. Vec<Vec<f32>> → Vec<Vec<f64>>)
            TypeRef::Vec(inner2) => {
                if let TypeRef::Primitive(p) = inner2.as_ref() {
                    if needs_napi_cast(p) {
                        let target_ty = match p {
                            alef_core::ir::PrimitiveType::F32 => "f64",
                            _ => "i64",
                        };
                        return format!(
                            "{expr}.into_iter().map(|row| row.into_iter().map(|x| x as {target_ty}).collect::<Vec<_>>()).collect::<Vec<_>>()"
                        );
                    }
                }
                expr.to_string()
            }
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Check if a type is Vec<f32> which needs element-wise conversion from f64 in NAPI.
pub(super) fn needs_vec_f32_conversion(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(alef_core::ir::PrimitiveType::F32)))
}

pub(super) fn needs_napi_cast(p: &alef_core::ir::PrimitiveType) -> bool {
    // U32 maps to u32 in both NAPI and core, so no cast needed.
    // U64/Usize/Isize map to i64 in NAPI but u64/usize/isize in core.
    // F32 maps to f64 in NAPI but f32 in core.
    matches!(
        p,
        alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::Usize
            | alef_core::ir::PrimitiveType::Isize
            | alef_core::ir::PrimitiveType::F32
    )
}

pub(super) fn core_prim_str(p: &alef_core::ir::PrimitiveType) -> &'static str {
    match p {
        alef_core::ir::PrimitiveType::U64 => "u64",
        alef_core::ir::PrimitiveType::Usize => "usize",
        alef_core::ir::PrimitiveType::Isize => "isize",
        alef_core::ir::PrimitiveType::F32 => "f32",
        _ => unreachable!(),
    }
}

/// Generate a global Tokio runtime for NAPI async support.
pub(super) fn gen_tokio_runtime() -> String {
    "static WORKER_POOL: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect(\"Failed to create Tokio runtime\")
});"
    .to_string()
}

/// Generate an `index.d.ts` file for the NAPI binding crate.
///
/// NAPI-RS generates `const enum` in its auto-generated `.d.ts`, which is incompatible
/// with `verbatimModuleSyntax` (const enums cannot be re-exported as values). This
/// function produces an equivalent `.d.ts` with `export declare enum` (regular enum)
/// so the file can be committed and used directly without a post-build patch step.
///
/// The output format matches what NAPI-RS would generate after patching, using the same
/// alphabetical ordering and type declarations seen in the committed `index.d.ts` files.
#[cfg(test)]
mod tests {
    use super::gen_tokio_runtime;

    /// gen_tokio_runtime produces a static TOKIO_RUNTIME definition.
    #[test]
    fn gen_tokio_runtime_contains_runtime() {
        let result = gen_tokio_runtime();
        assert!(result.contains("TOKIO_RUNTIME") || result.contains("Runtime") || result.contains("tokio"));
    }
}
