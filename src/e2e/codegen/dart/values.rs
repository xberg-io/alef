//! Dart value- and type-mapping helpers for e2e generation.

use crate::core::ir::TypeRef;

pub(super) fn render_native_dart_dto(
    type_name: &str,
    value: &serde_json::Value,
    type_defs: &[crate::core::ir::TypeDef],
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> Option<String> {
    render_native_dart_dto_at(type_name, value, type_defs, files, "")
}

fn render_native_dart_dto_at(
    type_name: &str,
    value: &serde_json::Value,
    type_defs: &[crate::core::ir::TypeDef],
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
) -> Option<String> {
    let object = value.as_object()?;
    let type_def = type_defs.iter().find(|candidate| candidate.name == type_name)?;
    let fields = type_def
        .fields
        .iter()
        .filter(|field| !field.binding_excluded && field.cfg.is_none())
        .map(|field| {
            let field_value = match object.get(&field.name) {
                Some(value) => value,
                None if field.optional || matches!(field.ty, TypeRef::Optional(_)) => &serde_json::Value::Null,
                None => return None,
            };
            let name =
                crate::codegen::naming::public_field_name(crate::core::config::Language::Dart, &field.name, None);
            let field_pointer = format!("{pointer}/{}", field.name);
            let value = render_native_dart_value(field_value, &field.ty, type_defs, files, &field_pointer)?;
            Some(crate::alef_context! { name => name, value => value })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(
        crate::e2e::template_env::render(
            "dart/typed_dto.jinja",
            crate::alef_context! { type_name => type_name, fields => fields },
        )
        .trim_end()
        .to_string(),
    )
}

fn render_native_dart_value(
    value: &serde_json::Value,
    ty: &TypeRef,
    type_defs: &[crate::core::ir::TypeDef],
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
) -> Option<String> {
    if files.iter().any(|file| file.field == pointer) && matches!(ty, TypeRef::Bytes) {
        return Some(format!("File('{}').readAsBytesSync()", escape_dart(value.as_str()?)));
    }
    match (value, ty) {
        (serde_json::Value::Null, _) => Some("null".into()),
        (value, TypeRef::Optional(inner)) => render_native_dart_value(value, inner, type_defs, files, pointer),
        (value, TypeRef::Named(name)) => render_native_dart_dto_at(name, value, type_defs, files, pointer),
        (serde_json::Value::Array(values), TypeRef::Vec(inner)) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                render_native_dart_value(value, inner, type_defs, files, &format!("{pointer}/{index}"))
            })
            .collect::<Option<Vec<_>>>()
            .map(|items| format!("[{}]", items.join(", "))),
        (serde_json::Value::String(value), TypeRef::String | TypeRef::Char | TypeRef::Path) => {
            Some(format!("'{}'", escape_dart(value)))
        }
        (serde_json::Value::Bool(value), TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) => {
            Some(value.to_string())
        }
        (serde_json::Value::Number(value), TypeRef::Primitive(_)) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn mime_from_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    match ext.to_lowercase().as_str() {
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "pdf" => Some("application/pdf"),
        "txt" | "text" => Some("text/plain"),
        "html" | "htm" => Some("text/html"),
        "json" => Some("application/json"),
        "xml" => Some("application/xml"),
        "csv" => Some("text/csv"),
        "md" | "markdown" => Some("text/markdown"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "zip" => Some("application/zip"),
        "odt" => Some("application/vnd.oasis.opendocument.text"),
        "ods" => Some("application/vnd.oasis.opendocument.spreadsheet"),
        "odp" => Some("application/vnd.oasis.opendocument.presentation"),
        "rtf" => Some("application/rtf"),
        "epub" => Some("application/epub+zip"),
        "msg" => Some("application/vnd.ms-outlook"),
        "eml" => Some("message/rfc822"),
        // Source-code extensions resolve to the internal `text/x-source-code` MIME.
        // The bytes-path can't extract these (CodeExtractor::extract_bytes needs a
        // shebang for language detection), so the caller code in this module
        // checks the inferred MIME and routes source-code files through
        // `extractFileSync`/`extractFile` (path-based) instead of remapping to
        // the bytes facade.
        "py" | "rs" | "go" | "java" | "kt" | "kts" | "swift" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "rb"
        | "php" | "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "cs" | "scala" | "ex" | "exs" | "erl"
        | "hrl" | "elm" | "ml" | "mli" | "fs" | "fsx" | "hs" | "lhs" | "lua" | "pl" | "pm" | "r" | "R" | "sh"
        | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "psd1" | "dart" | "groovy" | "gd" | "nim" | "zig" | "v"
        | "vhdl" | "sv" | "svh" => Some("text/x-source-code"),
        _ => None,
    }
}

/// Escape a string for embedding in a Dart single-quoted string literal.
pub(in crate::e2e::codegen) fn escape_dart(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('$', "\\$")
}

/// Derive the Dart top-level helper function name for constructing a mirror type from JSON.
///
/// The alef dart bridge-crate generator emits a Rust free function
/// `create_<snake_type>_from_json(json: String)` for each non-opaque mirror struct.
/// FRB generates the corresponding Dart function as `createTypeNameFromJson` (camelCase).
///
/// Example: `"ChatCompletionRequest"` → `"createChatCompletionRequestFromJson"`.
pub(super) fn type_name_to_create_from_json_dart(type_name: &str) -> String {
    // Convert PascalCase type name to snake_case.
    let mut snake = String::with_capacity(type_name.len() + 8);
    for (i, ch) in type_name.char_indices() {
        if ch.is_uppercase() {
            if i > 0 {
                snake.push('_');
            }
            snake.extend(ch.to_lowercase());
        } else {
            snake.push(ch);
        }
    }
    // snake is now e.g. "chat_completion_request"
    // Full Rust function name: "create_chat_completion_request_from_json"
    let rust_fn = format!("create_{snake}_from_json");
    // Convert to Dart camelCase: "createChatCompletionRequestFromJson"
    rust_fn
        .split('_')
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Build the Dart stringy field classification map for aggregating text accessors
/// in `Vec<T>` contains assertions. Similar to Swift's `build_swift_first_class_map`,
/// but Dart doesn't distinguish first-class vs opaque types — we just track stringy
/// fields per type for the `contains(where:)` closure aggregator.
pub(super) fn build_dart_first_class_map(
    type_defs: &[crate::core::ir::TypeDef],
    enum_defs: &[crate::core::ir::EnumDef],
    e2e_config: &crate::e2e::config::E2eConfig,
) -> crate::e2e::field_access::DartFirstClassMap {
    use crate::core::ir::TypeRef;
    use crate::e2e::field_access::{StringyField, StringyFieldKind};

    let mut field_types: std::collections::HashMap<String, std::collections::HashMap<String, String>> =
        std::collections::HashMap::new();

    fn inner_named(ty: &TypeRef) -> Option<String> {
        match ty {
            TypeRef::Named(n) => Some(n.clone()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }

    let enum_names: std::collections::HashSet<&str> = enum_defs.iter().map(|e| e.name.as_str()).collect();
    let classify_stringy = |ty: &TypeRef, field_optional: bool| -> Option<StringyFieldKind> {
        match ty {
            TypeRef::String => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Optional),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Optional),
                _ => None,
            },
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Vec),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Vec),
                _ => None,
            },
            _ => None,
        }
    };

    let mut stringy_fields_by_type: std::collections::HashMap<String, Vec<StringyField>> =
        std::collections::HashMap::new();
    for td in type_defs {
        let mut td_field_types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut td_stringy: Vec<StringyField> = Vec::new();
        for f in &td.fields {
            if let Some(named) = inner_named(&f.ty) {
                td_field_types.insert(f.name.clone(), named);
            }
            if f.binding_excluded {
                continue;
            }
            if let Some(kind) = classify_stringy(&f.ty, f.optional) {
                td_stringy.push(StringyField {
                    name: f.name.clone(),
                    kind,
                });
            }
        }
        if !td_field_types.is_empty() {
            field_types.insert(td.name.clone(), td_field_types);
        }
        if !td_stringy.is_empty() {
            stringy_fields_by_type.insert(td.name.clone(), td_stringy);
        }
    }

    // Best-effort root-type detection: pick a unique TypeDef that contains all
    // `result_fields`.
    let root_type = if e2e_config.result_fields.is_empty() {
        None
    } else {
        let matches: Vec<&crate::core::ir::TypeDef> = type_defs
            .iter()
            .filter(|td| {
                let names: std::collections::HashSet<&str> = td.fields.iter().map(|f| f.name.as_str()).collect();
                e2e_config.result_fields.iter().all(|rf| names.contains(rf.as_str()))
            })
            .collect();
        if matches.len() == 1 {
            Some(matches[0].name.clone())
        } else {
            None
        }
    };

    crate::e2e::field_access::DartFirstClassMap {
        field_types,
        root_type,
        stringy_fields_by_type,
    }
}

#[cfg(test)]
mod native_dto_tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef};

    #[test]
    fn renders_known_struct_as_native_dart_constructor() {
        let type_defs = [TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "display_name".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let rendered = render_native_dart_dto(
            "SampleRequest",
            &serde_json::json!({"display_name": "Ada"}),
            &type_defs,
            &[],
        );

        assert_eq!(rendered.as_deref(), Some("SampleRequest(displayName: 'Ada')"));
    }

    #[test]
    fn renders_file_pointer_as_uint8_list_read() {
        let type_defs = [TypeDef {
            name: "Upload".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::Bytes,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let files = [crate::e2e::fixture::FixtureDocsFileInput {
            field: "/content".into(),
            path: "guide.pdf".into(),
        }];
        let rendered = render_native_dart_dto(
            "Upload",
            &serde_json::json!({"content": "guide.pdf"}),
            &type_defs,
            &files,
        )
        .expect("native DTO");
        assert!(
            rendered.contains("content: File('guide.pdf').readAsBytesSync()"),
            "{rendered}"
        );
    }
}
