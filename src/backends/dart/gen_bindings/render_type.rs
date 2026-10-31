use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::type_map::DartMapper;

/// Map numeric vectors to FRB's typed lists: full-width `u64` uses generalized
/// `Uint64List`; other integers widen to `i64` (`Int64List`) and floats to `f64`
/// (`Float64List`). `Vec<u8>` is preserved as `Uint8List`. Trait abstract
/// method declarations must use these typed names to satisfy the
/// FRB-generated `create_*_dart_impl` factory signatures.
fn dart_typed_list_for(inner: &TypeRef) -> Option<&'static str> {
    if let TypeRef::Primitive(p) = inner {
        match p {
            PrimitiveType::U8 => Some("Uint8List"),
            PrimitiveType::U64 => Some("Uint64List"),
            PrimitiveType::I8
            | PrimitiveType::U16
            | PrimitiveType::I16
            | PrimitiveType::U32
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => Some("Int64List"),
            PrimitiveType::F32 | PrimitiveType::F64 => Some("Float64List"),
            _ => None,
        }
    } else {
        None
    }
}

pub(super) fn render_type(ty: &TypeRef, imports: &mut BTreeSet<String>) -> String {
    match ty {
        TypeRef::Bytes => {
            imports.insert("import 'dart:typed_data';".to_string());
            DartMapper.map_type(ty)
        }
        TypeRef::Optional(inner) => {
            format!("{}?", render_type(inner, imports))
        }
        TypeRef::Vec(inner) => {
            if let Some(typed) = dart_typed_list_for(inner) {
                let library = if typed == "Uint64List" {
                    "package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart"
                } else {
                    "dart:typed_data"
                };
                imports.insert(format!("import '{library}';"));
                typed.to_string()
            } else {
                format!("List<{}>", render_type(inner, imports))
            }
        }
        TypeRef::Map(k, v) => {
            format!("Map<{}, {}>", render_type(k, imports), render_type(v, imports))
        }
        _ => DartMapper.map_type(ty),
    }
}

pub(super) fn format_param(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    let ty_str = if p.optional {
        format!("{}?", render_type(&p.ty, imports))
    } else {
        render_type(&p.ty, imports)
    };
    format!("{ty_str} {}", dart_safe_ident(&p.name.to_lower_camel_case()))
}
