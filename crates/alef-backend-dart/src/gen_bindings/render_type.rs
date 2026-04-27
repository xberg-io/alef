use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{ParamDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::type_map::DartMapper;

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
            format!("List<{}>", render_type(inner, imports))
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
    format!("{ty_str} {}", p.name.to_lower_camel_case())
}
