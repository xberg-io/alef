use std::collections::HashSet;

use ahash::AHashMap;
use alef_core::ir::{CoreWrapper, EnumVariant, FieldDef, TypeRef};
use quote::ToTokens;
use syn;

use crate::type_resolver;

/// Check if a visibility is `pub`.
pub(crate) fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

/// Extract doc comments from attributes.
pub(crate) fn extract_doc_comments(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        let val = lit_str.value();
                        // Doc comments typically have a leading space
                        let trimmed = val.strip_prefix(' ').unwrap_or(&val);
                        lines.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    lines.join("\n")
}

/// Check if a `#[derive(...)]` attribute contains a specific derive.
/// Also checks `#[cfg_attr(..., derive(...))]` for conditional derives.
pub(crate) fn has_derive(attrs: &[syn::Attribute], derive_name: &str) -> bool {
    for attr in attrs {
        if attr.path().is_ident("derive") {
            if let Ok(nested) =
                attr.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::token::Comma>::parse_terminated)
            {
                for path in &nested {
                    if path.is_ident(derive_name) {
                        return true;
                    }
                }
            }
        } else if attr.path().is_ident("cfg_attr") {
            // Check cfg_attr for conditional derives, e.g.:
            // #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
            let tokens = attr.meta.to_token_stream().to_string();
            if tokens.contains("derive") && tokens.contains(derive_name) {
                return true;
            }
        }
    }
    false
}

/// Extract the condition string from a `#[cfg(...)]` attribute, if present.
/// Check if any attribute is a `#[cfg(...)]` — indicates feature-gated code.
pub(crate) fn has_cfg_attribute(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg"))
}

pub(crate) fn extract_cfg_condition(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("cfg") {
            // Get the token stream inside cfg(...)
            if let Ok(tokens) = attr.meta.require_list() {
                return Some(tokens.tokens.to_string());
            }
        }
    }
    None
}

/// Extract `rename_all` value from `#[serde(rename_all = "...")]` or
/// `#[cfg_attr(..., serde(rename_all = "..."))]` attributes.
pub(crate) fn extract_serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        let tokens = if let Ok(list) = attr.meta.require_list() {
            format!("{}", list.tokens)
        } else {
            continue;
        };
        if let Some(pos) = tokens.find("rename_all") {
            let rest = &tokens[pos..];
            if let Some(eq_pos) = rest.find('=') {
                let after_eq = rest[eq_pos + 1..].trim();
                if let Some(start) = after_eq.find('"') {
                    let after_start = &after_eq[start + 1..];
                    if let Some(end) = after_start.find('"') {
                        return Some(after_start[..end].to_string());
                    }
                }
            }
        }
    }
    None
}

/// Build the fully qualified rust_path for an item, taking into account
/// the accumulated module path.
pub(crate) fn build_rust_path(crate_name: &str, module_path: &str, name: &str) -> String {
    if module_path.is_empty() {
        format!("{crate_name}::{name}")
    } else {
        format!("{crate_name}::{module_path}::{name}")
    }
}

/// Check if a syn::Type is `Box<T>` or `Option<Box<T>>`.
pub(crate) fn syn_type_is_boxed(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident = segment.ident.to_string();
            if ident == "Box" {
                // Direct Box<T> — but not Box<dyn Trait> (those are opaque)
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            // Box<dyn Trait> is not a "boxed field" in our sense
                            if matches!(inner, syn::Type::TraitObject(_)) {
                                return false;
                            }
                            return true;
                        }
                    }
                }
            } else if ident == "Option" {
                // Option<Box<T>>
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            return syn_type_is_boxed(inner);
                        }
                    }
                }
            }
        }
    }
    false
}

/// Extract the fully qualified Rust path for a field's type when it uses a multi-segment
/// path (e.g., `crate::types::OutputFormat` → `kreuzberg::types::OutputFormat`).
/// Returns `None` for simple single-segment types like `OutputFormat` or primitives.
///
/// When `crate_name` is provided, `crate::` prefixes are resolved to the crate name
/// (e.g., `crate::types::OutputFormat` → `kreuzberg::types::OutputFormat`).
/// `super::` paths are still skipped since they require full module context.
pub(crate) fn extract_field_type_rust_path(ty: &syn::Type, crate_name: Option<&str>) -> Option<String> {
    // Unwrap Option<T> to look at inner type
    let inner_ty = if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    args.args.iter().find_map(|arg| {
                        if let syn::GenericArgument::Type(inner) = arg {
                            Some(inner)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let check_ty = inner_ty.unwrap_or(ty);

    // Unwrap Box<T> to look at inner type
    let check_ty = if let syn::Type::Path(type_path) = check_ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Box" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    args.args
                        .iter()
                        .find_map(|arg| {
                            if let syn::GenericArgument::Type(inner) = arg {
                                Some(inner)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(check_ty)
                } else {
                    check_ty
                }
            } else {
                check_ty
            }
        } else {
            check_ty
        }
    } else {
        check_ty
    };

    // Now check if the type has a multi-segment path
    if let syn::Type::Path(type_path) = check_ty {
        if type_path.path.segments.len() >= 2 {
            let first_segment = type_path.path.segments[0].ident.to_string();
            // Skip `super::` paths — these require full module context and would produce
            // invalid paths like `kreuzberg::super::super::pdf::PdfConfig` in codegen.
            if first_segment == "super" {
                return None;
            }
            // Resolve `crate::` paths using the crate name when available.
            // This enables disambiguation of types with the same short name but different
            // module paths (e.g., `crate::types::OutputFormat` vs `crate::core::config::OutputFormat`).
            if first_segment == "crate" {
                if let Some(name) = crate_name {
                    let mut segments: Vec<String> =
                        type_path.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    segments[0] = name.replace('-', "_").to_string();
                    return Some(segments.join("::"));
                }
                return None;
            }
            let segments: Vec<String> = type_path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            return Some(segments.join("::"));
        }
    }
    None
}

/// Get the last segment ident of a type, unwrapping Option if present.
fn outermost_ident(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            let ident = seg.ident.to_string();
            if ident == "Option" {
                // Recurse into Option<T>
                if let Some(inner) = type_resolver::extract_single_generic_arg_syn(seg) {
                    return outermost_ident(&inner);
                }
            }
            return Some(ident);
        }
    }
    None
}

/// Detect if a syn::Type is wrapped in Cow, Arc, or Bytes (before resolution).
pub(crate) fn detect_core_wrapper(ty: &syn::Type) -> alef_core::ir::CoreWrapper {
    use alef_core::ir::CoreWrapper;
    match outermost_ident(ty).as_deref() {
        Some("Cow") => CoreWrapper::Cow,
        Some("Arc") => CoreWrapper::Arc,
        Some("Bytes") => CoreWrapper::Bytes,
        _ => CoreWrapper::None,
    }
}

/// Detect if a Vec's inner type is wrapped in Arc (e.g., `Vec<Arc<T>>`).
pub(crate) fn detect_vec_inner_core_wrapper(ty: &syn::Type) -> alef_core::ir::CoreWrapper {
    use alef_core::ir::CoreWrapper;
    // Unwrap Option<Vec<Arc<T>>> → check Vec inner
    let check_ty = if let syn::Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            if seg.ident == "Option" {
                type_resolver::extract_single_generic_arg_syn(seg)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let ty_ref = check_ty.as_deref().unwrap_or(ty);

    if let syn::Type::Path(p) = ty_ref {
        if let Some(seg) = p.path.segments.last() {
            if seg.ident == "Vec" {
                if let Some(vec_inner) = type_resolver::extract_single_generic_arg_syn(seg) {
                    if let Some(ident) = outermost_ident(&vec_inner) {
                        if ident == "Arc" {
                            return CoreWrapper::Arc;
                        }
                    }
                }
            }
        }
    }
    CoreWrapper::None
}

/// If the resolved type is `TypeRef::Optional(inner)`, unwrap it and mark as optional.
pub(crate) fn unwrap_optional(ty: TypeRef) -> (TypeRef, bool) {
    match ty {
        TypeRef::Optional(inner) => (*inner, true),
        other => (other, false),
    }
}

/// Extract a struct field into a `FieldDef`.
///
/// When `crate_name` is provided, `crate::` prefixes in field type paths are resolved
/// to the crate name, enabling disambiguation of types with the same short name.
pub(crate) fn extract_field(field: &syn::Field, crate_name: Option<&str>) -> FieldDef {
    let name = field.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
    let doc = extract_doc_comments(&field.attrs);
    let cfg = extract_cfg_condition(&field.attrs);

    let is_boxed = syn_type_is_boxed(&field.ty);
    let type_rust_path = extract_field_type_rust_path(&field.ty, crate_name);
    let core_wrapper = detect_core_wrapper(&field.ty);
    let vec_inner_core_wrapper = detect_vec_inner_core_wrapper(&field.ty);

    let resolved = type_resolver::resolve_type(&field.ty);
    let (ty, optional) = unwrap_optional(resolved);

    FieldDef {
        name,
        ty,
        optional,
        default: None,
        doc,
        sanitized: false,
        is_boxed,
        type_rust_path,
        cfg,
        typed_default: None,
        core_wrapper,
        vec_inner_core_wrapper,
        newtype_wrapper: None,
    }
}

/// Extract an enum variant with its fields.
pub(crate) fn extract_enum_variant(v: &syn::Variant) -> EnumVariant {
    let is_tuple = matches!(&v.fields, syn::Fields::Unnamed(_));
    let variant_fields = match &v.fields {
        syn::Fields::Named(named) => named.named.iter().map(|f| extract_field(f, None)).collect(),
        syn::Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let ty = type_resolver::resolve_type(&f.ty);
                let optional = type_resolver::is_option_type(&f.ty).is_some();
                FieldDef {
                    name: format!("_{i}"),
                    ty,
                    optional,
                    default: None,
                    doc: extract_doc_comments(&f.attrs),
                    sanitized: false,
                    is_boxed: syn_type_is_boxed(&f.ty),
                    type_rust_path: extract_field_type_rust_path(&f.ty, None),
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                }
            })
            .collect(),
        syn::Fields::Unit => vec![],
    };
    // Extract #[serde(rename = "...")] or #[cfg_attr(..., serde(rename = "..."))]
    let serde_rename = v.attrs.iter().find_map(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("rename") {
            return None;
        }
        // Find rename = "value" pattern in the attribute string
        let pos = attr_str.find("rename")?;
        let rest = &attr_str[pos..];
        let eq_pos = rest.find('=')?;
        let after_eq = rest[eq_pos + 1..].trim_start();
        let start = after_eq.find('"')?;
        let value_start = &after_eq[start + 1..];
        let end = value_start.find('"')?;
        Some(value_start[..end].to_string())
    });

    EnumVariant {
        name: v.ident.to_string(),
        fields: variant_fields,
        doc: extract_doc_comments(&v.attrs),
        is_default: v.attrs.iter().any(|a| a.path().is_ident("default")),
        serde_rename,
        is_tuple,
    }
}

/// Check if a `#[derive(...)]` attribute contains a specific multi-segment derive path.
/// e.g. `has_derive_path(attrs, &["thiserror", "Error"])` matches `#[derive(thiserror::Error)]`.
/// Also checks `#[cfg_attr(..., derive(...))]` for conditional derives.
pub(crate) fn has_derive_path(attrs: &[syn::Attribute], segments: &[&str]) -> bool {
    for attr in attrs {
        if attr.path().is_ident("derive") {
            if let Ok(nested) =
                attr.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::token::Comma>::parse_terminated)
            {
                for path in &nested {
                    if path.segments.len() == segments.len()
                        && path
                            .segments
                            .iter()
                            .zip(segments.iter())
                            .all(|(seg, expected)| seg.ident == expected)
                    {
                        return true;
                    }
                }
            }
        } else if attr.path().is_ident("cfg_attr") {
            // Check cfg_attr for conditional derives, e.g.:
            // #[cfg_attr(feature = "serde", derive(thiserror::Error))]
            let tokens = attr.meta.to_token_stream().to_string();
            if tokens.contains("derive") && segments.iter().all(|seg| tokens.contains(seg)) {
                return true;
            }
        }
    }
    false
}

/// Check if an enum derives `thiserror::Error` (or just `Error` from a `use thiserror::Error`).
pub(crate) fn is_thiserror_enum(attrs: &[syn::Attribute]) -> bool {
    has_derive(attrs, "Error") || has_derive_path(attrs, &["thiserror", "Error"])
}

/// Extract the `#[error("...")]` message template from a variant's attributes.
pub(crate) fn extract_error_message_template(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("error") {
            // Parse as #[error("template string")]
            if let Ok(lit) = attr.parse_args::<syn::LitStr>() {
                return Some(lit.value());
            }
        }
    }
    None
}

/// Check if a field has a specific attribute (e.g. `#[source]`, `#[from]`).
pub(crate) fn has_field_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Represents what a `pub use` re-exports from a specific module.
#[derive(Debug)]
pub(crate) enum ReexportKind {
    /// `pub use module::*` — re-export everything
    Glob,
    /// `pub use module::{A, B}` — re-export specific names
    Names(HashSet<String>),
}

/// Collect pub use re-exports at the current module level, grouped by source module.
///
/// Returns a map from module name to the kind of re-export (glob or named).
/// Only tracks `pub use <ident>::...` where `<ident>` is not `self`/`super`/`crate`
/// (those are internal references handled elsewhere).
pub(crate) fn collect_reexport_map(items: &[syn::Item]) -> AHashMap<String, ReexportKind> {
    let mut map: AHashMap<String, ReexportKind> = AHashMap::new();
    for item in items {
        if let syn::Item::Use(item_use) = item {
            if is_pub(&item_use.vis) {
                collect_reexport_from_tree(&item_use.tree, &mut map);
            }
        }
    }
    map
}

/// Walk a use tree and populate the reexport map.
fn collect_reexport_from_tree(tree: &syn::UseTree, map: &mut AHashMap<String, ReexportKind>) {
    if let syn::UseTree::Path(use_path) = tree {
        let root_ident = use_path.ident.to_string();
        // For `self::submod::...`, skip `self` and recurse into the subtree
        // to find the actual module name. This handles `pub use self::core::{A, B};`
        // as a re-export from module `core`.
        if root_ident == "self" {
            collect_reexport_from_tree(&use_path.tree, map);
            return;
        }
        // Skip super/crate — those reference parent/root modules, not local submodules
        if root_ident == "super" || root_ident == "crate" {
            return;
        }
        collect_reexport_leaves(&root_ident, &use_path.tree, map);
    } else if let syn::UseTree::Group(group) = tree {
        for item in &group.items {
            collect_reexport_from_tree(item, map);
        }
    }
}

/// Collect leaves from a use subtree rooted at a known module name.
fn collect_reexport_leaves(module: &str, tree: &syn::UseTree, map: &mut AHashMap<String, ReexportKind>) {
    match tree {
        syn::UseTree::Glob(_) => {
            map.insert(module.to_string(), ReexportKind::Glob);
        }
        syn::UseTree::Name(use_name) => {
            let name = use_name.ident.to_string();
            match map.get_mut(module) {
                Some(ReexportKind::Glob) => {} // glob already covers everything
                Some(ReexportKind::Names(names)) => {
                    names.insert(name);
                }
                None => {
                    let mut names = HashSet::new();
                    names.insert(name);
                    map.insert(module.to_string(), ReexportKind::Names(names));
                }
            }
        }
        syn::UseTree::Rename(use_rename) => {
            let name = use_rename.rename.to_string();
            match map.get_mut(module) {
                Some(ReexportKind::Glob) => {}
                Some(ReexportKind::Names(names)) => {
                    names.insert(name);
                }
                None => {
                    let mut names = HashSet::new();
                    names.insert(name);
                    map.insert(module.to_string(), ReexportKind::Names(names));
                }
            }
        }
        syn::UseTree::Path(use_path) => {
            // Deeper path like `pub use module::submod::Thing` — treat as coming from `module`
            collect_reexport_leaves(module, &use_path.tree, map);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_reexport_leaves(module, item, map);
            }
        }
    }
}
