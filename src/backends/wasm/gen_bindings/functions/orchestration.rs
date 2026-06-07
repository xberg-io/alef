//! Free-function binding orchestration for WASM.

use super::imports_helpers::emit_rustdoc;
use super::input_dto::{gen_input_dto_for_type, should_have_input_dto};
use super::params::{format_param_unused, typeref_to_core_type_str, wasm_serde_recovery_call_args};
use super::returns::{gen_wasm_unimplemented_body, type_has_default, wasm_wrap_return_fn};
use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name};
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub(in crate::backends::wasm::gen_bindings) fn gen_function_with_emitted_dtos(
    func: &FunctionDef,
    mapper: &WasmMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    mutex_types: &AHashSet<String>,
    api: &crate::core::ir::ApiSurface,
    emitted_input_dtos: &AHashSet<String>,
) -> String {
    // Collect any Input DTOs needed for config-like parameters
    let mut input_dtos = String::new();
    let mut input_dto_names: HashMap<String, String> = HashMap::new();

    for p in &func.params {
        if let TypeRef::Named(name) = &p.ty {
            if !opaque_types.contains(name.as_str()) {
                // Find the TypeDef for this named type
                if let Some(type_def) = api.types.iter().find(|t| t.name == *name)
                    && should_have_input_dto(type_def)
                {
                    // Skip if already emitted (dedup)
                    if emitted_input_dtos.contains(name) {
                        input_dto_names.insert(name.clone(), format!("{}Input", name));
                        continue;
                    }
                    let (dto_code, dto_name) = gen_input_dto_for_type(name, core_import, type_def);
                    if !dto_code.is_empty() {
                        input_dtos.push_str(&dto_code);
                        input_dtos.push_str("\n\n");
                        input_dto_names.insert(name.clone(), dto_name);
                    }
                }
            }
        }
    }

    let can_delegate = crate::codegen::shared::can_auto_delegate_function(func, opaque_types);

    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate && !func.is_async)
        })
        .collect();

    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let mut attrs = emit_rustdoc(&func.doc);
    // Per-item clippy suppression: too_many_arguments when >7 params
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    if func.is_async {
        super::async_wrappers::gen_async_free_function(
            &input_dtos,
            func,
            mapper,
            core_import,
            opaque_types,
            mutex_types,
            api,
            &params,
            &return_type,
            &attrs,
            &js_name_attr,
            &return_annotation,
            &core_fn_path,
        )
    } else if can_delegate {
        let mut let_bindings = if crate::codegen::generators::has_named_params(&func.params, opaque_types) {
            crate::codegen::generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Nested Vec params (e.g. Vec<Vec<String>>) arrive as JsValue because wasm-bindgen
        // cannot pass them across the boundary directly. Emit a deserialization shadowing
        // binding so the core call sees a real `Vec<Vec<T>>`.
        let needs_result_wrap = func
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(_))))
            && func.error_type.is_none();
        for p in &func.params {
            if let TypeRef::Vec(outer_inner) = &p.ty
                && matches!(outer_inner.as_ref(), TypeRef::Vec(_))
            {
                let elem_ty = if let TypeRef::Vec(elem) = outer_inner.as_ref() {
                    typeref_to_core_type_str(elem.as_ref())
                } else {
                    "String".to_string()
                };
                let core_ty = format!("Vec<Vec<{elem_ty}>>");
                if p.optional {
                    let err_conv = format!(".expect(\"deserialize {}\")", p.name);
                    let_bindings.push_str(&crate::backends::wasm::template_env::render(
                        "serde_vec_nested_optional",
                        minijinja::context! {
                            param_name => &p.name,
                            core_ty => &core_ty,
                            err_conv => &err_conv,
                        },
                    ));
                    let_bindings.push_str("    ");
                } else {
                    let err_conv = format!(".expect(\"deserialize {}\")", p.name);
                    let_bindings.push_str(&crate::backends::wasm::template_env::render(
                        "serde_vec_nested_required",
                        minijinja::context! {
                            param_name => &p.name,
                            core_ty => &core_ty,
                            err_conv => &err_conv,
                        },
                    ));
                    let_bindings.push_str("    ");
                }
            }
        }
        let _ = needs_result_wrap;
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let body = if func.error_type.is_some() {
            let wrap = wasm_wrap_return_fn(
                "result",
                &func.return_type,
                opaque_types,
                func.returns_ref,
                func.returns_cow,
                prefix,
                mutex_types,
            );
            crate::backends::wasm::template_env::render(
                "gen_result_body",
                minijinja::context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                    return_expr => &wrap,
                    is_async => false,
                    map_wasm_error => false,
                    map_js_error => true,
                    ok_return => true,
                },
            )
        } else {
            let return_expr = wasm_wrap_return_fn(
                &core_call,
                &func.return_type,
                opaque_types,
                func.returns_ref,
                func.returns_cow,
                prefix,
                mutex_types,
            );
            crate::backends::wasm::template_env::render(
                "gen_direct_body",
                minijinja::context! {
                    let_bindings => &let_bindings,
                    return_expr => &return_expr,
                },
            )
        };
        let fn_code = crate::backends::wasm::template_env::render(
            "gen_free_function",
            minijinja::context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                is_async => false,
                function_name => &func.name,
                params => params.join(", "),
                return_annotation => &return_annotation,
                body => body.trim_end(),
            },
        );
        format!("{input_dtos}{fn_code}")
    } else if func.error_type.is_some()
        && (func.sanitized || crate::codegen::generators::has_named_params(&func.params, opaque_types))
    {
        // Serde recovery: accept Named non-opaque params as JsValue and deserialize
        // to core types via serde_wasm_bindgen. Also handles sanitized functions (Vec<tuple>).
        // WASM binding structs don't derive Serialize/Deserialize, so we can't round-trip
        // through the binding type; instead we accept raw JsValue/Vec<String> from JS and
        // deserialize directly to core types.
        let serde_params: Vec<String> = func
            .params
            .iter()
            .map(|p| match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    // Accept as JsValue so serde_wasm_bindgen::from_value can deserialize
                    let mapped_ty = if p.optional {
                        "Option<JsValue>".to_string()
                    } else {
                        "JsValue".to_string()
                    };
                    format!("{}: {}", p.name, mapped_ty)
                }
                TypeRef::Vec(inner) => {
                    // Sanitized Vec<tuple>: accept Vec<String> (JSON encoded)
                    if matches!(inner.as_ref(), TypeRef::Named(_)) {
                        if p.optional {
                            format!("{}: Option<Vec<String>>", p.name)
                        } else {
                            format!("{}: Vec<String>", p.name)
                        }
                    } else {
                        let ty = mapper.map_type(&p.ty);
                        let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                }
                _ => {
                    let ty = mapper.map_type(&p.ty);
                    let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                    format!("{}: {}", p.name, mapped_ty)
                }
            })
            .collect();

        // Generate serde_wasm_bindgen::from_value let-bindings for Named non-opaque params
        // and Vec<String> with is_ref=true (needs texts_refs intermediate)
        let mut serde_bindings = String::new();
        for p in &func.params {
            match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    let core_path = format!("{}::{}", core_import, name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";

                    // Check if this is a config-like type that needs camelCase conversion
                    if api
                        .types
                        .iter()
                        .find(|t| t.name == *name)
                        .is_some_and(should_have_input_dto)
                    {
                        // Use the Input DTO for deserialization with camelCase support
                        let input_dto_type = input_dto_names
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| format!("{}Input", name));
                        if p.optional {
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_config_optional",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    input_dto_type => &input_dto_type,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        } else {
                            let has_default = type_has_default(name, api);
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_config_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    input_dto_type => &input_dto_type,
                                    has_default => has_default,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    } else {
                        // Regular named type deserialization
                        if p.optional {
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_optional",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        } else {
                            let has_default = type_has_default(name, api);
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    has_default => has_default,
                                    is_mut => p.is_mut,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    }
                }
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    // Sanitized Vec<tuple>: deserialize from Vec<String> JSON
                    let inner_name = match inner.as_ref() {
                        TypeRef::Named(n) => n,
                        _ => "UnknownTuple",
                    };
                    let core_path = format!("{}::{}", core_import, inner_name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_named_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                core_path => &core_path,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_named_required",
                            minijinja::context! {
                                param_name => &p.name,
                                core_path => &core_path,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                TypeRef::Vec(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                        && p.sanitized
                        && p.original_type.is_some() =>
                {
                    // Sanitized Vec<tuple>: binding accepts Vec<String> (JSON-encoded tuple items).
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_tuple_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_tuple_required",
                            minijinja::context! {
                                param_name => &p.name,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                    // Vec<String> with is_ref=true: core expects &[&str].
                    // gen_call_args_with_let_bindings emits `&{name}_refs`, so we must create
                    // the intermediate Vec<&str> binding here.
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_string_refs_optional",
                            minijinja::context! {
                                param_name => &p.name,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_string_refs_required",
                            minijinja::context! {
                                param_name => &p.name,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                TypeRef::Vec(outer_inner) if matches!(outer_inner.as_ref(), TypeRef::Vec(_)) => {
                    // Nested Vec (e.g. Vec<Vec<String>>): wasm-bindgen cannot pass this across
                    // the boundary directly, so the param arrives as JsValue. Deserialize via
                    // serde_wasm_bindgen and shadow the original binding so gen_call_args can
                    // still reference the parameter by its original name.
                    let elem_ty = if let TypeRef::Vec(elem) = outer_inner.as_ref() {
                        typeref_to_core_type_str(elem.as_ref())
                    } else {
                        "String".to_string()
                    };
                    let core_ty = format!("Vec<Vec<{elem_ty}>>");
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_nested_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                core_ty => &core_ty,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_nested_required",
                            minijinja::context! {
                                param_name => &p.name,
                                core_ty => &core_ty,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                _ => {}
            }
        }

        let call_args = wasm_serde_recovery_call_args(&func.params, opaque_types);
        let core_call = format!("{core_fn_path}({call_args})");
        let wrap = wasm_wrap_return_fn(
            "result",
            &func.return_type,
            opaque_types,
            func.returns_ref,
            func.returns_cow,
            prefix,
            mutex_types,
        );
        let body = if matches!(func.return_type, TypeRef::Unit) {
            crate::backends::wasm::template_env::render(
                "gen_unit_result_body",
                minijinja::context! {
                    let_bindings => &serde_bindings,
                    core_call => &core_call,
                },
            )
        } else {
            crate::backends::wasm::template_env::render(
                "gen_result_body",
                minijinja::context! {
                    let_bindings => &serde_bindings,
                    core_call => &core_call,
                    return_expr => &wrap,
                    is_async => false,
                    map_wasm_error => false,
                    map_js_error => true,
                    ok_return => true,
                },
            )
        };
        let fn_code = crate::backends::wasm::template_env::render(
            "gen_free_function",
            minijinja::context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                is_async => false,
                function_name => &func.name,
                params => serde_params.join(", "),
                return_annotation => &return_annotation,
                body => body.trim_end(),
            },
        );
        format!("{input_dtos}{fn_code}")
    } else {
        let body = gen_wasm_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some());
        let fn_code = crate::backends::wasm::template_env::render(
            "gen_free_function",
            minijinja::context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                is_async => false,
                function_name => &func.name,
                params => params.join(", "),
                return_annotation => &return_annotation,
                body => &body,
            },
        );
        format!("{input_dtos}{fn_code}")
    }
}
