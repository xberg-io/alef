use crate::core::ir::{
    ApiSurface, CoreWrapper, NewtypeConversion, NewtypeWrapperMetadata, ReceiverKind, TypeDef, TypeRef,
    UnsupportedPublicItem,
};
use ahash::{AHashMap, AHashSet};

use super::ResolvedNewtype;

pub(super) type NewtypeCandidates = AHashMap<String, Vec<(String, TypeRef, ResolvedNewtype)>>;
pub(super) type NewtypeMap = AHashMap<String, (TypeRef, ResolvedNewtype)>;

pub(super) fn extracted_named_paths(surface: &ApiSurface) -> AHashMap<String, Vec<String>> {
    let mut paths: AHashMap<String, AHashSet<String>> = AHashMap::new();
    for typ in &surface.types {
        paths.entry(typ.name.clone()).or_default().insert(typ.rust_path.clone());
    }
    for enum_def in &surface.enums {
        paths
            .entry(enum_def.name.clone())
            .or_default()
            .insert(enum_def.rust_path.clone());
    }
    for error in &surface.errors {
        paths
            .entry(error.name.clone())
            .or_default()
            .insert(error.rust_path.clone());
    }
    paths
        .into_iter()
        .map(|(name, paths)| {
            let mut paths: Vec<_> = paths.into_iter().collect();
            paths.sort_unstable();
            (name, paths)
        })
        .collect()
}

pub(super) fn select_unambiguous_candidates(
    candidates: NewtypeCandidates,
    extracted_names: &AHashMap<String, Vec<String>>,
    diagnostics: &mut Vec<UnsupportedPublicItem>,
) -> (NewtypeMap, AHashSet<String>) {
    let mut newtype_map = AHashMap::new();
    let mut resolved_paths = AHashSet::new();
    for (name, mut matching) in candidates {
        let extracted_paths = extracted_names.get(&name).map(Vec::as_slice).unwrap_or_default();
        if matching.len() == 1 && extracted_paths.len() == 1 {
            let (rust_path, inner, wrapper) = matching.pop().expect("one candidate");
            resolved_paths.insert(rust_path);
            newtype_map.insert(name, (inner, wrapper));
            continue;
        }
        let reason = format!(
            "ambiguous transparent newtype name `{name}` resolves to multiple Rust paths: {}",
            extracted_paths.join(", ")
        );
        for (rust_path, _, _) in matching {
            diagnostics.push(UnsupportedPublicItem {
                item_kind: "struct".to_string(),
                item_path: rust_path,
                reason: reason.clone(),
                suggested_fix: "rename one wrapper so every binding-visible newtype has a unique short name"
                    .to_string(),
            });
        }
    }
    (newtype_map, resolved_paths)
}

pub(super) fn validate_transparent_string_methods(
    typ: &TypeDef,
    metadata: &[NewtypeWrapperMetadata],
) -> Result<(), String> {
    let [metadata] = metadata else {
        return Err("transparent_string wrapper metadata must contain exactly one conversion path".to_string());
    };
    let NewtypeConversion::TransparentString { from, into } = &metadata.conversion else {
        return Err("transparent_string wrapper metadata has the wrong conversion kind".to_string());
    };
    validate_from_method(typ, from)?;
    validate_into_method(typ, into)
}

fn validate_from_method(typ: &TypeDef, from: &str) -> Result<(), String> {
    let methods: Vec<_> = typ.methods.iter().filter(|method| method.name == from).collect();
    if methods.len() != 1 {
        return Err(format!(
            "transparent_string requires exactly one public `{from}(String) -> Self` method"
        ));
    }
    let method = methods[0];
    let returns_self = matches!(&method.return_type, TypeRef::Named(name) if name == "Self" || name == &typ.name);
    if method.is_static
        && !method.is_async
        && method.error_type.is_none()
        && method.params.len() == 1
        && !method.params[0].optional
        && !method.params[0].is_ref
        && !method.params[0].is_mut
        && method.params[0].core_wrapper == CoreWrapper::None
        && method.params[0].ty == TypeRef::String
        && returns_self
        && !method.returns_ref
        && !method.returns_cow
        && method.trait_source.is_none()
        && !method.binding_excluded
    {
        return Ok(());
    }
    Err(format!(
        "transparent_string `{from}` must be a public synchronous `fn(String) -> Self`"
    ))
}

fn validate_into_method(typ: &TypeDef, into: &str) -> Result<(), String> {
    let methods: Vec<_> = typ.methods.iter().filter(|method| method.name == into).collect();
    if methods.len() != 1 {
        return Err(format!(
            "transparent_string requires exactly one public `{into}(self) -> String` method"
        ));
    }
    let method = methods[0];
    if !method.is_static
        && !method.is_async
        && method.error_type.is_none()
        && method.receiver == Some(ReceiverKind::Owned)
        && method.params.is_empty()
        && method.return_type == TypeRef::String
        && !method.returns_ref
        && !method.returns_cow
        && method.trait_source.is_none()
        && !method.binding_excluded
    {
        return Ok(());
    }
    Err(format!(
        "transparent_string `{into}` must be a public synchronous `fn(self) -> String`"
    ))
}
