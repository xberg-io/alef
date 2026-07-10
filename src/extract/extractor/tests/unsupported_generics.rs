use super::*;

#[test]
fn test_generic_type_alias_not_extracted_as_typedef() {
    // Generic type aliases (e.g. BoxFuture<'a, T>) are not extracted as TypeDefs
    // (they're used only to detect result-wrapping patterns for async detection).
    let source = r#"
        pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
    "#;

    let surface = extract_from_source(source);
    // Generic aliases are not added to types
    assert_eq!(surface.types.len(), 0);
}

#[test]
fn test_extract_pub_trait() {
    // A public trait should be extracted as a TypeDef with is_trait=true.
    let source = r#"
        /// A backend for processing.
        pub trait Processor {
            /// Process a string.
            fn process(&self, input: String) -> String;

            /// Async reset.
            async fn reset(&mut self);
        }
    "#;

    let surface = extract_from_source(source);
    // Trait appears in types with is_trait=true
    let trait_def = surface
        .types
        .iter()
        .find(|t| t.name == "Processor")
        .expect("Processor trait not found");
    assert!(trait_def.is_trait);
    assert_eq!(trait_def.rust_path, "test_crate::Processor");
    assert_eq!(trait_def.doc, "A backend for processing.");
    assert_eq!(trait_def.methods.len(), 2);

    let process_method = trait_def.methods.iter().find(|m| m.name == "process").unwrap();
    assert!(!process_method.is_async);
    assert_eq!(process_method.return_type, TypeRef::String);

    let reset_method = trait_def.methods.iter().find(|m| m.name == "reset").unwrap();
    assert!(reset_method.is_async);
}

#[test]
fn test_generic_trait_not_extracted() {
    // Traits with generic parameters are not extracted, but they must remain
    // visible to validation as unsupported public API.
    let source = r#"
        pub trait Converter<T> {
            fn convert(&self, input: T) -> T;
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 0, "Generic trait should not be extracted");
    assert_eq!(surface.unsupported_public_items.len(), 1);
    let item = &surface.unsupported_public_items[0];
    assert_eq!(item.item_kind, "trait");
    assert!(item.item_path.ends_with("Converter"));
    assert!(item.reason.contains("generic traits"));
}

#[test]
fn test_is_return_type_marked_on_types() {
    // Types that appear as the return type of free functions should have is_return_type=true.
    // This post-processing only runs in the top-level extract() call, not extract_items().
    let tmp = std::env::temp_dir().join("alef_test_is_return_type");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub struct Response {
    pub status: u32,
}

pub struct Request {
    pub url: String,
}

pub fn send(req: Request) -> Response {
    unimplemented!()
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    let response = surface
        .types
        .iter()
        .find(|t| t.name == "Response")
        .expect("Response not found");
    assert!(response.is_return_type, "Response should be marked is_return_type=true");

    let request = surface
        .types
        .iter()
        .find(|t| t.name == "Request")
        .expect("Request not found");
    assert!(!request.is_return_type, "Request should not be marked is_return_type");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_extract_cfg_gated_generic_async_fn_is_recorded_as_unsupported() {
    let source = r#"
        #[cfg(all(feature = "tokio-runtime", feature = "embeddings"))]
        pub async fn embed_texts_async<T: AsRef<str> + Send + 'static>(
            texts: Vec<T>,
            config: &EmbeddingConfig,
        ) -> Result<Vec<Vec<f32>>, SampleCrateError> {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(
        surface.functions.len(),
        0,
        "generic free functions must not be silently monomorphized to String"
    );
    assert_eq!(surface.unsupported_public_items.len(), 1);
    let item = &surface.unsupported_public_items[0];
    assert_eq!(item.item_kind, "function");
    assert!(item.item_path.ends_with("embed_texts_async"));
    assert!(
        item.reason.contains("generic functions"),
        "unsupported reason should explain the generic shape: {:?}",
        item.reason
    );
}

#[test]
fn test_generic_trait_method_is_recorded_as_unsupported() {
    let source = r#"
        pub trait Renderer {
            fn render<T: AsRef<str>>(&self, input: T) -> String;
        }
    "#;

    let surface = extract_from_source(source);
    let renderer = surface
        .types
        .iter()
        .find(|typ| typ.name == "Renderer")
        .expect("non-generic trait should be extracted");
    assert!(
        renderer.methods.is_empty(),
        "generic trait method should not be extracted"
    );
    assert_eq!(surface.unsupported_public_items.len(), 1);
    let item = &surface.unsupported_public_items[0];
    assert_eq!(item.item_kind, "method");
    assert!(item.item_path.ends_with("Renderer.render"));
    assert!(item.reason.contains("generic trait methods"));
}

#[test]
fn test_public_method_on_generic_inherent_impl_is_recorded_as_unsupported() {
    let source = r#"
        pub struct Parser {
            pub name: String,
        }

        impl<T> Parser {
            pub fn parse(&self, input: T) -> String {
                unimplemented!()
            }

            fn private_helper(&self, input: T) -> String {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let parser = surface
        .types
        .iter()
        .find(|typ| typ.name == "Parser")
        .expect("Parser should still be extracted");
    assert!(
        parser.methods.iter().all(|method| method.name != "parse"),
        "method from generic impl block must not be extracted as concrete IR"
    );
    assert_eq!(surface.unsupported_public_items.len(), 1);
    let item = &surface.unsupported_public_items[0];
    assert_eq!(item.item_kind, "method");
    assert!(item.item_path.ends_with("Parser.parse"));
    assert!(
        item.reason.contains("generic impl blocks"),
        "unsupported reason should mention generic impl blocks: {:?}",
        item.reason
    );
}

#[test]
fn test_trait_impl_method_on_generic_impl_is_recorded_as_unsupported() {
    let source = r#"
        pub trait Renderer {
            fn render(&self) -> String;
        }

        pub struct JsonRenderer {
            pub name: String,
        }

        impl<T> Renderer for JsonRenderer {
            fn render(&self) -> String {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let renderer = surface
        .types
        .iter()
        .find(|typ| typ.name == "JsonRenderer")
        .expect("JsonRenderer should be extracted");
    assert!(
        renderer.methods.iter().all(|method| method.name != "render"),
        "trait method from generic impl block must not be extracted as concrete IR"
    );
    assert_eq!(surface.unsupported_public_items.len(), 1);
    let item = &surface.unsupported_public_items[0];
    assert_eq!(item.item_kind, "method");
    assert!(item.item_path.ends_with("JsonRenderer.render"));
    assert!(
        item.reason.contains("generic impl blocks"),
        "unsupported reason should mention generic impl blocks: {:?}",
        item.reason
    );
}

#[test]
fn test_asref_str_slice_param_is_unsupported_without_monomorphization_config() {
    let source = r#"
        pub fn download<S: AsRef<str>>(names: &[S]) -> Result<usize, Error> {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 0, "generic function must not be extracted");
    assert_eq!(surface.unsupported_public_items.len(), 1);
    let item = &surface.unsupported_public_items[0];
    assert_eq!(item.item_kind, "function");
    assert!(item.item_path.ends_with("download"));
    assert!(item.reason.contains("public generic functions"));
}

#[test]
fn test_asref_str_where_clause_is_unsupported_without_monomorphization_config() {
    let source = r#"
        pub fn load<S>(names: &[S]) -> usize
        where
            S: AsRef<str>,
        {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 0);
    assert_eq!(surface.unsupported_public_items.len(), 1);
    assert!(surface.unsupported_public_items[0].item_path.ends_with("load"));
}

#[test]
fn test_multi_generic_still_unsupported() {
    // Two generic params → conservative rejection, not monomorphized.
    let source = r#"
        pub fn combine<A: AsRef<str>, B: AsRef<str>>(a: &[A], b: &[B]) -> usize {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 0, "multi-generic must not be extracted");
    assert_eq!(
        surface.unsupported_public_items.len(),
        1,
        "must be recorded as unsupported"
    );
}

#[test]
fn test_asref_with_extra_bound_still_unsupported() {
    // Extra `Clone` bound beyond `AsRef<str>` → conservative rejection.
    let source = r#"
        pub fn echo<S: AsRef<str> + Clone>(name: S) -> String {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 0);
    assert_eq!(surface.unsupported_public_items.len(), 1);
}
