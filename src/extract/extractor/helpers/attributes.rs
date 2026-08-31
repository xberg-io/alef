use super::fields::has_dyn_trait_object;
use crate::core::ir::SerdeContainerConversion;

/// Check if a visibility is bare `pub` (not `pub(crate)` or other restricted variants).
pub(crate) fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
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
                    if path.is_ident(derive_name) || path.segments.last().is_some_and(|seg| seg.ident == derive_name) {
                        return true;
                    }
                }
            }
        } else if attr.path().is_ident("cfg_attr") {
            // #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
            // #[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]
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
    let mut found = false;
    let mut visit = |meta: &syn::Meta| {
        if let syn::Meta::List(list) = meta
            && list.path.is_ident("derive")
            && let Ok(inner_paths) =
                list.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
        {
            for path in &inner_paths {
                if predicate(path) {
                    found = true;
                }
            }
        }
    };
    cfg_attr_walk_inner_metas(attr, &mut visit);
    found
}

/// Structurally walk a `#[cfg_attr(condition, inner1, inner2, ...)]` attribute, skip the
/// condition, and invoke `visit` with each inner attribute meta found after it.
///
/// The condition is skipped by parsing it as a `syn::Meta` (correctly handling bare idents,
/// `feature = "x"`, and nested `any(...)`/`all(...)`/`not(...)` combinations) rather than by
/// string-matching — Alef never evaluates the predicate itself, since it cannot know which
/// features a downstream build enables; every inner attribute is treated as if it applied
/// unconditionally. `cfg_attr` may nest (`cfg_attr(a, cfg_attr(b, serde(...)))`); nested
/// `cfg_attr` lists are unwrapped recursively rather than surfaced to `visit`, so callers
/// always see the "real" inner attributes regardless of nesting depth. ~keep
fn cfg_attr_walk_inner_metas(attr: &syn::Attribute, visit: &mut impl FnMut(&syn::Meta)) {
    let Ok(meta_list) = attr.meta.require_list() else {
        return;
    };
    cfg_attr_meta_list_walk_inner_metas(meta_list, visit);
}

fn cfg_attr_meta_list_walk_inner_metas(meta_list: &syn::MetaList, visit: &mut impl FnMut(&syn::Meta)) {
    use syn::Token;
    use syn::parse::ParseStream;

    let parse_fn = |input: ParseStream<'_>| -> syn::Result<()> {
        let _condition: syn::Meta = input.parse()?;
        let _: Token![,] = input.parse()?;

        while !input.is_empty() {
            let inner_meta: syn::Meta = input.parse()?;
            match &inner_meta {
                syn::Meta::List(inner_list) if inner_list.path.is_ident("cfg_attr") => {
                    cfg_attr_meta_list_walk_inner_metas(inner_list, visit);
                }
                _ => visit(&inner_meta),
            }
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }
        Ok(())
    };

    let _ = syn::parse::Parser::parse2(parse_fn, meta_list.tokens.clone());
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

/// Check whether any attribute gates an item behind the `test` cfg.
///
/// `#[cfg(test)]` items (and the `all(test, …)`/`any(test, …)`/nested forms
/// where `test` is a positive gate) do not exist in normal, non-test builds.
/// Extracting them produces bindings that call functions/types absent from the
/// release surface, which fail to compile (E0599 / E0433). They must never enter
/// the IR.
///
/// A predicate is treated as test-gated when `test` appears as a positive
/// condition anywhere outside a `not(...)`. Real feature gates
/// (`feature = "…"`, `target_os = "…"`, etc.) are never matched.
pub(crate) fn is_test_gated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Meta>()
                .map(|meta| cfg_meta_gates_on_test(&meta))
                .unwrap_or(false)
    })
}

/// Recursively determine whether a `cfg(...)` predicate gates positively on `test`.
///
/// - `test` (a bare path) → `true`
/// - `all(...)` / `any(...)` → `true` if any nested predicate gates on `test`
/// - `not(...)` → `false` (a negated `test` means "not under test", not test-only)
/// - `key = "value"` (feature/target/etc.) → `false`
fn cfg_meta_gates_on_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) => {
            if list.path.is_ident("not") {
                return false;
            }
            if list.path.is_ident("all") || list.path.is_ident("any") {
                let nested =
                    list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::token::Comma>::parse_terminated);
                if let Ok(nested) = nested {
                    return nested.iter().any(cfg_meta_gates_on_test);
                }
            }
            false
        }
        syn::Meta::NameValue(_) => false,
    }
}

/// Extract `rename_all` value from `#[serde(rename_all = "...")]` or
/// `#[cfg_attr(..., serde(rename_all = "..."))]` attributes (`cfg_attr` condition can be
/// arbitrarily complex, e.g. `any(feature = "a", feature = "b")`, and `cfg_attr` may nest).
pub(crate) fn extract_serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    let mut found: Option<String> = None;
    for_each_serde_meta_list(attrs, |list| {
        if found.is_none() {
            found = serde_meta_list_lit_str(list, "rename_all");
        }
    });
    found
}

/// Extract `rename_all_fields` value from `#[serde(rename_all_fields = "...")]` or
/// `#[cfg_attr(..., serde(rename_all_fields = "..."))]` attributes on an enum.
///
/// This is a distinct serde container attribute from `rename_all`: `rename_all` cases enum
/// VARIANT names, while `rename_all_fields` cases the FIELD names of every struct-shaped
/// variant's payload. The two are independent -- an enum may set either, both, or neither --
/// so this must never fall back to (or be confused with) [`extract_serde_rename_all`]. ~keep
pub(crate) fn extract_serde_rename_all_fields(attrs: &[syn::Attribute]) -> Option<String> {
    let mut found: Option<String> = None;
    for_each_serde_meta_list(attrs, |list| {
        if found.is_none() {
            found = serde_meta_list_lit_str(list, "rename_all_fields");
        }
    });
    found
}

/// Invoke `visit` with the `syn::MetaList` of every `#[serde(...)]` attribute in `attrs`,
/// including ones nested inside `#[cfg_attr(...)]` (recursively, and regardless of how
/// complex the gating condition is — Alef never evaluates `cfg`/`cfg_attr` predicates, so a
/// gated `serde(...)` attribute is treated the same as a bare one). ~keep
fn for_each_serde_meta_list(attrs: &[syn::Attribute], mut visit: impl FnMut(&syn::MetaList)) {
    for attr in attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.meta.require_list() {
                visit(list);
            }
        } else if attr.path().is_ident("cfg_attr") {
            cfg_attr_walk_inner_metas(attr, &mut |meta| {
                if let syn::Meta::List(list) = meta
                    && list.path.is_ident("serde")
                {
                    visit(list);
                }
            });
        }
    }
}

/// Extract a `key = "value"` string literal from a `serde(...)` meta list (e.g. `rename_all`).
///
/// Uses `MetaList::parse_nested_meta` to walk the attribute tree without stringifying the
/// token stream — this only allocates the matched literal value (if any), not the full
/// attribute representation.
fn serde_meta_list_lit_str(list: &syn::MetaList, key: &str) -> Option<String> {
    let mut found: Option<String> = None;
    let _ = list.parse_nested_meta(|meta| {
        if meta.path.is_ident(key) {
            if let Ok(value) = meta.value()
                && let Ok(s) = value.parse::<syn::LitStr>()
            {
                found = Some(s.value());
            }
        } else if let Ok(value) = meta.value() {
            let _: syn::Expr = value.parse()?;
        } else {
            let _ = meta.parse_nested_meta(|_| Ok(()));
        }
        Ok(())
    });
    found
}

/// Extract the source annotation that excludes a top-level item from generated binding APIs.
///
/// Use [`extract_field_binding_exclusion_reason`] for struct fields — it additionally
/// detects trait-object types which cannot be marshaled through serde.
pub(crate) fn extract_binding_exclusion_reason(attrs: &[syn::Attribute]) -> Option<String> {
    if has_doc_hidden(attrs) {
        return Some("doc(hidden)".to_string());
    }
    if has_alef_skip(attrs) {
        return Some("alef(skip)".to_string());
    }
    None
}

/// Extract the binding exclusion reason for a struct field.
///
/// Checks attribute-level exclusion (same as [`extract_binding_exclusion_reason`]) and
/// additionally auto-excludes fields whose type contains a trait object (`dyn Trait`).
/// Trait objects cannot be marshaled through serde or constructed from non-Rust binding
/// code, so emitting them in a binding mirror causes compile failures in downstream
/// backends (swift, dart, etc.).
pub(crate) fn extract_field_binding_exclusion_reason(attrs: &[syn::Attribute], ty: &syn::Type) -> Option<String> {
    if let Some(reason) = extract_binding_exclusion_reason(attrs) {
        return Some(reason);
    }
    if has_dyn_trait_object(ty) {
        return Some("dyn-trait-object".to_string());
    }
    None
}

fn has_doc_hidden(attrs: &[syn::Attribute]) -> bool {
    // Match `#[doc(hidden)]` specifically — a list-form `doc` attribute whose only
    // argument is the bare ident `hidden`. Doc-comment attributes (`#[doc = "..."]`)
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("doc") {
            return false;
        }
        let Ok(list) = attr.meta.require_list() else {
            return false;
        };
        list.parse_args::<syn::Ident>()
            .map(|ident| ident == "hidden")
            .unwrap_or(false)
    })
}

fn has_alef_skip(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if meta_is_alef_skip(&attr.meta) {
            return true;
        }
        if !attr.path().is_ident("cfg_attr") {
            return false;
        }
        let mut found = false;
        cfg_attr_walk_inner_metas(attr, &mut |meta| found |= meta_is_alef_skip(meta));
        found
    })
}

fn meta_is_alef_skip(meta: &syn::Meta) -> bool {
    let path = meta.path();
    if path.segments.len() == 2
        && path.segments.first().is_some_and(|segment| segment.ident == "alef")
        && path.segments.last().is_some_and(|segment| segment.ident == "skip")
    {
        return true;
    }
    if !path.is_ident("alef") {
        return false;
    }
    let syn::Meta::List(list) = meta else {
        return false;
    };
    list.parse_args::<syn::Ident>().is_ok_and(|ident| ident == "skip")
}

/// True when any of the given attributes is `#[serde(flatten)]` (also matching
/// `#[cfg_attr(..., serde(flatten))]`). Used by Java/C# backends to emit
/// `@JsonAnyGetter`/`@JsonAnySetter` and `[JsonExtensionData]` respectively
/// for fields that carry sibling-fields-as-map semantics.
pub(crate) fn extract_serde_flatten(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return false;
        }
        attr_str.contains("flatten ,")
            || attr_str.contains("flatten,")
            || attr_str.contains("flatten )")
            || attr_str.contains("flatten)")
            || attr_str.ends_with("flatten")
    })
}

/// Extract the codec path from `#[serde(with = "...")]` or `#[serde(serialize_with = "...")]`
/// (also matching the `#[cfg_attr(..., serde(...))]` forms).
///
/// A field carrying either attribute is serialized by hand-written code, so serde's *derived*
/// wire shape for its type no longer describes the bytes. Backends must consult this before
/// imposing a derive-shape wrapper — see `FieldDef::serde_with`. `deserialize_with` alone is
/// deliberately ignored: it changes only the read side, so the serialized shape is still the
/// derived one. ~keep
pub(crate) fn extract_serde_with(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return None;
        }
        // Every probe rejects a match preceded by an identifier character, so the substring
        // `deserialize_with = "..."` matches neither the `with` needles nor the
        // `serialize_with` ones (both occur inside it). All occurrences are scanned, not just
        // the first: `#[serde(deserialize_with = "b", serialize_with = "a")]` puts a rejected
        // match ahead of the real one. ~keep
        for needle in ["serialize_with =", "serialize_with=", "with =", "with="] {
            for (pos, _) in attr_str.match_indices(needle) {
                let before = attr_str[..pos].trim_end();
                if before.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let after = attr_str[pos + needle.len()..].trim_start();
                let Some(start) = after.find('"') else { continue };
                let value = &after[start + 1..];
                let Some(end) = value.find('"') else { continue };
                return Some(value[..end].to_string());
            }
        }
        None
    })
}

/// Extract a `#[serde(rename = "...")]` value from a list of attributes (also
/// matching `#[cfg_attr(..., serde(rename = "..."))]`).
pub(crate) fn extract_serde_rename(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") || !attr_str.contains("rename") {
            return None;
        }
        let needles = ["rename =", "rename="];
        for needle in &needles {
            if let Some(pos) = attr_str.find(needle) {
                let before = &attr_str[..pos];
                if before.ends_with("rename_all_") || before.ends_with("rename_all") {
                    continue;
                }
                let rest = &attr_str[pos + needle.len()..];
                let after = rest.trim_start();
                let start = after.find('"')?;
                let value_start = &after[start + 1..];
                let end = value_start.find('"')?;
                return Some(value_start[..end].to_string());
            }
        }
        None
    })
}

/// Extract the function path from `#[serde(default = "path::to::fn")]` (also
/// matching `#[cfg_attr(..., serde(default = "..."))]`). Returns `None` for a
/// bare `#[serde(default)]` with no explicit path. Bindings that mirror the
/// core's serde behavior need the path to emit an equivalent field-level
/// default (e.g. `SsrfPolicy::from_env`) instead of falling back to `Default`. ~keep
pub(crate) fn extract_serde_default_path(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return None;
        }
        let needles = ["default =", "default="];
        for needle in &needles {
            if let Some(pos) = attr_str.find(needle) {
                let before = &attr_str[..pos];
                if before.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let after = attr_str[pos + needle.len()..].trim_start();
                let start = after.find('"')?;
                let value_start = &after[start + 1..];
                let end = value_start.find('"')?;
                return Some(value_start[..end].to_string());
            }
        }
        None
    })
}

/// Check if a field has `#[serde(default)]` attribute (also matching
/// `#[cfg_attr(..., serde(default))]`). Fields with this attribute can
/// be omitted from JSON and use the type's Default implementation.
pub(crate) fn has_serde_default(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return false;
        }
        // Look for `default` keyword: both bare `#[serde(default)]` and
        // `#[serde(default = "...")]` variants. Match `default` as a boundary word,
        attr_str.contains("default =")
            || attr_str.contains("default ,")
            || attr_str.contains("default,")
            || attr_str.contains("default )")
            || attr_str.contains("default)")
            || attr_str.ends_with("default")
    })
}

/// Check if a field carries `#[serde(skip_serializing_if = "...")]` (also matching
/// `#[cfg_attr(..., serde(skip_serializing_if = "..."))]`).
///
/// The predicate path itself (`Option::is_none`, `Vec::is_empty`, ...) does not matter
/// here — presence alone means serde may omit the field's JSON key entirely for some
/// values, independent of whether the field's Rust type is `Option<T>`. See
/// `FieldDef::serde_skip_serializing_if` for why this must be tracked separately from
/// `optional`.
pub(crate) fn extract_serde_skip_serializing_if(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        attr_str.contains("serde") && attr_str.contains("skip_serializing_if")
    })
}

/// Check if a field carries a bare `#[serde(skip)]` (also matching
/// `#[cfg_attr(..., serde(skip))]`) — full exclusion from both `Serialize` and
/// `Deserialize`, distinct from `skip_serializing_if` (see
/// [`FieldDef::serde_skip`](crate::core::ir::FieldDef::serde_skip)).
///
/// Parses each `serde(...)` attribute's comma-separated argument list and matches only a
/// bare `skip` path item, so a sibling `skip_serializing_if = "..."` (a `NameValue` meta,
/// not a bare `Path`) never false-positives just because both attributes share the "skip"
/// substring.
pub(crate) fn extract_serde_skip(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if meta_has_serde_skip(&attr.meta) {
            return true;
        }
        if !attr.path().is_ident("cfg_attr") {
            return false;
        }
        let mut found = false;
        cfg_attr_walk_inner_metas(attr, &mut |meta| found |= meta_has_serde_skip(meta));
        found
    })
}

fn meta_has_serde_skip(meta: &syn::Meta) -> bool {
    if !meta.path().is_ident("serde") {
        return false;
    }
    let syn::Meta::List(list) = meta else {
        return false;
    };
    list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        .map(|metas| metas.iter().any(|m| matches!(m, syn::Meta::Path(p) if p.is_ident("skip"))))
        .unwrap_or(false)
}

/// Check if a *container* (struct/enum) carries `#[serde(default)]` or
/// `#[serde(default = "path")]`, including through `#[cfg_attr(..., serde(default))]`.
///
/// Reuses [`has_serde_default`]'s needle logic per attribute, but skips `doc` attributes:
/// a container doc comment is far more likely than a field one to quote `#[serde(default)]`
/// in prose, and prose is not an attribute.
///
/// The *meaning* differs from the field-level reader. A container default fills every missing
/// key from the container's `Default` (or the named function), making all fields
/// absent-tolerant on the wire; a field-level default fills that one key from the field
/// type's `Default`. The two disagree exactly where it matters: a container whose `Default`
/// sets `timeout: 30` yields `30` for a missing key, whereas the same field marked
/// `#[serde(default)]` would yield `u32::default()` — `0`. ~keep
pub(crate) fn has_container_serde_default(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("doc"))
        .any(|attr| has_serde_default(std::slice::from_ref(attr)))
}

/// Extract a `key = "value"` string literal from `#[serde(...)]` container attributes (also
/// matching `#[cfg_attr(..., serde(key = "..."))]`), for the container-level conversion keys
/// `from` / `into` / `try_from`.
///
/// The boundary check rejects a match immediately preceded by an identifier character, so a
/// lookup for `from` never matches inside `try_from = "..."` — the same technique
/// [`extract_serde_rename`] uses to keep `rename` out of `rename_all`. ~keep
fn extract_serde_container_conversion_key(attrs: &[syn::Attribute], key: &str) -> Option<String> {
    let with_space = format!("{key} =");
    let without_space = format!("{key}=");
    attrs.iter().find_map(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return None;
        }
        for needle in [with_space.as_str(), without_space.as_str()] {
            for (pos, _) in attr_str.match_indices(needle) {
                let before = attr_str[..pos].trim_end();
                if before.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let after = attr_str[pos + needle.len()..].trim_start();
                let Some(start) = after.find('"') else { continue };
                let value = &after[start + 1..];
                let Some(end) = value.find('"') else { continue };
                return Some(value[..end].to_string());
            }
        }
        None
    })
}

/// Extract the type path from `#[serde(from = "...")]` on a struct/enum container (also
/// matching `#[cfg_attr(..., serde(from = "..."))]`).
///
/// A container-level `from` replaces serde's derived, field-by-field wire shape with
/// whatever the named type serializes as (frequently a tuple/array for a value type), via a
/// hand-written `From<Named> for T`. Alef cannot see that impl's logic, so it cannot know the
/// resulting shape — this is recorded so validation can flag the type instead of the
/// generator silently emitting an object-shaped DTO that does not match the wire. ~keep
fn extract_serde_container_from(attrs: &[syn::Attribute]) -> Option<String> {
    extract_serde_container_conversion_key(attrs, "from")
}

/// Extract the type path from `#[serde(into = "...")]` on a struct/enum container (also
/// matching `#[cfg_attr(..., serde(into = "..."))]`). See [`extract_serde_container_from`] —
/// `into` is `from`'s serialize-side counterpart and is independent of it: a type may declare
/// one without the other.
fn extract_serde_container_into(attrs: &[syn::Attribute]) -> Option<String> {
    extract_serde_container_conversion_key(attrs, "into")
}

/// Extract the type path from `#[serde(try_from = "...")]` on a struct/enum container (also
/// matching `#[cfg_attr(..., serde(try_from = "..."))]`). See [`extract_serde_container_from`]
/// — `try_from` is the fallible counterpart of `from` and is mutually exclusive with it in
/// valid serde usage, but alef does not need to enforce that; it only records what is present.
fn extract_serde_container_try_from(attrs: &[syn::Attribute]) -> Option<String> {
    extract_serde_container_conversion_key(attrs, "try_from")
}

/// True when a container carries `#[serde(transparent)]` (also matching
/// `#[cfg_attr(..., serde(transparent))]`).
///
/// Unlike `from`/`into`/`try_from`, a transparent container needs no companion type: serde
/// requires exactly one non-skipped field, and the container's wire shape is exactly that
/// field's own serialized shape, with no wrapping object. Still tracked as a shape alef cannot
/// yet mirror, since the generated DTO still emits an object with that one field as a named
/// property rather than the bare unwrapped value.
fn has_serde_transparent(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return false;
        }
        attr_str.contains("transparent ,")
            || attr_str.contains("transparent,")
            || attr_str.contains("transparent )")
            || attr_str.contains("transparent)")
            || attr_str.ends_with("transparent")
    })
}

/// Extract a struct's container-level `#[serde(from/into/try_from/transparent)]` into one
/// [`SerdeContainerConversion`], the sole entry point extraction calls for this concept -- see
/// that struct's doc for why it replaced four separate `TypeDef` fields.
pub(crate) fn extract_serde_container_conversion(attrs: &[syn::Attribute]) -> SerdeContainerConversion {
    SerdeContainerConversion {
        from: extract_serde_container_from(attrs),
        into: extract_serde_container_into(attrs),
        try_from: extract_serde_container_try_from(attrs),
        transparent: has_serde_transparent(attrs),
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
            // #[cfg_attr(feature = "serde", derive(thiserror::Error))]
            // #[cfg_attr(any(feature = "x", test), derive(thiserror::Error))]
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

/// Extract `#[alef(error_code = N)]` or its `cfg_attr` form from an error variant. ~keep
pub(crate) fn extract_alef_error_code(attrs: &[syn::Attribute]) -> Option<u32> {
    let mut result = None;
    let mut visit = |meta: &syn::Meta| {
        let syn::Meta::List(list) = meta else {
            return;
        };
        if !list.path.is_ident("alef") {
            return;
        }
        let _ = list.parse_nested_meta(|nested| {
            if nested.path.is_ident("error_code") {
                result = Some(0);
                let value: syn::LitInt = nested.value()?.parse()?;
                if let Ok(code) = value.base10_parse() {
                    result = Some(code);
                }
            }
            Ok(())
        });
    };
    for attr in attrs {
        if attr.path().is_ident("cfg_attr") {
            cfg_attr_walk_inner_metas(attr, &mut visit);
        } else {
            visit(&attr.meta);
        }
    }
    result
}

/// Check if a field has a specific attribute (e.g. `#[source]`, `#[from]`).
pub(crate) fn has_field_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Extract `#[deprecated]` / `#[deprecated(since = "...", note = "...")]` from attrs.
pub(crate) fn extract_deprecation(attrs: &[syn::Attribute]) -> Option<crate::core::ir::DeprecationInfo> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("deprecated") {
            return None;
        }
        let mut info = crate::core::ir::DeprecationInfo::default();
        // `#[deprecated]` with no args is valid — treat as deprecated with no metadata.
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("since") {
                if let Ok(v) = meta.value()
                    && let Ok(s) = v.parse::<syn::LitStr>()
                {
                    let raw = s.value();
                    info.since = Some(raw.strip_prefix('v').map(str::to_owned).unwrap_or(raw));
                }
            } else if meta.path.is_ident("note") {
                if let Ok(v) = meta.value()
                    && let Ok(s) = v.parse::<syn::LitStr>()
                {
                    info.note = Some(s.value());
                }
            } else if let Ok(v) = meta.value() {
                let _: syn::Expr = v.parse()?;
            }
            Ok(())
        });
        Some(info)
    })
}

/// Extract `#[alef(since = "...")]` / `#[cfg_attr(..., alef(since = "..."))]` from attrs.
pub(crate) fn extract_alef_since(attrs: &[syn::Attribute]) -> Option<String> {
    let raw = attrs.iter().find_map(|attr| {
        if attr.path().is_ident("alef") {
            let mut found = None;
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("since") {
                    if let Ok(v) = meta.value()
                        && let Ok(s) = v.parse::<syn::LitStr>()
                    {
                        found = Some(s.value());
                    }
                } else if let Ok(v) = meta.value() {
                    let _: syn::Expr = v.parse()?;
                }
                Ok(())
            });
            return found;
        }
        if attr.path().is_ident("cfg_attr") {
            let mut found = None;
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("alef") {
                    let _ = meta.parse_nested_meta(|inner| {
                        if inner.path.is_ident("since") {
                            if let Ok(v) = inner.value()
                                && let Ok(s) = v.parse::<syn::LitStr>()
                            {
                                found = Some(s.value());
                            }
                        } else if let Ok(v) = inner.value() {
                            let _: syn::Expr = v.parse()?;
                        }
                        Ok(())
                    });
                } else if let Ok(v) = meta.value() {
                    let _: syn::Expr = v.parse()?;
                } else {
                    let _ = meta.parse_nested_meta(|_| Ok(()));
                }
                Ok(())
            });
            return found;
        }
        None
    })?;
    // without double-v when the author writes #[alef(since = "v1.2.0")].
    Some(raw.strip_prefix('v').map(str::to_owned).unwrap_or(raw))
}

/// Build a `VersionAnnotation` from the item's attributes.
pub(crate) fn extract_version_annotation(attrs: &[syn::Attribute]) -> crate::core::ir::VersionAnnotation {
    crate::core::ir::VersionAnnotation {
        since: extract_alef_since(attrs),
        deprecated: extract_deprecation(attrs),
    }
}
