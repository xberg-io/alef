//! NAPI collapses an outer optional field whose `TypeRef` is also optional into one JS option.
//! Explicit-newtype paths describe the full Rust shape, so these conversions remove only that
//! collapsed outer layer while preserving every nested container and ordinary named conversion. ~keep

use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{
    FieldDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, PrimitiveType, TypeDef,
    TypeRef,
};
use ahash::AHashSet;

pub(super) fn default_to_core(field: &FieldDef) -> String {
    let Some(wrapper) = field
        .newtype_wrapper
        .as_deref()
        .filter(|wrapper| wrapper_needs_leaf_default(field, wrapper))
    else {
        return "Default::default()".to_string();
    };
    let value = apply_to_core("Default::default()", &field.ty, field.optional, wrapper);
    if field.is_boxed {
        if field.optional {
            format!("({value}).map(Box::new)")
        } else {
            format!("Box::new({value})")
        }
    } else {
        value
    }
}

pub(super) fn apply_to_core(expression: &str, ty: &TypeRef, optional: bool, wrapper: &str) -> String {
    if let Some(constructor) = root_optional_transparent_constructor(wrapper) {
        return format!("({expression}).map({constructor})");
    }
    crate::codegen::conversions::helpers::apply_field_newtype_to_core(expression, ty, optional, wrapper)
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

fn wrapper_needs_leaf_default(field: &FieldDef, wrapper: &str) -> bool {
    if field.optional {
        return false;
    }
    match NewtypeWrapper::decode(wrapper).expect("newtype metadata was validated before codegen") {
        NewtypeWrapper::Tuple(_) => !matches!(field.ty, TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _)),
        NewtypeWrapper::Explicit(paths) => paths.iter().any(|path| path.containers.is_empty()),
    }
}

pub(super) fn gen_from_binding_to_core(typ: &TypeDef, core_import: &str, config: &ConversionConfig<'_>) -> String {
    let generated = crate::codegen::conversions::gen_from_binding_to_core_cfg(typ, core_import, config);
    repair_flattened_fields(generated, typ, config, Direction::ToCore)
}

pub(super) fn gen_from_core_to_binding(
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    config: &ConversionConfig<'_>,
) -> String {
    let generated = crate::codegen::conversions::gen_from_core_to_binding_cfg(typ, core_import, opaque_types, config);
    repair_flattened_fields(generated, typ, config, Direction::FromCore)
}

#[derive(Clone, Copy)]
pub(super) enum Direction {
    ToCore,
    FromCore,
}

fn repair_flattened_fields(
    mut generated: String,
    typ: &TypeDef,
    config: &ConversionConfig<'_>,
    direction: Direction,
) -> String {
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let Some(wrapper) = field.newtype_wrapper.as_deref() else {
            continue;
        };
        let Ok(NewtypeWrapper::Explicit(mut paths)) = NewtypeWrapper::decode(wrapper) else {
            continue;
        };
        let binding_name = config.binding_field_name_owned(&typ.name, &field.name);
        if matches!(direction, Direction::ToCore)
            && let Some(constructor) = root_optional_transparent_constructor(wrapper)
        {
            let converted = format!("(val.{binding_name}).map({constructor})");
            let expression = if field.is_boxed {
                format!("({converted}).map(Box::new)")
            } else {
                converted
            };
            generated = replace_to_core_conversion(&generated, &field.name, &binding_name, &expression)
                .unwrap_or_else(|| {
                    panic!(
                        "generated NAPI conversion for root-optional explicit-newtype field `{}.{}` had no replaceable field initializer, assignment, or constructor argument",
                        typ.name, field.name
                    )
                });
            continue;
        }
        let TypeRef::Optional(inner) = &field.ty else {
            continue;
        };
        if !field.optional {
            continue;
        }
        if paths.is_empty()
            || paths
                .iter()
                .any(|path| path.containers.first() != Some(&NewtypeContainer::Optional))
        {
            continue;
        }
        for path in &mut paths {
            path.containers.remove(0);
        }

        let expression = match direction {
            Direction::ToCore => {
                let converted = convert(&format!("val.{binding_name}"), &field.ty, &paths, Direction::ToCore);
                format!("({converted}).map(Some)")
            }
            Direction::FromCore => convert(
                &format!("val.{}.flatten()", field.name),
                &TypeRef::Optional(inner.clone()),
                &paths,
                Direction::FromCore,
            ),
        };
        generated = match direction {
            Direction::ToCore => replace_to_core_conversion(&generated, &field.name, &binding_name, &expression),
            Direction::FromCore => replace_field_conversion(generated, &field.name, &expression),
        }
        .unwrap_or_else(|| {
            panic!(
                "generated NAPI conversion for flattened explicit-newtype field `{}.{}` had no replaceable field initializer, assignment, or constructor argument",
                typ.name, field.name
            )
        });
    }
    generated
}

pub(super) fn replace_to_core_conversion(
    generated: &str,
    field_name: &str,
    binding_name: &str,
    expression: &str,
) -> Option<String> {
    replace_field_conversion(generated.to_string(), field_name, expression)
        .or_else(|| replace_constructor_argument(generated, binding_name, expression))
}

fn replace_constructor_argument(generated: &str, binding_name: &str, expression: &str) -> Option<String> {
    let marker = format!("val.{binding_name}");
    let matches: Vec<_> = generated
        .match_indices(&marker)
        .filter_map(|(index, _)| {
            let next = generated[index + marker.len()..].chars().next();
            (!next.is_some_and(|character| character == '_' || character.is_alphanumeric())).then_some(index)
        })
        .collect();
    let [marker_start] = matches.as_slice() else {
        return None;
    };
    let line_start = generated[..*marker_start].rfind('\n').map_or(0, |index| index + 1);
    let expression_start = line_start + generated[line_start..].find(|character: char| !character.is_whitespace())?;
    let relative_end = top_level_delimiter(&generated[expression_start..], ',')?;
    let expression_end = expression_start + relative_end;
    let mut output = generated.to_string();
    output.replace_range(expression_start..expression_end, expression);
    Some(output)
}

pub(super) fn replace_field_conversion(generated: String, name: &str, expression: &str) -> Option<String> {
    let initializer_marker = format!("{name}: ");
    if let Some(replaced) = replace_delimited(
        &generated,
        &initializer_marker,
        ',',
        crate::codegen::template_env::render(
            "binding_helpers/struct_field_line.jinja",
            crate::alef_context! { name => name, expr => expression },
        )
        .trim(),
    ) {
        return Some(replaced);
    }

    let assignment_marker = format!("__result.{name} = ");
    replace_delimited(
        &generated,
        &assignment_marker,
        ';',
        crate::backends::napi::template_env::render(
            "converted_field_assignment.jinja",
            crate::alef_context! { name => name, expr => expression },
        )
        .trim(),
    )
}

fn replace_delimited(generated: &str, marker: &str, delimiter: char, replacement: &str) -> Option<String> {
    let start = generated.find(marker)?;
    let expression_start = start + marker.len();
    let relative_end = top_level_delimiter(&generated[expression_start..], delimiter)?;
    let end = expression_start + relative_end + delimiter.len_utf8();
    Some(format!("{}{}{}", &generated[..start], replacement, &generated[end..]))
}

fn top_level_delimiter(value: &str, delimiter: char) -> Option<usize> {
    let mut depth = 0_i32;
    for (index, character) in value.char_indices() {
        match character {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            character if character == delimiter && depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn convert(expression: &str, ty: &TypeRef, paths: &[NewtypeWrapperMetadata], direction: Direction) -> String {
    convert_with_enum_keys(expression, ty, paths, direction, None)
}

pub(super) struct ContainerPolicy<'a> {
    pub(super) enum_keys: &'a ahash::AHashMap<String, String>,
    pub(super) fallible_types: &'a ahash::AHashSet<String>,
}

impl ContainerPolicy<'_> {
    pub(super) fn is_fallible(&self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Named(name) => self.fallible_types.contains(name),
            TypeRef::Map(key, value) => {
                matches!(key.as_ref(), TypeRef::Named(name) if self.enum_keys.contains_key(name))
                    || self.is_fallible(value)
            }
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => self.is_fallible(inner),
            _ => false,
        }
    }
}

pub(super) fn convert_with_enum_keys(
    expression: &str,
    ty: &TypeRef,
    paths: &[NewtypeWrapperMetadata],
    direction: Direction,
    policy: Option<&ContainerPolicy<'_>>,
) -> String {
    if let Some(path) = paths.iter().find(|path| path.containers.is_empty()) {
        return convert_wrapper_leaf(expression, path, direction);
    }

    let fallible = matches!(direction, Direction::ToCore) && policy.is_some_and(|policy| policy.is_fallible(ty));
    match ty {
        TypeRef::Optional(inner) => {
            if matches!(direction, Direction::ToCore)
                && let Some(constructor) = optional_leaf_constructor(paths)
            {
                return format!("({expression}).map({constructor})");
            }
            let nested = advance(paths, NewtypeContainer::Optional);
            let converted = convert_with_enum_keys("value", inner, &nested, direction, policy);
            optional_expression(expression, &converted, fallible)
        }
        TypeRef::Vec(inner) => {
            let nested = advance(paths, NewtypeContainer::Vec);
            let converted = convert_with_enum_keys("value", inner, &nested, direction, policy);
            collect_expression(expression, "value", &converted, fallible)
        }
        TypeRef::Map(key, value) => {
            let key_paths = advance(paths, NewtypeContainer::MapKey);
            let value_paths = advance(paths, NewtypeContainer::MapValue);
            let key = convert_map_key(key, &key_paths, direction, policy);
            let value = convert_with_enum_keys("value", value, &value_paths, direction, policy);
            collect_expression(expression, "(key, value)", &format!("({key}, {value})"), fallible)
        }
        _ => convert_leaf(expression, ty, direction, policy),
    }
}

fn optional_expression(expression: &str, converted: &str, fallible: bool) -> String {
    if fallible {
        format!(
            "({expression}).map(|value| -> std::result::Result<_, serde_json::Error> {{ Ok({converted}) }}).transpose()?"
        )
    } else {
        format!("({expression}).map(|value| {converted})")
    }
}

fn collect_expression(expression: &str, binding: &str, converted: &str, fallible: bool) -> String {
    if fallible {
        format!(
            "({expression}).into_iter().map(|{binding}| -> std::result::Result<_, serde_json::Error> {{ Ok({converted}) }}).collect::<std::result::Result<_, _>>()?"
        )
    } else {
        format!("({expression}).into_iter().map(|{binding}| {converted}).collect()")
    }
}

fn convert_leaf(expression: &str, ty: &TypeRef, direction: Direction, policy: Option<&ContainerPolicy<'_>>) -> String {
    match ty {
        TypeRef::Named(name)
            if matches!(direction, Direction::ToCore)
                && policy.is_some_and(|policy| policy.fallible_types.contains(name)) =>
        {
            format!("({expression}).try_into()?")
        }
        TypeRef::Named(_) | TypeRef::Bytes => format!("({expression}).into()"),
        TypeRef::Path => match direction {
            Direction::ToCore => format!("std::path::PathBuf::from({expression})"),
            Direction::FromCore => format!("({expression}).to_string_lossy().to_string()"),
        },
        TypeRef::Primitive(PrimitiveType::F32) => match direction {
            Direction::ToCore => format!("({expression}) as f32"),
            Direction::FromCore => format!("({expression}) as f64"),
        },
        TypeRef::Primitive(PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize) => match direction {
            Direction::ToCore => format!("({expression}) as {}", primitive_name(ty)),
            Direction::FromCore => format!("({expression}) as i64"),
        },
        TypeRef::Duration => match direction {
            Direction::ToCore => format!("std::time::Duration::from_millis(({expression}).max(0) as u64)"),
            Direction::FromCore => format!("({expression}).as_millis() as i64"),
        },
        TypeRef::Char => match direction {
            Direction::ToCore => format!("({expression}).chars().next().unwrap_or_default()"),
            Direction::FromCore => format!("({expression}).to_string()"),
        },
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Unit | TypeRef::Json => expression.to_string(),
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            unreachable!("container conversion handled before leaves")
        }
    }
}

fn convert_map_key(
    key: &TypeRef,
    paths: &[NewtypeWrapperMetadata],
    direction: Direction,
    policy: Option<&ContainerPolicy<'_>>,
) -> String {
    if let TypeRef::Named(name) = key
        && let Some(path) = policy.and_then(|policy| policy.enum_keys.get(name))
    {
        return match direction {
            Direction::ToCore => format!("serde_json::from_value::<{path}>(serde_json::Value::String(key))?"),
            Direction::FromCore => "serde_json::to_value(key).expect(\"failed to serialize enum map key\").as_str().expect(\"enum map key must serialize as a string\").to_owned()".into(),
        };
    }
    convert_with_enum_keys("key", key, paths, direction, policy)
}

fn optional_leaf_constructor(paths: &[NewtypeWrapperMetadata]) -> Option<String> {
    let [metadata] = paths else {
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

fn advance(paths: &[NewtypeWrapperMetadata], container: NewtypeContainer) -> Vec<NewtypeWrapperMetadata> {
    paths
        .iter()
        .filter_map(|path| {
            let (first, rest) = path.containers.split_first()?;
            (*first == container).then(|| {
                let mut advanced = path.clone();
                advanced.containers = rest.to_vec();
                advanced
            })
        })
        .collect()
}

fn convert_wrapper_leaf(expression: &str, metadata: &NewtypeWrapperMetadata, direction: Direction) -> String {
    match (&metadata.conversion, direction) {
        (NewtypeConversion::TransparentString { from, .. }, Direction::ToCore) => {
            format!("{}::{from}({expression})", metadata.rust_path)
        }
        (NewtypeConversion::TransparentString { into, .. }, Direction::FromCore) => {
            format!("({expression}).{into}()")
        }
        (NewtypeConversion::TupleField, Direction::ToCore) => format!("{}({expression})", metadata.rust_path),
        (NewtypeConversion::TupleField, Direction::FromCore) => format!("({expression}).0"),
    }
}

fn primitive_name(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(primitive) => primitive.rust_source_display(),
        _ => unreachable!("primitive_name requires a primitive"),
    }
}
