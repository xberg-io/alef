use super::*;

#[test]
fn test_field_with_bool_default_literal() {
    let source = r#"
        pub struct Settings {
            pub verbose: bool,
        }

        impl Default for Settings {
            fn default() -> Self {
                Settings { verbose: true }
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let settings = &surface.types[0];
    assert_eq!(settings.fields.len(), 1);

    let verbose_field = &settings.fields[0];
    assert_eq!(verbose_field.name, "verbose");
    assert_eq!(
        verbose_field.typed_default,
        Some(crate::core::ir::DefaultValue::BoolLiteral(true)),
        "bool field should have BoolLiteral(true) default"
    );
}

#[test]
fn test_field_with_int_default_literal() {
    let source = r#"
        pub struct Limits {
            pub max_retries: i32,
        }

        impl Default for Limits {
            fn default() -> Self {
                Limits { max_retries: 3 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let limits = &surface.types[0];
    assert_eq!(limits.fields.len(), 1);

    let max_retries = &limits.fields[0];
    assert_eq!(max_retries.name, "max_retries");
    assert_eq!(
        max_retries.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(3)),
        "int field should have IntLiteral(3) default"
    );
}

#[test]
fn test_field_with_negative_int_default() {
    let source = r#"
        pub struct Temperature {
            pub celsius: i32,
        }

        impl Default for Temperature {
            fn default() -> Self {
                Temperature { celsius: -273 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let temperature = &surface.types[0];
    let celsius_field = &temperature.fields[0];

    assert_eq!(
        celsius_field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(-273)),
        "negative int literal should be parsed correctly"
    );
}

#[test]
fn test_field_with_float_default_literal() {
    let source = r#"
        pub struct Thresholds {
            pub confidence: f64,
        }

        impl Default for Thresholds {
            fn default() -> Self {
                Thresholds { confidence: 0.95 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let thresholds = &surface.types[0];
    let confidence_field = &thresholds.fields[0];

    assert_eq!(
        confidence_field.typed_default,
        Some(crate::core::ir::DefaultValue::FloatLiteral(0.95)),
        "float field should have FloatLiteral(0.95) default"
    );
}

#[test]
fn test_field_with_negative_float_default() {
    let source = r#"
        pub struct Adjustment {
            pub offset: f64,
        }

        impl Default for Adjustment {
            fn default() -> Self {
                Adjustment { offset: -1.5 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let adjustment = &surface.types[0];
    let offset_field = &adjustment.fields[0];

    assert_eq!(
        offset_field.typed_default,
        Some(crate::core::ir::DefaultValue::FloatLiteral(-1.5)),
        "negative float literal should be parsed correctly"
    );
}

#[test]
fn test_field_with_string_literal_default() {
    let source = r#"
        pub struct Message {
            pub text: String,
        }

        impl Default for Message {
            fn default() -> Self {
                Message { text: "hello world".into() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let message = &surface.types[0];
    let text_field = &message.fields[0];

    assert_eq!(
        text_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral("hello world".to_string())),
        "string field with .into() should have StringLiteral default"
    );
}

#[test]
fn test_field_missing_from_struct_literal() {
    let source = r#"
        pub struct Partial {
            pub field_a: u32,
            pub field_b: String,
        }

        impl Default for Partial {
            fn default() -> Self {
                Partial { field_a: 99 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let partial = &surface.types[0];

    assert_eq!(partial.fields.len(), 2);

    let field_a = &partial.fields[0];
    assert_eq!(
        field_a.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(99))
    );

    let field_b = &partial.fields[1];
    assert_eq!(
        field_b.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Field not in struct literal should have Empty default"
    );
}

#[test]
fn test_struct_literal_wrapped_in_block() {
    let source = r#"
        pub struct WithBlock {
            pub value: i32,
        }

        impl Default for WithBlock {
            fn default() -> Self {
                {
                    WithBlock { value: 77 }
                }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let with_block = &surface.types[0];
    let value_field = &with_block.fields[0];

    assert_eq!(
        value_field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(77)),
        "Struct literal inside block expression should be found"
    );
}

#[test]
fn test_field_with_false_bool_default() {
    let source = r#"
        pub struct DisabledFeature {
            pub active: bool,
        }

        impl Default for DisabledFeature {
            fn default() -> Self {
                DisabledFeature { active: false }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let disabled = &surface.types[0];
    let active_field = &disabled.fields[0];

    assert_eq!(
        active_field.typed_default,
        Some(crate::core::ir::DefaultValue::BoolLiteral(false)),
        "false bool literal should be parsed as BoolLiteral(false)"
    );
}

#[test]
fn test_field_with_zero_defaults() {
    let source = r#"
        pub struct Zeroes {
            pub int_val: i32,
            pub float_val: f64,
        }

        impl Default for Zeroes {
            fn default() -> Self {
                Zeroes { int_val: 0, float_val: 0.0 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let zeroes = &surface.types[0];

    assert_eq!(
        zeroes.fields[0].typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(0))
    );
    assert_eq!(
        zeroes.fields[1].typed_default,
        Some(crate::core::ir::DefaultValue::FloatLiteral(0.0))
    );
}

#[test]
fn test_field_with_str_literal_default() {
    let source = r#"
        pub struct Prefix {
            pub label: String,
        }

        impl Default for Prefix {
            fn default() -> Self {
                // Use String::from to get the str literal processed
                Prefix { label: String::from("hello") }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let prefix = &surface.types[0];
    let label_field = &prefix.fields[0];

    assert_eq!(
        label_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral("hello".to_string())),
        "String::from(literal) exercises the Lit::Str branch"
    );
}

#[test]
fn test_field_with_duration_from_secs_non_literal_fallback() {
    let source = r#"
        use std::time::Duration;

        pub struct Config {
            pub timeout: Duration,
        }

        fn get_secs() -> u64 { 10 }

        impl Default for Config {
            fn default() -> Self {
                Config { timeout: Duration::from_secs(get_secs()) }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = &surface.types[0];
    let timeout_field = &config.fields[0];

    assert_eq!(
        timeout_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Duration::from_secs with non-literal arg should fall back to Empty"
    );
}
