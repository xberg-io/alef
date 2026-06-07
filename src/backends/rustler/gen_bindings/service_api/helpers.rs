//! Shared service-API type mapping and lookup helpers.

use crate::backends::rustler::template_env::render;
use crate::core::ir::{ApiSurface, HandlerContractDef, TypeRef};
use minijinja::context;

/// Convert a `TypeRef` to a simple Elixir type annotation string.
#[allow(dead_code)]
fn elixir_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String.t()".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean()".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float()".to_owned(),
                _ => "integer()".to_owned(),
            }
        }
        TypeRef::Bytes => "binary()".to_owned(),
        TypeRef::Optional(inner) => format!("{} | nil", elixir_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("list({})", elixir_type_annotation(inner)),
        TypeRef::Map(k, v) => format!(
            "map() :: %{{optional({}) => {}}}",
            elixir_type_annotation(k),
            elixir_type_annotation(v)
        ),
        TypeRef::Unit => "nil".to_owned(),
        TypeRef::Named(n) => n.to_string(),
        TypeRef::Json => "any()".to_owned(),
        TypeRef::Path => "String.t()".to_owned(),
        TypeRef::Duration => "non_neg_integer()".to_owned(),
    }
}

pub(super) fn push_elixir_param(params: &mut String, name: &str, optional: bool) {
    params.push_str(", ");
    params.push_str(name);
    if optional {
        params.push_str(" \\\\ nil");
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
pub(super) fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Format a Rust doc as an Elixir heredoc body at the given column indent.
/// Returns just the lines between `"""` markers (does not emit the markers
/// themselves). Each non-blank source line is indented to `indent` spaces so
/// the closing `"""` at the same column strips that prefix from the heredoc
/// at compile time; blank lines stay bare.
pub(super) fn elixir_heredoc_body(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub(super) fn push_elixir_doc(out: &mut String, doc: &str, attr: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(&render(
        "service_api_doc.ex.jinja",
        context! {
            attr => attr,
            body => elixir_heredoc_body(doc, 2),
        },
    ));
}

/// Map a `TypeRef` to a Rust type string for use in generated NIF signatures.
pub(super) fn typeref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "u8".to_owned(),
                PrimitiveType::U16 => "u16".to_owned(),
                PrimitiveType::U32 => "u32".to_owned(),
                PrimitiveType::U64 => "u64".to_owned(),
                PrimitiveType::I8 => "i8".to_owned(),
                PrimitiveType::I16 => "i16".to_owned(),
                PrimitiveType::I32 => "i32".to_owned(),
                PrimitiveType::I64 => "i64".to_owned(),
                PrimitiveType::F32 => "f32".to_owned(),
                PrimitiveType::F64 => "f64".to_owned(),
                PrimitiveType::Usize => "usize".to_owned(),
                PrimitiveType::Isize => "isize".to_owned(),
            }
        }
        TypeRef::Bytes => "Vec<u8>".to_owned(),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_rust_type(k, core_import),
            typeref_to_rust_type(v, core_import)
        ),
        TypeRef::Unit => "()".to_owned(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
        TypeRef::Json => "serde_json::Value".to_owned(),
        TypeRef::Path => "std::path::PathBuf".to_owned(),
        TypeRef::Duration => "std::time::Duration".to_owned(),
    }
}
