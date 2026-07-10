use super::*;

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
            unimplemented!()
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
                unimplemented!()
            }

            /// Start listening.
            pub async fn listen(&self, port: u16) -> Result<(), std::io::Error> {
                unimplemented!()
            }

            /// Shutdown mutably.
            pub fn shutdown(&mut self) {
                unimplemented!()
            }

            // Private, should be excluded
            fn internal(&self) {}
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let server = &surface.types[0];
    assert_eq!(server.name, "Server");
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
                unimplemented!()
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
fn test_extract_pub_type_alias() {
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
fn test_pub_type_alias_with_doc_hidden_is_binding_excluded() {
    // `#[doc(hidden)]` on a type alias should mark it as binding_excluded so
    let source = r#"
        #[doc(hidden)]
        pub type InternalBuffer = std::vec::Vec<u8>;
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    let alias = &surface.types[0];
    assert_eq!(alias.name, "InternalBuffer");
    assert!(alias.binding_excluded);
    assert_eq!(alias.binding_exclusion_reason.as_deref(), Some("doc(hidden)"));
}
