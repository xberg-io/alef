use super::{Chunk, Record};
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::Language;
use crate::core::ir::{ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeRef};

fn scalar(field: &FieldDef) -> bool {
    !field.optional
        && !field.sanitized
        && matches!(
            field.ty,
            TypeRef::String
                | TypeRef::Primitive(
                    PrimitiveType::Bool
                        | PrimitiveType::I8
                        | PrimitiveType::I16
                        | PrimitiveType::I32
                        | PrimitiveType::I64
                        | PrimitiveType::Isize
                        | PrimitiveType::U8
                        | PrimitiveType::U16
                        | PrimitiveType::U32
                        | PrimitiveType::U64
                        | PrimitiveType::Usize
                )
        )
}

fn field_index(variant: &ErrorVariant, name: &str) -> Option<usize> {
    super::super::gen_rust_crate::error_fields::visible(variant)
        .enumerate()
        .find(|(_, (index, field))| {
            let matches = if let Ok(position) = name.parse::<usize>() {
                variant.is_tuple && position == *index
            } else {
                field.name == name
            };
            matches && scalar(field)
        })
        .map(|(visible, _)| visible)
}

fn placeholder(characters: &mut impl Iterator<Item = char>) -> Option<String> {
    let mut name = String::new();
    for character in characters.by_ref() {
        match character {
            '}' => return Some(name),
            '{' => return None,
            other => name.push(other),
        }
    }
    None
}

fn chunks(variant: &ErrorVariant) -> Option<Vec<Chunk>> {
    let mut characters = variant.message_template.as_deref()?.chars().peekable();
    let mut text = String::new();
    let mut result = Vec::new();
    while let Some(character) = characters.next() {
        if matches!(character, '{' | '}') && characters.peek() == Some(&character) {
            characters.next();
            text.push(character);
            continue;
        }
        match character {
            '{' => {
                result.push(Chunk::Text {
                    text: std::mem::take(&mut text),
                });
                let name = placeholder(&mut characters)?;
                result.push(Chunk::Field {
                    field: field_index(variant, &name)?,
                });
            }
            '}' => return None,
            other => text.push(other),
        }
    }
    result.push(Chunk::Text { text });
    Some(result)
}

pub(crate) fn records(errors: &[&ErrorDef]) -> Vec<Record> {
    errors
        .iter()
        .filter(|error| !error.variants.iter().all(|variant| variant.is_unit))
        .flat_map(|error| {
            error.variants.iter().filter_map(move |variant| {
                let fields = super::super::gen_rust_crate::error_fields::visible(variant).count();
                let (fields, chunks) = if variant.is_tuple && fields == 0 {
                    (1, vec![Chunk::Field { field: 0 }])
                } else {
                    (fields, chunks(variant)?)
                };
                Some(Record {
                    owner: super::super::naming::frb_dart_type_name(&error.name),
                    factory: public_host_identifier(Language::Dart, PublicIdentifierKind::Method, &variant.name),
                    fields,
                    chunks,
                })
            })
        })
        .collect()
}

// Machine-readable metadata must survive the consumer's uncomment pass between generation and build. ~keep
pub(crate) fn metadata_comments(errors: &[&ErrorDef]) -> String {
    let records = records(errors)
        .iter()
        .map(|record| serde_json::to_string(record).expect("error display metadata is serializable"))
        .collect::<Vec<_>>();
    if records.is_empty() {
        return String::new();
    }
    super::super::template_env::make_env()
        .get_template("rust_error_display_metadata.jinja")
        .expect("registered display metadata template")
        .render(crate::alef_context! { records })
        .expect("rendering error display metadata")
}
