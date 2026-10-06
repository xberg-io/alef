//! Deprecation and version annotation extraction, split out of attributes.rs.

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
