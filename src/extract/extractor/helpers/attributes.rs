use super::fields::has_dyn_trait_object;

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
            } else if let Ok(value) = meta.value() {
                // Consume the value so parse_nested_meta can advance to the next key.
                // Without this, sibling keys (e.g. `tag = "..."` before `rename_all`) leave
                // the cursor mid-value and the outer parse aborts before reaching `rename_all`.
                let _: syn::Expr = value.parse()?;
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
                        } else if let Ok(value) = inner_meta.value() {
                            let _: syn::Expr = value.parse()?;
                        }
                        Ok(())
                    });
                } else if let Ok(value) = meta.value() {
                    let _: syn::Expr = value.parse()?;
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
    // must NOT trigger this, even if the comment text contains the word "hidden".
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
        let attr_str = quote::quote!(#attr).to_string();
        let is_direct_alef = attr.path().is_ident("alef") && attr_str.contains("skip");
        let is_cfg_attr_alef =
            attr.path().is_ident("cfg_attr") && attr_str.contains("alef") && attr_str.contains("skip");
        is_direct_alef || is_cfg_attr_alef
    })
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
        // The `flatten` token must appear as a standalone serde directive, not as
        // part of another identifier. Look for the boundary patterns serde emits.
        attr_str.contains("flatten ,")
            || attr_str.contains("flatten,")
            || attr_str.contains("flatten )")
            || attr_str.contains("flatten)")
            || attr_str.ends_with("flatten")
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
        // `rename_all` also contains `rename`; ensure we anchor on `rename =` (or `rename=`)
        // and not on `rename_all`.
        let needles = ["rename =", "rename="];
        for needle in &needles {
            if let Some(pos) = attr_str.find(needle) {
                // Reject `rename_all`: the pos check fails when preceded by `_all`.
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
/// default (e.g. `SsrfPolicy::from_env`) instead of falling back to `Default`.
pub(crate) fn extract_serde_default_path(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let attr_str = quote::quote!(#attr).to_string();
        if !attr_str.contains("serde") {
            return None;
        }
        let needles = ["default =", "default="];
        for needle in &needles {
            if let Some(pos) = attr_str.find(needle) {
                // Anchor on the serde `default` key, not a longer identifier
                // such as `some_default = ...`.
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
        // not part of `default_` or `use_default`.
        attr_str.contains("default =")
            || attr_str.contains("default ,")
            || attr_str.contains("default,")
            || attr_str.contains("default )")
            || attr_str.contains("default)")
            || attr_str.ends_with("default")
    })
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
                if let Ok(v) = meta.value() {
                    if let Ok(s) = v.parse::<syn::LitStr>() {
                        let raw = s.value();
                        info.since = Some(raw.strip_prefix('v').map(str::to_owned).unwrap_or(raw));
                    }
                }
            } else if meta.path.is_ident("note") {
                if let Ok(v) = meta.value() {
                    if let Ok(s) = v.parse::<syn::LitStr>() {
                        info.note = Some(s.value());
                    }
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
                    if let Ok(v) = meta.value() {
                        if let Ok(s) = v.parse::<syn::LitStr>() {
                            found = Some(s.value());
                        }
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
                            if let Ok(v) = inner.value() {
                                if let Ok(s) = v.parse::<syn::LitStr>() {
                                    found = Some(s.value());
                                }
                            }
                        } else if let Ok(v) = inner.value() {
                            let _: syn::Expr = v.parse()?;
                        }
                        Ok(())
                    });
                } else if let Ok(v) = meta.value() {
                    // Simple `key = value` condition (e.g., `feature = "x"`).
                    let _: syn::Expr = v.parse()?;
                } else {
                    // Compound cfg predicate (e.g., `all(...)`, `any(...)`, `not(...)`):
                    // consume the parenthesized inner tokens so parse_nested_meta can
                    // continue to the next comma-separated item.
                    let _ = meta.parse_nested_meta(|_| Ok(()));
                }
                Ok(())
            });
            return found;
        }
        None
    })?;
    // Normalize: strip a leading 'v' so the docs template always emits "v{semver}"
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
