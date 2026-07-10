use crate::core::ir::{CoreWrapper, EnumVariant, FieldDef};

use crate::extract::type_resolver;

use super::attributes::{extract_binding_exclusion_reason, extract_cfg_condition, extract_version_annotation};
use super::field_types::{extract_field_type_rust_path, syn_type_is_boxed};
use super::fields::extract_field;
use super::rustdoc::extract_doc_comments;

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
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
        version: extract_version_annotation(&v.attrs),
    }
}
