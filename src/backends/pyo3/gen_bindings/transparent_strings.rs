use crate::codegen::generators::{AdapterBodies, RustBindingConfig};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, NewtypeWrapper, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn has_explicit_paths(wrapper: &str) -> bool {
    NewtypeWrapper::decode(wrapper).is_ok_and(|decoded| !decoded.explicit_paths().is_empty())
}

fn converted_param(param: &ParamDef, promoted: bool, index: usize) -> Option<(String, String)> {
    let wrapper = param.newtype_wrapper.as_deref()?;
    if !has_explicit_paths(wrapper) {
        return None;
    }
    let source = if promoted {
        format!("{}.expect(\"'{}' is required\")", param.name, param.name)
    } else {
        param.name.clone()
    };
    let converted = crate::codegen::conversions::helpers::apply_field_newtype_to_core(
        &source,
        &param.ty,
        param.optional && !promoted,
        wrapper,
    );
    if !param.is_ref {
        return Some((String::new(), converted));
    }
    let local = format!("__alef_wrapper_arg_{index}");
    let mutable = if param.is_mut { "mut " } else { "" };
    let binding = format!("let {mutable}{local} = {converted};\n        ");
    let argument = match (param.optional && !promoted, param.is_mut) {
        (true, true) => format!("{local}.as_mut()"),
        (true, false) => format!("{local}.as_ref()"),
        (false, true) => format!("&mut {local}"),
        (false, false) => format!("&{local}"),
    };
    Some((binding, argument))
}

fn converted_call_args(params: &[ParamDef], opaque_types: &ahash::AHashSet<String>) -> (String, String) {
    let fallback = crate::codegen::generators::binding_helpers::gen_call_args_vec(params, opaque_types);
    let converted: Vec<_> = params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let promoted = crate::codegen::shared::is_promoted_optional(params, index);
            converted_param(param, promoted, index).unwrap_or_else(|| (String::new(), fallback[index].clone()))
        })
        .collect();
    let bindings: String = converted.iter().map(|(binding, _)| binding.as_str()).collect();
    let args = converted
        .into_iter()
        .map(|(_, argument)| argument)
        .collect::<Vec<_>>()
        .join(", ");
    (bindings, args)
}

fn core_self_conversion(
    typ: &TypeDef,
    method: &MethodDef,
    core_import: &str,
    opaque_types: &ahash::AHashSet<String>,
) -> String {
    if method.is_static {
        return String::new();
    }
    if matches!(method.receiver, Some(ReceiverKind::RefMut)) {
        crate::codegen::generators::binding_helpers::gen_lossy_binding_to_core_fields_mut(
            typ,
            core_import,
            true,
            opaque_types,
            false,
            false,
            &[],
        )
    } else {
        crate::codegen::generators::binding_helpers::gen_lossy_binding_to_core_fields(
            typ,
            core_import,
            true,
            opaque_types,
            false,
            false,
            &[],
        )
    }
}

fn converted_return(
    expr: &str,
    typ: &TypeDef,
    method: &MethodDef,
    opaque_types: &ahash::AHashSet<String>,
    mapper: &dyn TypeMapper,
) -> String {
    let owned = if method.returns_cow {
        format!("({expr}).into_owned()")
    } else if method.returns_ref {
        format!("({expr}).clone()")
    } else {
        expr.to_string()
    };
    let unwrapped = crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap(
        &owned,
        &method.return_newtype_wrapper,
    );
    crate::codegen::generators::binding_helpers::wrap_return_with_mutex_mapped(
        &unwrapped,
        &method.return_type,
        &typ.name,
        opaque_types,
        &ahash::AHashSet::new(),
        false,
        false,
        false,
        mapper,
    )
}

fn method_adapter(
    typ: &TypeDef,
    method: &MethodDef,
    core_import: &str,
    opaque_types: &ahash::AHashSet<String>,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
) -> Option<String> {
    let unsupported_async_writeback =
        method.is_async && matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none();
    if typ.is_opaque
        || method.sanitized
        || method.params.iter().any(|param| param.sanitized)
        || unsupported_async_writeback
    {
        return None;
    }
    if !method
        .params
        .iter()
        .any(|param| param.newtype_wrapper.as_deref().is_some_and(has_explicit_paths))
    {
        return None;
    }
    let field_conversions = core_self_conversion(typ, method, core_import, opaque_types);
    let receiver = if method.is_static {
        typ.rust_path.replace('-', "_")
    } else {
        "core_self".to_string()
    };
    let (argument_bindings, args) = converted_call_args(&method.params, opaque_types);
    let call = if method.is_static {
        format!("{receiver}::{}({args})", method.name)
    } else {
        format!("{receiver}.{}({args})", method.name)
    };
    let converted_result = converted_return("result", typ, method, opaque_types, mapper);
    let functional_ref_mut =
        matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none() && !method.is_async;
    if functional_ref_mut {
        let error_conversion = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?";
        return Some(if method.error_type.is_some() {
            format!("{field_conversions}{argument_bindings}{call}{error_conversion};\n        Ok(core_self.into())")
        } else {
            format!("{field_conversions}{argument_bindings}{call};\n        core_self.into()")
        });
    }
    if method.is_async {
        let return_type = mapper.map_type(&method.return_type);
        return Some(crate::codegen::generators::binding_helpers::gen_async_body(
            &call,
            cfg,
            method.error_type.is_some(),
            &converted_result,
            false,
            &format!("{field_conversions}{argument_bindings}"),
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        ));
    }
    if method.error_type.is_some() {
        let error_conversion = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?";
        if matches!(method.return_type, TypeRef::Unit) {
            return Some(format!(
                "{field_conversions}{argument_bindings}{call}{error_conversion};\n        Ok(())"
            ));
        }
        return Some(format!(
            "{field_conversions}{argument_bindings}let result = {call}{error_conversion};\n        Ok({converted_result})"
        ));
    }
    let converted_call = converted_return(&call, typ, method, opaque_types, mapper);
    Some(format!("{field_conversions}{argument_bindings}{converted_call}"))
}

pub(super) fn add_adapters(
    adapters: &mut AdapterBodies,
    api: &crate::core::ir::ApiSurface,
    core_import: &str,
    opaque_types: &ahash::AHashSet<String>,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
) {
    for typ in &api.types {
        for method in &typ.methods {
            if let Some(body) = method_adapter(typ, method, core_import, opaque_types, mapper, cfg) {
                adapters.entry(format!("{}.{}", typ.name, method.name)).or_insert(body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::pyo3::type_map::Pyo3Mapper;
    use crate::core::ir::{NewtypeContainer, NewtypeWrapperMetadata, PrimitiveType};

    fn wrapper(containers: Vec<NewtypeContainer>) -> String {
        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "toolkit::SecretString",
            "from",
            "into_inner",
            containers,
        )])
    }

    fn adapter(typ: &TypeDef, method: &MethodDef) -> Option<String> {
        let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("toolkit", false);
        method_adapter(
            typ,
            method,
            "toolkit",
            &ahash::AHashSet::new(),
            &Pyo3Mapper::default(),
            &cfg,
        )
    }

    fn optional_map_method() -> (TypeDef, MethodDef) {
        let method = MethodDef {
            name: "optional_secret_map".to_string(),
            params: vec![ParamDef {
                name: "value".to_string(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                optional: true,
                is_ref: true,
                newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue])),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        };
        let typ = TypeDef {
            name: "Segment".to_string(),
            rust_path: "toolkit::Segment".to_string(),
            methods: vec![method.clone()],
            ..TypeDef::default()
        };
        (typ, method)
    }

    #[test]
    fn optional_map_param_borrows_after_explicit_wrapper_conversion() {
        let (_, method) = optional_map_method();
        assert_eq!(
            converted_param(&method.params[0], false, 0),
            Some((
                "let __alef_wrapper_arg_0 = (value).map(|value| (value).into_iter().map(|(key, value)| (key, toolkit::SecretString::from(value))).collect());\n        ".to_string(),
                "__alef_wrapper_arg_0.as_ref()".to_string()
            ))
        );
    }

    #[test]
    fn scalar_wrapper_param_borrows_the_constructed_value() {
        let param = ParamDef {
            name: "value".to_string(),
            ty: TypeRef::String,
            is_ref: true,
            newtype_wrapper: Some(wrapper(vec![])),
            ..ParamDef::default()
        };
        assert_eq!(
            converted_param(&param, false, 0),
            Some((
                "let __alef_wrapper_arg_0 = toolkit::SecretString::from(value);\n        ".to_string(),
                "&__alef_wrapper_arg_0".to_string()
            ))
        );
    }

    #[test]
    fn promoted_wrapper_param_unwraps_the_required_binding_value() {
        let params = vec![
            ParamDef {
                name: "optional".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "required".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                newtype_wrapper: Some(wrapper(vec![])),
                ..ParamDef::default()
            },
        ];
        let (bindings, args) = converted_call_args(&params, &ahash::AHashSet::new());
        assert!(
            bindings.contains("SecretString::from(required.expect(\"'required' is required\"))"),
            "{bindings}"
        );
        assert_eq!(args, "optional, &__alef_wrapper_arg_1");
    }

    #[test]
    fn adapter_preserves_plain_fallible_and_async_return_handling() {
        let (typ, method) = optional_map_method();
        let plain = adapter(&typ, &method).expect("plain adapter");
        assert!(plain.contains("let __alef_wrapper_arg_0"), "{plain}");
        assert!(plain.ends_with("__alef_wrapper_arg_0.as_ref())"), "{plain}");
        let mut fallible = method.clone();
        fallible.error_type = Some("toolkit::Error".to_string());
        let fallible = adapter(&typ, &fallible).expect("fallible adapter");
        assert!(fallible.contains("PyRuntimeError::new_err"), "{fallible}");
        assert!(fallible.ends_with("Ok(result)"), "{fallible}");
        let mut asynchronous = method;
        asynchronous.is_async = true;
        let asynchronous = adapter(&typ, &asynchronous).expect("async adapter");
        assert!(asynchronous.contains("future_into_py(py, async move"), "{asynchronous}");
        assert!(asynchronous.contains(".await;"), "{asynchronous}");
    }

    #[test]
    fn adapter_unwraps_explicit_wrapper_returns() {
        let (typ, mut method) = optional_map_method();
        method.return_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        method.return_newtype_wrapper = Some(wrapper(vec![NewtypeContainer::MapValue]));
        let output = adapter(&typ, &method).expect("wrapper-return adapter");
        assert!(output.contains("(value).into_inner()"), "{output}");
    }

    #[test]
    fn adapter_materializes_borrowed_wrapper_returns_before_unwrapping() {
        let (typ, mut method) = optional_map_method();
        method.return_type = TypeRef::String;
        method.return_newtype_wrapper = Some(wrapper(vec![]));
        method.returns_ref = true;
        let output = adapter(&typ, &method).expect("borrowed wrapper-return adapter");
        assert!(output.contains("(core_self.optional_secret_map("), "{output}");
        assert!(output.contains(").clone()).into_inner()"), "{output}");
        method.returns_ref = false;
        method.returns_cow = true;
        let output = adapter(&typ, &method).expect("cow wrapper-return adapter");
        assert!(output.contains(").into_owned()).into_inner()"), "{output}");
    }

    #[test]
    fn adapter_uses_the_core_type_path_for_static_methods() {
        let (typ, mut method) = optional_map_method();
        method.is_static = true;
        method.receiver = None;
        let output = adapter(&typ, &method).expect("static adapter");
        assert!(output.contains("toolkit::Segment::optional_secret_map"), "{output}");
        assert!(!output.contains("let core_self"), "{output}");
    }

    #[test]
    fn adapter_rejects_sanitized_methods_and_params() {
        let (typ, mut method) = optional_map_method();
        method.sanitized = true;
        assert_eq!(adapter(&typ, &method), None);
        method.sanitized = false;
        method.params[0].sanitized = true;
        assert_eq!(adapter(&typ, &method), None);
    }

    #[test]
    fn adapter_preserves_non_trait_ref_mut_writeback_with_stable_wrapper_locals() {
        let (typ, mut method) = optional_map_method();
        method.receiver = Some(ReceiverKind::RefMut);
        method.params[0].is_mut = true;
        let output = adapter(&typ, &method).expect("functional ref-mut adapter");
        assert!(output.contains("let mut __alef_wrapper_arg_0"), "{output}");
        assert!(output.contains("__alef_wrapper_arg_0.as_mut()"), "{output}");
        assert!(output.ends_with("core_self.into()"), "{output}");
    }
}
