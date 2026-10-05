use super::attributes::{
    extract_alef_error_code, extract_alef_since, extract_deprecation, extract_serde_rename_all,
    extract_serde_rename_all_fields, extract_serde_skip_serializing_if, extract_serde_with, has_alef_sensitive,
    has_container_serde_default, has_derive, has_derive_path,
};
use super::enum_variants::extract_enum_variant;
use super::fields::extract_field;
use super::normalize_rustdoc;

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
    assert!(normalized.contains("use foo::Bar;"));
    assert!(normalized.contains("let x = 1;"));
}

#[test]
fn test_normalize_rustdoc_preserves_pound_outside_fence() {
    let raw = "Summary line.\n\n# Errors\n\nMay fail.";
    assert_eq!(normalize_rustdoc(raw), "Summary line.\n\n# Errors\n\nMay fail.");
}

#[test]
fn test_normalize_rustdoc_preserves_pound_in_non_rust_fence() {
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
    let item: syn::ItemStruct = syn::parse_str(&format!("{input} struct _Dummy;")).unwrap();
    item.attrs
}

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
    assert!(has_derive(&attrs, "Error"));
    assert!(!has_derive(&attrs, "thiserror"));
}

#[test]
fn test_has_derive_cfg_attr_qualified_path_last_segment() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]"#);
    assert!(has_derive(&attrs, "Serialize"));
    assert!(has_derive(&attrs, "Deserialize"));
    assert!(!has_derive(&attrs, "serde"));
}

#[test]
fn test_has_derive_cfg_attr_negative_no_derive() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "x", serde(rename_all = "camelCase"))]"#);
    assert!(!has_derive(&attrs, "Serialize"));
}

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

// --- extract_serde_rename_all ---

#[test]
fn test_extract_serde_rename_all_bare_attribute() {
    let attrs = parse_attrs(r#"#[serde(rename_all = "snake_case")]"#);
    assert_eq!(extract_serde_rename_all(&attrs).as_deref(), Some("snake_case"));
}

#[test]
fn test_extract_serde_rename_all_cfg_attr_simple_condition() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]"#);
    assert_eq!(extract_serde_rename_all(&attrs).as_deref(), Some("snake_case"));
}

#[test]
fn test_extract_serde_rename_all_cfg_attr_any_condition() {
    // ~keep: the shape a downstream consumer hit in the wild — `rename_all` lives behind an
    // `any(...)` gate, which the naive `parse_nested_meta` walk used to fail to skip, silently
    // dropping the rename_all and emitting verbatim PascalCase wire names.
    let attrs =
        parse_attrs(r#"#[cfg_attr(any(feature = "serde", feature = "metadata"), serde(rename_all = "snake_case"))]"#);
    assert_eq!(extract_serde_rename_all(&attrs).as_deref(), Some("snake_case"));
}

#[test]
fn test_extract_serde_rename_all_nested_cfg_attr() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "a", cfg_attr(feature = "b", serde(rename_all = "camelCase")))]"#);
    assert_eq!(extract_serde_rename_all(&attrs).as_deref(), Some("camelCase"));
}

#[test]
fn test_extract_serde_rename_all_cfg_attr_multiple_inner_attrs() {
    let attrs = parse_attrs(
        r#"#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "kebab-case"))]"#,
    );
    assert_eq!(extract_serde_rename_all(&attrs).as_deref(), Some("kebab-case"));
}

#[test]
fn test_extract_serde_rename_all_absent_returns_none() {
    let attrs = parse_attrs(r#"#[derive(Debug, Clone)]"#);
    assert_eq!(extract_serde_rename_all(&attrs), None);
}

// --- extract_serde_rename_all_fields ---
//
// `rename_all_fields` is a distinct serde container attribute from `rename_all`: it cases the
// FIELD names of struct-shaped variants, while `rename_all` cases VARIANT names. The two are
// independent -- setting one must never report a value for the other. ~keep

#[test]
fn test_extract_serde_rename_all_fields_bare_attribute() {
    let attrs = parse_attrs(r#"#[serde(rename_all_fields = "camelCase")]"#);
    assert_eq!(extract_serde_rename_all_fields(&attrs).as_deref(), Some("camelCase"));
}

#[test]
fn test_extract_serde_rename_all_fields_cfg_attr_condition() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", serde(rename_all_fields = "snake_case"))]"#);
    assert_eq!(extract_serde_rename_all_fields(&attrs).as_deref(), Some("snake_case"));
}

#[test]
fn test_extract_serde_rename_all_fields_absent_returns_none() {
    let attrs = parse_attrs(r#"#[derive(Debug, Clone)]"#);
    assert_eq!(extract_serde_rename_all_fields(&attrs), None);
}

#[test]
fn test_extract_serde_rename_all_fields_present_alongside_rename_all_reads_independently() {
    let attrs = parse_attrs(r#"#[serde(rename_all = "SCREAMING_SNAKE_CASE", rename_all_fields = "camelCase")]"#);
    assert_eq!(
        extract_serde_rename_all(&attrs).as_deref(),
        Some("SCREAMING_SNAKE_CASE"),
        "rename_all must still resolve to the variant-name rule"
    );
    assert_eq!(
        extract_serde_rename_all_fields(&attrs).as_deref(),
        Some("camelCase"),
        "rename_all_fields must resolve to its own field-name rule, not rename_all's value"
    );
}

#[test]
fn test_extract_serde_rename_all_only_set_leaves_rename_all_fields_none() {
    let attrs = parse_attrs(r#"#[serde(rename_all = "snake_case")]"#);
    assert_eq!(extract_serde_rename_all(&attrs).as_deref(), Some("snake_case"));
    assert_eq!(
        extract_serde_rename_all_fields(&attrs),
        None,
        "setting rename_all alone must not leak into rename_all_fields"
    );
}

#[test]
fn test_extract_serde_rename_all_fields_only_set_leaves_rename_all_none() {
    let attrs = parse_attrs(r#"#[serde(rename_all_fields = "kebab-case")]"#);
    assert_eq!(extract_serde_rename_all_fields(&attrs).as_deref(), Some("kebab-case"));
    assert_eq!(
        extract_serde_rename_all(&attrs),
        None,
        "setting rename_all_fields alone must not leak into rename_all"
    );
}

use super::detect_core_wrapper;
use crate::core::ir::CoreWrapper;

fn parse_type(s: &str) -> syn::Type {
    syn::parse_str(s).unwrap()
}

#[test]
fn test_detect_core_wrapper_arc_mutex_returns_arc_mutex() {
    let ty = parse_type("Arc<Mutex<HashMap<String, String>>>");
    assert_eq!(detect_core_wrapper(&ty), CoreWrapper::ArcMutex);
}

#[test]
fn test_detect_core_wrapper_arc_rwlock_returns_arc_mutex() {
    let ty = parse_type("Arc<RwLock<Vec<u8>>>");
    assert_eq!(detect_core_wrapper(&ty), CoreWrapper::ArcMutex);
}

#[test]
fn test_detect_core_wrapper_option_arc_mutex_peeks_through_option() {
    let ty = parse_type("Option<Arc<Mutex<String>>>");
    assert_eq!(detect_core_wrapper(&ty), CoreWrapper::ArcMutex);
}

#[test]
fn test_detect_core_wrapper_plain_arc_returns_arc() {
    let ty = parse_type("Arc<String>");
    assert_eq!(detect_core_wrapper(&ty), CoreWrapper::Arc);
}

#[test]
fn test_detect_core_wrapper_arc_dyn_trait_stays_plain_arc() {
    let ty = parse_type("Arc<dyn MyTrait>");
    assert_eq!(detect_core_wrapper(&ty), CoreWrapper::Arc);
}

#[test]
fn test_detect_core_wrapper_tokio_sync_mutex_last_segment_match() {
    let ty = parse_type("Arc<tokio::sync::Mutex<u64>>");
    assert_eq!(detect_core_wrapper(&ty), CoreWrapper::ArcMutex);
}

#[test]
fn test_extract_deprecation_bare_deprecated_returns_empty_info() {
    let attrs = parse_attrs("#[deprecated]");
    let result = extract_deprecation(&attrs);
    assert!(result.is_some());
    let info = result.unwrap();
    assert!(info.since.is_none());
    assert!(info.note.is_none());
}

#[test]
fn test_extract_deprecation_with_since_returns_version() {
    let attrs = parse_attrs(r#"#[deprecated(since = "1.2.0")]"#);
    let info = extract_deprecation(&attrs).unwrap();
    assert_eq!(info.since.as_deref(), Some("1.2.0"));
    assert!(info.note.is_none());
}

#[test]
fn test_extract_deprecation_with_note_returns_message() {
    let attrs = parse_attrs(r#"#[deprecated(note = "use new_fn instead")]"#);
    let info = extract_deprecation(&attrs).unwrap();
    assert!(info.since.is_none());
    assert_eq!(info.note.as_deref(), Some("use new_fn instead"));
}

#[test]
fn test_extract_deprecation_with_both_fields() {
    let attrs = parse_attrs(r#"#[deprecated(since = "2.0.0", note = "use new_fn instead")]"#);
    let info = extract_deprecation(&attrs).unwrap();
    assert_eq!(info.since.as_deref(), Some("2.0.0"));
    assert_eq!(info.note.as_deref(), Some("use new_fn instead"));
}

#[test]
fn test_extract_deprecation_absent_returns_none() {
    let attrs = parse_attrs("#[derive(Clone)]");
    assert!(extract_deprecation(&attrs).is_none());
}

#[test]
fn test_extract_alef_since_bare_alef_since_returns_version() {
    let attrs = parse_attrs(r#"#[alef(since = "1.5.0")]"#);
    assert_eq!(extract_alef_since(&attrs).as_deref(), Some("1.5.0"));
}

#[test]
fn test_extract_alef_since_cfg_attr_alef_since_returns_version() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "alef-meta", alef(since = "0.9.0"))]"#);
    assert_eq!(extract_alef_since(&attrs).as_deref(), Some("0.9.0"));
}

#[test]
fn test_extract_alef_since_strips_leading_v_prefix() {
    let attrs = parse_attrs(r#"#[alef(since = "v1.2.0")]"#);
    assert_eq!(extract_alef_since(&attrs).as_deref(), Some("1.2.0"));
}

#[test]
fn test_extract_alef_error_code_from_direct_and_cfg_attr_forms() {
    let direct: Vec<syn::Attribute> = vec![syn::parse_quote!(#[alef(error_code = 101)])];
    let gated: Vec<syn::Attribute> = vec![syn::parse_quote!(#[cfg_attr(alef, alef(error_code = 202))])];
    let invalid: Vec<syn::Attribute> = vec![syn::parse_quote!(#[alef(error_code = -1)])];

    assert_eq!(extract_alef_error_code(&direct), Some(101));
    assert_eq!(extract_alef_error_code(&gated), Some(202));
    assert_eq!(extract_alef_error_code(&invalid), Some(0));
}

#[test]
fn test_extract_deprecation_since_strips_leading_v_prefix() {
    let attrs = parse_attrs(r#"#[deprecated(since = "v2.0.0", note = "use new_fn")]"#);
    let info = extract_deprecation(&attrs).unwrap();
    assert_eq!(info.since.as_deref(), Some("2.0.0"));
}

#[test]
fn test_extract_alef_since_absent_returns_none() {
    let attrs = parse_attrs("#[alef(skip)]");
    assert!(extract_alef_since(&attrs).is_none());
}

#[test]
fn alef_sensitive_accepts_direct_and_cfg_attr_forms() {
    let direct = parse_attrs("#[alef(sensitive)]");
    let qualified = parse_attrs("#[alef::sensitive]");
    let gated = parse_attrs("#[cfg_attr(alef, alef(sensitive))]");
    let nested = parse_attrs("#[cfg_attr(feature = \"bindings\", cfg_attr(alef, alef(sensitive))) ]");

    assert!(has_alef_sensitive(&direct));
    assert!(has_alef_sensitive(&qualified));
    assert!(has_alef_sensitive(&gated));
    assert!(has_alef_sensitive(&nested));
}

#[test]
fn alef_sensitive_ignores_lookalike_metadata() {
    let lookalike = parse_attrs("#[alef(sensitive_value = true)]");
    let name_value = parse_attrs("#[alef(sensitive = false)]");
    let list = parse_attrs("#[alef(sensitive(reason = \"secret\"))]");
    let qualified_name_value = parse_attrs("#[alef::sensitive = false]");
    let qualified_list = parse_attrs("#[alef::sensitive(reason = \"secret\")]");
    let nested_name_value = parse_attrs("#[cfg_attr(alef, alef(sensitive = false))]");
    let nested_list = parse_attrs("#[cfg_attr(alef, alef(sensitive(reason = \"secret\")))]");

    assert!(!has_alef_sensitive(&lookalike));
    assert!(!has_alef_sensitive(&name_value));
    assert!(!has_alef_sensitive(&list));
    assert!(!has_alef_sensitive(&qualified_name_value));
    assert!(!has_alef_sensitive(&qualified_list));
    assert!(!has_alef_sensitive(&nested_name_value));
    assert!(!has_alef_sensitive(&nested_list));
}

#[test]
fn sensitive_metadata_survives_field_and_variant_extraction() {
    let field: syn::Field = syn::parse_quote! {
        #[cfg_attr(alef, alef(sensitive))]
        pub token: String
    };
    let variant: syn::Variant = syn::parse_quote! {
        #[cfg_attr(alef, alef(sensitive))]
        Bearer { token: String }
    };

    assert!(extract_field(&field, None).0.sensitive);
    assert!(extract_enum_variant(&variant).sensitive);
    assert!(!extract_enum_variant(&variant).fields[0].sensitive);
}

// --- extract_serde_with ---

#[test]
fn serde_with_captures_the_codec_module_path() {
    let attrs = parse_attrs(r#"#[serde(with = "duration_ms")]"#);
    assert_eq!(extract_serde_with(&attrs).as_deref(), Some("duration_ms"));
}

#[test]
fn serde_with_captures_the_path_when_preceded_by_another_key() {
    let attrs = parse_attrs(r#"#[serde(default, with = "option_duration_ms")]"#);
    assert_eq!(extract_serde_with(&attrs).as_deref(), Some("option_duration_ms"));
}

#[test]
fn serde_with_captures_serialize_with() {
    let attrs = parse_attrs(r#"#[serde(serialize_with = "ordered_map")]"#);
    assert_eq!(extract_serde_with(&attrs).as_deref(), Some("ordered_map"));
}

#[test]
fn serde_with_ignores_a_lone_deserialize_with() {
    let attrs = parse_attrs(r#"#[serde(deserialize_with = "lenient_u64")]"#);
    assert_eq!(
        extract_serde_with(&attrs),
        None,
        "deserialize_with changes only the read side, so the serialized shape is still derived"
    );
}

#[test]
fn serde_with_finds_serialize_with_listed_after_deserialize_with() {
    // `deserialize_with = "..."` contains `serialize_with = "..."` as a substring, so the
    // rejected match sits ahead of the real one — scanning only the first hit loses it. ~keep
    let attrs = parse_attrs(r#"#[serde(deserialize_with = "read_ms", serialize_with = "write_ms")]"#);
    assert_eq!(extract_serde_with(&attrs).as_deref(), Some("write_ms"));
}

#[test]
fn serde_with_reads_through_cfg_attr() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", serde(with = "duration_ms"))]"#);
    assert_eq!(extract_serde_with(&attrs).as_deref(), Some("duration_ms"));
}

#[test]
fn serde_with_absent_returns_none() {
    let attrs = parse_attrs(r#"#[serde(rename = "type")]"#);
    assert!(extract_serde_with(&attrs).is_none());
}

#[test]
fn serde_with_ignores_a_non_serde_attribute() {
    let attrs = parse_attrs("#[alef(skip)]");
    assert!(extract_serde_with(&attrs).is_none());
}

// --- extract_serde_skip_serializing_if ---

#[test]
fn serde_skip_serializing_if_detects_vec_is_empty() {
    // The exact shape that produced tslp's `data_extraction_json_empty_object` crash:
    // `Vec<DataNode>` is a required field, but serde omits the JSON key when it's empty.
    let attrs = parse_attrs(r#"#[serde(default, skip_serializing_if = "Vec::is_empty")]"#);
    assert!(extract_serde_skip_serializing_if(&attrs));
}

#[test]
fn serde_skip_serializing_if_detects_option_is_none() {
    let attrs = parse_attrs(r#"#[serde(default, skip_serializing_if = "Option::is_none")]"#);
    assert!(extract_serde_skip_serializing_if(&attrs));
}

#[test]
fn serde_skip_serializing_if_reads_through_cfg_attr() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]"#);
    assert!(extract_serde_skip_serializing_if(&attrs));
}

#[test]
fn serde_skip_serializing_if_absent_returns_false() {
    let attrs = parse_attrs(r#"#[serde(rename = "type")]"#);
    assert!(!extract_serde_skip_serializing_if(&attrs));
}

#[test]
fn serde_skip_serializing_if_ignores_a_non_serde_attribute() {
    let attrs = parse_attrs("#[alef(skip)]");
    assert!(!extract_serde_skip_serializing_if(&attrs));
}

// --- has_container_serde_default ---

#[test]
fn container_serde_default_reads_a_bare_container_attribute() {
    let attrs = parse_attrs("#[serde(default)]");
    assert!(has_container_serde_default(&attrs));
}

#[test]
fn container_serde_default_reads_a_named_default_path() {
    let attrs = parse_attrs(r#"#[serde(default = "RetryPolicy::conservative")]"#);
    assert!(has_container_serde_default(&attrs));
}

#[test]
fn container_serde_default_reads_it_alongside_other_container_keys() {
    let attrs = parse_attrs(r#"#[serde(rename_all = "camelCase", default, deny_unknown_fields)]"#);
    assert!(has_container_serde_default(&attrs));
}

#[test]
fn container_serde_default_reads_through_cfg_attr() {
    let attrs = parse_attrs(r#"#[cfg_attr(feature = "serde", serde(default))]"#);
    assert!(has_container_serde_default(&attrs));
}

#[test]
fn container_serde_default_ignores_a_derived_default() {
    // `#[derive(serde::Serialize, serde::Deserialize, Default)]` mentions both `serde` and
    // `Default`, but deriving `Default` says nothing about wire-optionality — every field of
    // such a struct is still a required key. Confusing the two is the exact regression the
    // Go backend's `needs_omitempty_pointer` documents. ~keep
    let attrs = parse_attrs("#[derive(serde::Serialize, serde::Deserialize, Default)]");
    assert!(!has_container_serde_default(&attrs));
}

#[test]
fn container_serde_default_ignores_prose_in_a_doc_comment() {
    // The trailing newline matters: `parse_attrs` appends `struct _Dummy;`, which a doc
    // comment would otherwise swallow to end of line. ~keep
    let attrs = parse_attrs("/// Missing keys fall back through #[serde(default)] on each field.\n");
    assert!(
        !has_container_serde_default(&attrs),
        "a doc comment is prose, not an attribute"
    );
}

#[test]
fn container_serde_default_absent_returns_false() {
    let attrs = parse_attrs(r#"#[serde(rename_all = "snake_case")]"#);
    assert!(!has_container_serde_default(&attrs));
}
