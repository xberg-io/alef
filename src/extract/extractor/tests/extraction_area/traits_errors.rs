use super::*;

#[test]
fn test_trait_impl_methods_extracted() {
    let source = r#"
        pub struct DefaultClient {
            pub base_url: String,
        }

        impl DefaultClient {
            pub fn new(base_url: String) -> DefaultClient {
                unimplemented!()
            }
        }

        trait LlmClient {
            async fn chat(&self, prompt: String) -> Result<String, MyError>;
            fn model(&self) -> String;
        }

        impl LlmClient for DefaultClient {
            async fn chat(&self, prompt: String) -> Result<String, MyError> {
                unimplemented!()
            }

            fn model(&self) -> String {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let client = &surface.types[0];
    assert_eq!(client.name, "DefaultClient");
    assert_eq!(client.methods.len(), 3);

    let method_names: Vec<&str> = client.methods.iter().map(|m| m.name.as_str()).collect();
    assert!(method_names.contains(&"new"));
    assert!(method_names.contains(&"chat"));
    assert!(method_names.contains(&"model"));

    let chat = client.methods.iter().find(|m| m.name == "chat").unwrap();
    assert!(chat.is_async);
    assert_eq!(chat.receiver, Some(ReceiverKind::Ref));
    assert_eq!(chat.error_type.as_deref(), Some("MyError"));
}

#[test]
fn test_trait_impl_no_duplicate_methods() {
    let source = r#"
        pub struct MyType {}

        impl MyType {
            pub fn do_thing(&self) -> String {
                unimplemented!()
            }
        }

        trait SomeTrait {
            fn do_thing(&self) -> String;
        }

        impl SomeTrait for MyType {
            fn do_thing(&self) -> String {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let my_type = &surface.types[0];
    let do_thing_count = my_type.methods.iter().filter(|m| m.name == "do_thing").count();
    assert_eq!(do_thing_count, 1);
}

#[test]
fn test_trait_impl_ignored_for_unknown_type() {
    let source = r#"
        trait SomeTrait {
            fn method(&self);
        }

        impl SomeTrait for UnknownType {
            fn method(&self) {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 0);
}

#[test]
fn test_extract_thiserror_enum() {
    let source = r#"
        #[derive(Debug, thiserror::Error)]
        pub enum MyError {
            /// An I/O error.
            #[error("I/O error: {0}")]
            Io(#[from] std::io::Error),

            /// A parsing error.
            #[error("Parsing error: {message}")]
            Parsing {
                message: String,
                #[source]
                source: Option<Box<dyn std::error::Error + Send + Sync>>,
            },

            /// A timeout error.
            #[error("Extraction timed out after {elapsed_ms}ms")]
            Timeout { elapsed_ms: u64, limit_ms: u64 },

            /// A missing dependency.
            #[error("Missing dependency: {0}")]
            MissingDependency(String),

            /// An unknown error.
            #[error("Unknown error")]
            Unknown,
        }
    "#;

    let surface = extract_from_source(source);

    assert_eq!(surface.enums.len(), 0, "thiserror enum should not be in enums");
    assert_eq!(surface.errors.len(), 1, "thiserror enum should be in errors");

    let err = &surface.errors[0];
    assert_eq!(err.name, "MyError");
    assert_eq!(err.variants.len(), 5);

    // Io variant: tuple with #[from]
    let io = &err.variants[0];
    assert_eq!(io.name, "Io");
    assert_eq!(io.message_template.as_deref(), Some("I/O error: {0}"));
    assert!(io.has_from, "Io should have from");
    assert!(io.has_source, "Io should have source (implied by from)");
    assert!(!io.is_unit, "Io is not a unit variant");
    assert_eq!(io.fields.len(), 1);

    // Parsing variant: struct with #[source]
    let parsing = &err.variants[1];
    assert_eq!(parsing.name, "Parsing");
    assert_eq!(parsing.message_template.as_deref(), Some("Parsing error: {message}"));
    assert!(!parsing.has_from, "Parsing should not have from");
    assert!(parsing.has_source, "Parsing should have source");
    assert!(!parsing.is_unit);
    assert_eq!(parsing.fields.len(), 2);
    assert_eq!(parsing.fields[0].name, "message");
    assert_eq!(parsing.fields[1].name, "source");

    let timeout = &err.variants[2];
    assert_eq!(timeout.name, "Timeout");
    assert_eq!(
        timeout.message_template.as_deref(),
        Some("Extraction timed out after {elapsed_ms}ms")
    );
    assert!(!timeout.has_from);
    assert!(!timeout.has_source);
    assert!(!timeout.is_unit);
    assert_eq!(timeout.fields.len(), 2);

    let missing = &err.variants[3];
    assert_eq!(missing.name, "MissingDependency");
    assert_eq!(missing.message_template.as_deref(), Some("Missing dependency: {0}"));
    assert!(!missing.has_from);
    assert!(!missing.has_source);
    assert!(!missing.is_unit);
    assert_eq!(missing.fields.len(), 1);

    let unknown = &err.variants[4];
    assert_eq!(unknown.name, "Unknown");
    assert_eq!(unknown.message_template.as_deref(), Some("Unknown error"));
    assert!(!unknown.has_from);
    assert!(!unknown.has_source);
    assert!(unknown.is_unit);
    assert_eq!(unknown.fields.len(), 0);
}

#[test]
fn test_extract_thiserror_with_use_import() {
    let source = r#"
        #[derive(Debug, Error)]
        pub enum AppError {
            #[error("not found")]
            NotFound,

            #[error("invalid input: {0}")]
            InvalidInput(String),
        }
    "#;

    let surface = extract_from_source(source);

    assert_eq!(surface.enums.len(), 0);
    assert_eq!(surface.errors.len(), 1);

    let err = &surface.errors[0];
    assert_eq!(err.name, "AppError");
    assert_eq!(err.variants.len(), 2);

    assert!(err.variants[0].is_unit);
    assert_eq!(err.variants[0].message_template.as_deref(), Some("not found"));

    assert!(!err.variants[1].is_unit);
    assert_eq!(err.variants[1].fields.len(), 1);
}

#[test]
fn test_non_thiserror_enum_not_in_errors() {
    let source = r#"
        #[derive(Debug, Clone)]
        pub enum Format {
            Pdf,
            Html,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);
    assert_eq!(surface.errors.len(), 0, "non-thiserror enum should not be in errors");
}

#[test]
fn test_pub_trait_with_supertrait() {
    let source = r#"
        pub trait Backend: Send + Sync {}

        pub trait WorkerBackend: Backend {}
    "#;

    let surface = extract_from_source(source);
    let worker = surface
        .types
        .iter()
        .find(|t| t.name == "WorkerBackend")
        .expect("WorkerBackend not found");
    assert!(worker.is_trait);
    assert_eq!(worker.super_traits, vec!["Backend"]);

    let backend = surface
        .types
        .iter()
        .find(|t| t.name == "Backend")
        .expect("Backend not found");
    assert!(
        backend.super_traits.is_empty(),
        "Send/Sync should be filtered from super_traits"
    );
}

#[test]
fn test_resolve_trait_sources_retroactive() {
    let source = r#"
        pub struct Widget {
            pub label: String,
        }

        impl Renderable for Widget {
            fn render(&self) -> String {
                unimplemented!()
            }
        }

        pub trait Renderable {
            fn render(&self) -> String;
        }
    "#;

    let surface = extract_from_source(source);

    let widget = surface
        .types
        .iter()
        .find(|t| t.name == "Widget")
        .expect("Widget not found");
    let render = widget
        .methods
        .iter()
        .find(|m| m.name == "render")
        .expect("render not found");
    assert!(
        render.trait_source.is_some(),
        "trait_source should be resolved even when trait is defined after impl"
    );
}

#[test]
fn test_trait_method_with_default_impl() {
    let source = r#"
        pub trait Logger {
            fn log(&self, message: String);

            fn log_error(&self, message: String) {
                self.log(message)
            }
        }
    "#;

    let surface = extract_from_source(source);
    let logger = surface
        .types
        .iter()
        .find(|t| t.name == "Logger")
        .expect("Logger not found");
    assert!(logger.is_trait);

    let log_method = logger.methods.iter().find(|m| m.name == "log").unwrap();
    assert!(!log_method.has_default_impl, "log() has no default impl");

    let log_error = logger.methods.iter().find(|m| m.name == "log_error").unwrap();
    assert!(log_error.has_default_impl, "log_error() has a default impl body");
}

#[test]
fn test_thiserror_enum_with_inherent_impl_does_not_create_opaque_type() {
    let source = r#"
        use thiserror::Error;

        #[derive(Debug, Error)]
        pub enum ApiError {
            #[error("not found: {0}")]
            NotFound(String),
            #[error("internal: {0}")]
            Internal(String),
        }

        impl ApiError {
            pub fn status_code(&self) -> u16 {
                match self {
                    Self::NotFound(_) => 404,
                    Self::Internal(_) => 500,
                }
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.errors.len(), 1, "ApiError should be in errors");
    assert_eq!(
        surface.types.len(),
        0,
        "ApiError must NOT appear in types — impl block on error enum must not create opaque TypeDef"
    );
    let err = &surface.errors[0];
    assert_eq!(err.name, "ApiError");
    assert_eq!(err.variants.len(), 2);
}
