use super::*;

#[test]
fn test_field_with_string_from_default() {
    let source = r#"
        pub struct Label {
            pub name: String,
        }

        impl Default for Label {
            fn default() -> Self {
                Label { name: String::from("default") }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let label = &surface.types[0];
    let name_field = &label.fields[0];

    assert_eq!(
        name_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral("default".to_string())),
        "String::from(...) should be extracted as StringLiteral"
    );
}

#[test]
fn test_field_with_string_new_default() {
    let source = r#"
        pub struct Buffer {
            pub data: String,
        }

        impl Default for Buffer {
            fn default() -> Self {
                Buffer { data: String::new() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let buffer = &surface.types[0];
    let data_field = &buffer.fields[0];

    assert_eq!(
        data_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral(String::new())),
        "String::new() should be extracted as StringLiteral(\"\")"
    );
}

#[test]
fn test_field_with_string_to_string_default() {
    let source = r#"
        pub struct Display {
            pub content: String,
        }

        impl Default for Display {
            fn default() -> Self {
                Display { content: "placeholder".to_string() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let display = &surface.types[0];
    let content_field = &display.fields[0];

    assert_eq!(
        content_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral("placeholder".to_string())),
        "\"str\".to_string() should extract the string literal"
    );
}

#[test]
fn test_field_with_char_default() {
    let source = r#"
        pub struct Separator {
            pub delimiter: char,
        }

        impl Default for Separator {
            fn default() -> Self {
                Separator { delimiter: ',' }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let separator = &surface.types[0];
    let delimiter_field = &separator.fields[0];

    assert_eq!(
        delimiter_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral(",".to_string())),
        "char literal should be extracted as StringLiteral"
    );
}

#[test]
fn test_field_with_vec_new_default() {
    let source = r#"
        pub struct Collection {
            pub items: Vec<String>,
        }

        impl Default for Collection {
            fn default() -> Self {
                Collection { items: Vec::new() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let collection = &surface.types[0];
    let items_field = &collection.fields[0];

    assert_eq!(
        items_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Vec::new() should extract as Empty"
    );
}

#[test]
fn test_field_with_enum_variant_default() {
    let source = r#"
        #[derive(Clone)]
        pub enum Status {
            Pending,
            Active,
            Inactive,
        }

        pub struct Task {
            pub status: Status,
        }

        impl Default for Task {
            fn default() -> Self {
                Task { status: Status::Pending }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let task = surface.types.iter().find(|t| t.name == "Task").unwrap();
    let status_field = &task.fields[0];

    assert_eq!(
        status_field.typed_default,
        Some(crate::core::ir::DefaultValue::EnumVariant("Pending".to_string())),
        "SomeEnum::Variant should extract EnumVariant"
    );
}

#[test]
fn test_multiple_fields_with_different_defaults() {
    let source = r#"
        pub struct Config {
            pub name: String,
            pub count: u32,
            pub enabled: bool,
            pub threshold: f64,
        }

        impl Default for Config {
            fn default() -> Self {
                Config {
                    name: "default".into(),
                    count: 42,
                    enabled: false,
                    threshold: 0.5,
                }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = &surface.types[0];

    assert_eq!(config.fields.len(), 4);

    let name_field = &config.fields[0];
    assert_eq!(name_field.name, "name");
    assert_eq!(
        name_field.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral("default".to_string()))
    );

    let count_field = &config.fields[1];
    assert_eq!(count_field.name, "count");
    assert_eq!(
        count_field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(42))
    );

    let enabled_field = &config.fields[2];
    assert_eq!(enabled_field.name, "enabled");
    assert_eq!(
        enabled_field.typed_default,
        Some(crate::core::ir::DefaultValue::BoolLiteral(false))
    );

    let threshold_field = &config.fields[3];
    assert_eq!(threshold_field.name, "threshold");
    assert_eq!(
        threshold_field.typed_default,
        Some(crate::core::ir::DefaultValue::FloatLiteral(0.5))
    );
}

#[test]
fn test_field_with_default_default_call() {
    let source = r#"
        pub struct Delegated {
            pub inner: u64,
        }

        impl Default for Delegated {
            fn default() -> Self {
                Delegated { inner: u64::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let delegated = &surface.types[0];
    let inner_field = &delegated.fields[0];

    assert_eq!(
        inner_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "T::default() should extract as Empty"
    );
}

#[test]
fn test_field_with_generic_default_call() {
    let source = r#"
        pub struct Generic {
            pub value: String,
        }

        impl Default for Generic {
            fn default() -> Self {
                Generic { value: Default::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let generic = &surface.types[0];
    let value_field = &generic.fields[0];

    assert_eq!(
        value_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Default::default() should extract as Empty"
    );
}

#[test]
fn test_field_with_hashmap_new_default() {
    let source = r#"
        use std::collections::HashMap;

        pub struct Cache {
            pub data: HashMap<String, String>,
        }

        impl Default for Cache {
            fn default() -> Self {
                Cache { data: HashMap::new() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let cache = &surface.types[0];
    let data_field = &cache.fields[0];

    assert_eq!(
        data_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "HashMap::new() should extract as Empty"
    );
}

#[test]
fn test_complex_expression_defaults_to_empty() {
    let source = r#"
        pub struct Complex {
            pub result: u32,
        }

        impl Default for Complex {
            fn default() -> Self {
                Complex { result: some_function() }
            }
        }

        fn some_function() -> u32 {
            42
        }
    "#;

    let surface = extract_from_source(source);
    let complex = &surface.types[0];
    let result_field = &complex.fields[0];

    assert_eq!(
        result_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Complex expressions like function calls should default to Empty"
    );
}

#[test]
fn test_field_with_duration_from_secs_default() {
    let source = r#"
        use std::time::Duration;

        pub struct Timeout {
            pub wait: Duration,
        }

        impl Default for Timeout {
            fn default() -> Self {
                Timeout { wait: Duration::from_secs(5) }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let timeout = &surface.types[0];
    let wait_field = &timeout.fields[0];

    assert_eq!(
        wait_field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(5000)),
        "Duration::from_secs(5) should be 5000 milliseconds"
    );
}

#[test]
fn test_field_with_duration_from_millis_default() {
    let source = r#"
        use std::time::Duration;

        pub struct Backoff {
            pub delay: Duration,
        }

        impl Default for Backoff {
            fn default() -> Self {
                Backoff { delay: Duration::from_millis(250) }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let backoff = &surface.types[0];
    let delay_field = &backoff.fields[0];

    assert_eq!(
        delay_field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(250)),
        "Duration::from_millis(250) should be 250 milliseconds"
    );
}

#[test]
fn test_field_with_vec_macro_default() {
    let source = r#"
        pub struct Pipeline {
            pub stages: Vec<String>,
        }

        impl Default for Pipeline {
            fn default() -> Self {
                Pipeline { stages: vec![] }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let pipeline = &surface.types[0];
    let stages_field = &pipeline.fields[0];

    assert_eq!(
        stages_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "vec![] should extract as Empty"
    );
}

#[test]
fn test_field_with_none_default() {
    let source = r#"
        pub struct Optional {
            pub value: Option<String>,
        }

        impl Default for Optional {
            fn default() -> Self {
                Optional { value: None }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let optional_type = &surface.types[0];
    let value_field = &optional_type.fields[0];

    assert_eq!(
        value_field.typed_default,
        Some(crate::core::ir::DefaultValue::None),
        "Bare None should extract as DefaultValue::None"
    );
}

#[test]
fn test_unary_negation_on_non_numeric_falls_back_to_empty() {
    let source = r#"
        pub struct Unusual {
            pub val: i32,
        }

        fn compute() -> i32 { 0 }

        impl Default for Unusual {
            fn default() -> Self {
                // This will be parsed as Unary(Neg, Call(...)) — the inner call returns Empty,
                // so the negation should also return Empty.
                Unusual { val: -(compute()) }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let unusual = &surface.types[0];
    let val_field = &unusual.fields[0];

    assert_eq!(
        val_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Negating a non-literal expression should fall back to Empty"
    );
}
