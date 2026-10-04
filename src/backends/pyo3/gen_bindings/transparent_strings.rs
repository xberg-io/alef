use crate::codegen::generators::{AdapterBodies, RustBindingConfig};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{
    FieldDef, MethodDef, NewtypeContainer, NewtypeWrapper, ParamDef, ReceiverKind, TypeDef, TypeRef,
};

fn has_explicit_paths(wrapper: &str) -> bool {
    NewtypeWrapper::decode(wrapper).is_ok_and(|decoded| !decoded.explicit_paths().is_empty())
}

fn flattened_optional_wrapper(wrapper: &str) -> String {
    let NewtypeWrapper::Explicit(mut paths) =
        NewtypeWrapper::decode(wrapper).expect("newtype metadata must be validated before PyO3 codegen")
    else {
        return wrapper.to_string();
    };
    for path in &mut paths {
        if path.containers.first() == Some(&NewtypeContainer::Optional) {
            path.containers.remove(0);
        }
    }
    NewtypeWrapper::encode_explicit(&paths)
}

fn contains_named(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_named(inner),
        TypeRef::Map(key, value) => contains_named(key) || contains_named(value),
        _ => false,
    }
}

fn convert_named_leaves(expr: &str, ty: &TypeRef) -> String {
    if !contains_named(ty) {
        return expr.to_string();
    }
    match ty {
        TypeRef::Named(_) => format!("({expr}).into()"),
        TypeRef::Optional(inner) => {
            let converted = convert_named_leaves("value", inner);
            format!("({expr}).map(|value| {converted})")
        }
        TypeRef::Vec(inner) => {
            let converted = convert_named_leaves("value", inner);
            format!("({expr}).into_iter().map(|value| {converted}).collect()")
        }
        TypeRef::Map(key, value) => {
            let converted_key = convert_named_leaves("key", key);
            let converted_value = convert_named_leaves("value", value);
            format!("({expr}).into_iter().map(|(key, value)| ({converted_key}, {converted_value})).collect()")
        }
        _ => expr.to_string(),
    }
}

fn explicit_field_wrapper(field: &FieldDef) -> Option<&str> {
    field
        .newtype_wrapper
        .as_deref()
        .filter(|wrapper| has_explicit_paths(wrapper))
}

fn binding_to_core_field_expr(field: &FieldDef, binding_name: &str) -> Option<String> {
    let wrapper = explicit_field_wrapper(field)?;
    let source = format!("val.{binding_name}");
    let flattened = field.optional && matches!(field.ty, TypeRef::Optional(_));
    if !flattened {
        return None;
    }
    let adjusted_wrapper = flattened_optional_wrapper(wrapper);
    let converted =
        crate::codegen::conversions::helpers::apply_field_newtype_to_core(&source, &field.ty, false, &adjusted_wrapper);
    let converted = convert_named_leaves(&converted, &field.ty);
    Some(format!("({converted}).map(Some)"))
}

fn core_to_binding_field_expr(field: &FieldDef) -> Option<String> {
    let wrapper = explicit_field_wrapper(field)?;
    let flattened = field.optional && matches!(field.ty, TypeRef::Optional(_));
    if !flattened {
        return None;
    }
    let source = format!("val.{}.flatten()", field.name);
    let adjusted_wrapper = flattened_optional_wrapper(wrapper);
    let converted = crate::codegen::conversions::helpers::apply_field_newtype_from_core(
        &source,
        &field.ty,
        false,
        &adjusted_wrapper,
    );
    Some(convert_named_leaves(&converted, &field.ty))
}

fn expression_terminator(source: &str, terminator: char) -> Option<usize> {
    let mut delimiters = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '[' | '{' | '<' => delimiters.push(ch),
            ')' | ']' | '}' | '>' => {
                delimiters.pop()?;
            }
            _ if ch == terminator && delimiters.is_empty() => return Some(index),
            _ => {}
        }
    }
    None
}

fn replace_field_conversion(generated: &str, field_name: &str, expr: &str) -> Option<String> {
    let initializer_prefix = format!("{field_name}: ");
    let assignment_prefix = format!("__result.{field_name} = ");
    let initializers: Vec<_> = generated
        .match_indices(&initializer_prefix)
        .map(|(index, _)| index)
        .collect();
    let assignments: Vec<_> = generated
        .match_indices(&assignment_prefix)
        .map(|(index, _)| index)
        .collect();
    let (start, prefix, terminator) = match (initializers.as_slice(), assignments.as_slice()) {
        ([start], []) => (*start, initializer_prefix, ','),
        ([], [start]) => (*start, assignment_prefix, ';'),
        _ => return None,
    };
    let expression_start = start + prefix.len();
    let relative_end = expression_terminator(&generated[expression_start..], terminator)?;
    let end = expression_start + relative_end + terminator.len_utf8();
    let mut output = generated.to_string();
    output.replace_range(start..end, &format!("{prefix}{expr}{terminator}"));
    Some(output)
}

pub(super) fn rewrite_binding_to_core_fields(
    mut generated: String,
    typ: &TypeDef,
    config: &crate::codegen::conversions::ConversionConfig<'_>,
) -> String {
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let binding_name = config.binding_field_name_owned(&typ.name, &field.name);
        if let Some(expr) = binding_to_core_field_expr(field, &binding_name) {
            generated = replace_field_conversion(&generated, &field.name, &expr).unwrap_or_else(|| {
                panic!(
                    "PyO3 explicit-wrapper binding-to-core conversion for {}.{} must replace exactly one field",
                    typ.name, field.name
                )
            });
        }
    }
    generated
}

pub(super) fn rewrite_core_to_binding_fields(
    mut generated: String,
    typ: &TypeDef,
    config: &crate::codegen::conversions::ConversionConfig<'_>,
) -> String {
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let binding_name = config.binding_field_name_owned(&typ.name, &field.name);
        if let Some(expr) = core_to_binding_field_expr(field) {
            generated = replace_field_conversion(&generated, &binding_name, &expr).unwrap_or_else(|| {
                panic!(
                    "PyO3 explicit-wrapper core-to-binding conversion for {}.{} must replace exactly one field",
                    typ.name, field.name
                )
            });
        }
    }
    generated
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
    use crate::core::ir::{NewtypeWrapperMetadata, PrimitiveType};

    fn wrapper(containers: Vec<NewtypeContainer>) -> String {
        wrapper_with_operations(containers, "from", "into_inner")
    }

    fn wrapper_with_operations(containers: Vec<NewtypeContainer>, from: &str, into: &str) -> String {
        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "toolkit::SecretString",
            from,
            into,
            containers,
        )])
    }

    fn wrappers(paths: &[Vec<NewtypeContainer>]) -> String {
        NewtypeWrapper::encode_explicit(
            &paths
                .iter()
                .map(|containers| {
                    NewtypeWrapperMetadata::transparent_string(
                        "toolkit::SecretString",
                        "from",
                        "into_inner",
                        containers.clone(),
                    )
                })
                .collect::<Vec<_>>(),
        )
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
    fn flattened_nested_optional_field_uses_explicit_wrapper_operations_in_both_directions() {
        let field = FieldDef {
            name: "credential".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            newtype_wrapper: Some(wrapper_with_operations(
                vec![NewtypeContainer::Optional, NewtypeContainer::Optional],
                "new_secret",
                "expose_secret",
            )),
            ..FieldDef::default()
        };

        let to_core = binding_to_core_field_expr(&field, "credential").expect("binding-to-core conversion");
        assert!(to_core.contains("SecretString::new_secret(value)"), "{to_core}");
        assert!(to_core.ends_with(".map(Some)"), "{to_core}");
        assert!(!to_core.contains("(value).map(|value|"), "{to_core}");

        let from_core = core_to_binding_field_expr(&field).expect("core-to-binding conversion");
        assert!(from_core.contains("val.credential.flatten()"), "{from_core}");
        assert!(from_core.contains("expose_secret()"), "{from_core}");
        assert!(!from_core.contains("to_string()"), "{from_core}");
    }

    #[test]
    fn flattened_nested_collection_converts_wrapper_keys_and_named_values() {
        let field = FieldDef {
            name: "segments".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Segment".to_string())),
            ))))),
            optional: true,
            newtype_wrapper: Some(wrappers(&[vec![
                NewtypeContainer::Optional,
                NewtypeContainer::Optional,
                NewtypeContainer::Vec,
                NewtypeContainer::MapKey,
            ]])),
            ..FieldDef::default()
        };

        let to_core = binding_to_core_field_expr(&field, "segments").expect("binding-to-core conversion");
        assert!(to_core.contains("toolkit::SecretString::from(key)"), "{to_core}");
        assert!(to_core.contains("(value).into()"), "{to_core}");
        assert!(to_core.ends_with(".map(Some)"), "{to_core}");

        let from_core = core_to_binding_field_expr(&field).expect("core-to-binding conversion");
        assert!(from_core.contains("val.segments.flatten()"), "{from_core}");
        assert!(from_core.contains("(key).into_inner()"), "{from_core}");
        assert!(from_core.contains("(value).into()"), "{from_core}");
    }

    #[test]
    fn field_conversion_replacement_requires_exactly_one_emitted_form() {
        let initializer = "        Self {\n            credential: stale,\n        }\n";
        let replaced =
            replace_field_conversion(initializer, "credential", "converted").expect("one initializer must be replaced");
        assert!(replaced.contains("credential: converted,"), "{replaced}");
        assert!(!replaced.contains("credential: stale,"), "{replaced}");

        let assignment = "        let mut __result = Config::default();\n        __result.credential = stale;\n";
        let replaced = replace_field_conversion(assignment, "credential", "converted")
            .expect("one default-seeded assignment must be replaced");
        assert!(replaced.contains("__result.credential = converted;"), "{replaced}");
        assert!(!replaced.contains("__result.credential = stale;"), "{replaced}");

        assert_eq!(
            replace_field_conversion("Self { other: stale }\n", "credential", "converted"),
            None
        );
        assert_eq!(
            replace_field_conversion("__result.credential = stale", "credential", "converted"),
            None
        );
        assert_eq!(
            replace_field_conversion(
                "            credential: first,\n            credential: second,\n",
                "credential",
                "converted",
            ),
            None
        );
    }

    #[test]
    fn field_conversion_replacement_respects_expression_boundaries() {
        let cases = [
            (
                "compact initializer",
                "Self { credential: stale, other: keep }",
                "Self { credential: converted, other: keep }",
            ),
            (
                "compact assignment",
                "let mut out = Config::default(); __result.credential = stale; __result.other = keep;",
                "let mut out = Config::default(); __result.credential = converted; __result.other = keep;",
            ),
            (
                "nested delimiters",
                "Self { credential: call((a, b), [c, d], Thing { x: 1, y: 2 }), other: keep }",
                "Self { credential: converted, other: keep }",
            ),
            (
                "quoted terminators",
                r#"Self { credential: call("a,;\"quoted\""), other: keep }"#,
                "Self { credential: converted, other: keep }",
            ),
            (
                "generic comma",
                "Self { credential: values.collect::<HashMap<_, _>>(), other: keep }",
                "Self { credential: converted, other: keep }",
            ),
        ];

        for (case_name, input, expected) in cases {
            assert_eq!(
                replace_field_conversion(input, "credential", "converted").as_deref(),
                Some(expected),
                "{case_name}"
            );
        }
    }

    #[test]
    fn non_flattened_optional_boxed_wrapper_keeps_shared_conversions() {
        let typ = TypeDef {
            name: "Config".to_string(),
            rust_path: "toolkit::Config".to_string(),
            fields: vec![FieldDef {
                name: "secret".to_string(),
                ty: TypeRef::String,
                optional: true,
                is_boxed: true,
                newtype_wrapper: Some(wrapper_with_operations(
                    vec![NewtypeContainer::Optional],
                    "new_secret",
                    "expose_secret",
                )),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        };
        let config = crate::codegen::conversions::ConversionConfig::default();
        let shared_to_core = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "toolkit", &config);
        assert_eq!(
            rewrite_binding_to_core_fields(shared_to_core.clone(), &typ, &config),
            shared_to_core
        );

        let shared_from_core = crate::codegen::conversions::gen_from_core_to_binding_cfg(
            &typ,
            "toolkit",
            &ahash::AHashSet::new(),
            &config,
        );
        assert_eq!(
            rewrite_core_to_binding_fields(shared_from_core.clone(), &typ, &config),
            shared_from_core
        );
    }

    #[test]
    fn field_rewrite_preserves_default_seed_and_binding_exclusions() {
        let typ = TypeDef {
            name: "Config".to_string(),
            rust_path: "toolkit::Config".to_string(),
            has_default: true,
            fields: vec![
                FieldDef {
                    name: "credential".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "hidden".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    binding_excluded: true,
                    newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "timeout".to_string(),
                    ty: TypeRef::Duration,
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        };
        let config = crate::codegen::conversions::ConversionConfig {
            option_duration_on_defaults: true,
            ..Default::default()
        };
        let to_core = rewrite_binding_to_core_fields(
            crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "toolkit", &config),
            &typ,
            &config,
        );
        assert!(to_core.contains("__result.credential ="), "{to_core}");
        assert!(to_core.contains("SecretString::from(value)"), "{to_core}");
        assert!(
            to_core.contains("let mut __result = toolkit::Config::default()"),
            "{to_core}"
        );
        assert!(!to_core.contains("(value).map(|value|"), "{to_core}");
        assert!(!to_core.contains("hidden:"), "{to_core}");

        let from_core = rewrite_core_to_binding_fields(
            crate::codegen::conversions::gen_from_core_to_binding_cfg(
                &typ,
                "toolkit",
                &ahash::AHashSet::new(),
                &config,
            ),
            &typ,
            &config,
        );
        assert!(from_core.contains("credential:"), "{from_core}");
        assert!(from_core.contains("into_inner()"), "{from_core}");
        assert!(!from_core.contains("hidden:"), "{from_core}");
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
