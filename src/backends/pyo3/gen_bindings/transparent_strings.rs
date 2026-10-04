use crate::codegen::generators::{AdapterBodies, RustBindingConfig};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, NewtypeWrapper, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn has_explicit_paths(wrapper: &str) -> bool {
    NewtypeWrapper::decode(wrapper).is_ok_and(|decoded| !decoded.explicit_paths().is_empty())
}

fn converted_param(param: &ParamDef) -> Option<String> {
    let wrapper = param.newtype_wrapper.as_deref()?;
    if !has_explicit_paths(wrapper) {
        return None;
    }
    let converted = crate::codegen::conversions::helpers::apply_field_newtype_to_core(
        &param.name,
        &param.ty,
        param.optional,
        wrapper,
    );
    Some(if param.is_ref && param.optional {
        format!("({converted}).as_ref()")
    } else if param.is_ref && param.is_mut {
        format!("&mut {converted}")
    } else if param.is_ref {
        format!("&{converted}")
    } else {
        converted
    })
}

fn call_args(params: &[ParamDef], opaque_types: &ahash::AHashSet<String>) -> String {
    params
        .iter()
        .map(|param| {
            converted_param(param).unwrap_or_else(|| {
                crate::codegen::generators::binding_helpers::gen_call_args(std::slice::from_ref(param), opaque_types)
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
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
    let unwrapped =
        crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap(expr, &method.return_newtype_wrapper);
    crate::codegen::generators::binding_helpers::wrap_return_with_mutex_mapped(
        &unwrapped,
        &method.return_type,
        &typ.name,
        opaque_types,
        &ahash::AHashSet::new(),
        false,
        method.returns_ref,
        method.returns_cow,
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
    let shared_ref_mut_owns_writeback =
        matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none();
    if typ.is_opaque
        || method.sanitized
        || method.params.iter().any(|param| param.sanitized)
        || shared_ref_mut_owns_writeback
    {
        return None;
    }
    if !method.params.iter().any(|param| converted_param(param).is_some()) {
        return None;
    }
    let field_conversions = core_self_conversion(typ, method, core_import, opaque_types);
    let receiver = if method.is_static {
        typ.rust_path.replace('-', "_")
    } else {
        "core_self".to_string()
    };
    let args = call_args(&method.params, opaque_types);
    let call = if method.is_static {
        format!("{receiver}::{}({args})", method.name)
    } else {
        format!("{receiver}.{}({args})", method.name)
    };
    let converted_result = converted_return("result", typ, method, opaque_types, mapper);
    if method.is_async {
        let return_type = mapper.map_type(&method.return_type);
        return Some(crate::codegen::generators::binding_helpers::gen_async_body(
            &call,
            cfg,
            method.error_type.is_some(),
            &converted_result,
            false,
            &field_conversions,
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        ));
    }
    if method.error_type.is_some() {
        let error_conversion = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?";
        if matches!(method.return_type, TypeRef::Unit) {
            return Some(format!("{field_conversions}{call}{error_conversion};\n        Ok(())"));
        }
        return Some(format!(
            "{field_conversions}let result = {call}{error_conversion};\n        Ok({converted_result})"
        ));
    }
    let converted_call = converted_return(&call, typ, method, opaque_types, mapper);
    Some(format!("{field_conversions}{converted_call}"))
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
            converted_param(&method.params[0]).as_deref(),
            Some(
                "((value).map(|value| (value).into_iter().map(|(key, value)| (key, toolkit::SecretString::from(value))).collect())).as_ref()"
            )
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
            converted_param(&param).as_deref(),
            Some("&toolkit::SecretString::from(value)")
        );
    }

    #[test]
    fn adapter_preserves_plain_fallible_and_async_return_handling() {
        let (typ, method) = optional_map_method();
        let plain = adapter(&typ, &method).expect("plain adapter");
        assert!(plain.ends_with(".as_ref())"), "{plain}");
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
    fn adapter_rejects_sanitized_methods_and_params() {
        let (typ, mut method) = optional_map_method();
        method.sanitized = true;
        assert_eq!(adapter(&typ, &method), None);
        method.sanitized = false;
        method.params[0].sanitized = true;
        assert_eq!(adapter(&typ, &method), None);
    }

    #[test]
    fn adapter_leaves_non_trait_ref_mut_to_shared_writeback() {
        let (typ, mut method) = optional_map_method();
        method.receiver = Some(ReceiverKind::RefMut);
        assert_eq!(adapter(&typ, &method), None);
    }
}
