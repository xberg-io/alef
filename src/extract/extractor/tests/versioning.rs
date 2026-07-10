use super::*;

#[test]
fn test_extract_version_annotation_since_flows_through_to_function_ir() {
    let surface = extract_from_source(
        r#"
        #[alef(since = "1.3.0")]
        pub fn versioned_fn() -> u32 { 0 }
        "#,
    );
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(
        surface.functions[0].version.since.as_deref(),
        Some("1.3.0"),
        "since version must be extracted from #[alef(since = ...)] on a function"
    );
    assert!(surface.functions[0].version.deprecated.is_none());
}

#[test]
fn test_extract_deprecated_flows_through_to_function_ir() {
    let surface = extract_from_source(
        r#"
        #[deprecated(since = "2.0.0", note = "use new_fn instead")]
        pub fn old_fn() {}
        "#,
    );
    assert_eq!(surface.functions.len(), 1);
    let dep = surface.functions[0]
        .version
        .deprecated
        .as_ref()
        .expect("deprecated must be set");
    assert_eq!(dep.since.as_deref(), Some("2.0.0"));
    assert_eq!(dep.note.as_deref(), Some("use new_fn instead"));
}

#[test]
fn test_extract_version_annotation_on_struct_flows_through_to_type_ir() {
    let surface = extract_from_source(
        r#"
        #[alef(since = "0.9.0")]
        pub struct NewConfig {
            pub value: u32,
        }
        "#,
    );
    assert_eq!(surface.types.len(), 1);
    assert_eq!(
        surface.types[0].version.since.as_deref(),
        Some("0.9.0"),
        "since version must be extracted from #[alef(since = ...)] on a struct"
    );
}

#[test]
fn test_extract_version_annotation_on_enum_variant_flows_through_to_ir() {
    let surface = extract_from_source(
        r#"
        pub enum Status {
            #[alef(since = "1.1.0")]
            Active,
            Legacy,
        }
        "#,
    );
    assert_eq!(surface.enums.len(), 1);
    let variants = &surface.enums[0].variants;
    let active = variants
        .iter()
        .find(|v| v.name == "Active")
        .expect("Active variant must be present");
    assert_eq!(
        active.version.since.as_deref(),
        Some("1.1.0"),
        "since version must be extracted from #[alef(since = ...)] on an enum variant"
    );
    let legacy = variants
        .iter()
        .find(|v| v.name == "Legacy")
        .expect("Legacy variant must be present");
    assert!(legacy.version.since.is_none(), "unannotated variant must have no since");
}

#[test]
fn test_extract_function_deprecated_annotation_is_populated() {
    let source = r#"
        #[deprecated(since = "1.5.0", note = "use new_api instead")]
        pub fn old_api() {}
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);
    let dep = surface.functions[0].version.deprecated.as_ref().unwrap();
    assert_eq!(dep.since.as_deref(), Some("1.5.0"));
    assert_eq!(dep.note.as_deref(), Some("use new_api instead"));
}

#[test]
fn test_extract_struct_since_annotation_is_populated() {
    let source = r#"
        #[derive(Clone, Debug)]
        #[alef(since = "2.0.0")]
        pub struct NewType {
            pub field: String,
        }
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].version.since.as_deref(), Some("2.0.0"));
}

#[test]
fn test_extract_enum_since_annotation_is_populated() {
    let source = r#"
        #[alef(since = "3.0.0")]
        pub enum Status { Active, Inactive }
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);
    assert_eq!(surface.enums[0].version.since.as_deref(), Some("3.0.0"));
}

#[test]
fn test_extract_since_v_prefix_is_stripped() {
    let source = r#"
        #[alef(since = "v1.2.0")]
        pub fn prefixed() {}
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.functions[0].version.since.as_deref(), Some("1.2.0"));
}

#[test]
fn test_extract_cfg_attr_feature_gate_since_is_populated() {
    let source = r#"
        #[cfg_attr(feature = "alef-meta", alef(since = "0.9.0"))]
        pub fn gated() {}
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.functions[0].version.since.as_deref(), Some("0.9.0"));
}
