use super::*;

#[test]
fn test_newtype_wrapper_resolved() {
    let source = r#"
        /// An element identifier.
        pub struct ElementId(String);

        /// A widget with an element id.
        pub struct Widget {
            pub id: ElementId,
            pub label: String,
        }
    "#;

    let surface = extract_from_source(source);

    // The newtype `ElementId` should be kept in the IR (not resolved away)
    let element_id = surface
        .types
        .iter()
        .find(|t| t.name == "ElementId")
        .expect("Newtype ElementId should be kept in types");
    // Tuple structs are kept but their positional fields may be stripped
    assert!(element_id.fields.is_empty() || element_id.fields[0].name == "_0");

    // Widget should exist with `id` referencing ElementId as a Named type
    let widget = surface
        .types
        .iter()
        .find(|t| t.name == "Widget")
        .expect("Widget should exist");
    assert!(!widget.is_opaque);
    assert_eq!(widget.fields.len(), 2);
    assert_eq!(widget.fields[0].name, "id");
    assert_eq!(
        widget.fields[0].ty,
        TypeRef::Named("ElementId".to_string()),
        "ElementId should be kept as Named reference"
    );
    assert_eq!(widget.fields[1].name, "label");
    assert_eq!(widget.fields[1].ty, TypeRef::String);
}

#[test]
fn test_newtype_wrapper_with_methods_not_resolved() {
    // Newtypes that have impl methods should NOT be resolved — they're real types.
    let source = r#"
        pub struct Token(String);

        impl Token {
            pub fn value(&self) -> &str {
                &self.0
            }
        }
    "#;

    let surface = extract_from_source(source);

    // Token has methods, so it should remain in the surface (not resolved away)
    assert!(
        surface.types.iter().any(|t| t.name == "Token"),
        "Newtype with methods should be kept"
    );
}

#[test]
fn test_newtype_in_optional_and_vec_resolved() {
    let source = r#"
        pub struct Id(u64);

        pub struct Container {
            pub primary: Option<Id>,
            pub all_ids: Vec<Id>,
        }
    "#;

    let surface = extract_from_source(source);

    // Newtype Id should be kept in the IR (not resolved away)
    assert!(
        surface.types.iter().any(|t| t.name == "Id"),
        "Newtype Id should be kept in types"
    );

    let container = surface
        .types
        .iter()
        .find(|t| t.name == "Container")
        .expect("Container should exist");
    // primary: Option<Id> → Optional(Named("Id"))
    assert_eq!(container.fields[0].name, "primary");
    assert!(container.fields[0].optional);
    assert_eq!(
        container.fields[0].ty,
        TypeRef::Named("Id".to_string()),
        "Id should be kept as Named reference"
    );

    // all_ids: Vec<Id> → Vec(Named("Id"))
    assert_eq!(container.fields[1].name, "all_ids");
    assert_eq!(
        container.fields[1].ty,
        TypeRef::Vec(Box::new(TypeRef::Named("Id".to_string())))
    );
}

#[test]
fn test_tuple_struct_wrapping_named_type_not_resolved() {
    // A tuple struct wrapping a complex Named type (like a builder pattern)
    // should NOT be resolved as a transparent newtype.
    let source = r#"
        pub struct RenderOptions {
            pub format: String,
        }

        pub struct RenderOptionsBuilder(RenderOptions);

        impl RenderOptionsBuilder {
            pub fn format(&mut self, fmt: String) -> &mut Self {
                self.0.format = fmt;
                self
            }
        }
    "#;

    let surface = extract_from_source(source);

    // RenderOptionsBuilder wraps a Named type AND has methods — should be kept
    assert!(
        surface.types.iter().any(|t| t.name == "RenderOptionsBuilder"),
        "Tuple struct wrapping Named type should not be resolved away"
    );
}

#[test]
fn test_tuple_struct_wrapping_named_type_no_methods_not_resolved() {
    // Even without methods, a tuple struct wrapping a complex Named type
    // should NOT be resolved as a transparent newtype.
    let source = r#"
        pub struct Inner {
            pub value: u32,
        }

        pub struct Wrapper(Inner);

        pub struct Consumer {
            pub item: Wrapper,
        }
    "#;

    let surface = extract_from_source(source);

    // Wrapper wraps a Named type — should be kept even without methods
    assert!(
        surface.types.iter().any(|t| t.name == "Wrapper"),
        "Tuple struct wrapping Named type should not be resolved even without methods"
    );

    // Consumer should reference Wrapper as Named, not have it inlined
    let consumer = surface
        .types
        .iter()
        .find(|t| t.name == "Consumer")
        .expect("Consumer should exist");
    assert_eq!(
        consumer.fields[0].ty,
        TypeRef::Named("Wrapper".to_string()),
        "Wrapper reference should remain as Named"
    );
}

#[test]
fn wrapper_struct_alongside_per_element_struct_is_extracted() {
    // fixture with neutral batch/result names while preserving the coverage.
    // Regression for sample_crawler's BatchScrapeResults: a wrapper struct
    // declared in the same module as the per-element struct and the
    // function returning it must appear in surface.types so codegen
    // resolves the function's return type to Named, not String.
    let source = r#"
        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        pub struct BatchScrapeResult {
            pub url: String,
            pub error: Option<String>,
        }

        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        pub struct BatchScrapeResults {
            pub results: Vec<BatchScrapeResult>,
            pub total_count: usize,
            pub completed_count: usize,
            pub failed_count: usize,
        }

        impl From<Vec<BatchScrapeResult>> for BatchScrapeResults {
            fn from(results: Vec<BatchScrapeResult>) -> Self {
                let total_count = results.len();
                Self { results, total_count, completed_count: 0, failed_count: 0 }
            }
        }

        pub async fn batch_scrape(urls: Vec<String>) -> Result<BatchScrapeResults, MyError> {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"BatchScrapeResult"),
        "per-element struct must be extracted; got: {names:?}"
    );
    assert!(
        names.contains(&"BatchScrapeResults"),
        "wrapper struct must be extracted; got: {names:?}"
    );

    let wrapper = surface
        .types
        .iter()
        .find(|t| t.name == "BatchScrapeResults")
        .expect("wrapper present");
    let field_names: Vec<&str> = wrapper.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        field_names,
        vec!["results", "total_count", "completed_count", "failed_count"]
    );
    assert!(!wrapper.is_opaque, "wrapper struct must not be opaque");

    let func = surface
        .functions
        .iter()
        .find(|f| f.name == "batch_scrape")
        .expect("batch_scrape extracted");
    assert_eq!(
        func.return_type,
        TypeRef::Named("BatchScrapeResults".into()),
        "return_type must resolve to Named, not String"
    );
}
