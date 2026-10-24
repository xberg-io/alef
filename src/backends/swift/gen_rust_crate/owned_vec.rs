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

use super::extern_block::getter_bridge_type;
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

#[derive(Serialize)]
struct TypeAccessors<'a> {
    name: &'a str,
    getters: &'a [OwnedVecGetter],
}

/// The Swift file with every owned-array accessor, or `None` when no getter needs one.
pub(crate) fn emit_swift_file(
    types: &[(String, Vec<OwnedVecGetter>)],
    rust_bridge_sources: &Path,
) -> Option<GeneratedFile> {
    let entries: Vec<TypeAccessors<'_>> = types
        .iter()
        .filter(|(_, getters)| !getters.is_empty())
        .map(|(name, getters)| TypeAccessors { name, getters })
        .collect();
    if entries.is_empty() {
        return None;
    }
    let content = crate::backends::swift::template_env::render(
        "swift_owned_vec_accessors.swift.jinja",
        crate::alef_context! {
            header => crate::core::hash::SELF_MARKING_HEADER_LINE,
            types => &entries,
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
        for ty in ["Vec<u8>", "Vec<f32>", "Vec<bool>", "String", "Choice", "Vec<Vec<String>>", "Option<String>"] {
            assert_eq!(owned(ty), None, "{ty} must keep its bridge shape");
        }
    }

    #[test]
    fn raw_getter_name_is_camel_case_with_suffix() {
        assert_eq!(bridge_getter_swift_name("model_prefixes"), "modelPrefixesRustVec");
    }
}
