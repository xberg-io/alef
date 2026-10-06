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
mod tests;
#[cfg(test)]
mod wrapper_rewrite_tests;
