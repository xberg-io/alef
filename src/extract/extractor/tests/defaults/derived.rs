use super::*;

#[test]
fn test_struct_with_default_derive() {
    let source = r#"
        /// A configuration with sensible defaults.
        #[derive(Default, Clone)]
        pub struct Config {
            pub name: String,
            pub count: u32,
            pub enabled: bool,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let config = &surface.types[0];
    assert_eq!(config.name, "Config");
    // has_default should be true for types with #[derive(Default)]
    assert!(
        config.has_default,
        "Config with #[derive(Default)] should have has_default=true"
    );
}

#[test]
fn test_struct_without_default() {
    let source = r#"
        /// A configuration without defaults.
        pub struct Custom {
            pub value: String,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let custom = &surface.types[0];
    assert_eq!(custom.name, "Custom");
    assert!(
        !custom.has_default,
        "Struct without Default should have has_default=false"
    );
}

#[test]
fn test_impl_default_without_fn_default() {
    let source = r#"
        pub struct Incomplete {
            pub value: u32,
        }

        impl Default for Incomplete {
            // Missing fn default() - no matching method
        }
    "#;

    let surface = extract_from_source(source);
    let incomplete = &surface.types[0];
    let value_field = &incomplete.fields[0];

    assert_eq!(
        value_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Fields should have Empty when fn default() is missing"
    );
}

#[test]
fn test_enum_with_default_derive_and_default_variant() {
    let source = r#"
        #[derive(Default, Clone)]
        pub enum Priority {
            #[default]
            Normal,
            High,
            Low,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);

    let priority = &surface.enums[0];
    assert_eq!(priority.name, "Priority");
    assert_eq!(priority.variants.len(), 3);

    let normal = &priority.variants[0];
    assert_eq!(normal.name, "Normal");
    assert!(
        normal.is_default,
        "Normal variant with #[default] should have is_default=true"
    );

    let high = &priority.variants[1];
    assert_eq!(high.name, "High");
    assert!(!high.is_default, "Non-default variant should have is_default=false");

    let low = &priority.variants[2];
    assert_eq!(low.name, "Low");
    assert!(!low.is_default);
}

#[test]
fn test_enum_without_default() {
    let source = r#"
        pub enum Format {
            Json,
            Xml,
            Yaml,
        }
    "#;

    let surface = extract_from_source(source);
    let format = &surface.enums[0];

    for variant in &format.variants {
        assert!(
            !variant.is_default,
            "Variants without #[default] should be is_default=false"
        );
    }
}

#[test]
fn test_enum_with_manual_default_impl() {
    let source = r#"
        pub enum ClassificationMode {
            Known,
            Custom(String),
        }

        impl Default for ClassificationMode {
            fn default() -> Self {
                Self::Custom(String::new())
            }
        }
    "#;

    let surface = extract_from_source(source);
    let mode = &surface.enums[0];

    assert!(mode.has_default, "manual Default impl should set has_default=true");
    assert!(
        mode.variants.iter().all(|variant| !variant.is_default),
        "manual enum Default impls should not synthesize a default variant"
    );
}
