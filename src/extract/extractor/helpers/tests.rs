use super::attributes::{extract_alef_since, extract_deprecation, has_derive, has_derive_path};
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
