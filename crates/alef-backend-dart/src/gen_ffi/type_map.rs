use alef_core::ir::{ParamDef, PrimitiveType, TypeRef};

use crate::type_map::DartMapper;
use alef_codegen::type_mapper::TypeMapper;
use heck::ToLowerCamelCase;

/// The `dart:ffi` native C type for a function parameter (in the native typedef).
pub(super) fn native_param_type(p: &ParamDef) -> String {
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Utf8>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => native_primitive(prim),
        TypeRef::Char => "Uint32".to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// The Dart callable type for a function parameter (in the Dart typedef).
pub(super) fn dart_callable_type(p: &ParamDef) -> String {
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Utf8>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => dart_primitive_callable(prim),
        TypeRef::Char => "int".to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// Dart public wrapper parameter declaration (e.g. `String name`).
pub(super) fn dart_wrapper_param(p: &ParamDef) -> String {
    let ty = dart_type(&p.ty, p.optional);
    let name = p.name.to_lower_camel_case();
    format!("{ty} {name}")
}

/// Argument expression to pass into the low-level `_fnName` call.
pub(super) fn call_arg_name(p: &ParamDef) -> String {
    let name = p.name.to_lower_camel_case();
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{name}Native.cast<Utf8>()")
        }
        _ => name,
    }
}

/// Native C return type (used in the native typedef).
pub(super) fn native_return_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "Void".to_string(),
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Char>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => native_primitive(prim),
        TypeRef::Char => "Uint32".to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// Dart callable return type (used in the Dart typedef).
pub(super) fn dart_callable_return(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "void".to_string(),
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Char>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => dart_primitive_callable(prim),
        TypeRef::Char => "int".to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// Public Dart return type in the wrapper function signature.
pub(super) fn dart_public_return(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "void".to_string(),
        _ => dart_type(ty, false),
    }
}

/// Convert a raw C return value to the public Dart type.
pub(super) fn unwrap_return_expr(raw: &str, ty: &TypeRef, _free_symbol: &str, _error_code_symbol: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json => {
            // Marshal the null-terminated C string to Dart, then free the FFI allocation.
            format!(
                "() {{ final s = {raw}.cast<Utf8>().toDartString(); _freeString({raw}.cast<Char>()); return s; }}()"
            )
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Vec/Map are serialised as JSON strings across the C boundary.
            format!(
                "() {{ final s = {raw}.cast<Utf8>().toDartString(); _freeString({raw}.cast<Char>()); return s; }}()"
            )
        }
        _ => raw.to_string(),
    }
}

/// `dart:ffi` native (C) type for a primitive.
pub(super) fn native_primitive(prim: &PrimitiveType) -> String {
    match prim {
        PrimitiveType::Bool => "Bool".to_string(),
        PrimitiveType::U8 => "Uint8".to_string(),
        PrimitiveType::I8 => "Int8".to_string(),
        PrimitiveType::U16 => "Uint16".to_string(),
        PrimitiveType::I16 => "Int16".to_string(),
        PrimitiveType::U32 => "Uint32".to_string(),
        PrimitiveType::I32 => "Int32".to_string(),
        PrimitiveType::U64 => "Uint64".to_string(),
        PrimitiveType::I64 => "Int64".to_string(),
        PrimitiveType::Usize => "Size".to_string(),
        PrimitiveType::Isize => "IntPtr".to_string(),
        PrimitiveType::F32 => "Float".to_string(),
        PrimitiveType::F64 => "Double".to_string(),
    }
}

/// Dart callable (non-native) type for a primitive.
pub(super) fn dart_primitive_callable(prim: &PrimitiveType) -> String {
    match prim {
        PrimitiveType::Bool => "bool".to_string(),
        PrimitiveType::F32 | PrimitiveType::F64 => "double".to_string(),
        _ => "int".to_string(),
    }
}

/// Public Dart type (high-level) for a type ref.
pub(super) fn dart_type(ty: &TypeRef, optional: bool) -> String {
    let inner = match ty {
        TypeRef::Bytes => "Uint8List".to_string(),
        TypeRef::Optional(inner) => return dart_type(inner, true),
        TypeRef::Vec(inner) => format!("List<{}>", dart_type(inner, false)),
        TypeRef::Map(k, v) => format!("Map<{}, {}>", dart_type(k, false), dart_type(v, false)),
        TypeRef::Primitive(prim) => DartMapper.primitive(prim).into_owned(),
        _ => DartMapper.map_type(ty),
    };
    if optional { format!("{inner}?") } else { inner }
}

pub(super) fn dart_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}
