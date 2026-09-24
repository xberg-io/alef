use std::collections::HashSet;

use crate::backends::kotlin::to_lower_camel;
use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};

use super::helpers::{jni_zero_literal, kotlin_nullable_type_for_optional, unwrap_optional};

pub(super) fn is_dto_named(ty: &TypeRef, opaque_types: &HashSet<String>) -> bool {
    matches!(ty, TypeRef::Named(name) if !opaque_types.contains(name))
}

pub(super) fn is_generic_container(ty: &TypeRef) -> bool {
    matches!(unwrap_optional(ty), TypeRef::Vec(_) | TypeRef::Map(_, _))
}

pub(super) fn facade_return_type(ty: &TypeRef, opaque_types: &HashSet<String>) -> String {
    if let TypeRef::Named(name) = ty {
        return name.clone();
    }
    if is_generic_container(ty) {
        return render_kotlin_type(ty, opaque_types);
    }
    jni_return_type_str(ty).to_string()
}

fn facade_param_type(ty: &TypeRef, opaque_types: &HashSet<String>) -> String {
    let inner = unwrap_optional(ty);
    if let TypeRef::Named(name) = inner {
        return name.clone();
    }
    if is_binary(inner) {
        return "ByteArray".to_string();
    }
    if is_generic_container(inner) {
        return render_kotlin_type(inner, opaque_types);
    }
    jni_param_type_str(ty).to_string()
}

pub(super) fn facade_param(param: &ParamDef, opaque_types: &HashSet<String>) -> String {
    let name = to_lower_camel(&param.name);
    let inner = unwrap_optional(&param.ty);
    if !param.optional {
        return format!("{name}: {}", facade_param_type(&param.ty, opaque_types));
    }
    if is_dto_named(inner, opaque_types) {
        let TypeRef::Named(type_name) = inner else {
            unreachable!()
        };
        return format!("{name}: {type_name}? = null");
    }
    if matches!(inner, TypeRef::Named(type_name) if opaque_types.contains(type_name)) {
        return format!("{name}: {} = null", facade_param_type(&param.ty, opaque_types));
    }
    format!("{name}: {} = null", kotlin_nullable_type_for_optional(&param.ty))
}

pub(super) fn bridge_arg(param: &ParamDef, opaque_types: &HashSet<String>) -> String {
    let name = to_lower_camel(&param.name);
    let inner = unwrap_optional(&param.ty);
    if let TypeRef::Named(type_name) = inner {
        if opaque_types.contains(type_name) {
            return format!("{name}.handle");
        }
        return json_bridge_arg(&name, param.optional);
    }
    if is_binary(inner) {
        return binary_bridge_arg(&name, param.optional);
    }
    if is_generic_container(inner) {
        if let Some(element) = dto_list_element(inner) {
            return typed_list_bridge_arg(&name, element, param.optional);
        }
        return json_bridge_arg(&name, param.optional);
    }
    if param.optional {
        return format!("{name} ?: {}", jni_zero_literal(inner));
    }
    name
}

/// The element type of a `Vec<Named>`, which is the only shape that needs a type-pinned writer.
fn dto_list_element(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Serialize a `List<Dto>` through a writer pinned to the declared element type.
///
/// ~keep A bare `mapper.writeValueAsString(list)` erases the element type to `Object`. Jackson
/// then picks each element's serializer from its runtime class with no base-type context, so the
/// `@JsonTypeInfo` discriminator declared on a sealed base is never written and a strict native
/// deserializer rejects the payload with "missing field `type`". The annotations on the sealed
/// base are not enough on their own: nothing else tells Jackson what the declared element type
/// is, and a consumer whose native side is strict rejects every such call. The Java emitter pins
/// it via `constructCollectionType`; this is the Kotlin equivalent. Applied to every `List<Dto>`,
/// not just sealed ones, because pinning a declared type is correct regardless and a per-type
/// carve-out would silently miss the next polymorphic DTO.
fn typed_list_bridge_arg(name: &str, element: &str, optional: bool) -> String {
    let writer = format!(
        "mapper.writerFor(mapper.typeFactory.constructCollectionType(List::class.java, {element}::class.java))"
    );
    if optional {
        format!("{name}?.let {{ {writer}.writeValueAsString(it) }} ?: \"\"")
    } else {
        format!("{writer}.writeValueAsString({name})")
    }
}

fn json_bridge_arg(name: &str, optional: bool) -> String {
    if optional {
        format!("{name}?.let {{ mapper.writeValueAsString(it) }} ?: \"\"")
    } else {
        format!("mapper.writeValueAsString({name})")
    }
}

fn binary_bridge_arg(name: &str, optional: bool) -> String {
    if optional {
        format!("{name}?.let {{ java.util.Base64.getEncoder().encodeToString(it) }} ?: \"\"")
    } else {
        format!("java.util.Base64.getEncoder().encodeToString({name})")
    }
}

fn is_binary(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Bytes)
        || matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)))
}

fn jni_return_type_str(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "Unit",
        TypeRef::Primitive(primitive) => primitive_kotlin_type(primitive),
        TypeRef::String => "String",
        TypeRef::Optional(_) => "String?",
        TypeRef::Bytes => "ByteArray",
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => "ByteArray",
        TypeRef::Vec(_) | TypeRef::Named(_) | TypeRef::Map(_, _) => "String",
        _ => "Long",
    }
}

pub(super) fn jni_param_type_str(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(primitive) => primitive_kotlin_type(primitive),
        TypeRef::String => "String",
        TypeRef::Bytes | TypeRef::Vec(_) => "String",
        _ => "String",
    }
}

fn primitive_kotlin_type(primitive: &PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Bool => "Boolean",
        PrimitiveType::I8 | PrimitiveType::U8 => "Byte",
        PrimitiveType::I16 | PrimitiveType::U16 => "Short",
        PrimitiveType::I32 | PrimitiveType::U32 => "Int",
        PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long",
        PrimitiveType::F32 => "Float",
        PrimitiveType::F64 => "Double",
    }
}

pub(super) fn render_kotlin_type(ty: &TypeRef, opaque_types: &HashSet<String>) -> String {
    match ty {
        TypeRef::Unit => "Unit".to_string(),
        TypeRef::Primitive(primitive) => primitive_kotlin_type(primitive).to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String".to_string(),
        TypeRef::Bytes => "ByteArray".to_string(),
        TypeRef::Json => "Any".to_string(),
        TypeRef::Duration => "Long".to_string(),
        TypeRef::Named(name) => {
            let _ = opaque_types;
            name.clone()
        }
        TypeRef::Vec(inner) => format!("List<{}>", render_kotlin_type(inner, opaque_types)),
        TypeRef::Map(key, value) => format!(
            "Map<{}, {}>",
            render_kotlin_type(key, opaque_types),
            render_kotlin_type(value, opaque_types)
        ),
        TypeRef::Optional(inner) => format!("{}?", render_kotlin_type(inner, opaque_types)),
    }
}
