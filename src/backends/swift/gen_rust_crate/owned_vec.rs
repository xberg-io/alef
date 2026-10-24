//! Owned-element accessors for getters that bridge a `Vec` of handles or strings.
//!
//! swift-bridge returns such a getter as a `RustVec<Element>`. Indexing it yields an element
//! reference that points into the vector's own buffer and does not keep the vector alive, so
//! `result.items()[0].name()` reads freed memory once Swift releases the temporary vector.
//! swift-bridge documents this as the caller's duty and offers no owning element form.
//!
//! The extern declaration of such a getter therefore gets the Swift name `<field>RustVec`, and a
//! Swift extension in the `RustBridge` target declares the original `<field>()` on top of it. The
//! extension moves every element out of the vector with `pop()`, which yields owned values, and
//! returns a Swift array.
//!
//! A free function or a method that returns such a vector gets the same treatment: its extern
//! declaration takes the raw name and a Swift function declares the original name on top of it.

use super::bridge_error::BRIDGE_ERROR_TYPE;
use super::extern_block::{BridgeFn, getter_bridge_type};
use super::wrappers::is_unbridgeable_getter;
use crate::backends::swift::gen_rust_crate::default_construction::EnumKinds;
use crate::core::backend::GeneratedFile;
use crate::core::ir::TypeDef;
use crate::core::keywords::swift_ident;
use heck::ToLowerCamelCase;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The Swift element type of a bridged vector getter, and whether the getter is optional.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OwnedVecElement {
    pub swift_type: String,
    pub optional: bool,
}

/// One getter that returns an owned Swift array instead of a `RustVec`.
#[derive(Debug, Serialize)]
pub(crate) struct OwnedVecGetter {
    pub swift_name: String,
    pub raw_name: String,
    pub element: String,
    pub optional: bool,
}

/// The Swift name of the extern getter that returns the raw `RustVec`.
pub(crate) fn bridge_getter_swift_name(field_name: &str) -> String {
    format!("{}RustVec", field_name.to_lower_camel_case())
}

/// The Swift name of the extern function that returns the raw `RustVec`.
pub(crate) fn raw_swift_name(public_name: &str) -> String {
    format!("{public_name}RustVec")
}

/// Reads a swift-bridge return type and reports the element type when it is a `RustVec` whose
/// elements are indexed by reference: `String` elements (`RustString`) and opaque handles.
///
/// Primitive vectors are indexed by value and are safe, so they return `None`. A nested
/// container, a map or a lowercase primitive name also returns `None`.
pub(crate) fn owned_vec_element(bridge_ty: &str) -> Option<OwnedVecElement> {
    let (inner, optional) = match bridge_ty.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        Some(inner) => (inner, true),
        None => (bridge_ty, false),
    };
    let element = inner.strip_prefix("Vec<")?.strip_suffix('>')?;
    let swift_type = if element == "String" {
        "RustString".to_string()
    } else if element.starts_with(|c: char| c.is_ascii_uppercase())
        && element.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        element.to_string()
    } else {
        return None;
    };
    Some(OwnedVecElement { swift_type, optional })
}

/// An owned-array return of an extern function: the element type, and whether the function
/// throws.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OwnedVecReturn {
    pub element: OwnedVecElement,
    pub throws: bool,
}

/// Reads the return type of an extern function. A `Result` return keeps its error type out of the
/// Swift signature: swift-bridge maps it to `throws`.
pub(crate) fn owned_vec_return(return_ty: &str) -> Option<OwnedVecReturn> {
    let error_tail = format!(", {BRIDGE_ERROR_TYPE}>");
    let (ok, throws) = match return_ty.strip_prefix("Result<").and_then(|s| s.strip_suffix(error_tail.as_str())) {
        Some(ok) => (ok, true),
        None => (return_ty, false),
    };
    Some(OwnedVecReturn {
        element: owned_vec_element(ok)?,
        throws,
    })
}

/// A free function that returns an owned Swift array on top of a raw `RustVec` extern.
#[derive(Debug, Serialize)]
pub(crate) struct OwnedVecFunction {
    pub swift_name: String,
    pub generics: String,
    pub params: String,
    pub is_async: bool,
    pub throws: bool,
    pub return_type: String,
    pub body: String,
}

/// Maps a swift-bridge parameter type to the Swift type that swift-bridge generates for it.
///
/// `String` becomes the shared generic `GenericIntoRustString`, as in every swift-bridge
/// signature. A type outside this small grammar is an error, so a function the wrapper cannot
/// forward never ships with the unsafe return type.
fn swift_param_type(bridge_ty: &str, uses_string: &mut bool) -> Option<String> {
    if let Some(inner) = bridge_ty.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        return Some(format!("Optional<{}>", swift_param_type(inner, uses_string)?));
    }
    if let Some(inner) = bridge_ty.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        return Some(format!("RustVec<{}>", swift_param_type(inner, uses_string)?));
    }
    if let Some(inner) = bridge_ty.strip_prefix("&mut ") {
        return Some(format!("{}RefMut", swift_param_type(inner, uses_string)?));
    }
    if let Some(inner) = bridge_ty.strip_prefix('&') {
        return Some(format!("{}Ref", swift_param_type(inner, uses_string)?));
    }
    let primitive = match bridge_ty {
        "bool" => "Bool",
        "u8" => "UInt8",
        "u16" => "UInt16",
        "u32" => "UInt32",
        "u64" => "UInt64",
        "usize" => "UInt",
        "i8" => "Int8",
        "i16" => "Int16",
        "i32" => "Int32",
        "i64" => "Int64",
        "isize" => "Int",
        "f32" => "Float",
        "f64" => "Double",
        _ => "",
    };
    if !primitive.is_empty() {
        return Some(primitive.to_string());
    }
    if bridge_ty == "String" {
        *uses_string = true;
        return Some("GenericIntoRustString".to_string());
    }
    let is_handle = bridge_ty.starts_with(|c: char| c.is_ascii_uppercase())
        && bridge_ty.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    is_handle.then(|| bridge_ty.to_string())
}

/// The Swift function that returns an owned array for `bridge_fn`, or `None` when the function
/// does not return a vector of handles or strings.
pub(crate) fn owned_vec_function(bridge_fn: &BridgeFn) -> anyhow::Result<Option<OwnedVecFunction>> {
    let Some(ret) = owned_vec_return(&bridge_fn.return_ty) else {
        return Ok(None);
    };
    let mut uses_string = false;
    let mut decls = Vec::new();
    let mut args = Vec::new();
    for (name, ty) in &bridge_fn.params {
        let swift_ty = swift_param_type(ty, &mut uses_string).ok_or_else(|| {
            anyhow::anyhow!(
                "Swift function {} returns a vector but its parameter {name}: {ty} has no owned-array wrapper",
                bridge_fn.public_name
            )
        })?;
        decls.push(format!("_ {name}: {swift_ty}"));
        args.push(name.clone());
    }
    let call = format!(
        "{}{}{}({})",
        if ret.throws { "try " } else { "" },
        if bridge_fn.is_async { "await " } else { "" },
        bridge_fn.swift_name,
        args.join(", ")
    );
    let (return_type, body) = if ret.element.optional {
        (
            format!("[{}]?", ret.element.swift_type),
            format!("({call}).map {{ $0.alefIntoOwnedArray() }}"),
        )
    } else {
        (format!("[{}]", ret.element.swift_type), format!("{call}.alefIntoOwnedArray()"))
    };
    Ok(Some(OwnedVecFunction {
        swift_name: bridge_fn.public_name.clone(),
        generics: if uses_string {
            "<GenericIntoRustString: IntoRustString>".to_string()
        } else {
            String::new()
        },
        params: decls.join(", "),
        is_async: bridge_fn.is_async,
        throws: ret.throws,
        return_type,
        body,
    }))
}

/// The getters of `ty` that need an owned-array accessor, in field order.
#[allow(clippy::too_many_arguments)]
pub(crate) fn owned_vec_getters(
    ty: &TypeDef,
    exclude_fields: &HashSet<String>,
    type_paths: &HashMap<String, String>,
    no_serde_names: &HashSet<&str>,
    first_class_names: &HashSet<&str>,
    enum_kinds: EnumKinds<'_, '_>,
    text_types: &HashSet<&str>,
    configured_features: &HashSet<&str>,
) -> Vec<OwnedVecGetter> {
    let parent_first_class = first_class_names.contains(ty.name.as_str());
    ty.fields
        .iter()
        .filter(|field| {
            !is_unbridgeable_getter(
                ty,
                field,
                exclude_fields,
                type_paths,
                no_serde_names,
                configured_features,
            )
        })
        .filter_map(|field| {
            let bridge_ty = getter_bridge_type(field, enum_kinds, no_serde_names, parent_first_class, text_types);
            let element = owned_vec_element(&bridge_ty)?;
            Some(OwnedVecGetter {
                swift_name: swift_ident(&field.name.to_lower_camel_case()),
                raw_name: bridge_getter_swift_name(&field.name),
                element: element.swift_type,
                optional: element.optional,
            })
        })
        .collect()
}

/// A type name with the owned-array getters it needs.
pub(crate) type TypeOwnedVecGetters = (String, Vec<OwnedVecGetter>);

/// Everything the generated Swift accessor file declares.
#[derive(Debug, Default)]
pub(crate) struct OwnedVecAccessors {
    pub types: Vec<TypeOwnedVecGetters>,
    pub functions: Vec<OwnedVecFunction>,
}

#[derive(Serialize)]
struct TypeAccessors<'a> {
    name: &'a str,
    getters: &'a [OwnedVecGetter],
}

/// The Swift file with every owned-array accessor, or `None` when no getter needs one.
pub(crate) fn emit_swift_file(accessors: &OwnedVecAccessors, rust_bridge_sources: &Path) -> Option<GeneratedFile> {
    let functions = &accessors.functions;
    let entries: Vec<TypeAccessors<'_>> = accessors
        .types
        .iter()
        .filter(|(_, getters)| !getters.is_empty())
        .map(|(name, getters)| TypeAccessors { name, getters })
        .collect();
    if entries.is_empty() && functions.is_empty() {
        return None;
    }
    let content = crate::backends::swift::template_env::render(
        "swift_owned_vec_accessors.swift.jinja",
        crate::alef_context! {
            header => crate::core::hash::SELF_MARKING_HEADER_LINE,
            types => &entries,
            functions => functions,
        },
    );
    Some(GeneratedFile {
        path: rust_bridge_sources.join("ZSwiftOwnedVecAccessors.swift"),
        content,
        generated_header: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_and_handle_vectors_are_owned_and_primitives_are_not() {
        let owned = |ty: &str| owned_vec_element(ty);
        assert_eq!(
            owned("Vec<Choice>"),
            Some(OwnedVecElement {
                swift_type: "Choice".into(),
                optional: false
            })
        );
        assert_eq!(
            owned("Vec<String>"),
            Some(OwnedVecElement {
                swift_type: "RustString".into(),
                optional: false
            })
        );
        assert_eq!(
            owned("Option<Vec<Choice>>"),
            Some(OwnedVecElement {
                swift_type: "Choice".into(),
                optional: true
            })
        );
        for ty in [
            "Vec<u8>",
            "Vec<f32>",
            "Vec<bool>",
            "String",
            "Choice",
            "Vec<Vec<String>>",
            "Option<String>",
        ] {
            assert_eq!(owned(ty), None, "{ty} must keep its bridge shape");
        }
    }

    #[test]
    fn raw_getter_name_is_camel_case_with_suffix() {
        assert_eq!(bridge_getter_swift_name("model_prefixes"), "modelPrefixesRustVec");
    }
}
