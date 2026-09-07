use crate::core::ir::EnumVariant;

use super::attributes::{
    extract_binding_exclusion_reason, extract_cfg_condition, extract_version_annotation, has_serde_untagged,
};
use super::fields::extract_field;
use super::rustdoc::extract_doc_comments;

/// Extract an enum variant with its fields.
pub(crate) fn extract_enum_variant(v: &syn::Variant) -> EnumVariant {
    let is_tuple = matches!(&v.fields, syn::Fields::Unnamed(_));
    // The serde-default half of `extract_field`'s return is discarded here: a variant's fields
    // never go through `#[derive(Default)]`/manual `impl Default` folding, so there is nothing
    // for `postprocess::warn_on_default_disagreement` to compare it against. ~keep
    let variant_fields = match &v.fields {
        syn::Fields::Named(named) => named.named.iter().map(|f| extract_field(f, None).0).collect(),
        syn::Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let (mut field, _) = extract_field(f, None);
                field.name = format!("_{i}");
                field
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
        let pos = attr_str.find("rename")?;
        let rest = &attr_str[pos..];
        let eq_pos = rest.find('=')?;
        let after_eq = rest[eq_pos + 1..].trim_start();
        let start = after_eq.find('"')?;
        let value_start = &after_eq[start + 1..];
        let end = value_start.find('"')?;
        Some(value_start[..end].to_string())
    });

    let binding_exclusion_reason = extract_binding_exclusion_reason(&v.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let cfg = extract_cfg_condition(&v.attrs);
    let serde_untagged = has_serde_untagged(&v.attrs);

    EnumVariant {
        name: v.ident.to_string(),
        fields: variant_fields,
        doc: extract_doc_comments(&v.attrs),
        is_default: v.attrs.iter().any(|a| a.path().is_ident("default")),
        serde_rename,
        is_tuple,
        binding_excluded,
        binding_exclusion_reason,
        originally_had_data_fields: false,
        cfg,
        serde_untagged,
        version: extract_version_annotation(&v.attrs),
    }
}
