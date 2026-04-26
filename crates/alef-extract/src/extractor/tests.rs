use super::reexports::{UseFilter, collect_use_names, find_crate_source, merge_surface, merge_surface_filtered};
use super::*;
use alef_core::ir::{PrimitiveType, ReceiverKind, TypeRef};

/// Helper: parse source and extract into an ApiSurface.
fn extract_from_source(source: &str) -> ApiSurface {
    let file = syn::parse_str::<syn::File>(source).expect("failed to parse test source");
    let mut surface = ApiSurface {
        crate_name: "test_crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let mut visited = Vec::new();
    let mut rwa = ahash::AHashSet::new();
    extract_items(
        &file.items,
        Path::new("test.rs"),
        "test_crate",
        "",
        &mut surface,
        None,
        &mut visited,
        &mut rwa,
    )
    .unwrap();
    resolve_newtypes(&mut surface);
    surface
}

#[test]
fn test_extract_simple_struct() {
    let source = r#"
        /// A configuration struct.
        #[derive(Clone, Debug)]
        pub struct Config {
            /// The name field.
            pub name: String,
            /// Optional timeout in seconds.
            pub timeout: Option<u64>,
            // Private field, should be excluded
            secret: String,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let config = &surface.types[0];
    assert_eq!(config.name, "Config");
    assert_eq!(config.rust_path, "test_crate::Config");
    assert!(config.is_clone);
    assert!(!config.is_opaque);
    assert_eq!(config.doc, "A configuration struct.");

    assert_eq!(config.fields.len(), 2);

    let name_field = &config.fields[0];
    assert_eq!(name_field.name, "name");
    assert_eq!(name_field.ty, TypeRef::String);
    assert!(!name_field.optional);
    assert_eq!(name_field.doc, "The name field.");

    let timeout_field = &config.fields[1];
    assert_eq!(timeout_field.name, "timeout");
    assert_eq!(timeout_field.ty, TypeRef::Primitive(PrimitiveType::U64));
    assert!(timeout_field.optional);
    assert_eq!(timeout_field.doc, "Optional timeout in seconds.");
}

#[test]
fn test_extract_enum() {
    let source = r#"
        /// Output format.
        pub enum Format {
            /// Plain text.
            Text,
            /// JSON output.
            Json,
            /// Custom with config.
            Custom { name: String },
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);

    let fmt = &surface.enums[0];
    assert_eq!(fmt.name, "Format");
    assert_eq!(fmt.variants.len(), 3);
    assert_eq!(fmt.variants[0].name, "Text");
    assert!(fmt.variants[0].fields.is_empty());
    assert_eq!(fmt.variants[2].name, "Custom");
    assert_eq!(fmt.variants[2].fields.len(), 1);
    assert_eq!(fmt.variants[2].fields[0].name, "name");
}

#[test]
fn test_extract_free_function() {
    let source = r#"
        /// Process the input.
        pub async fn process(input: String, count: u32) -> Result<Vec<String>, MyError> {
            todo!()
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);

    let func = &surface.functions[0];
    assert_eq!(func.name, "process");
    assert!(func.is_async);
    assert_eq!(func.error_type.as_deref(), Some("MyError"));
    assert_eq!(func.return_type, TypeRef::Vec(Box::new(TypeRef::String)));
    assert_eq!(func.params.len(), 2);
    assert_eq!(func.params[0].name, "input");
    assert_eq!(func.params[0].ty, TypeRef::String);
    assert_eq!(func.params[1].name, "count");
    assert_eq!(func.params[1].ty, TypeRef::Primitive(PrimitiveType::U32));
}

#[test]
fn test_extract_impl_block() {
    let source = r#"
        pub struct Server {
            pub host: String,
        }

        impl Server {
            /// Create a new server.
            pub fn new(host: String) -> Self {
                todo!()
            }

            /// Start listening.
            pub async fn listen(&self, port: u16) -> Result<(), std::io::Error> {
                todo!()
            }

            /// Shutdown mutably.
            pub fn shutdown(&mut self) {
                todo!()
            }

            // Private, should be excluded
            fn internal(&self) {}
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let server = &surface.types[0];
    assert_eq!(server.name, "Server");
    // `new` returning Self is skipped (constructor generated from fields)
    assert_eq!(server.methods.len(), 2);

    let listen_method = &server.methods[0];
    assert_eq!(listen_method.name, "listen");
    assert!(listen_method.is_async);
    assert!(!listen_method.is_static);
    assert_eq!(listen_method.receiver, Some(ReceiverKind::Ref));
    assert_eq!(listen_method.error_type.as_deref(), Some("std::io::Error"));
    assert_eq!(listen_method.return_type, TypeRef::Unit);

    let shutdown_method = &server.methods[1];
    assert_eq!(shutdown_method.name, "shutdown");
    assert_eq!(shutdown_method.receiver, Some(ReceiverKind::RefMut));
}

#[test]
fn test_private_items_excluded() {
    let source = r#"
        struct PrivateStruct {
            pub field: u32,
        }

        pub(crate) struct CrateStruct {
            pub field: u32,
        }

        fn private_fn() {}

        pub fn public_fn() {}
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 0);
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].name, "public_fn");
}

#[test]
fn test_opaque_struct() {
    let source = r#"
        pub struct Handle {
            inner: u64,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(surface.types[0].is_opaque);
    assert!(surface.types[0].fields.is_empty());
}

#[test]
fn test_inline_module() {
    let source = r#"
        pub mod inner {
            pub fn helper() -> bool {
                true
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].name, "helper");
}

#[test]
fn test_enum_with_tuple_variants() {
    let source = r#"
        pub enum Value {
            Int(i64),
            Pair(String, u32),
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);
    let val = &surface.enums[0];
    assert_eq!(val.variants[0].fields.len(), 1);
    assert_eq!(val.variants[0].fields[0].name, "_0");
    assert_eq!(val.variants[1].fields.len(), 2);
}

#[test]
fn test_method_with_owned_self() {
    let source = r#"
        pub struct Builder {}

        impl Builder {
            pub fn build(self) -> String {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let builder = &surface.types[0];
    assert_eq!(builder.methods.len(), 1);
    assert_eq!(builder.methods[0].receiver, Some(ReceiverKind::Owned));
    assert!(!builder.methods[0].is_static);
}

#[test]
fn test_trait_impl_methods_extracted() {
    let source = r#"
        pub struct DefaultClient {
            pub base_url: String,
        }

        impl DefaultClient {
            pub fn new(base_url: String) -> DefaultClient {
                todo!()
            }
        }

        trait LlmClient {
            async fn chat(&self, prompt: String) -> Result<String, MyError>;
            fn model(&self) -> String;
        }

        impl LlmClient for DefaultClient {
            async fn chat(&self, prompt: String) -> Result<String, MyError> {
                todo!()
            }

            fn model(&self) -> String {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let client = &surface.types[0];
    assert_eq!(client.name, "DefaultClient");
    // Should have: new (not skipped because it doesn't return Self), chat, model
    // Actually new returns DefaultClient not Self, so it's included
    assert_eq!(client.methods.len(), 3);

    let method_names: Vec<&str> = client.methods.iter().map(|m| m.name.as_str()).collect();
    assert!(method_names.contains(&"new"));
    assert!(method_names.contains(&"chat"));
    assert!(method_names.contains(&"model"));

    // Verify chat is async
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
                todo!()
            }
        }

        trait SomeTrait {
            fn do_thing(&self) -> String;
        }

        impl SomeTrait for MyType {
            fn do_thing(&self) -> String {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let my_type = &surface.types[0];
    // Should not have duplicate do_thing
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
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    // UnknownType is not in the surface, so trait impl methods should be ignored
    assert_eq!(surface.types.len(), 0);
}

#[test]
fn test_pub_use_self_super_skipped() {
    let source = r#"
        pub use self::inner::Helper;
        pub use super::other::Thing;
        pub use crate::root::Item;

        pub mod inner {
            pub struct Helper {
                pub value: u32,
            }
        }
    "#;

    let surface = extract_from_source(source);
    // self/super/crate use paths are skipped (handled by mod resolution)
    // The inline module should still be extracted
    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "Helper");
}

#[test]
fn test_collect_use_names_single() {
    let tree: syn::UseTree = syn::parse_str("Foo").unwrap();
    match collect_use_names(&tree) {
        UseFilter::Names(names) => assert_eq!(names, vec!["Foo"]),
        UseFilter::All => panic!("expected Names"),
    }
}

#[test]
fn test_collect_use_names_group() {
    let tree: syn::UseTree = syn::parse_str("{Foo, Bar, Baz}").unwrap();
    match collect_use_names(&tree) {
        UseFilter::Names(names) => {
            assert_eq!(names.len(), 3);
            assert!(names.contains(&"Foo".to_string()));
            assert!(names.contains(&"Bar".to_string()));
            assert!(names.contains(&"Baz".to_string()));
        }
        UseFilter::All => panic!("expected Names"),
    }
}

#[test]
fn test_collect_use_names_glob() {
    let tree: syn::UseTree = syn::parse_str("*").unwrap();
    assert!(matches!(collect_use_names(&tree), UseFilter::All));
}

#[test]
fn test_merge_surface_no_duplicates() {
    let mut dst = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Existing".into(),
            rust_path: "test::Existing".into(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let src = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Existing".into(),
                rust_path: "test::Existing".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            },
            TypeDef {
                name: "NewType".into(),
                rust_path: "test::NewType".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    merge_surface(&mut dst, src, None);
    assert_eq!(dst.types.len(), 2);
    assert_eq!(dst.types[0].name, "Existing");
    assert_eq!(dst.types[1].name, "NewType");
}

#[test]
fn test_merge_surface_filtered() {
    let mut dst = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let src = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Wanted".into(),
                rust_path: "test::Wanted".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            },
            TypeDef {
                name: "NotWanted".into(),
                rust_path: "test::NotWanted".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    merge_surface_filtered(&mut dst, src, &["Wanted".to_string()], None);
    assert_eq!(dst.types.len(), 1);
    assert_eq!(dst.types[0].name, "Wanted");
}

#[test]
fn test_find_crate_source_no_workspace() {
    // With no workspace root, should return None
    assert!(find_crate_source("some_crate", None).is_none());
}

#[test]
fn test_pub_use_reexport_from_workspace_crate() {
    // Create a temporary workspace structure
    let tmp = std::env::temp_dir().join("alef_test_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/other_crate/src")).unwrap();

    // Write workspace Cargo.toml
    std::fs::write(
        tmp.join("Cargo.toml"),
        r#"
[workspace]
members = ["crates/other_crate"]

[workspace.dependencies]
other_crate = { path = "crates/other_crate" }
"#,
    )
    .unwrap();

    // Write other_crate's lib.rs with a pub struct
    std::fs::write(
        tmp.join("crates/other_crate/src/lib.rs"),
        r#"
/// Server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

/// CORS settings.
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
}

/// Internal helper, not re-exported.
pub struct InternalHelper {
    pub data: String,
}
"#,
    )
    .unwrap();

    // Write our crate's lib.rs that re-exports specific items
    let our_lib = tmp.join("crates/my_crate/src/lib.rs");
    std::fs::create_dir_all(our_lib.parent().unwrap()).unwrap();
    std::fs::write(
        &our_lib,
        r#"
pub use other_crate::{ServerConfig, CorsConfig};
"#,
    )
    .unwrap();

    let sources: Vec<&Path> = vec![our_lib.as_path()];
    let surface = extract(&sources, "my_crate", "0.1.0", Some(&tmp)).unwrap();

    // Should have extracted ServerConfig and CorsConfig but not InternalHelper
    assert_eq!(surface.types.len(), 2);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"ServerConfig"));
    assert!(names.contains(&"CorsConfig"));
    assert!(!names.contains(&"InternalHelper"));

    // Verify they use our crate name in rust_path
    let server = surface.types.iter().find(|t| t.name == "ServerConfig").unwrap();
    assert_eq!(server.rust_path, "my_crate::ServerConfig");
    assert!(server.is_clone);

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_pub_use_glob_reexport() {
    let tmp = std::env::temp_dir().join("alef_test_glob_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/other_crate/src")).unwrap();

    std::fs::write(
        tmp.join("Cargo.toml"),
        r#"
[workspace]
members = ["crates/other_crate"]

[workspace.dependencies]
other_crate = { path = "crates/other_crate" }
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("crates/other_crate/src/lib.rs"),
        r#"
pub struct Alpha { pub value: u32 }
pub struct Beta { pub name: String }
"#,
    )
    .unwrap();

    let our_lib = tmp.join("crates/my_crate/src/lib.rs");
    std::fs::create_dir_all(our_lib.parent().unwrap()).unwrap();
    std::fs::write(&our_lib, "pub use other_crate::*;\n").unwrap();

    let sources: Vec<&Path> = vec![our_lib.as_path()];
    let surface = extract(&sources, "my_crate", "0.1.0", Some(&tmp)).unwrap();

    assert_eq!(surface.types.len(), 2);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Alpha"));
    assert!(names.contains(&"Beta"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_returns_ref_detection() {
    let source = r#"
        pub struct MyType {
            inner: String,
        }

        impl MyType {
            pub fn name(&self) -> &str {
                &self.inner
            }

            pub fn owned_name(&self) -> String {
                self.inner.clone()
            }

            pub fn opt_name(&self) -> Option<&str> {
                Some(&self.inner)
            }

            pub fn opt_owned(&self) -> Option<String> {
                Some(self.inner.clone())
            }

            pub fn result_ref(&self) -> Result<&str, String> {
                Ok(&self.inner)
            }

            pub fn result_owned(&self) -> Result<String, String> {
                Ok(self.inner.clone())
            }
        }
    "#;

    let surface = extract_from_source(source);
    let my_type = &surface.types[0];

    let find_method = |name: &str| my_type.methods.iter().find(|m| m.name == name).unwrap();

    // &str return → returns_ref = true
    assert!(find_method("name").returns_ref, "name() should have returns_ref=true");
    // String return → returns_ref = false
    assert!(
        !find_method("owned_name").returns_ref,
        "owned_name() should have returns_ref=false"
    );
    // Option<&str> → returns_ref = true
    assert!(
        find_method("opt_name").returns_ref,
        "opt_name() should have returns_ref=true"
    );
    // Option<String> → returns_ref = false
    assert!(
        !find_method("opt_owned").returns_ref,
        "opt_owned() should have returns_ref=false"
    );
    // Result<&str, _> → returns_ref = true (after Result unwrapping)
    assert!(
        find_method("result_ref").returns_ref,
        "result_ref() should have returns_ref=true"
    );
    // Result<String, _> → returns_ref = false
    assert!(
        !find_method("result_owned").returns_ref,
        "result_owned() should have returns_ref=false"
    );
}

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
        pub struct ConversionOptions {
            pub format: String,
        }

        pub struct ConversionOptionsBuilder(ConversionOptions);

        impl ConversionOptionsBuilder {
            pub fn format(&mut self, fmt: String) -> &mut Self {
                self.0.format = fmt;
                self
            }
        }
    "#;

    let surface = extract_from_source(source);

    // ConversionOptionsBuilder wraps a Named type AND has methods — should be kept
    assert!(
        surface.types.iter().any(|t| t.name == "ConversionOptionsBuilder"),
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

    // Should be in errors, NOT in enums
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

    // Timeout variant: struct, no source/from
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

    // MissingDependency: tuple variant, no source/from
    let missing = &err.variants[3];
    assert_eq!(missing.name, "MissingDependency");
    assert_eq!(missing.message_template.as_deref(), Some("Missing dependency: {0}"));
    assert!(!missing.has_from);
    assert!(!missing.has_source);
    assert!(!missing.is_unit);
    assert_eq!(missing.fields.len(), 1);

    // Unknown: unit variant
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
    // When Error is imported via `use thiserror::Error`, the derive is just `Error`
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
        Some(alef_core::ir::DefaultValue::BoolLiteral(true)),
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
        Some(alef_core::ir::DefaultValue::IntLiteral(3)),
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
        Some(alef_core::ir::DefaultValue::IntLiteral(-273)),
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
        Some(alef_core::ir::DefaultValue::FloatLiteral(0.95)),
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
        Some(alef_core::ir::DefaultValue::FloatLiteral(-1.5)),
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
        Some(alef_core::ir::DefaultValue::StringLiteral("hello world".to_string())),
        "string field with .into() should have StringLiteral default"
    );
}

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
        Some(alef_core::ir::DefaultValue::StringLiteral("default".to_string())),
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
        Some(alef_core::ir::DefaultValue::StringLiteral(String::new())),
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
        Some(alef_core::ir::DefaultValue::StringLiteral("placeholder".to_string())),
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
        Some(alef_core::ir::DefaultValue::StringLiteral(",".to_string())),
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
        Some(alef_core::ir::DefaultValue::Empty),
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
    // Filter for Task type (Status is also extracted as an enum)
    let task = surface.types.iter().find(|t| t.name == "Task").unwrap();
    let status_field = &task.fields[0];

    assert_eq!(
        status_field.typed_default,
        Some(alef_core::ir::DefaultValue::EnumVariant("Pending".to_string())),
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

    // Check name field
    let name_field = &config.fields[0];
    assert_eq!(name_field.name, "name");
    assert_eq!(
        name_field.typed_default,
        Some(alef_core::ir::DefaultValue::StringLiteral("default".to_string()))
    );

    // Check count field
    let count_field = &config.fields[1];
    assert_eq!(count_field.name, "count");
    assert_eq!(
        count_field.typed_default,
        Some(alef_core::ir::DefaultValue::IntLiteral(42))
    );

    // Check enabled field
    let enabled_field = &config.fields[2];
    assert_eq!(enabled_field.name, "enabled");
    assert_eq!(
        enabled_field.typed_default,
        Some(alef_core::ir::DefaultValue::BoolLiteral(false))
    );

    // Check threshold field
    let threshold_field = &config.fields[3];
    assert_eq!(threshold_field.name, "threshold");
    assert_eq!(
        threshold_field.typed_default,
        Some(alef_core::ir::DefaultValue::FloatLiteral(0.5))
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

    // When fn default() is not found, fields should get Empty
    assert_eq!(
        value_field.typed_default,
        Some(alef_core::ir::DefaultValue::Empty),
        "Fields should have Empty when fn default() is missing"
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
        Some(alef_core::ir::DefaultValue::Empty),
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
        Some(alef_core::ir::DefaultValue::Empty),
        "Default::default() should extract as Empty"
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

    // field_a is in the struct literal
    let field_a = &partial.fields[0];
    assert_eq!(field_a.typed_default, Some(alef_core::ir::DefaultValue::IntLiteral(99)));

    // field_b is NOT in the struct literal → should be Empty
    let field_b = &partial.fields[1];
    assert_eq!(
        field_b.typed_default,
        Some(alef_core::ir::DefaultValue::Empty),
        "Field not in struct literal should have Empty default"
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

    // The Normal variant should be marked as default
    let normal = &priority.variants[0];
    assert_eq!(normal.name, "Normal");
    assert!(
        normal.is_default,
        "Normal variant with #[default] should have is_default=true"
    );

    // Other variants should not be marked as default
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

    // No variants should be marked as default
    for variant in &format.variants {
        assert!(
            !variant.is_default,
            "Variants without #[default] should be is_default=false"
        );
    }
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
        Some(alef_core::ir::DefaultValue::IntLiteral(77)),
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
        Some(alef_core::ir::DefaultValue::BoolLiteral(false)),
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
        Some(alef_core::ir::DefaultValue::IntLiteral(0))
    );
    assert_eq!(
        zeroes.fields[1].typed_default,
        Some(alef_core::ir::DefaultValue::FloatLiteral(0.0))
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
        Some(alef_core::ir::DefaultValue::Empty),
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
        Some(alef_core::ir::DefaultValue::Empty),
        "Complex expressions like function calls should default to Empty"
    );
}

#[test]
fn test_boxfuture_wrapping_result_is_async_with_error_type() {
    // When a method returns BoxFuture<'_, Result<T, E>>, the extractor should:
    // 1. Mark it as async
    // 2. Set the return type to T (not Result<T, E>)
    // 3. Set error_type to the error string
    let source = r#"
        use std::future::Future;
        use std::pin::Pin;

        pub struct ChatResponse {
            pub content: String,
        }

        pub struct LlmClient;

        impl LlmClient {
            pub fn complete(&self, prompt: String) -> Pin<Box<dyn Future<Output = Result<ChatResponse, anyhow::Error>> + Send + '_>> {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let client = surface
        .types
        .iter()
        .find(|t| t.name == "LlmClient")
        .expect("LlmClient not found");
    let method = client
        .methods
        .iter()
        .find(|m| m.name == "complete")
        .expect("complete method not found");

    assert!(method.is_async, "BoxFuture-returning method should be marked async");
    assert_eq!(
        method.return_type,
        TypeRef::Named("ChatResponse".into()),
        "return_type should be the inner T, not Result<T, E>"
    );
    assert!(
        method.error_type.is_some(),
        "error_type should be Some when BoxFuture wraps Result"
    );
}

#[test]
fn test_boxfuture_alias_wrapping_result_is_async_with_error_type() {
    // BoxFuture<'_, Result<T, E>> via the futures crate alias pattern
    let source = r#"
        pub struct ChatResponse {
            pub content: String,
        }

        pub struct DefaultClient;

        // Simulate a BoxFuture<'_, Result<ChatResponse, MyError>> return
        impl DefaultClient {
            pub fn chat(&self, prompt: String) -> futures::future::BoxFuture<'_, Result<ChatResponse, MyError>> {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let client = surface
        .types
        .iter()
        .find(|t| t.name == "DefaultClient")
        .expect("DefaultClient not found");
    let method = client
        .methods
        .iter()
        .find(|m| m.name == "chat")
        .expect("chat method not found");

    assert!(method.is_async, "BoxFuture-returning method should be marked async");
    assert_eq!(
        method.return_type,
        TypeRef::Named("ChatResponse".into()),
        "return_type should be the inner T, not Result<T, E>"
    );
    assert_eq!(
        method.error_type.as_deref(),
        Some("MyError"),
        "error_type should be the error string from Result<T, E>"
    );
}

#[test]
fn test_boxfuture_non_result_has_no_error_type() {
    // BoxFuture<'_, T> where T is not a Result should have no error_type
    let source = r#"
        pub struct Payload {
            pub data: String,
        }

        pub struct StreamClient;

        impl StreamClient {
            pub fn fetch(&self) -> futures::future::BoxFuture<'_, Payload> {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let client = surface
        .types
        .iter()
        .find(|t| t.name == "StreamClient")
        .expect("StreamClient not found");
    let method = client
        .methods
        .iter()
        .find(|m| m.name == "fetch")
        .expect("fetch method not found");

    assert!(method.is_async, "BoxFuture-returning method should be marked async");
    assert_eq!(
        method.return_type,
        TypeRef::Named("Payload".into()),
        "return_type should be T directly"
    );
    assert!(
        method.error_type.is_none(),
        "error_type should be None when BoxFuture does not wrap Result"
    );
}

// ---------------------------------------------------------------------------
// mod.rs coverage: derive_module_path, apply_parent_reexport_shortening,
// type aliases, trait extraction, is_return_type marking, Map TypeRef resolution
// ---------------------------------------------------------------------------

#[test]
fn test_extract_pub_type_alias() {
    // Non-generic type alias: `pub type Foo = Bar;`
    // Should be extracted as an opaque TypeDef with no fields.
    let source = r#"
        /// A result alias.
        pub type Result = std::result::Result<String, String>;
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    let alias = &surface.types[0];
    assert_eq!(alias.name, "Result");
    assert!(alias.is_opaque);
    assert!(alias.fields.is_empty());
    assert_eq!(alias.doc, "A result alias.");
}

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
fn test_pub_trait_with_supertrait() {
    let source = r#"
        pub trait Backend: Send + Sync {}

        pub trait OcrBackend: Backend {}
    "#;

    let surface = extract_from_source(source);
    let ocr = surface
        .types
        .iter()
        .find(|t| t.name == "OcrBackend")
        .expect("OcrBackend not found");
    assert!(ocr.is_trait);
    assert_eq!(ocr.super_traits, vec!["Backend"]);

    // Send and Sync are marker traits — filtered out
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
fn test_generic_trait_not_extracted() {
    // Traits with generic parameters are skipped (same as generic type aliases).
    let source = r#"
        pub trait Converter<T> {
            fn convert(&self, input: T) -> T;
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 0, "Generic trait should not be extracted");
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
    todo!()
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
fn test_newtype_wrapper_recorded_on_field() {
    // When a newtype with an explicit `pub` inner field is used in another struct's field,
    // the newtype_wrapper should be recorded on the consuming field before resolution.
    // Note: tuple struct fields need explicit `pub` to be treated as resolvable newtypes.
    let source = r#"
        pub struct UserId(pub u64);

        pub struct User {
            pub id: UserId,
        }
    "#;

    let surface = extract_from_source(source);

    // UserId(pub u64) has a pub _0 field with a simple type → resolved away
    assert!(
        surface.types.iter().all(|t| t.name != "UserId"),
        "Simple newtype with pub inner field should be resolved away"
    );

    let user = surface.types.iter().find(|t| t.name == "User").expect("User not found");
    let id_field = &user.fields[0];
    // After resolution, ty becomes Primitive(U64)
    assert_eq!(id_field.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::U64));
    // newtype_wrapper should hold the original rust_path for codegen
    assert!(
        id_field.newtype_wrapper.is_some(),
        "newtype_wrapper should be set on field whose type was a resolved newtype"
    );
    assert!(
        id_field.newtype_wrapper.as_deref().unwrap().contains("UserId"),
        "newtype_wrapper should reference UserId"
    );
}

#[test]
fn test_newtype_wrapper_recorded_on_method_param() {
    let source = r#"
        pub struct Token(pub String);

        pub struct Auth {
            pub active: bool,
        }

        impl Auth {
            pub fn verify(&self, token: Token) -> bool {
                todo!()
            }
        }
    "#;

    let surface = extract_from_source(source);

    // Token(pub String) has explicit pub inner field — resolved away
    assert!(
        surface.types.iter().all(|t| t.name != "Token"),
        "Token newtype should be resolved away"
    );

    let auth = surface.types.iter().find(|t| t.name == "Auth").expect("Auth not found");
    let verify = auth
        .methods
        .iter()
        .find(|m| m.name == "verify")
        .expect("verify not found");
    let token_param = &verify.params[0];
    // After resolution, type is String
    assert_eq!(token_param.ty, TypeRef::String);
    assert!(
        token_param.newtype_wrapper.is_some(),
        "newtype_wrapper should be set on method param whose type was a resolved newtype"
    );
}

#[test]
fn test_newtype_wrapper_recorded_on_function_return() {
    let source = r#"
        pub struct Handle(pub u32);

        pub fn create_handle() -> Handle {
            todo!()
        }
    "#;

    let surface = extract_from_source(source);

    // Handle(pub u32) has explicit pub inner field — resolved away
    assert!(
        surface.types.iter().all(|t| t.name != "Handle"),
        "Handle newtype should be resolved away"
    );

    let func = surface
        .functions
        .iter()
        .find(|f| f.name == "create_handle")
        .expect("create_handle not found");
    assert_eq!(func.return_type, TypeRef::Primitive(alef_core::ir::PrimitiveType::U32));
    assert!(
        func.return_newtype_wrapper.is_some(),
        "return_newtype_wrapper should be set for resolved newtype return"
    );
}

#[test]
fn test_map_typeref_newtype_resolution() {
    // TypeRef::Map containing Named references should have those resolved.
    // Since HashMap fields are mapped to TypeRef::Map by the extractor,
    // we verify that a newtype used as map values gets resolved.
    // Note: the inner field must be `pub` for the newtype to be resolvable.
    let source = r#"
        pub struct Score(pub i32);

        pub struct Leaderboard {
            pub scores: std::collections::HashMap<String, Score>,
        }
    "#;

    // Score(pub i32) has an explicit pub inner field — resolvable newtype
    let surface = extract_from_source(source);

    // Score is resolved away
    assert!(
        surface.types.iter().all(|t| t.name != "Score"),
        "Score newtype should be resolved away"
    );

    let board = surface
        .types
        .iter()
        .find(|t| t.name == "Leaderboard")
        .expect("Leaderboard not found");
    let scores_field = &board.fields[0];
    // Map<String, i32> after resolution
    if let TypeRef::Map(_, v) = &scores_field.ty {
        assert_eq!(**v, TypeRef::Primitive(alef_core::ir::PrimitiveType::I32));
    } else {
        panic!("Expected TypeRef::Map for HashMap field, got {:?}", scores_field.ty);
    }
}

#[test]
fn test_resolve_trait_sources_retroactive() {
    // When a trait impl appears before the trait definition in the same source,
    // trait_source should still be resolved after the full extraction pass.
    let source = r#"
        pub struct Widget {
            pub label: String,
        }

        impl Renderable for Widget {
            fn render(&self) -> String {
                todo!()
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
    // trait_source should be filled in by resolve_trait_sources
    assert!(
        render.trait_source.is_some(),
        "trait_source should be resolved even when trait is defined after impl"
    );
}

#[test]
fn test_extract_via_top_level_function_with_multiple_sources() {
    // Test the public `extract()` entry point with two source files that would
    // be visited. The second file should be skipped if already processed via mod.
    let tmp = std::env::temp_dir().join("alef_test_extract_multi");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub struct Config {
    pub timeout: u32,
}

pub fn run(config: Config) -> bool {
    true
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "1.0.0", None).unwrap();

    assert_eq!(surface.crate_name, "my_crate");
    assert_eq!(surface.version, "1.0.0");
    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "Config");
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].name, "run");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_derive_module_path_via_extract_with_submodule_files() {
    // Verify that extract() with multiple source files derives correct module paths.
    // When cache/types.rs is given as an explicit source with src/ as root,
    // the items should have rust_path `my_crate::cache::types::Item`.
    let tmp = std::env::temp_dir().join("alef_test_derive_module_path");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(tmp.join("src/cache/mod.rs"), "pub mod types;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/types.rs"),
        r#"
pub struct CacheEntry {
    pub key: String,
    pub value: String,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    let entry = &surface.types[0];
    assert_eq!(entry.name, "CacheEntry");
    assert_eq!(entry.rust_path, "my_crate::cache::types::CacheEntry");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_parent_reexport_shortening_via_extract() {
    // When cache/types.rs defines CacheEntry and cache/mod.rs has `pub use types::CacheEntry;`,
    // the extract() pass should shorten the rust_path to `my_crate::cache::CacheEntry`.
    let tmp = std::env::temp_dir().join("alef_test_parent_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/mod.rs"),
        r#"
pub mod types;
pub use types::CacheEntry;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/cache/types.rs"),
        r#"
pub struct CacheEntry {
    pub key: String,
    pub ttl: u64,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    let entry = &surface.types[0];
    assert_eq!(entry.name, "CacheEntry");
    assert_eq!(
        entry.rust_path, "my_crate::cache::CacheEntry",
        "Named re-export in parent mod.rs should shorten the rust_path"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_parent_reexport_glob_shortening_via_extract() {
    // When cache/mod.rs has `pub use types::*;`, all items from types.rs should
    // be shortened to `my_crate::cache::ItemName`.
    let tmp = std::env::temp_dir().join("alef_test_parent_glob_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/mod.rs"),
        r#"
pub mod types;
pub use types::*;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/cache/types.rs"),
        r#"
pub struct Entry {
    pub key: String,
}

pub struct Store {
    pub size: u32,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 2);
    for ty in &surface.types {
        assert!(
            ty.rust_path.starts_with("my_crate::cache::"),
            "Glob re-export should shorten path to parent level, got: {}",
            ty.rust_path
        );
        assert!(
            !ty.rust_path.contains("types"),
            "types:: should not appear in shortened path, got: {}",
            ty.rust_path
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// reexports.rs coverage: collect_use_names rename, resolve_use_tree group/bare,
// find_crate_source heuristic & dependencies table, extract_module file-based,
// private module pruning with named re-exports
// ---------------------------------------------------------------------------

#[test]
fn test_collect_use_names_rename() {
    // `use Foo as Bar` should return the alias name "Bar"
    let tree: syn::UseTree = syn::parse_str("Foo as Bar").unwrap();
    match super::reexports::collect_use_names(&tree) {
        super::reexports::UseFilter::Names(names) => {
            assert_eq!(names, vec!["Bar"]);
        }
        super::reexports::UseFilter::All => panic!("expected Names"),
    }
}

#[test]
fn test_collect_use_names_nested_path() {
    // `some::module::Type` — the leaf is Type
    let tree: syn::UseTree = syn::parse_str("some::module::Type").unwrap();
    match super::reexports::collect_use_names(&tree) {
        super::reexports::UseFilter::Names(names) => {
            assert_eq!(names, vec!["Type"]);
        }
        super::reexports::UseFilter::All => panic!("expected Names"),
    }
}

#[test]
fn test_collect_use_names_group_with_glob_returns_all() {
    // `{Foo, *}` — a group containing a glob means All
    let tree: syn::UseTree = syn::parse_str("{Foo, *}").unwrap();
    assert!(matches!(
        super::reexports::collect_use_names(&tree),
        super::reexports::UseFilter::All
    ));
}

#[test]
fn test_resolve_use_tree_group_variant() {
    // `pub use self::inner::{Foo};` — group variant of UseTree going through resolve_use_tree
    // Since these are self-references, they should be skipped without error.
    let source = r#"
        pub use self::{inner::Foo};

        pub mod inner {
            pub struct Foo { pub val: u32 }
        }
    "#;

    // Should not panic, and the inline module is still extracted
    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "Foo");
}

#[test]
fn test_find_crate_source_with_dependencies_table() {
    // Create a workspace with a [dependencies] path dep (not workspace.dependencies)
    let tmp = std::env::temp_dir().join("alef_test_find_crate_dep");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/dep_crate/src")).unwrap();

    std::fs::write(
        tmp.join("Cargo.toml"),
        r#"
[dependencies]
dep_crate = { path = "crates/dep_crate" }
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("crates/dep_crate/src/lib.rs"),
        "pub struct DepType { pub x: u32 }\n",
    )
    .unwrap();

    let result = super::reexports::find_crate_source("dep_crate", Some(&tmp));
    assert!(result.is_some(), "Should find crate source via [dependencies] path dep");
    assert!(result.unwrap().ends_with("lib.rs"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_find_crate_source_heuristic_crates_dir() {
    // When the Cargo.toml has no matching dependency entry, the heuristic
    // looks for crates/{name}/src/lib.rs directly.
    let tmp = std::env::temp_dir().join("alef_test_find_crate_heuristic");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/my_lib/src")).unwrap();

    // Cargo.toml with no deps — heuristic will be used
    std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    std::fs::write(tmp.join("crates/my_lib/src/lib.rs"), "pub struct Heuristic;\n").unwrap();

    let result = super::reexports::find_crate_source("my_lib", Some(&tmp));
    assert!(result.is_some(), "Should find via heuristic crates/{{name}}/src/lib.rs");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_find_crate_source_hyphen_underscore_alt() {
    // Crate directory named `my-lib` on disk but referenced as `my_lib`.
    // The heuristic should try the alternative hyphen/underscore name.
    let tmp = std::env::temp_dir().join("alef_test_find_crate_alt");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/my-lib/src")).unwrap();
    // Cargo.toml with no matching deps — so heuristic alt-name path is exercised
    std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    std::fs::write(tmp.join("crates/my-lib/src/lib.rs"), "pub struct AltType;\n").unwrap();

    // Reference with underscores — should find the hyphenated directory via alt name
    let result = super::reexports::find_crate_source("my_lib", Some(&tmp));
    assert!(result.is_some(), "Should find crate via hyphen/underscore alt name");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_extract_module_external_file_based() {
    // Test that an external `pub mod foo;` declaration is followed to foo.rs
    let tmp = std::env::temp_dir().join("alef_test_extract_mod_external");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub mod models;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/models.rs"),
        r#"
pub struct ModelItem {
    pub id: u32,
    pub name: String,
}

pub fn find_model(id: u32) -> ModelItem {
    todo!()
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "ModelItem");
    assert_eq!(surface.types[0].rust_path, "my_crate::models::ModelItem");
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].rust_path, "my_crate::models::find_model");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_extract_module_mod_rs_subdir() {
    // `pub mod cache;` that resolves to `cache/mod.rs`
    let tmp = std::env::temp_dir().join("alef_test_extract_mod_subdir");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/mod.rs"),
        r#"
pub struct CacheClient {
    pub url: String,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "CacheClient");
    assert_eq!(surface.types[0].rust_path, "my_crate::cache::CacheClient");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_private_module_with_named_reexport_prunes_non_reexported() {
    // A private module (`mod foo;`) with `pub use foo::Public` should expose
    // only Public, not the private types.
    let source = r#"
        mod inner;
        pub use inner::Public;

        mod inner {
            pub struct Public { pub value: u32 }
            pub struct Hidden { pub secret: String }
        }
    "#;

    let surface = extract_from_source(source);
    // Only Public should survive, Hidden is pruned
    assert_eq!(surface.types.len(), 1, "Only re-exported items should survive");
    assert_eq!(surface.types[0].name, "Public");
}

#[test]
fn test_private_module_glob_reexport_exposes_all() {
    // `mod inner; pub use inner::*;` — all public items from inner should be exposed
    let source = r#"
        mod inner;
        pub use inner::*;

        mod inner {
            pub struct Alpha { pub x: u32 }
            pub struct Beta { pub y: String }
        }
    "#;

    let surface = extract_from_source(source);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Alpha"), "Alpha should be exposed via glob re-export");
    assert!(names.contains(&"Beta"), "Beta should be exposed via glob re-export");
}

#[test]
fn test_merge_surface_includes_functions_and_enums() {
    // merge_surface should also merge functions and enums, not just types.
    let mut dst = ApiSurface {
        crate_name: "dst".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let src = ApiSurface {
        crate_name: "src".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![alef_core::ir::FunctionDef {
            name: "my_fn".into(),
            rust_path: "src::my_fn".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![alef_core::ir::EnumDef {
            name: "MyEnum".into(),
            rust_path: "src::MyEnum".into(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    super::reexports::merge_surface(&mut dst, src, None);
    assert_eq!(dst.functions.len(), 1);
    assert_eq!(dst.functions[0].name, "my_fn");
    assert_eq!(dst.enums.len(), 1);
    assert_eq!(dst.enums[0].name, "MyEnum");
}

#[test]
fn test_merge_surface_filtered_includes_functions_and_enums() {
    // merge_surface_filtered should also filter and merge functions and enums.
    let mut dst = ApiSurface {
        crate_name: "dst".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let src = ApiSurface {
        crate_name: "src".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![
            alef_core::ir::FunctionDef {
                name: "wanted_fn".into(),
                rust_path: "src::wanted_fn".into(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
            alef_core::ir::FunctionDef {
                name: "unwanted_fn".into(),
                rust_path: "src::unwanted_fn".into(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
        ],
        enums: vec![
            alef_core::ir::EnumDef {
                name: "WantedEnum".into(),
                rust_path: "src::WantedEnum".into(),
                original_rust_path: String::new(),
                variants: vec![],
                doc: String::new(),
                cfg: None,
                serde_tag: None,
                serde_rename_all: None,
            },
            alef_core::ir::EnumDef {
                name: "UnwantedEnum".into(),
                rust_path: "src::UnwantedEnum".into(),
                original_rust_path: String::new(),
                variants: vec![],
                doc: String::new(),
                cfg: None,
                serde_tag: None,
                serde_rename_all: None,
            },
        ],
        errors: vec![],
    };

    let names = vec!["wanted_fn".to_string(), "WantedEnum".to_string()];
    super::reexports::merge_surface_filtered(&mut dst, src, &names, None);
    assert_eq!(dst.functions.len(), 1);
    assert_eq!(dst.functions[0].name, "wanted_fn");
    assert_eq!(dst.enums.len(), 1);
    assert_eq!(dst.enums[0].name, "WantedEnum");
}

// ---------------------------------------------------------------------------
// defaults.rs coverage: Duration::from_secs/from_millis, vec![], None,
// string literal direct, unary negation on non-numeric
// ---------------------------------------------------------------------------

#[test]
fn test_field_with_duration_from_secs_default() {
    // Duration::from_secs(5) should extract as IntLiteral(5000) — milliseconds
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
        Some(alef_core::ir::DefaultValue::IntLiteral(5000)),
        "Duration::from_secs(5) should be 5000 milliseconds"
    );
}

#[test]
fn test_field_with_duration_from_millis_default() {
    // Duration::from_millis(250) should extract as IntLiteral(250)
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
        Some(alef_core::ir::DefaultValue::IntLiteral(250)),
        "Duration::from_millis(250) should be 250 milliseconds"
    );
}

#[test]
fn test_field_with_vec_macro_default() {
    // `vec![]` (empty token macro) should extract as Empty
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
        Some(alef_core::ir::DefaultValue::Empty),
        "vec![] should extract as Empty"
    );
}

#[test]
fn test_field_with_none_default() {
    // Bare `None` should extract as DefaultValue::None
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
        Some(alef_core::ir::DefaultValue::None),
        "Bare None should extract as DefaultValue::None"
    );
}

#[test]
fn test_field_with_str_literal_default() {
    // A bare `&str` literal used directly as a string expression (not via .into())
    // The extractor handles `syn::Lit::Str` directly — exercises the Lit::Str branch.
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
        Some(alef_core::ir::DefaultValue::StringLiteral("hello".to_string())),
        "String::from(literal) exercises the Lit::Str branch"
    );
}

#[test]
fn test_field_with_duration_from_secs_non_literal_fallback() {
    // Duration::from_secs with a non-literal arg falls back to Empty
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
        Some(alef_core::ir::DefaultValue::Empty),
        "Duration::from_secs with non-literal arg should fall back to Empty"
    );
}

#[test]
fn test_unary_negation_on_non_numeric_falls_back_to_empty() {
    // Negating something that isn't an int or float literal — should return Empty.
    // We exercise this indirectly by using a call expression that itself returns Empty.
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
        Some(alef_core::ir::DefaultValue::Empty),
        "Negating a non-literal expression should fall back to Empty"
    );
}

#[test]
fn test_trait_method_with_default_impl() {
    // Trait methods with default implementations should have has_default_impl=true.
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
