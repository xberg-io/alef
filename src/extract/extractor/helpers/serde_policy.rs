use std::collections::BTreeSet;

use crate::core::ir::SerdeDeserializationPolicy;

use super::attributes::for_each_serde_meta_list;

fn serde_entries(attrs: &[syn::Attribute]) -> Vec<syn::Meta> {
    let mut entries = Vec::new();
    for_each_serde_meta_list(attrs, |list| {
        if let Ok(values) =
            list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        {
            entries.extend(values);
        }
    });
    entries
}

fn unsupported_attribute(meta: &syn::Meta) -> Option<&'static str> {
    for key in [
        "alias",
        "flatten",
        "skip",
        "skip_deserializing",
        "skip_serializing",
        "tag",
    ] {
        if meta.path().is_ident(key) {
            return Some(key);
        }
    }
    if matches!(meta, syn::Meta::List(_)) && (meta.path().is_ident("rename") || meta.path().is_ident("rename_all")) {
        return Some("directional rename");
    }
    None
}

pub(crate) fn extract_serde_deserialization_policy(item: &syn::ItemStruct) -> SerdeDeserializationPolicy {
    let container = serde_entries(&item.attrs);
    let deny_unknown_fields = container
        .iter()
        .any(|meta| matches!(meta, syn::Meta::Path(path) if path.is_ident("deny_unknown_fields")));
    let unsupported: BTreeSet<_> = container
        .iter()
        .filter_map(unsupported_attribute)
        .chain(item.fields.iter().flat_map(|field| {
            serde_entries(&field.attrs)
                .iter()
                .filter_map(unsupported_attribute)
                .collect::<Vec<_>>()
        }))
        .map(str::to_string)
        .collect();
    SerdeDeserializationPolicy {
        deny_unknown_fields,
        unsupported_field_attributes: unsupported.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::extract_serde_deserialization_policy;

    #[test]
    fn source_metadata_ignores_docs_and_unrelated_attribute_tokens() {
        let item = syn::parse_quote! {
            #[doc = "#[serde(deny_unknown_fields)]"]
            #[other(deny_unknown_fields)]
            pub struct Ordinary {
                #[doc = "serde(alias = old)"]
                pub value: String,
            }
        };
        assert!(extract_serde_deserialization_policy(&item).is_empty());
    }

    #[test]
    fn source_metadata_tracks_nested_cfg_and_unsupported_input_keys() {
        let item = syn::parse_quote! {
            #[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
            pub struct Strict {
                #[cfg_attr(feature = "serde", serde(alias = "old_value"))]
                pub value: String,
                #[serde(skip_deserializing)]
                pub cache: String,
            }
        };
        let policy = extract_serde_deserialization_policy(&item);
        assert!(policy.deny_unknown_fields);
        assert_eq!(policy.unsupported_field_attributes, ["alias", "skip_deserializing"]);
    }
}
