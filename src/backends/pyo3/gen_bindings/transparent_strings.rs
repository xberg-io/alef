use crate::codegen::generators::{AdapterBodies, RustBindingConfig};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{
    CoreWrapper, EnumDef, FieldDef, FunctionDef, MethodDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper,
    ParamDef, ReceiverKind, TypeDef, TypeRef,
};

fn has_explicit_paths(wrapper: &str) -> bool {
    NewtypeWrapper::decode(wrapper).is_ok_and(|decoded| !decoded.explicit_paths().is_empty())
}

fn root_optional_transparent_constructor(wrapper: &str) -> Option<String> {
    let decoded = NewtypeWrapper::decode(wrapper).ok()?;
    let [metadata] = decoded.explicit_paths() else {
        return None;
    };
    if metadata.containers.as_slice() != [NewtypeContainer::Optional] {
        return None;
    }
    let NewtypeConversion::TransparentString { from, .. } = &metadata.conversion else {
        return None;
    };
    Some(format!("{}::{from}", metadata.rust_path))
}

fn apply_newtype_to_core(expr: &str, ty: &TypeRef, optional: bool, wrapper: &str) -> String {
    root_optional_transparent_constructor(wrapper).map_or_else(
        || crate::codegen::conversions::helpers::apply_field_newtype_to_core(expr, ty, optional, wrapper),
        |constructor| format!("({expr}).map({constructor})"),
    )
}

fn root_optional_closure_rewrite(source: &str, wrapper: &str) -> Option<(String, String)> {
    let constructor = root_optional_transparent_constructor(wrapper)?;
    Some((
        format!("({source}).map(|value| {constructor}(value))"),
        format!("({source}).map({constructor})"),
    ))
}

fn apply_expected_closure_rewrites(
    mut generated: String,
    rewrites: impl IntoIterator<Item = (String, String)>,
    owner: &str,
) -> String {
    let mut expected = std::collections::BTreeMap::<String, (String, usize)>::new();
    for (redundant, direct) in rewrites {
        let entry = expected.entry(redundant).or_insert((direct, 0));
        entry.1 += 1;
    }
    for (redundant, (direct, expected_count)) in expected {
        let redundant_count = generated.matches(&redundant).count();
        let direct_count = generated.matches(&direct).count();
        match (redundant_count, direct_count) {
            (count, 0) if count == expected_count => generated = generated.replace(&redundant, &direct),
            (0, count) if count >= expected_count => {}
            _ => panic!(
                "PyO3 root-optional transparent-string conversion for {owner} expected {expected_count} exact closure(s), \
                 found {redundant_count} redundant and {direct_count} direct"
            ),
        }
    }
    generated
}

pub(super) fn rewrite_function_root_optional_params(generated: String, function: &FunctionDef) -> String {
    let rewrites = function.params.iter().enumerate().filter_map(|(index, param)| {
        let source = if crate::codegen::shared::is_promoted_optional(&function.params, index) {
            format!("{}.expect(\"'{}' is required\")", param.name, param.name)
        } else {
            param.name.clone()
        };
        root_optional_closure_rewrite(&source, param.newtype_wrapper.as_deref()?)
    });
    apply_expected_closure_rewrites(generated, rewrites, &function.name)
}

pub(super) fn rewrite_data_enum_root_optional_fields(
    generated: String,
    enum_def: &EnumDef,
    is_host_enum: bool,
) -> String {
    let rewrites = crate::codegen::generators::collect_pyo3_variant_constructors(enum_def)
        .into_iter()
        .filter(|constructor| {
            enum_def
                .variants
                .iter()
                .find(|variant| variant.name == constructor.variant_name)
                .is_some_and(|variant| {
                    crate::codegen::generators::variant_constructor_is_reachable(variant, is_host_enum)
                })
        })
        .flat_map(|constructor| {
            constructor
                .params
                .iter()
                .enumerate()
                .filter_map(|(index, param)| {
                    let source = if crate::codegen::shared::is_promoted_optional(&constructor.params, index) {
                        format!("{}.unwrap_or_default()", param.name)
                    } else {
                        param.name.clone()
                    };
                    root_optional_closure_rewrite(&source, param.newtype_wrapper.as_deref()?)
                })
                .collect::<Vec<_>>()
        });
    apply_expected_closure_rewrites(generated, rewrites, &enum_def.name)
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

fn container_type_hint(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Optional(inner) => format!("Option<{}>", container_type_hint(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", container_type_hint(inner)),
        TypeRef::Map(key, value) => format!(
            "std::collections::HashMap<{}, {}>",
            container_type_hint(key),
            container_type_hint(value)
        ),
        _ => "_".to_string(),
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
            let type_hint = container_type_hint(inner);
            format!("({expr}).map(|value: {type_hint}| {converted})")
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

fn has_additional_core_wrappers(field: &FieldDef) -> bool {
    field.is_boxed || field.core_wrapper != CoreWrapper::None || field.vec_inner_core_wrapper != CoreWrapper::None
}

pub(super) fn reject_unsupported_flattened_wrapper_fields(
    api: &crate::core::ir::ApiSurface,
    mut type_is_emitted: impl FnMut(&TypeDef) -> bool,
) -> anyhow::Result<()> {
    for typ in api.types.iter().filter(|typ| type_is_emitted(typ)) {
        for field in &typ.fields {
            if field.binding_excluded {
                continue;
            }
            let flattened_explicit =
                field.optional && matches!(field.ty, TypeRef::Optional(_)) && explicit_field_wrapper(field).is_some();
            if flattened_explicit && has_additional_core_wrappers(field) {
                anyhow::bail!(
                    "PyO3 cannot generate flattened explicit-wrapper field {}.{} with additional core wrapper {:?}, \
                     boxed={}, vec-inner wrapper {:?}",
                    typ.name,
                    field.name,
                    field.core_wrapper,
                    field.is_boxed,
                    field.vec_inner_core_wrapper,
                );
            }
        }
    }
    Ok(())
}

fn binding_to_core_field_expr(field: &FieldDef, binding_name: &str) -> Option<String> {
    let wrapper = explicit_field_wrapper(field)?;
    if has_additional_core_wrappers(field) {
        return None;
    }
    let source = format!("val.{binding_name}");
    let flattened = field.optional && matches!(field.ty, TypeRef::Optional(_));
    if !flattened {
        let constructor = root_optional_transparent_constructor(wrapper)?;
        return Some(format!("({source}).map({constructor})"));
    }
    let adjusted_wrapper = flattened_optional_wrapper(wrapper);
    let converted = apply_newtype_to_core(&source, &field.ty, false, &adjusted_wrapper);
    let converted = convert_named_leaves(&converted, &field.ty);
    Some(format!("({converted}).map(Some)"))
}

fn core_to_binding_field_expr(field: &FieldDef) -> Option<String> {
    let wrapper = explicit_field_wrapper(field)?;
    if has_additional_core_wrappers(field) {
        return None;
    }
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

fn starts_at_identifier_boundary(source: &str, index: usize) -> bool {
    source[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_' && ch != '#')
}

fn ends_at_identifier_boundary(source: &str, index: usize) -> bool {
    source[index..]
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_' && ch != '#')
}

fn replace_field_conversion(generated: &str, field_name: &str, source_name: &str, expr: &str) -> Option<String> {
    let initializer_prefix = format!("{field_name}: ");
    let assignment_prefix = format!("__result.{field_name} = ");
    let constructor_access = format!("val.{source_name}");
    let initializers: Vec<_> = generated
        .match_indices(&initializer_prefix)
        .filter(|(index, _)| starts_at_identifier_boundary(generated, *index))
        .map(|(index, _)| index)
        .collect();
    let assignments: Vec<_> = generated
        .match_indices(&assignment_prefix)
        .map(|(index, _)| index)
        .collect();
    let (start, prefix, terminator) = match (initializers.as_slice(), assignments.as_slice()) {
        ([start], []) => (*start, initializer_prefix, ','),
        ([], [start]) => (*start, assignment_prefix, ';'),
        ([], []) => {
            let constructor_arguments: Vec<_> = generated
                .match_indices(&constructor_access)
                .filter(|(index, matched)| {
                    starts_at_identifier_boundary(generated, *index)
                        && ends_at_identifier_boundary(generated, *index + matched.len())
                })
                .map(|(index, _)| index)
                .collect();
            let [start] = constructor_arguments.as_slice() else {
                return None;
            };
            let relative_end = expression_terminator(&generated[*start..], ',')?;
            let end = *start + relative_end + 1;
            let mut output = generated.to_string();
            output.replace_range(*start..end, &format!("{expr},"));
            return Some(output);
        }
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
            generated = replace_field_conversion(&generated, &field.name, &binding_name, &expr).unwrap_or_else(|| {
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
            generated = replace_field_conversion(&generated, &binding_name, &field.name, &expr).unwrap_or_else(|| {
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
    let converted = apply_newtype_to_core(&source, &param.ty, param.optional && !promoted, wrapper);
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
    use crate::backends::pyo3::gen_bindings::Pyo3Backend;
    use crate::backends::pyo3::type_map::Pyo3Mapper;
    use crate::core::backend::Backend;
    use crate::core::ir::{ApiSurface, EnumVariant, NewtypeWrapperMetadata, PrimitiveType};

    fn wrapper(containers: Vec<NewtypeContainer>) -> String {
        wrapper_with_operations(containers, "from", "into_inner")
    }

    fn fixture_wrapper(containers: Vec<NewtypeContainer>) -> String {
        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "fixture_core::SecretString",
            "from",
            "into_inner",
            containers,
        )])
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

    fn optional_credential_constructor_owner(name: &str, constructor: &str, has_lifetime_params: bool) -> TypeDef {
        let field = FieldDef {
            name: "optional_credential".to_string(),
            ty: TypeRef::String,
            optional: true,
            newtype_wrapper: Some(fixture_wrapper(vec![NewtypeContainer::Optional])),
            ..FieldDef::default()
        };
        TypeDef {
            name: name.to_string(),
            rust_path: format!("fixture_core::{name}"),
            fields: vec![field.clone()],
            methods: vec![MethodDef {
                name: constructor.to_string(),
                params: vec![ParamDef {
                    name: field.name,
                    ty: field.ty,
                    optional: field.optional,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named(name.to_string()),
                is_static: true,
                ..MethodDef::default()
            }],
            has_lifetime_params,
            ..TypeDef::default()
        }
    }

    fn gate_fixture_config() -> crate::core::config::ResolvedCrateConfig {
        let config: crate::core::config::new_config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.python]
module_name = "toolkit"
"#,
        )
        .expect("fixture config must parse");
        config.resolve().expect("fixture config must resolve").remove(0)
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
    fn gate_fixture_root_optional_enum_and_function_paths_use_constructor_items() {
        let optional_wrapper = wrapper(vec![NewtypeContainer::Optional]);
        let optional_param = |is_ref| ParamDef {
            name: "value".to_string(),
            ty: TypeRef::String,
            optional: true,
            is_ref,
            newtype_wrapper: Some(optional_wrapper.clone()),
            ..ParamDef::default()
        };
        let api = ApiSurface {
            crate_name: "toolkit".to_string(),
            version: "0.1.0".to_string(),
            enums: vec![EnumDef {
                name: "Authentication".to_string(),
                rust_path: "toolkit::Authentication".to_string(),
                variants: vec![EnumVariant {
                    name: "Optional".to_string(),
                    fields: vec![FieldDef {
                        name: "value".to_string(),
                        ty: TypeRef::String,
                        optional: true,
                        newtype_wrapper: Some(optional_wrapper.clone()),
                        ..FieldDef::default()
                    }],
                    ..EnumVariant::default()
                }],
                has_serde: true,
                ..EnumDef::default()
            }],
            functions: vec![
                FunctionDef {
                    name: "optional_secret".to_string(),
                    rust_path: "toolkit::optional_secret".to_string(),
                    params: vec![optional_param(false)],
                    return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                    return_newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional])),
                    ..FunctionDef::default()
                },
                FunctionDef {
                    name: "borrow_optional_secret".to_string(),
                    rust_path: "toolkit::borrow_optional_secret".to_string(),
                    params: vec![optional_param(true)],
                    return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                    return_newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional])),
                    ..FunctionDef::default()
                },
            ],
            ..ApiSurface::default()
        };

        let files = Pyo3Backend
            .generate_bindings(&api, &gate_fixture_config())
            .expect("gate fixture bindings must generate");
        let generated = &files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
            .expect("PyO3 binding must emit lib.rs")
            .content;
        let redundant = "(value).map(|value| toolkit::SecretString::from(value))";
        let direct = "(value).map(toolkit::SecretString::from)";
        assert_eq!(generated.matches(redundant).count(), 0, "{generated}");
        assert_eq!(generated.matches(direct).count(), 3, "{generated}");
    }

    fn cfg_optional_wrapper_enum(rust_path: &str) -> EnumDef {
        EnumDef {
            name: "Authentication".to_string(),
            rust_path: rust_path.to_string(),
            variants: vec![
                EnumVariant {
                    name: "None".to_string(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Optional".to_string(),
                    fields: vec![FieldDef {
                        name: "value".to_string(),
                        ty: TypeRef::String,
                        optional: true,
                        newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional])),
                        ..FieldDef::default()
                    }],
                    cfg: Some("feature = \"extras\"".to_string()),
                    ..EnumVariant::default()
                },
            ],
            has_serde: true,
            ..EnumDef::default()
        }
    }

    fn generated_lib_for_enum(enum_def: EnumDef) -> String {
        let api = ApiSurface {
            crate_name: "toolkit".to_string(),
            version: "0.1.0".to_string(),
            enums: vec![enum_def],
            ..ApiSurface::default()
        };
        Pyo3Backend
            .generate_bindings(&api, &gate_fixture_config())
            .expect("cfg enum bindings must generate")
            .into_iter()
            .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
            .expect("PyO3 binding must emit lib.rs")
            .content
    }

    #[test]
    fn foreign_cfg_variant_is_excluded_from_constructor_rewrite_eligibility() {
        let generated = generated_lib_for_enum(cfg_optional_wrapper_enum("foreign_core::Authentication"));

        assert!(!generated.contains("fn _factory_optional"), "{generated}");
        assert!(
            !generated.contains("(value).map(toolkit::SecretString::from)"),
            "{generated}"
        );
    }

    #[test]
    fn host_cfg_variant_remains_eligible_for_constructor_rewrite() {
        let generated = generated_lib_for_enum(cfg_optional_wrapper_enum("toolkit::Authentication"));

        assert!(
            generated.contains("#[cfg(feature = \"extras\")]\n    #[pyo3(name = \"optional\")]"),
            "{generated}"
        );
        assert!(generated.contains("fn _factory_optional"), "{generated}");
        assert!(
            generated.contains("(value).map(toolkit::SecretString::from)"),
            "{generated}"
        );
        assert!(
            !generated.contains("map(|value| toolkit::SecretString::from(value))"),
            "{generated}"
        );
    }

    #[test]
    fn single_optional_custom_constructor_specializes_ordinary_field() {
        let field = FieldDef {
            name: "credential".to_string(),
            ty: TypeRef::String,
            optional: true,
            newtype_wrapper: Some(wrapper_with_operations(
                vec![NewtypeContainer::Optional],
                "new_secret",
                "expose_secret",
            )),
            ..FieldDef::default()
        };

        assert_eq!(
            binding_to_core_field_expr(&field, "credential"),
            Some("(val.credential).map(toolkit::SecretString::new_secret)".to_string())
        );

        let typ = TypeDef {
            name: "Config".to_string(),
            rust_path: "toolkit::Config".to_string(),
            fields: vec![field],
            ..TypeDef::default()
        };
        let config = crate::codegen::conversions::ConversionConfig::default();
        let shared = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "toolkit", &config);
        assert!(
            shared.contains("credential: (val.credential).map(|value| toolkit::SecretString::new_secret(value)),"),
            "negative control must contain the redundant closure: {shared}"
        );
        let rewritten = rewrite_binding_to_core_fields(shared, &typ, &config);
        assert!(
            rewritten.contains("credential: (val.credential).map(toolkit::SecretString::new_secret),"),
            "{rewritten}"
        );
        assert!(!rewritten.contains(".map(|value|"), "{rewritten}");
    }

    #[test]
    fn report_optional_credential_rewrite_ignores_nested_field_suffix_match() {
        let typ = TypeDef {
            name: "Report".to_string(),
            rust_path: "fixture_core::Report".to_string(),
            fields: vec![
                FieldDef {
                    name: "optional_credential".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    newtype_wrapper: Some(fixture_wrapper(vec![NewtypeContainer::Optional])),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "nested_optional_credential".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    newtype_wrapper: Some(fixture_wrapper(vec![
                        NewtypeContainer::Optional,
                        NewtypeContainer::Optional,
                    ])),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        };
        let config = crate::codegen::conversions::ConversionConfig::default();
        let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "fixture_core", &config);
        let rewritten = rewrite_binding_to_core_fields(generated, &typ, &config);

        assert!(
            rewritten.contains("optional_credential: (val.optional_credential).map(fixture_core::SecretString::from),"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains(concat!(
                "nested_optional_credential: ",
                "((val.nested_optional_credential).map(fixture_core::SecretString::from)).map(Some),"
            )),
            "{rewritten}"
        );
    }

    #[test]
    fn static_new_owner_rewrites_exact_optional_credential_argument() {
        let typ = optional_credential_constructor_owner("Report", "new", false);
        let config = crate::codegen::conversions::ConversionConfig::default();
        let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "fixture_core", &config);
        assert!(generated.contains("Self::new("), "{generated}");
        assert!(generated.contains("val.optional_credential,"), "{generated}");

        let rewritten = rewrite_binding_to_core_fields(generated, &typ, &config);
        assert!(
            rewritten.contains("(val.optional_credential).map(fixture_core::SecretString::from),"),
            "{rewritten}"
        );
    }

    #[test]
    fn lifetime_owner_rewrites_exact_optional_credential_argument() {
        let typ = optional_credential_constructor_owner("BorrowedReport", "with_owned", true);
        let config = crate::codegen::conversions::ConversionConfig::default();
        let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(&typ, "fixture_core", &config);
        assert!(
            generated.contains("fixture_core::BorrowedReport::with_owned("),
            "{generated}"
        );
        assert!(generated.contains("val.optional_credential,"), "{generated}");

        let rewritten = rewrite_binding_to_core_fields(generated, &typ, &config);
        assert!(
            rewritten.contains("(val.optional_credential).map(fixture_core::SecretString::from),"),
            "{rewritten}"
        );
    }

    #[test]
    fn single_optional_custom_constructor_specializes_owned_and_borrowed_params() {
        let mut param = ParamDef {
            name: "credential".to_string(),
            ty: TypeRef::String,
            optional: true,
            newtype_wrapper: Some(wrapper_with_operations(
                vec![NewtypeContainer::Optional],
                "new_secret",
                "expose_secret",
            )),
            ..ParamDef::default()
        };

        assert_eq!(
            converted_param(&param, false, 0),
            Some((
                String::new(),
                "(credential).map(toolkit::SecretString::new_secret)".to_string()
            ))
        );

        param.is_ref = true;
        assert_eq!(
            converted_param(&param, false, 0),
            Some((
                concat!(
                    "let __alef_wrapper_arg_0 = ",
                    "(credential).map(toolkit::SecretString::new_secret);\n        "
                )
                .to_string(),
                "__alef_wrapper_arg_0.as_ref()".to_string()
            ))
        );
    }

    #[test]
    fn nested_optional_map_and_multipath_wrappers_keep_structural_conversion() {
        let nested_wrapper = wrapper_with_operations(
            vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
            "new_secret",
            "expose_secret",
        );
        let field = FieldDef {
            name: "credentials".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: true,
            newtype_wrapper: Some(nested_wrapper.clone()),
            ..FieldDef::default()
        };
        assert_eq!(binding_to_core_field_expr(&field, "credentials"), None);

        let param = ParamDef {
            name: "credentials".to_string(),
            ty: field.ty.clone(),
            optional: true,
            newtype_wrapper: Some(nested_wrapper),
            ..ParamDef::default()
        };
        assert_eq!(
            converted_param(&param, false, 0),
            Some((
                String::new(),
                concat!(
                    "(credentials).map(|value| (value).into_iter().map(|(key, value)| ",
                    "(key, toolkit::SecretString::new_secret(value))).collect())"
                )
                .to_string()
            ))
        );

        let multipath = wrappers(&[
            vec![NewtypeContainer::Optional, NewtypeContainer::MapKey],
            vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
        ]);
        let multipath_param = ParamDef {
            name: "credentials".to_string(),
            ty: field.ty,
            optional: true,
            newtype_wrapper: Some(multipath),
            ..ParamDef::default()
        };
        assert_eq!(
            converted_param(&multipath_param, false, 0),
            Some((
                String::new(),
                concat!(
                    "(credentials).map(|value| (value).into_iter().map(|(key, value)| ",
                    "(toolkit::SecretString::from(key), toolkit::SecretString::from(value))).collect())"
                )
                .to_string()
            ))
        );
    }

    #[test]
    fn named_leaf_hint_keeps_binding_hash_map_and_leaves_btree_target_collect_inferred() {
        struct BindingSegment;
        struct CoreSegment;
        impl From<BindingSegment> for CoreSegment {
            fn from(_: BindingSegment) -> Self {
                Self
            }
        }

        let ty = TypeRef::Optional(Box::new(TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("Segment".to_string())),
        )));
        assert_eq!(
            convert_named_leaves("values", &ty),
            concat!(
                "(values).map(|value: std::collections::HashMap<_, _>| ",
                "(value).into_iter().map(|(key, value)| (key, (value).into())).collect())"
            )
        );

        let values: Option<std::collections::HashMap<String, BindingSegment>> = None;
        let converted: Option<std::collections::BTreeMap<String, CoreSegment>> =
            values.map(|value: std::collections::HashMap<_, _>| {
                value.into_iter().map(|(key, value)| (key, value.into())).collect()
            });
        assert_eq!(converted.map(|value| value.len()), None);
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
        assert_eq!(
            to_core,
            "((val.credential).map(toolkit::SecretString::new_secret)).map(Some)"
        );

        let from_core = core_to_binding_field_expr(&field).expect("core-to-binding conversion");
        assert!(from_core.contains("val.credential.flatten()"), "{from_core}");
        assert!(from_core.contains("expose_secret()"), "{from_core}");
        assert!(!from_core.contains("to_string()"), "{from_core}");
    }

    #[test]
    fn flattened_fields_with_additional_core_wrappers_are_rejected_before_generation() {
        let base = FieldDef {
            name: "credential".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
            ..FieldDef::default()
        };

        let mut cases = Vec::new();
        let mut boxed = base.clone();
        boxed.is_boxed = true;
        cases.push(("boxed", boxed));
        for (name, core_wrapper) in [
            ("cow", CoreWrapper::Cow),
            ("box", CoreWrapper::Box),
            ("arc", CoreWrapper::Arc),
            ("arc_mutex", CoreWrapper::ArcMutex),
        ] {
            let mut field = base.clone();
            field.core_wrapper = core_wrapper;
            cases.push((name, field));
        }
        let mut vec_inner = base.clone();
        vec_inner.ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        vec_inner.vec_inner_core_wrapper = CoreWrapper::Arc;
        vec_inner.newtype_wrapper = Some(wrapper(vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
            NewtypeContainer::Vec,
        ]));
        cases.push(("vec_inner_arc", vec_inner));

        for (case_name, field) in cases {
            assert_flattened_wrapper_case_rejected(case_name, field);
        }

        assert_supported_flattened_wrapper_generates_both_directions(base);
    }

    fn assert_flattened_wrapper_case_rejected(case_name: &str, field: FieldDef) {
        let api = ApiSurface {
            crate_name: "toolkit".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "Config".to_string(),
                rust_path: "toolkit::Config".to_string(),
                fields: vec![field],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let error = Pyo3Backend
            .generate_bindings(&api, &gate_fixture_config())
            .expect_err("unsupported flattened wrapper composition must fail before emission");
        let message = error.to_string();
        assert!(
            message.contains("PyO3 cannot generate flattened explicit-wrapper field Config.credential"),
            "{case_name}: {message}"
        );
    }

    fn assert_supported_flattened_wrapper_generates_both_directions(base: FieldDef) {
        let supported_api = ApiSurface {
            crate_name: "toolkit".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "Config".to_string(),
                rust_path: "toolkit::Config".to_string(),
                fields: vec![base],
                ..TypeDef::default()
            }],
            functions: vec![FunctionDef {
                name: "consume_config".to_string(),
                rust_path: "toolkit::consume_config".to_string(),
                params: vec![ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("Config".to_string()),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Unit,
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };
        let generated = Pyo3Backend
            .generate_bindings(&supported_api, &gate_fixture_config())
            .expect("flattened explicit wrapper without an additional core wrapper remains supported")
            .into_iter()
            .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
            .expect("PyO3 binding must emit lib.rs")
            .content;
        assert!(generated.contains("toolkit::SecretString::from"), "{generated}");
        assert!(generated.contains(".into_inner()"), "{generated}");
    }

    #[test]
    fn opaque_type_flattened_wrapper_metadata_is_not_rejected() {
        let api = ApiSurface {
            crate_name: "toolkit".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "OpaqueConfig".to_string(),
                rust_path: "toolkit::OpaqueConfig".to_string(),
                is_opaque: true,
                fields: vec![FieldDef {
                    name: "credential".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    core_wrapper: CoreWrapper::Arc,
                    newtype_wrapper: Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional])),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };

        Pyo3Backend
            .generate_bindings(&api, &gate_fixture_config())
            .expect("opaque PyO3 handles do not emit or validate their source fields");
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
        assert_eq!(
            to_core,
            concat!(
                "(((val.segments).map(|value| (value).into_iter().map(|value| ",
                "(value).into_iter().map(|(key, value)| ",
                "(toolkit::SecretString::from(key), value)).collect()).collect())).map(|value: ",
                "Vec<std::collections::HashMap<_, _>>| ",
                "(value).into_iter().map(|value| (value).into_iter().map(|(key, value)| ",
                "(key, (value).into())).collect()).collect())).map(Some)"
            )
        );

        let from_core = core_to_binding_field_expr(&field).expect("core-to-binding conversion");
        assert!(from_core.contains("val.segments.flatten()"), "{from_core}");
        assert!(from_core.contains("(key).into_inner()"), "{from_core}");
        assert!(
            from_core.contains("|value: Vec<std::collections::HashMap<_, _>>|"),
            "{from_core}"
        );
        assert!(from_core.contains("(value).into()"), "{from_core}");
    }

    #[test]
    fn field_conversion_replacement_requires_exactly_one_emitted_form() {
        let initializer = "        Self {\n            credential: stale,\n        }\n";
        let replaced = replace_field_conversion(initializer, "credential", "credential", "converted")
            .expect("one initializer must be replaced");
        assert!(replaced.contains("credential: converted,"), "{replaced}");
        assert!(!replaced.contains("credential: stale,"), "{replaced}");

        let assignment = "        let mut __result = Config::default();\n        __result.credential = stale;\n";
        let replaced = replace_field_conversion(assignment, "credential", "credential", "converted")
            .expect("one default-seeded assignment must be replaced");
        assert!(replaced.contains("__result.credential = converted;"), "{replaced}");
        assert!(!replaced.contains("__result.credential = stale;"), "{replaced}");

        assert_eq!(
            replace_field_conversion("Self { other: stale }\n", "credential", "credential", "converted"),
            None
        );
        assert_eq!(
            replace_field_conversion("__result.credential = stale", "credential", "credential", "converted"),
            None
        );
        assert_eq!(
            replace_field_conversion(
                "            credential: first,\n            credential: second,\n",
                "credential",
                "credential",
                "converted",
            ),
            None
        );
        assert_eq!(
            replace_field_conversion(
                "Self::new(val.credential, val.credential,)",
                "credential",
                "credential",
                "converted"
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
            (
                "field-name suffix collision",
                "Self { credential: stale, nested_credential: keep }",
                "Self { credential: converted, nested_credential: keep }",
            ),
        ];

        for (case_name, input, expected) in cases {
            assert_eq!(
                replace_field_conversion(input, "credential", "credential", "converted").as_deref(),
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
        assert!(to_core.contains(".map(toolkit::SecretString::from)"), "{to_core}");
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
