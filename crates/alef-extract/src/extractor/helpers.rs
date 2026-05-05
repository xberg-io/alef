use std::collections::HashSet;

use ahash::AHashMap;
use alef_core::ir::{CoreWrapper, EnumVariant, FieldDef, TypeRef};
use syn;

use crate::type_resolver;

/// Check if a visibility is bare `pub` (not `pub(crate)` or other restricted variants).
pub(crate) fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

/// Extract doc comments from attributes.
///
/// Output is post-processed by [`normalize_rustdoc`] so binding emitters
/// never see rustdoc-hidden setup lines (`# tokio_test::block_on(async {`)
/// or unresolved intra-doc-link syntax (`[\`crate::Foo\`]`).
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
    let raw = lines.join("\n");
    normalize_rustdoc(&raw)
}

/// Pre-process raw rustdoc so binding emitters can treat it as plain prose.
///
/// 1. Inside ```rust / ```rust,no_run fences, drops lines starting with `# `
///    (rustdoc's "hidden" syntax used to inject test scaffolding such as
///    `# tokio_test::block_on(async {` or `# Ok::<(), Error>(())`).
/// 2. Converts intra-doc-link syntax `` [`crate::Foo`] `` and
///    `` [`super::Bar`] `` to plain `` `Foo` `` / `` `Bar` `` so unresolved
///    paths don't leak into JS / Java / dart output.
///
/// Any other content is preserved verbatim (existing per-host renderers
/// continue to translate `# Errors` / `# Returns` / etc).
pub fn normalize_rustdoc(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // Step 1: walk lines, track ```rust fence depth, drop `# ` lines inside.
    let mut filtered = String::with_capacity(raw.len());
    let mut in_rust_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            // Toggle fence state. We only treat fences with NO language tag
            // OR an explicit `rust` / `rust,...` tag as Rust-doc fences.
            if in_rust_fence {
                in_rust_fence = false;
            } else {
                let lang = rest.split(',').next().unwrap_or("").trim();
                if lang.is_empty() || lang.eq_ignore_ascii_case("rust") {
                    in_rust_fence = true;
                }
            }
            filtered.push_str(line);
            filtered.push('\n');
            continue;
        }
        if in_rust_fence {
            // Rustdoc-hidden lines start with `# ` (a hash followed by a space)
            // or are exactly `#` (rare). Drop them entirely.
            let after_hash = trimmed.strip_prefix('#');
            if let Some(suffix) = after_hash {
                if suffix.is_empty() || suffix.starts_with(' ') {
                    continue;
                }
            }
        }
        filtered.push_str(line);
        filtered.push('\n');
    }

    // Step 2: rewrite `[` `crate::Foo` `]` / `[` `super::Bar` `]` into
    // `` `Foo` `` / `` `Bar` ``. We only touch the `[\`...\`]` form so
    // legitimate Markdown links are preserved.
    let mut out = String::with_capacity(filtered.len());
    let chars: Vec<char> = filtered.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Detect `[` `\`` opening of an intra-doc link.
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '`' {
            // Find the matching `\`]`. The contents are a Rust path.
            let start = i + 2;
            let mut j = start;
            while j + 1 < chars.len() {
                if chars[j] == '`' && chars[j + 1] == ']' {
                    break;
                }
                j += 1;
            }
            if j + 1 < chars.len() && chars[j] == '`' && chars[j + 1] == ']' {
                let inner: String = chars[start..j].iter().collect();
                // Only rewrite paths that begin with `crate::` or `super::`
                // (or `self::`). Leave other intra-doc-link forms to the
                // existing per-host renderer.
                let stripped = inner
                    .strip_prefix("crate::")
                    .or_else(|| inner.strip_prefix("super::"))
                    .or_else(|| inner.strip_prefix("self::"));
                if let Some(rest) = stripped {
                    // Take the last path segment as the human-friendly name.
                    let last = rest.rsplit("::").next().unwrap_or(rest);
                    out.push('`');
                    out.push_str(last);
                    out.push('`');
                    i = j + 2;
                    // Skip optional trailing `(...)` link target.
                    if i < chars.len() && chars[i] == '(' {
                        let mut depth = 1;
                        i += 1;
                        while i < chars.len() && depth > 0 {
                            match chars[i] {
                                '(' => depth += 1,
                                ')' => depth -= 1,
                                _ => {}
                            }
                            i += 1;
                        }
                    }
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    // Drop the trailing newline introduced by the line-walk step so the
    // output round-trips with raw input that has no trailing newline.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Check if a `#[derive(...)]` attribute contains a specific derive.
/// Also checks `#[cfg_attr(..., derive(...))]` for conditional derives.
///
/// Matches both the bare-ident form `#[derive(Serialize)]` and the
/// namespaced form `#[derive(serde::Serialize)]` — the latter is common
/// when serde isn't in `use` scope.
pub(crate) fn has_derive(attrs: &[syn::Attribute], derive_name: &str) -> bool {
    for attr in attrs {
        if attr.path().is_ident("derive") {
            if let Ok(nested) =
                attr.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::token::Comma>::parse_terminated)
            {
                for path in &nested {
                    // Accept both `Serialize` (single-segment) and
                    // `serde::Serialize` (two-segment). The cfg_attr branch
                    // below already does this — we mirror that here.
                    if path.is_ident(derive_name) || path.segments.last().is_some_and(|seg| seg.ident == derive_name) {
                        return true;
                    }
                }
            }
        } else if attr.path().is_ident("cfg_attr") {
            // Check cfg_attr for conditional derives, e.g.:
            // #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
            // #[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]
            //
            // Walk with parse_nested_meta: the first element is the condition (skipped),
            // subsequent elements are the attributes to apply. We look for `derive(...)` and
            // check each path inside it via path.is_ident(derive_name) (last segment).
            if cfg_attr_has_derive_name(attr, derive_name) {
                return true;
            }
        }
    }
    false
}

/// Walk a `cfg_attr(condition, derive(Foo, Bar))` attribute structurally and check whether
/// the inner derive list contains a path whose last segment matches `derive_name`.
///
/// Parses the raw token stream inside `cfg_attr(...)` via `syn::Meta` — the condition is
/// consumed as one `Meta` item (handles bare idents, `key = "val"`, and nested calls like
/// `any(...)`/`all(...)`), then the remaining items are inspected for `derive(...)`.
/// No `to_token_stream().to_string()` allocation.
fn cfg_attr_has_derive_name(attr: &syn::Attribute, derive_name: &str) -> bool {
    cfg_attr_walk_derives(attr, |path| {
        path.is_ident(derive_name) || path.segments.last().is_some_and(|seg| seg.ident == derive_name)
    })
}

/// Walk a `cfg_attr(condition, derive(Foo::Bar))` attribute structurally and check whether
/// the inner derive list contains a path whose segments exactly match `segments`.
///
/// Same parsing strategy as [`cfg_attr_has_derive_name`].
fn cfg_attr_has_derive_path(attr: &syn::Attribute, segments: &[&str]) -> bool {
    cfg_attr_walk_derives(attr, |path| {
        path.segments.len() == segments.len()
            && path
                .segments
                .iter()
                .zip(segments.iter())
                .all(|(seg, expected)| seg.ident == *expected)
    })
}

/// Core helper: parse a `cfg_attr(condition, ...)` token stream and call `predicate` on every
/// path inside any `derive(...)` list found after the condition.
///
/// The condition is skipped by parsing it as a `syn::Meta` (which correctly handles bare
/// idents, `feature = "x"`, `any(...)`, `all(...)`, `not(...)`, and combinations). A comma
/// is then consumed, and the remaining attribute metas are iterated.
fn cfg_attr_walk_derives(attr: &syn::Attribute, mut predicate: impl FnMut(&syn::Path) -> bool) -> bool {
    let meta_list = match attr.meta.require_list() {
        Ok(list) => list,
        Err(_) => return false,
    };

    use syn::Token;
    use syn::parse::ParseStream;

    let mut found = false;
    let parse_fn = |input: ParseStream<'_>| -> syn::Result<()> {
        // Skip the cfg condition — parse it as a Meta so nested parens (any/all/not) are consumed.
        let _condition: syn::Meta = input.parse()?;

        // Consume the comma separating condition from the attribute list.
        let _: Token![,] = input.parse()?;

        // Iterate the remaining attribute metas.
        while !input.is_empty() {
            let attr_meta: syn::Meta = input.parse()?;
            if let syn::Meta::List(list) = &attr_meta {
                if list.path.is_ident("derive") {
                    let inner_paths =
                        list.parse_args_with(syn::punctuated::Punctuated::<syn::Path, Token![,]>::parse_terminated)?;
                    for path in &inner_paths {
                        if predicate(path) {
                            found = true;
                        }
                    }
                }
            }
            // Consume trailing comma between multiple conditional attributes (rare but valid).
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }
        Ok(())
    };

    let _ = syn::parse::Parser::parse2(parse_fn, meta_list.tokens.clone());
    found
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
///
/// Uses `attr.parse_nested_meta` to walk the attribute tree without
/// stringifying the token stream — the previous implementation called
/// `format!("{}", list.tokens).to_string()` on every attribute, which
/// allocates the full attribute representation per type/enum and then does
/// O(n) string scanning. This implementation only allocates the matched
/// literal value (if any).
pub(crate) fn extract_serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    fn extract_from_serde(attr: &syn::Attribute) -> Option<String> {
        let mut found: Option<String> = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                if let Ok(value) = meta.value() {
                    if let Ok(s) = value.parse::<syn::LitStr>() {
                        found = Some(s.value());
                    }
                }
            } else {
                // Skip arbitrary nested values without erroring out the parse.
                let _ = meta.value();
            }
            Ok(())
        });
        found
    }

    for attr in attrs {
        if attr.path().is_ident("serde") {
            if let Some(v) = extract_from_serde(attr) {
                return Some(v);
            }
        } else if attr.path().is_ident("cfg_attr") {
            // `cfg_attr(feature = "X", serde(rename_all = "..."))` — the
            // serde inner attribute is the second argument. Walk and inspect.
            let mut inner: Option<String> = None;
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("serde") {
                    let _ = meta.parse_nested_meta(|inner_meta| {
                        if inner_meta.path.is_ident("rename_all") {
                            if let Ok(value) = inner_meta.value() {
                                if let Ok(s) = value.parse::<syn::LitStr>() {
                                    inner = Some(s.value());
                                }
                            }
                        } else {
                            let _ = inner_meta.value();
                        }
                        Ok(())
                    });
                } else {
                    let _ = meta.value();
                }
                Ok(())
            });
            if let Some(v) = inner {
                return Some(v);
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
            // #[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]
            //
            // Structured walk — no to_token_stream().to_string() allocation.
            if cfg_attr_has_derive_path(attr, segments) {
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

#[cfg(test)]
mod tests {
    use super::{has_derive, has_derive_path, normalize_rustdoc};

    // --- normalize_rustdoc ---

    #[test]
    fn test_normalize_rustdoc_strips_rustdoc_hidden_lines_inside_rust_fence() {
        let raw = "Convert document.\n\n```rust\n# tokio_test::block_on(async {\nuse foo::Bar;\nlet x = 1;\n# Ok::<(), Error>(())\n# });\n```\n";
        let normalized = normalize_rustdoc(raw);
        assert!(
            !normalized.contains("tokio_test"),
            "must drop tokio_test scaffolding: {normalized}"
        );
        assert!(
            !normalized.contains("# }"),
            "must drop closing brace scaffolding: {normalized}"
        );
        assert!(
            !normalized.contains("# Ok::"),
            "must drop trailing Ok scaffolding: {normalized}"
        );
        // Code body survives.
        assert!(normalized.contains("use foo::Bar;"));
        assert!(normalized.contains("let x = 1;"));
    }

    #[test]
    fn test_normalize_rustdoc_preserves_pound_outside_fence() {
        // `# Errors` is a section heading, not rustdoc-hidden — must survive.
        let raw = "Summary line.\n\n# Errors\n\nMay fail.";
        assert_eq!(normalize_rustdoc(raw), "Summary line.\n\n# Errors\n\nMay fail.");
    }

    #[test]
    fn test_normalize_rustdoc_preserves_pound_in_non_rust_fence() {
        // `# foo` inside a python fence is part of the snippet.
        let raw = "Example:\n\n```python\n# This is a python comment\nx = 1\n```";
        let normalized = normalize_rustdoc(raw);
        assert!(normalized.contains("# This is a python comment"));
    }

    #[test]
    fn test_normalize_rustdoc_rewrites_crate_link() {
        let raw = "See [`crate::ConversionOptions`] for details.";
        assert_eq!(normalize_rustdoc(raw), "See `ConversionOptions` for details.");
    }

    #[test]
    fn test_normalize_rustdoc_rewrites_super_link() {
        let raw = "Inherits from [`super::ExtractionConfig`] field.";
        assert_eq!(normalize_rustdoc(raw), "Inherits from `ExtractionConfig` field.");
    }

    #[test]
    fn test_normalize_rustdoc_rewrites_link_with_target() {
        let raw = "When set on [`ExtractionConfig`](super::ExtractionConfig), it works.";
        assert_eq!(
            normalize_rustdoc(raw),
            // The leading link target is `ExtractionConfig` (no crate::/super:: prefix on
            // the *backtick* portion), so this form is left for downstream renderers.
            "When set on [`ExtractionConfig`](super::ExtractionConfig), it works."
        );
    }

    #[test]
    fn test_normalize_rustdoc_self_link() {
        let raw = "See [`self::Foo`] for details.";
        assert_eq!(normalize_rustdoc(raw), "See `Foo` for details.");
    }

    #[test]
    fn test_normalize_rustdoc_empty() {
        assert_eq!(normalize_rustdoc(""), "");
    }

    #[test]
    fn test_normalize_rustdoc_no_changes_for_plain_prose() {
        let raw = "Plain documentation without fences or links.";
        assert_eq!(normalize_rustdoc(raw), raw);
    }

    #[test]
    fn test_normalize_rustdoc_handles_rust_no_run_fence() {
        let raw = "```rust,no_run\n# async fn example() {\nlet result = foo().await;\n# }\n```";
        let normalized = normalize_rustdoc(raw);
        assert!(
            !normalized.contains("# async fn"),
            "must drop async fn scaffolding: {normalized}"
        );
        assert!(normalized.contains("let result = foo().await;"));
    }

    fn parse_attrs(input: &str) -> Vec<syn::Attribute> {
        // Wrap the attribute in a dummy struct so syn can parse it.
        let item: syn::ItemStruct = syn::parse_str(&format!("{input} struct _Dummy;")).unwrap();
        item.attrs
    }

    // --- has_derive ---

    #[test]
    fn test_has_derive_bare_positive() {
        let attrs = parse_attrs("#[derive(Debug, Clone)]");
        assert!(has_derive(&attrs, "Debug"));
        assert!(has_derive(&attrs, "Clone"));
    }

    #[test]
    fn test_has_derive_bare_negative() {
        let attrs = parse_attrs("#[derive(Debug)]");
        assert!(!has_derive(&attrs, "Clone"));
    }

    #[test]
    fn test_has_derive_cfg_attr_simple() {
        // #[cfg_attr(feature = "x", derive(Foo))]
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", derive(Foo))]"#);
        assert!(has_derive(&attrs, "Foo"));
        assert!(!has_derive(&attrs, "Bar"));
    }

    #[test]
    fn test_has_derive_cfg_attr_multi_derive() {
        // multiple derives inside cfg_attr
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", derive(Foo, Bar, Baz))]"#);
        assert!(has_derive(&attrs, "Foo"));
        assert!(has_derive(&attrs, "Bar"));
        assert!(has_derive(&attrs, "Baz"));
        assert!(!has_derive(&attrs, "Qux"));
    }

    #[test]
    fn test_has_derive_cfg_attr_any_condition() {
        // #[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]
        let attrs = parse_attrs(r#"#[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]"#);
        // Last segment of thiserror::Error is "Error"
        assert!(has_derive(&attrs, "Error"));
        assert!(!has_derive(&attrs, "thiserror"));
    }

    #[test]
    fn test_has_derive_cfg_attr_qualified_path_last_segment() {
        // serde::Serialize — last segment is "Serialize"
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]"#);
        assert!(has_derive(&attrs, "Serialize"));
        assert!(has_derive(&attrs, "Deserialize"));
        assert!(!has_derive(&attrs, "serde"));
    }

    #[test]
    fn test_has_derive_cfg_attr_negative_no_derive() {
        // cfg_attr with a non-derive inner attribute
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", serde(rename_all = "camelCase"))]"#);
        assert!(!has_derive(&attrs, "Serialize"));
    }

    // --- has_derive_path ---

    #[test]
    fn test_has_derive_path_bare_single_segment() {
        let attrs = parse_attrs("#[derive(Debug)]");
        assert!(has_derive_path(&attrs, &["Debug"]));
        assert!(!has_derive_path(&attrs, &["Clone"]));
    }

    #[test]
    fn test_has_derive_path_bare_multi_segment() {
        let attrs = parse_attrs("#[derive(thiserror::Error)]");
        assert!(has_derive_path(&attrs, &["thiserror", "Error"]));
        assert!(!has_derive_path(&attrs, &["Error"]));
        assert!(!has_derive_path(&attrs, &["thiserror"]));
    }

    #[test]
    fn test_has_derive_path_cfg_attr_simple() {
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", derive(Foo))]"#);
        assert!(has_derive_path(&attrs, &["Foo"]));
        assert!(!has_derive_path(&attrs, &["Bar"]));
    }

    #[test]
    fn test_has_derive_path_cfg_attr_multi_segment() {
        // #[cfg_attr(feature = "x", derive(thiserror::Error))]
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", derive(thiserror::Error))]"#);
        assert!(has_derive_path(&attrs, &["thiserror", "Error"]));
        assert!(!has_derive_path(&attrs, &["Error"]));
    }

    #[test]
    fn test_has_derive_path_cfg_attr_any_condition() {
        let attrs = parse_attrs(r#"#[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]"#);
        assert!(has_derive_path(&attrs, &["thiserror", "Error"]));
        assert!(!has_derive_path(&attrs, &["thiserror"]));
        assert!(!has_derive_path(&attrs, &["Error"]));
    }

    #[test]
    fn test_has_derive_path_cfg_attr_negative() {
        let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", serde(rename_all = "camelCase"))]"#);
        assert!(!has_derive_path(&attrs, &["serde"]));
        assert!(!has_derive_path(&attrs, &["rename_all"]));
    }

    #[test]
    fn test_has_derive_path_empty_attrs() {
        let attrs: Vec<syn::Attribute> = vec![];
        assert!(!has_derive(&attrs, "Debug"));
        assert!(!has_derive_path(&attrs, &["Debug"]));
    }
}
