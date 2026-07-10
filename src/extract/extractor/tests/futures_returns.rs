use super::*;

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
                unimplemented!()
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
                unimplemented!()
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
                unimplemented!()
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
    assert_eq!(id_field.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::U64));
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
                unimplemented!()
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
            unimplemented!()
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
    assert_eq!(
        func.return_type,
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)
    );
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
        assert_eq!(**v, TypeRef::Primitive(crate::core::ir::PrimitiveType::I32));
    } else {
        panic!("Expected TypeRef::Map for HashMap field, got {:?}", scores_field.ty);
    }
}
