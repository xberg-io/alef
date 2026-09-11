use super::{
    OptionsFieldBridges, is_python_builtin_name, python_safe_name, qualify_parameter_type, substitute_capsule_type,
};
use crate::backends::pyo3::type_map::python_type;
use crate::core::ir::{FunctionDef, TypeRef};

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_function_stub(
    func: &FunctionDef,
    bridge_param_names: &std::collections::HashSet<&str>,
    capsule_names: &std::collections::HashSet<&str>,
    options_field_bridges: &OptionsFieldBridges<'_>,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
    opaque_types: &ahash::AHashSet<String>,
) -> String {
    // The native module has no way to synthesize a `Default`, so the stub grants no extra
    // defaults; parameter existence and order still come from the shared decision so this stub
    // and the `api.py` facade cannot drift apart. ~keep
    let mut params: Vec<String> = crate::backends::pyo3::py_signature::python_signature_params(&func.params, |_| false)
        .into_iter()
        .map(|entry| {
            let (p, optional) = (entry.param, entry.defaulted);
            let type_str = if bridge_param_names.contains(p.name.as_str()) {
                "object".to_string()
            } else {
                substitute_capsule_type(&python_type(&p.ty), capsule_names)
            };
            let type_str = qualify_parameter_type(&p.name, &type_str);
            if optional {
                let param_type = if type_str.ends_with("| None") {
                    type_str
                } else {
                    format!("{type_str} | None")
                };
                format!("{}: {} = None", p.name, param_type)
            } else {
                format!("{}: {}", p.name, type_str)
            }
        })
        .collect();

    let bridge_kwarg = func.params.iter().find_map(|p| {
        let type_name = match &p.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        }?;
        let (kwarg_name, type_alias, trait_name) = options_field_bridges.get(type_name)?;
        Some((*kwarg_name, *type_alias, *trait_name))
    });
    if let Some((kwarg_name, type_alias, trait_name)) = bridge_kwarg {
        let visitor_type = trait_name.or(type_alias).unwrap_or("object");
        params.push(format!("{kwarg_name}: {visitor_type} | object | None = None"));
    }

    let streaming_key = (None::<String>, func.name.clone());
    let is_streaming = streaming_return_types.contains_key(&streaming_key);
    // The stub must document the signature the binding actually emits: a `&mut T` DTO parameter
    // on a unit-returning sync function makes the binding return the updated `T` instead of
    // `None` (see `codegen::mut_writeback` and `codegen::generators::functions::gen_function`). ~keep
    let writeback_return_type = if !func.is_async {
        crate::codegen::mut_writeback::effective_return_type(&func.params, &func.return_type, opaque_types)
    } else {
        None
    };
    let stub_return_type = writeback_return_type.as_ref().unwrap_or(&func.return_type);
    let return_type = if let Some(item_type) = streaming_return_types.get(&streaming_key) {
        format!("AsyncIterator[{item_type}]")
    } else {
        substitute_capsule_type(&python_type(stub_return_type), capsule_names)
    };
    let safe_name = python_safe_name(&func.name);
    // See the identical fix/comment in `gen_stubs/classes.rs::gen_method_stub`: a streaming
    // free function is an async GENERATOR under the hood (call returns the AsyncIterator
    // synchronously), so the stub must stay `def`, not `async def`, regardless of
    // `func.is_async`. ~keep
    let def_kw = if func.is_async && !is_streaming {
        "async def"
    } else {
        "def"
    };

    let has_builtin_param = params
        .iter()
        .any(|p| is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));
    let single_line = format!(
        "{} {}({}) -> {}: ...",
        def_kw,
        safe_name,
        params.join(", "),
        return_type
    );
    if single_line.len() <= 100 && !has_builtin_param {
        single_line
    } else {
        let mut wrapped = format!("{} {}(\n", def_kw, safe_name);
        for param in &params {
            let name = param.split(':').next().unwrap_or("").trim();
            if is_python_builtin_name(name) {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_wrapped_noqa.jinja",
                    minijinja::context! { param => param, indent => "    " },
                ));
            } else {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_wrapped.jinja",
                    minijinja::context! { param => param, indent => "    " },
                ));
            }
        }
        wrapped.push_str(&crate::backends::pyo3::template_env::render(
            "stub_method_signature_end.jinja",
            minijinja::context! { return_type => &return_type },
        ));
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ParamDef;

    #[test]
    fn free_function_parameter_named_bytes_qualifies_its_builtin_annotation() {
        let function = FunctionDef {
            name: "decode".to_string(),
            params: vec![ParamDef {
                name: "bytes".to_string(),
                ty: TypeRef::Bytes,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            ..Default::default()
        };

        let stub = gen_function_stub(
            &function,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &OptionsFieldBridges::default(),
            &std::collections::HashMap::new(),
            &ahash::AHashSet::new(),
        );

        assert!(stub.contains("bytes: builtins.bytes"), "{stub}");
        assert!(!stub.contains("bytes: bytes"), "{stub}");
    }

    /// Free-function counterpart of `classes::tests::
    /// streaming_method_stub_keeps_a_plain_def_keyword_despite_method_is_async`: a streaming
    /// free function's stub must keep a plain `def`, not `async def`, for the identical reason
    /// — its real wrapper (`adapter_streaming_wrapper.jinja`) is an async generator whose CALL
    /// returns the `AsyncIterator` synchronously. ~keep
    #[test]
    fn streaming_free_function_stub_keeps_a_plain_def_keyword_despite_func_is_async() {
        let func = FunctionDef {
            name: "watch_events".to_string(),
            return_type: TypeRef::Named("Ignored".to_string()),
            is_async: true,
            ..FunctionDef::default()
        };
        let mut streaming_return_types = std::collections::HashMap::new();
        streaming_return_types.insert((None, "watch_events".to_string()), "Event".to_string());

        let stub = gen_function_stub(
            &func,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &OptionsFieldBridges::default(),
            &streaming_return_types,
            &ahash::AHashSet::new(),
        );

        assert!(
            stub.trim_start().starts_with("def watch_events("),
            "a streaming free function's call is synchronous; the stub must keep a plain \
             `def`, not `async def`, got:\n{stub}"
        );
        assert!(
            stub.contains("-> AsyncIterator[Event]"),
            "the streaming return type must still be AsyncIterator[Item]:\n{stub}"
        );
    }

    /// Negative control: a non-streaming async free function must keep `async def`.
    #[test]
    fn a_non_streaming_async_free_function_stub_keeps_async_def() {
        let func = FunctionDef {
            name: "fetch_status".to_string(),
            return_type: TypeRef::Named("Ignored".to_string()),
            is_async: true,
            ..FunctionDef::default()
        };

        let stub = gen_function_stub(
            &func,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &OptionsFieldBridges::default(),
            &std::collections::HashMap::new(),
            &ahash::AHashSet::new(),
        );

        assert!(
            stub.trim_start().starts_with("async def fetch_status("),
            "a non-streaming async function must keep `async def`, got:\n{stub}"
        );
    }

    /// Regression test for issue #380: the `.pyi` stub must document the signature the binding
    /// actually emits. A `&mut T` DTO parameter on a unit-returning sync function makes the
    /// binding return the updated `T`, so the stub must say `-> Record`, not `-> None`.
    #[test]
    fn mut_dto_param_stub_returns_the_updated_dto_type() {
        use crate::core::ir::ParamDef;

        let func = FunctionDef {
            name: "tag_record".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        };

        let stub = gen_function_stub(
            &func,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &OptionsFieldBridges::default(),
            &std::collections::HashMap::new(),
            &ahash::AHashSet::new(),
        );

        assert_eq!(
            stub, "def tag_record(record: Record) -> Record: ...",
            "the stub must return the mutated DTO type instead of `None`, got:\n{stub}"
        );
    }

    /// Negative control for issue #380: an immutable `&T` DTO param must not gain write-back
    /// semantics in the stub -- the return stays `None`.
    #[test]
    fn immutable_dto_param_stub_keeps_none_return() {
        use crate::core::ir::ParamDef;

        let func = FunctionDef {
            name: "read_record".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: false,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        };

        let stub = gen_function_stub(
            &func,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &OptionsFieldBridges::default(),
            &std::collections::HashMap::new(),
            &ahash::AHashSet::new(),
        );

        assert_eq!(
            stub, "def read_record(record: Record) -> None: ...",
            "immutable borrow must keep a `None` return, got:\n{stub}"
        );
    }
}
