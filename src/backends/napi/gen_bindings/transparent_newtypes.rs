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
    let value = crate::codegen::conversions::helpers::apply_field_newtype_to_core(
        "Default::default()",
        &field.ty,
        field.optional,
        wrapper,
    );
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
    repair_flattened_fields(generated, typ, Direction::ToCore)
}

pub(super) fn gen_from_core_to_binding(
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    config: &ConversionConfig<'_>,
) -> String {
    let generated = crate::codegen::conversions::gen_from_core_to_binding_cfg(typ, core_import, opaque_types, config);
    repair_flattened_fields(generated, typ, Direction::FromCore)
}

#[derive(Clone, Copy)]
enum Direction {
    ToCore,
    FromCore,
}

fn repair_flattened_fields(mut generated: String, typ: &TypeDef, direction: Direction) -> String {
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let TypeRef::Optional(inner) = &field.ty else {
            continue;
        };
        if !field.optional {
            continue;
        }
        let Some(wrapper) = field.newtype_wrapper.as_deref() else {
            continue;
        };
        let Ok(NewtypeWrapper::Explicit(mut paths)) = NewtypeWrapper::decode(wrapper) else {
            continue;
        };
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
                let converted = convert(&format!("val.{}", field.name), &field.ty, &paths, Direction::ToCore);
                format!("({converted}).map(Some)")
            }
            Direction::FromCore => convert(
                &format!("val.{}.flatten()", field.name),
                &TypeRef::Optional(inner.clone()),
                &paths,
                Direction::FromCore,
            ),
        };
        generated = replace_field_conversion(generated, &field.name, &expression).unwrap_or_else(|| {
            panic!(
                "generated NAPI conversion for flattened explicit-newtype field `{}.{}` had no replaceable field initializer or assignment",
                typ.name, field.name
            )
        });
    }
    generated
}

fn replace_field_conversion(generated: String, name: &str, expression: &str) -> Option<String> {
    let initializer_marker = format!("{name}: ");
    if let Some(replaced) = replace_delimited(
        &generated,
        &initializer_marker,
        ',',
        crate::codegen::template_env::render(
            "binding_helpers/struct_field_line.jinja",
            minijinja::context! { name => name, expr => expression },
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
            minijinja::context! { name => name, expr => expression },
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
    if let Some(path) = paths.iter().find(|path| path.containers.is_empty()) {
        return convert_wrapper_leaf(expression, path, direction);
    }

    match ty {
        TypeRef::Optional(inner) => {
            let nested = advance(paths, NewtypeContainer::Optional);
            let converted = convert("value", inner, &nested, direction);
            format!("({expression}).map(|value| {converted})")
        }
        TypeRef::Vec(inner) => {
            let nested = advance(paths, NewtypeContainer::Vec);
            let converted = convert("value", inner, &nested, direction);
            format!("({expression}).into_iter().map(|value| {converted}).collect()")
        }
        TypeRef::Map(key, value) => {
            let key_paths = advance(paths, NewtypeContainer::MapKey);
            let value_paths = advance(paths, NewtypeContainer::MapValue);
            let key = convert("key", key, &key_paths, direction);
            let value = convert("value", value, &value_paths, direction);
            format!("({expression}).into_iter().map(|(key, value)| ({key}, {value})).collect()")
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
    }
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
