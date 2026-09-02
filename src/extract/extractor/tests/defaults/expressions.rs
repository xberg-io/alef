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
fn test_zero_arg_function_call_default_folds_a_visible_constant_body() {
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

    // ~keep The `impl Default` initializer path folds a zero-arg call against the same free
    // function index `#[serde(default = "path")]` fields already use. Before both paths shared
    // the folder, a field carrying BOTH -- `#[serde(default = "f")]` beside `Self { x: f(), .. }`
    // -- regressed from the folded literal to a bare `FunctionCall`, because the `impl Default`
    // read overwrote what the serde read had resolved. The two must agree, and the resolved
    // literal is the more accurate of the two answers.
    assert_eq!(
        result_field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(42)),
        "a zero-arg call whose body is visible and constant should fold to that constant"
    );
}

#[test]
fn test_zero_arg_function_call_default_stays_a_function_call_when_the_body_is_not_constant() {
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
            std::time::SystemTime::now().elapsed().unwrap().as_secs() as u32
        }
    "#;

    let surface = extract_from_source(source);
    let complex = &surface.types[0];
    let result_field = &complex.fields[0];

    // The folder declines anything it cannot resolve to a literal, so the callee path survives
    // for callers that need to name the call rather than its value. This is the guard that keeps
    // the fold above from being a guess.
    assert_eq!(
        result_field.typed_default,
        Some(crate::core::ir::DefaultValue::FunctionCall("some_function".to_string())),
        "a call the folder cannot resolve must keep naming its callee"
    );
}

#[test]
fn test_call_with_args_is_unresolved_not_empty() {
    let source = r#"
        pub struct Complex {
            pub result: u32,
        }

        impl Default for Complex {
            fn default() -> Self {
                Complex { result: some_function(1, 2) }
            }
        }

        fn some_function(a: u32, b: u32) -> u32 {
            a + b
        }
    "#;

    let surface = extract_from_source(source);
    let complex = &surface.types[0];
    let result_field = &complex.fields[0];

    assert!(
        matches!(
            result_field.typed_default,
            Some(crate::core::ir::DefaultValue::Unresolved(_))
        ),
        "a call with arguments cannot be folded, so its value is unknown -- `Empty` would assert \
         the default IS the type-zero; got {:?}",
        result_field.typed_default
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

/// A non-empty `vec!` literal must carry its elements, not collapse to `Empty`.
///
/// Before this, every macro default -- empty or not -- returned `Empty`, so a Rust default of
/// `vec!["markdown"]` reached the backends indistinguishable from `vec![]` and every binding
/// emitted an empty collection. The guard that looked like it separated the two cases was dead:
/// both of its branches returned `Empty`.
#[test]
fn non_empty_vec_macro_default_keeps_its_elements() {
    let source = r#"
        pub struct Pipeline {
            pub stages: Vec<String>,
        }

        impl Default for Pipeline {
            fn default() -> Self {
                Pipeline { stages: vec!["markdown".to_owned(), "html".to_owned()] }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let stages_field = &surface.types[0].fields[0];

    assert_eq!(
        stages_field.typed_default,
        Some(crate::core::ir::DefaultValue::ListLiteral(vec![
            crate::core::ir::DefaultValue::StringLiteral("markdown".to_string()),
            crate::core::ir::DefaultValue::StringLiteral("html".to_string()),
        ])),
        "a populated vec! literal must reach the IR with its elements intact"
    );
}

/// The all-or-nothing rule: one unrepresentable element makes the whole literal unreadable.
///
/// A partially-lowered list would hand a backend a default that silently differs from the Rust
/// one, which is strictly worse than the pre-existing loss.
#[test]
fn vec_macro_default_with_an_unrepresentable_element_is_unresolved() {
    let source = r#"
        pub struct Pipeline {
            pub stages: Vec<String>,
        }

        impl Default for Pipeline {
            fn default() -> Self {
                Pipeline { stages: vec!["markdown".to_owned(), compute_stage()] }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let stages_field = &surface.types[0].fields[0];

    assert!(
        matches!(
            stages_field.typed_default,
            Some(crate::core::ir::DefaultValue::Unresolved(_))
        ),
        "a literal containing a non-representable element must not lower to a partial list, and \
         the whole list is a value alef never read; got {:?}",
        stages_field.typed_default
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
fn test_unary_negation_on_non_numeric_is_unresolved() {
    let source = r#"
        pub struct Unusual {
            pub val: i32,
        }

        fn compute() -> i32 { 0 }

        impl Default for Unusual {
            fn default() -> Self {
                // Parsed as Unary(Neg, Paren(Call(..))) — the inner call is unreadable, so the
                // negation is unreadable too.
                Unusual { val: -(compute()) }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let unusual = &surface.types[0];
    let val_field = &unusual.fields[0];

    assert!(
        matches!(
            val_field.typed_default,
            Some(crate::core::ir::DefaultValue::Unresolved(_))
        ),
        "negating a non-literal expression yields a value alef never read; got {:?}",
        val_field.typed_default
    );
}

/// The reported defect, end to end through the real extractor so the field's declared type is
/// the one alef actually derives.
///
/// `Self::DEFAULT_MODEL` is a two-segment path exactly like `Status::Pending`, and the extractor
/// lowered both to `EnumVariant`. On a `String` field `codegen::config_gen::shared` then renders
/// an `EnumVariant` as its snake-cased name, so the binding shipped `"default_model"` — a value
/// that appears nowhere in the source crate. The const is readable, so the fix is not to report
/// the field as unresolved but to resolve it.
#[test]
fn associated_const_default_on_a_string_field_is_the_consts_value_not_a_variant_name() {
    let source = r#"
        pub struct LlmConfig {
            pub model: String,
        }

        impl LlmConfig {
            pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
        }

        impl Default for LlmConfig {
            fn default() -> Self {
                Self { model: Self::DEFAULT_MODEL.to_string() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "LlmConfig").unwrap();
    let model = &config.fields[0];

    assert_eq!(
        model.typed_default,
        Some(crate::core::ir::DefaultValue::StringLiteral(
            "claude-sonnet-4-5".to_string()
        ))
    );
    assert_ne!(
        model.typed_default,
        Some(crate::core::ir::DefaultValue::EnumVariant("DEFAULT_MODEL".to_string())),
        "an associated const is not an enum variant, and a `String` field cannot hold one"
    );
    assert_ne!(
        crate::codegen::config_gen::default_value_for_field(model, "python"),
        "\"default_model\"",
        "the snake-cased const name is fabricated; it must not reach a generated binding"
    );
}

/// The same shape with the const declared out of module reach. There is no value to recover, so
/// the field is reported — but it must still not be lowered to a variant name, because that is
/// what the renderer turns into a plausible-looking fabricated string.
#[test]
fn unreachable_associated_const_on_a_string_field_is_unresolved_not_a_variant_name() {
    let source = r#"
        pub struct LlmConfig {
            pub model: String,
        }

        impl Default for LlmConfig {
            fn default() -> Self {
                Self { model: Self::DEFAULT_MODEL.to_string() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "LlmConfig").unwrap();
    let model = &config.fields[0];

    assert!(
        matches!(model.typed_default, Some(crate::core::ir::DefaultValue::Unresolved(_))),
        "got {:?}",
        model.typed_default
    );
    assert_ne!(
        crate::codegen::config_gen::default_value_for_field(model, "python"),
        "\"default_model\"",
        "the fabricated snake-cased const name must be absent from generated output"
    );
}
