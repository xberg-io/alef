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

    assert!(find_method("name").returns_ref, "name() should have returns_ref=true");
    assert!(
        !find_method("owned_name").returns_ref,
        "owned_name() should have returns_ref=false"
    );
    assert!(
        find_method("opt_name").returns_ref,
        "opt_name() should have returns_ref=true"
    );
    assert!(
        !find_method("opt_owned").returns_ref,
        "opt_owned() should have returns_ref=false"
    );
    assert!(
        find_method("result_ref").returns_ref,
        "result_ref() should have returns_ref=true"
    );
    assert!(
        !find_method("result_owned").returns_ref,
        "result_owned() should have returns_ref=false"
    );
}

#[test]
fn test_boxfuture_wrapping_result_is_async_with_error_type() {
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

#[test]
fn test_newtype_wrapper_recorded_on_field() {
    let source = r#"
        pub struct UserId(pub u64);

        pub struct User {
            pub id: UserId,
        }
    "#;

    let surface = extract_from_source(source);

    assert!(
        surface.types.iter().all(|t| t.name != "UserId"),
        "Simple newtype with pub inner field should be resolved away"
    );

    let user = surface.types.iter().find(|t| t.name == "User").expect("User not found");
    let id_field = &user.fields[0];
    assert_eq!(id_field.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::U64));
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
    let source = r#"
        pub struct Score(pub i32);

        pub struct Leaderboard {
            pub scores: std::collections::HashMap<String, Score>,
        }
    "#;

    let surface = extract_from_source(source);

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
    if let TypeRef::Map(_, v) = &scores_field.ty {
        assert_eq!(**v, TypeRef::Primitive(crate::core::ir::PrimitiveType::I32));
    } else {
        panic!("Expected TypeRef::Map for HashMap field, got {:?}", scores_field.ty);
    }
}
