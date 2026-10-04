//! NAPI-RS function and method code generation.

mod adapter_wrappers;
mod call_args;
mod conversion_bindings;
mod return_wrapping;

pub(super) use adapter_wrappers::{gen_adapter_wrapper, gen_tokio_runtime};
pub(super) use call_args::{
    core_prim_str, napi_apply_primitive_casts_to_call_args, napi_gen_call_args, napi_newtype_param_bindings,
    needs_napi_cast,
};
use call_args::{is_bytes_param, needs_vec_f32_conversion};
use conversion_bindings::{gen_napi_buffer_conversion_bindings, gen_vec_f32_conversion_bindings};
pub(super) use return_wrapping::{napi_wrap_return, napi_wrap_return_fn};

use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::codegen::generators::{self, RustBindingConfig};
use crate::codegen::naming::to_node_name;
use crate::codegen::shared::{self, function_params};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;

use crate::backends::napi::type_map::NapiMapper;

pub(super) fn promoted_required(params: &[crate::core::ir::ParamDef]) -> Vec<&crate::core::ir::ParamDef> {
    params
        .iter()
        .enumerate()
        .filter_map(|(index, parameter)| shared::is_promoted_optional(params, index).then_some(parameter))
        .collect()
}

pub(super) fn promoted_required_prefix(params: &[crate::core::ir::ParamDef]) -> String {
    promoted_required(params)
        .iter()
        .map(|parameter| {
            format!(
                "let {} = {}.ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"missing required parameter '{}'\"))?;\n    ",
                parameter.name, parameter.name, parameter.name
            )
        })
        .collect()
}

pub(super) fn wrap_promoted_required_body(
    body: String,
    params: &[crate::core::ir::ParamDef],
    core_has_error: bool,
    is_async: bool,
    returns_unit: bool,
) -> String {
    let prefix = promoted_required_prefix(params);
    if prefix.is_empty() {
        return body;
    }
    if core_has_error || (is_async && returns_unit) {
        format!("{prefix}{body}")
    } else if returns_unit {
        format!("{prefix}{body};\n    Ok(())")
    } else {
        format!("{prefix}Ok({{ {body} }})")
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_function(
    func: &FunctionDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    prefix: &str,
    capsule_types: &std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>,
    mutex_types: &AHashSet<String>,
) -> String {
    let augmented_params: Vec<crate::core::ir::ParamDef> = func
        .params
        .iter()
        .map(|p| {
            let mut p2 = p.clone();
            if !p2.optional
                && let TypeRef::Named(n) = &p2.ty
                && default_types.contains(n.as_str())
                && !opaque_types.contains(n.as_str())
            {
                p2.optional = true;
            }
            p2
        })
        .collect();
    let promoted_required = promoted_required(&augmented_params);
    let params = function_params(&augmented_params, &|ty| {
        if let TypeRef::Named(n) = ty {
            if capsule_types.contains_key(n.as_str())
                && let Some(capsule_cfg) = capsule_types.get(n.as_str())
            {
                return capsule_cfg.from_module.clone();
            }
            if opaque_types.contains(n.as_str()) {
                return format!("&{prefix}{n}");
            }
        }
        mapper.map_type(ty)
    });
    let default_coerce_prefix: String = augmented_params
        .iter()
        .zip(func.params.iter())
        .enumerate()
        .filter_map(|(idx, (aug, orig))| {
            if aug.optional && !orig.optional && !crate::codegen::shared::is_promoted_optional(&func.params, idx) {
                let is_named_non_opaque = matches!(&orig.ty,
                    TypeRef::Named(n) if !opaque_types.contains(n.as_str())
                );
                if is_named_non_opaque && promoted_required.is_empty() {
                    return None;
                }
                let mut_kw = if orig.is_mut { "mut " } else { "" };
                Some(format!(
                    "    let {}{} = {}.unwrap_or_default();\n",
                    mut_kw, orig.name, orig.name
                ))
            } else {
                None
            }
        })
        .collect();
    // A `&mut T` DTO parameter on a unit-returning function (sync or async) cannot write back
    // through the mutated intermediate (the JS object handed to the binding is by-value), so the
    // binding returns the updated `T` instead. The async branch below special-cases the writeback
    // body directly rather than going through `gen_async_body`, since that helper's unit-return
    // handling hardcodes an `Ok(())` tail that would no longer match this declared return type.
    // See `mut_writeback` for the policy; `generate_bindings` calls `reject_unsupported_writeback`
    // before this function runs, so any shape this module cannot express has already been
    // rejected. ~keep
    let writeback_param = crate::codegen::mut_writeback::writeback_param(&func.params, &func.return_type, opaque_types);
    let writeback_var = writeback_param.map(|p| format!("{}_core", p.name));

    let return_type = match writeback_param {
        Some(p) => mapper.map_type(&p.ty),
        None => mapper.map_type(&func.return_type),
    };
    let return_annotation =
        mapper.wrap_return(&return_type, func.error_type.is_some() || !promoted_required.is_empty());

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

    let use_let_bindings = generators::has_named_params(&func.params, opaque_types)
        || func.params.iter().any(|p| needs_vec_f32_conversion(&p.ty))
        || func.params.iter().any(|p| is_bytes_param(&p.ty));
    let call_args = if use_let_bindings {
        let base_args = if promoted_required.is_empty() {
            generators::gen_call_args_with_let_bindings_mutex(&func.params, opaque_types, mutex_types)
        } else {
            generators::gen_call_args_with_let_bindings_mutex_no_promote(&func.params, opaque_types, mutex_types)
        };
        napi_apply_primitive_casts_to_call_args(&base_args, &func.params)
    } else {
        napi_gen_call_args(&func.params, opaque_types)
    };

    let can_delegate_fn = generators::can_auto_delegate_function_with_named_let_bindings(func, opaque_types);

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    let async_kw = if func.is_async { "async " } else { "" };

    let body = if !can_delegate_fn {
        if cfg.has_serde && use_let_bindings && func.error_type.is_some() {
            let mut serde_bindings =
                generators::gen_serde_let_bindings(&func.params, opaque_types, core_import, err_conv, "    ");
            serde_bindings.push_str(&napi_newtype_param_bindings(&func.params));
            let vec_str_bindings: String = func.params.iter().filter(|p| {
                p.is_ref && p.vec_inner_is_ref && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char))
            }).map(|p| {
                format!("let {}_refs: Vec<&str> = {}.iter().map(|s| s.as_str()).collect();\n    ", p.name, p.name)
            }).collect();
            let core_call = format!("{core_fn_path}({call_args})");
            let await_kw = if func.is_async { ".await" } else { "" };

            if matches!(func.return_type, TypeRef::Unit) {
                format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}?;\n    Ok(())")
            } else {
                let value = apply_return_newtype_unwrap("val", &func.return_newtype_wrapper);
                let wrapped = napi_wrap_return_fn(
                    &value,
                    &func.return_type,
                    opaque_types,
                    func.returns_ref,
                    prefix,
                    Some(capsule_types),
                    mutex_types,
                );
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
        let mut let_bindings = if use_let_bindings {
            if promoted_required.is_empty() {
                generators::gen_named_let_bindings_with_augmented(
                    &augmented_params,
                    &func.params,
                    opaque_types,
                    core_import,
                )
            } else {
                generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
            }
        } else {
            String::new()
        };
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));
        let_bindings.push_str(&gen_napi_buffer_conversion_bindings(&func.params));
        let_bindings.push_str(&napi_newtype_param_bindings(&func.params));
        let core_call = format!("{core_fn_path}({call_args})");
        if let Some(var) = &writeback_var {
            // Async `&mut` DTO write-back: the core future resolves to `()` (or `Result<(), E>`),
            // which the sync writeback branch below discards in favor of the mutated
            // intermediate. Mirror that here with an `.await` ahead of the discard. ~keep
            if func.error_type.is_some() {
                format!("{let_bindings}{core_call}.await.map(|_| {var}.into()){err_conv}")
            } else {
                format!("{let_bindings}{core_call}.await;\n            {var}.into()")
            }
        } else {
            let result = apply_return_newtype_unwrap("result", &func.return_newtype_wrapper);
            let return_wrap = napi_wrap_return_fn(
                &result,
                &func.return_type,
                opaque_types,
                func.returns_ref,
                prefix,
                Some(capsule_types),
                mutex_types,
            );
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
        }
    } else {
        let core_call = format!("{core_fn_path}({call_args})");
        let mut let_bindings = if use_let_bindings {
            if promoted_required.is_empty() {
                generators::gen_named_let_bindings_with_augmented(
                    &augmented_params,
                    &func.params,
                    opaque_types,
                    core_import,
                )
            } else {
                generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
            }
        } else {
            String::new()
        };
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));
        let_bindings.push_str(&gen_napi_buffer_conversion_bindings(&func.params));
        let_bindings.push_str(&napi_newtype_param_bindings(&func.params));

        if let Some(var) = &writeback_var {
            // The core call mutates `{var}` and returns `()`; the binding hands back the
            // mutated intermediate instead of the (discarded) unit value. ~keep
            if func.error_type.is_some() {
                format!("{let_bindings}{core_call}.map(|_| {var}.into()){err_conv}")
            } else {
                format!("{let_bindings}{core_call};\n    {var}.into()")
            }
        } else if func.error_type.is_some() {
            let value = apply_return_newtype_unwrap("val", &func.return_newtype_wrapper);
            let wrapped = napi_wrap_return_fn(
                &value,
                &func.return_type,
                opaque_types,
                func.returns_ref,
                prefix,
                Some(capsule_types),
                mutex_types,
            );
            if wrapped == "val" {
                format!("{let_bindings}{core_call}{err_conv}")
            } else {
                format!("{let_bindings}{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            let result = apply_return_newtype_unwrap(&core_call, &func.return_newtype_wrapper);
            format!(
                "{let_bindings}{}",
                napi_wrap_return_fn(
                    &result,
                    &func.return_type,
                    opaque_types,
                    func.returns_ref,
                    prefix,
                    Some(capsule_types),
                    mutex_types
                )
            )
        }
    };

    let mut attrs = String::new();
    let sanitized_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&func.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_doc, "");
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    let body = if default_coerce_prefix.is_empty() {
        body
    } else {
        format!("{}{}", default_coerce_prefix, body)
    };
    let body = wrap_promoted_required_body(
        body,
        &augmented_params,
        func.error_type.is_some(),
        func.is_async,
        matches!(func.return_type, TypeRef::Unit),
    );
    crate::backends::napi::template_env::render(
        "function_wrapper.jinja",
        minijinja::context! {
            attrs => attrs,
            js_name_attr => js_name_attr,
            async_kw => async_kw,
            func_name => &func.name,
            params => params,
            return_annotation => return_annotation,
            body => body,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::gen_tokio_runtime;

    fn transparent_string_wrapper(containers: Vec<crate::core::ir::NewtypeContainer>) -> String {
        use crate::core::ir::{NewtypeWrapper, NewtypeWrapperMetadata};

        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "sample_core::SecretString",
            "from",
            "into_inner",
            containers,
        )])
    }

    /// gen_tokio_runtime produces a static runtime with an enlarged worker stack so deep
    /// consumer futures (e.g. an OCR pipeline) do not overflow the default ~2 MB stack (SIGBUS).
    #[test]
    fn gen_tokio_runtime_contains_runtime() {
        let result = gen_tokio_runtime();
        assert!(result.contains("TOKIO_RUNTIME") || result.contains("Runtime") || result.contains("tokio"));
        assert!(
            result.contains("thread_stack_size"),
            "worker pool must enlarge the stack:\n{result}"
        );
    }

    fn record_param(is_ref: bool, is_mut: bool) -> crate::core::ir::ParamDef {
        crate::core::ir::ParamDef {
            name: "record".to_owned(),
            ty: crate::core::ir::TypeRef::Named("Record".to_owned()),
            is_ref,
            is_mut,
            ..crate::core::ir::ParamDef::default()
        }
    }

    fn gen_probe_function(func: &crate::core::ir::FunctionDef) -> String {
        gen_probe_function_with_defaults(func, ahash::AHashSet::new())
    }

    fn gen_probe_function_with_defaults(
        func: &crate::core::ir::FunctionDef,
        default_types: ahash::AHashSet<String>,
    ) -> String {
        use crate::backends::napi::gen_bindings::NapiBackend;
        use crate::backends::napi::type_map::NapiMapper;

        let mapper = NapiMapper::new("Js".to_owned());
        let cfg = NapiBackend::binding_config("sample_core", "Js", true);
        let opaque_types = ahash::AHashSet::new();
        let capsule_types = std::collections::HashMap::new();
        let mutex_types = ahash::AHashSet::new();

        super::gen_function(
            func,
            &mapper,
            &cfg,
            &opaque_types,
            &default_types,
            "Js",
            &capsule_types,
            &mutex_types,
        )
    }

    #[test]
    fn napi_required_params_after_defaultable_params_are_unwrapped_before_conversion() {
        use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

        let func = FunctionDef {
            name: "redact_external".to_owned(),
            rust_path: "sample_core::redact_external".to_owned(),
            params: vec![
                ParamDef {
                    name: "document".to_owned(),
                    ty: TypeRef::Named("Document".to_owned()),
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "config".to_owned(),
                    ty: TypeRef::Named("Config".to_owned()),
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "result".to_owned(),
                    ty: TypeRef::Named("ResultData".to_owned()),
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "findings_json".to_owned(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "offset_encoding".to_owned(),
                    ty: TypeRef::String,
                    optional: true,
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("Document".to_owned()),
            is_async: true,
            error_type: Some("Error".to_owned()),
            ..FunctionDef::default()
        };
        let default_types = ["Document".to_owned(), "Config".to_owned()].into_iter().collect();

        let output = gen_probe_function_with_defaults(&func, default_types);

        assert!(
            output.contains("let result = result.ok_or_else"),
            "required DTO parameters promoted behind defaultable parameters must be unwrapped:\n{output}"
        );
        assert!(
            output.contains("let findings_json = findings_json.ok_or_else"),
            "required parameters promoted behind defaultable parameters must be unwrapped:\n{output}"
        );
        assert!(
            output.contains("let document = document.unwrap_or_default();"),
            "defaultable DTO parameters must be made concrete before conversion:\n{output}"
        );
        assert!(
            output.contains("let result_core: sample_core::ResultData = result.into();"),
            "required DTO parameters must be converted only after they are unwrapped:\n{output}"
        );
        let unwrap = output
            .find("let result = result.ok_or_else")
            .expect("required DTO unwrap");
        let convert = output.find("let result_core:").expect("required DTO conversion");
        assert!(
            unwrap < convert,
            "required DTO unwrap must precede conversion:\n{output}"
        );
        assert!(
            output.contains("sample_core::redact_external(document_core, config_core, result_core, &findings_json"),
            "the core call must borrow the unwrapped string rather than Option<String>:\n{output}"
        );
    }

    /// Regression test for issue #380: a `&mut T` DTO parameter on a unit-returning sync
    /// function previously rendered as `pub fn tag_record(record: JsRecord) -> ()`, mutating a
    /// dropped `_core` intermediate and leaving the caller's JS object untouched with no
    /// diagnostic. The binding must instead return the mutated intermediate.
    #[test]
    fn napi_mut_dto_param_returns_the_updated_value() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "tag_record".to_owned(),
            rust_path: "sample_core::tag_record".to_owned(),
            params: vec![record_param(true, true)],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(
            output.contains("-> JsRecord"),
            "expected the binding to return the mutated DTO type instead of `()`:\n{output}"
        );
        assert!(
            !output.contains("-> ()"),
            "must not still advertise a unit return:\n{output}"
        );
        // Load-bearing round-trip: the core call must still pass `&mut record_core` AND the tail
        // must hand back `record_core.into()`.
        assert!(
            output.contains("sample_core::tag_record(&mut record_core)"),
            "expected the core call to still pass `&mut record_core`:\n{output}"
        );
        assert!(
            output.contains("record_core.into()"),
            "expected the mutated intermediate to be returned:\n{output}"
        );
    }

    /// Negative control for issue #380: an immutable `&T` DTO param must not gain write-back
    /// semantics -- the return type must stay `()`.
    #[test]
    fn napi_immutable_dto_param_keeps_unit_return() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "read_record".to_owned(),
            rust_path: "sample_core::read_record".to_owned(),
            params: vec![record_param(true, false)],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(
            output.contains("-> ()"),
            "immutable borrow must keep unit return:\n{output}"
        );
        assert!(
            !output.contains("record_core.into()"),
            "immutable borrow must not gain a write-back tail:\n{output}"
        );
    }

    /// Negative control for issue #380: an owned `T` DTO param must render unaffected by the
    /// write-back rewrite.
    #[test]
    fn napi_owned_dto_param_unaffected_by_writeback() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "consume_record".to_owned(),
            rust_path: "sample_core::consume_record".to_owned(),
            params: vec![record_param(false, false)],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(output.contains("-> ()"), "owned param must keep unit return:\n{output}");
        assert!(
            !output.contains("record_core.into()"),
            "owned param must not gain a write-back tail:\n{output}"
        );
    }

    #[test]
    fn required_parameter_after_optional_returns_invalid_arg_instead_of_panicking() {
        use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

        let func = FunctionDef {
            name: "bounded".to_owned(),
            rust_path: "sample_core::bounded".to_owned(),
            params: vec![
                ParamDef {
                    name: "encoding".into(),
                    ty: TypeRef::String,
                    optional: true,
                    ..Default::default()
                },
                ParamDef {
                    name: "max".into(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Unit,
            ..Default::default()
        };

        let output = gen_probe_function(&func);
        assert!(
            output.contains("max.ok_or_else(|| napi::Error::new(napi::Status::InvalidArg"),
            "{output}"
        );
        assert!(output.contains("-> Result<()>"), "{output}");
        assert!(!output.contains("expect(\"'max' is required\")"), "{output}");
    }

    #[test]
    fn async_named_and_vec_required_parameters_after_optional_are_unwrapped_once() {
        use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

        let func = FunctionDef {
            name: "configure".to_owned(),
            rust_path: "sample_core::configure".to_owned(),
            params: vec![
                ParamDef {
                    name: "label".into(),
                    ty: TypeRef::String,
                    optional: true,
                    ..Default::default()
                },
                ParamDef {
                    name: "record".into(),
                    ty: TypeRef::Named("Record".into()),
                    ..Default::default()
                },
                ParamDef {
                    name: "records".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Record".into()))),
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("Record".into()),
            is_async: true,
            ..Default::default()
        };

        let output = gen_probe_function(&func);
        assert!(
            output.contains("record: Option<JsRecord>, records: Option<Vec<JsRecord>>"),
            "{output}"
        );
        assert!(output.contains("record.ok_or_else"), "{output}");
        assert!(output.contains("records.ok_or_else"), "{output}");
        assert!(
            output.contains("let record_core: sample_core::Record = record.into();"),
            "{output}"
        );
        assert!(
            output.contains("let records_core: Vec<_> = records.into_iter().map(Into::into).collect();"),
            "{output}"
        );
        assert!(!output.contains(".expect("), "{output}");
        assert!(output.contains("-> Result<JsRecord>"), "{output}");
    }

    /// Regression test for issue #380 (async path): a `&mut T` DTO parameter on a unit-returning
    /// `async fn` previously rendered as `pub async fn tag_record_async(record: JsRecord) -> ()`
    /// with a body that mutated a dropped `_core` intermediate and then returned `Ok(())` -- the
    /// future resolved to nothing and the caller's JS object was left untouched with no
    /// diagnostic. The binding must instead resolve to the mutated intermediate.
    #[test]
    fn napi_async_mut_dto_param_returns_the_updated_value() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "tag_record_async".to_owned(),
            rust_path: "sample_core::tag_record_async".to_owned(),
            params: vec![record_param(true, true)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(
            output.contains("-> JsRecord"),
            "expected the binding to return the mutated DTO type instead of `()`:\n{output}"
        );
        assert!(
            !output.contains("-> ()"),
            "must not still advertise a unit return:\n{output}"
        );
        // Load-bearing round-trip: the core call must still `.await` while passing
        // `&mut record_core`, AND the tail must hand back `record_core.into()`.
        assert!(
            output.contains("sample_core::tag_record_async(&mut record_core)"),
            "expected the core call to still pass `&mut record_core`:\n{output}"
        );
        assert!(
            output.contains(".await"),
            "expected the core call to still be awaited:\n{output}"
        );
        assert!(
            output.contains("record_core.into()"),
            "expected the mutated intermediate to be returned:\n{output}"
        );
        assert!(
            !output.contains("Ok(())"),
            "must not resolve the future to unit and drop the mutated value:\n{output}"
        );
    }

    /// Async write-back must also work when the core function returns `Result<(), E>`: the
    /// binding must map the `Ok(())` to `Ok(record_core.into())` rather than discarding it.
    #[test]
    fn napi_async_mut_dto_param_with_error_returns_the_updated_value() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "tag_record_async".to_owned(),
            rust_path: "sample_core::tag_record_async".to_owned(),
            params: vec![record_param(true, true)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: Some("ProbeError".to_owned()),
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(
            output.contains("Result<JsRecord>"),
            "expected a fallible write-back to return Result<JsRecord>:\n{output}"
        );
        assert!(
            output.contains("sample_core::tag_record_async(&mut record_core)"),
            "expected the core call to still pass `&mut record_core`:\n{output}"
        );
        assert!(
            output.contains(".map(|_| record_core.into())"),
            "expected the Result<(), E> to be mapped into the mutated intermediate:\n{output}"
        );
    }

    /// Negative control: an async immutable `&T` DTO param must not gain write-back semantics.
    #[test]
    fn napi_async_immutable_dto_param_keeps_unit_return() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "read_record_async".to_owned(),
            rust_path: "sample_core::read_record_async".to_owned(),
            params: vec![record_param(true, false)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(
            !output.contains("record_core.into()"),
            "immutable borrow must not gain a write-back tail:\n{output}"
        );
    }

    /// Negative control: an async owned `T` DTO param must render unaffected by write-back.
    #[test]
    fn napi_async_owned_dto_param_unaffected_by_writeback() {
        use crate::core::ir::{FunctionDef, TypeRef};

        let func = FunctionDef {
            name: "consume_record_async".to_owned(),
            rust_path: "sample_core::consume_record_async".to_owned(),
            params: vec![record_param(false, false)],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: None,
            ..FunctionDef::default()
        };

        let output = gen_probe_function(&func);

        assert!(
            !output.contains("record_core.into()"),
            "owned param must not gain a write-back tail:\n{output}"
        );
    }

    #[test]
    fn napi_transparent_string_owned_optional_and_map_calls_round_trip() {
        use crate::core::ir::{FunctionDef, NewtypeContainer, ParamDef, TypeRef};

        let cases = [
            (
                "owned",
                TypeRef::String,
                false,
                Vec::new(),
                TypeRef::String,
                "sample_core::SecretString::from(value)",
            ),
            (
                "optional",
                TypeRef::String,
                true,
                vec![NewtypeContainer::Optional],
                TypeRef::Optional(Box::new(TypeRef::String)),
                "sample_core::SecretString::from",
            ),
            (
                "map",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                true,
                vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
                TypeRef::Optional(Box::new(TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::String),
                ))),
                "sample_core::SecretString::from(value)",
            ),
        ];

        for (name, ty, optional, containers, return_type, call_fragment) in cases {
            let wrapper = transparent_string_wrapper(containers);
            let output = gen_probe_function(&FunctionDef {
                name: format!("round_trip_{name}"),
                rust_path: format!("sample_core::round_trip_{name}"),
                params: vec![ParamDef {
                    name: "value".to_owned(),
                    ty: ty.clone(),
                    optional,
                    newtype_wrapper: Some(wrapper.clone()),
                    ..ParamDef::default()
                }],
                return_type,
                return_newtype_wrapper: Some(wrapper),
                ..FunctionDef::default()
            });

            assert!(
                output.contains(call_fragment),
                "{name} input wrapper was not constructed:\n{output}"
            );
            assert!(
                output.contains("into_inner()"),
                "{name} output wrapper was not consumed:\n{output}"
            );
            assert!(
                !output.contains("to_string()"),
                "{name} must not require Display:\n{output}"
            );
        }
    }

    #[test]
    fn napi_transparent_string_borrows_use_stable_wrapper_locals() {
        use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

        for (name, is_mut, declaration, call) in [
            ("borrow", false, "let value_newtype", "&value_newtype"),
            ("borrow_mut", true, "let mut value_newtype", "&mut value_newtype"),
        ] {
            let output = gen_probe_function(&FunctionDef {
                name: name.to_owned(),
                rust_path: format!("sample_core::{name}"),
                params: vec![ParamDef {
                    name: "value".to_owned(),
                    ty: TypeRef::String,
                    is_ref: true,
                    is_mut,
                    newtype_wrapper: Some(transparent_string_wrapper(Vec::new())),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Unit,
                ..FunctionDef::default()
            });

            let declaration_index = output.find(declaration).expect("wrapper local declaration");
            let call_index = output.find(call).expect("wrapper local borrow");
            assert!(
                declaration_index < call_index,
                "wrapper local must be declared before it is borrowed:\n{output}"
            );
        }
    }
}
