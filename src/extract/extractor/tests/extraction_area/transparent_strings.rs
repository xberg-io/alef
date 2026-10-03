use crate::core::ir::{NewtypeContainer, NewtypeConversion, NewtypeWrapper, TypeRef};

use super::super::extract_from_source;

fn assert_conversion_paths(encoded: &str, expected: &[(&str, Vec<NewtypeContainer>)]) {
    let decoded = NewtypeWrapper::decode(encoded).expect("conversion metadata must decode");
    let paths = decoded.explicit_paths();
    assert_eq!(paths.len(), expected.len());
    for (metadata, (rust_path, containers)) in paths.iter().zip(expected) {
        assert_eq!(metadata.rust_path, *rust_path);
        assert_eq!(&metadata.containers, containers);
        assert_eq!(
            metadata.conversion,
            NewtypeConversion::TransparentString {
                from: "from".to_string(),
                into: "into_inner".to_string(),
            }
        );
    }
}

#[test]
fn marked_private_string_wrapper_is_resolved_across_all_supported_positions() {
    let surface = extract_from_source(
        r#"
        #[derive(Clone, PartialEq, Eq, Hash)]
        #[cfg_attr(alef, alef(transparent_string(from = "from", into = "into_inner")))]
        pub struct Credential(String);

        impl Credential {
            pub fn from(value: String) -> Self { Self(value) }
            pub fn into_inner(self) -> String { self.0 }
        }

        pub struct Credentials {
            pub token: Credential,
            pub optional: Option<Credential>,
            pub nested: Vec<Option<Credential>>,
            pub key_lookup: std::collections::HashMap<Credential, String>,
            pub both: std::collections::HashMap<Credential, Vec<Credential>>,
        }

        pub enum Auth {
            Bearer(Credential),
            Header { value: Option<Credential> },
        }

        pub struct Vault;
        impl Vault {
            pub fn echo(&self, value: Credential) -> Credential { value }
        }

        pub fn echo(value: Credential) -> Credential { value }
        pub fn optional_echo(value: Option<Credential>) -> Option<Credential> { value }
        pub async fn async_echo(value: Credential) -> Credential { value }
        pub fn fallible_echo(value: Credential) -> Result<Credential, String> { Ok(value) }
        "#,
    );

    assert!(surface.types.iter().all(|typ| typ.name != "Credential"));
    assert!(surface.unsupported_public_items.is_empty());
    let credentials = surface
        .types
        .iter()
        .find(|typ| typ.name == "Credentials")
        .expect("Credentials must be extracted");

    let expected = [
        ("token", false, vec![("test_crate::Credential", vec![])]),
        (
            "optional",
            true,
            vec![("test_crate::Credential", vec![NewtypeContainer::Optional])],
        ),
        (
            "nested",
            false,
            vec![(
                "test_crate::Credential",
                vec![NewtypeContainer::Vec, NewtypeContainer::Optional],
            )],
        ),
        (
            "key_lookup",
            false,
            vec![("test_crate::Credential", vec![NewtypeContainer::MapKey])],
        ),
        (
            "both",
            false,
            vec![
                ("test_crate::Credential", vec![NewtypeContainer::MapKey]),
                (
                    "test_crate::Credential",
                    vec![NewtypeContainer::MapValue, NewtypeContainer::Vec],
                ),
            ],
        ),
    ];
    for (name, optional, paths) in expected {
        let field = credentials
            .fields
            .iter()
            .find(|field| field.name == name)
            .expect("field must be extracted");
        assert_eq!(field.optional, optional);
        assert_conversion_paths(
            field
                .newtype_wrapper
                .as_deref()
                .expect("metadata must survive resolution"),
            &paths,
        );
    }

    assert_eq!(credentials.fields[0].ty, TypeRef::String);
    assert_eq!(credentials.fields[1].ty, TypeRef::String);
    assert!(matches!(credentials.fields[2].ty, TypeRef::Vec(_)));
    assert!(matches!(credentials.fields[3].ty, TypeRef::Map(_, _)));

    let auth = surface
        .enums
        .iter()
        .find(|item| item.name == "Auth")
        .expect("Auth must be extracted");
    assert_conversion_paths(
        auth.variants[0].fields[0]
            .newtype_wrapper
            .as_deref()
            .expect("tuple payload metadata"),
        &[("test_crate::Credential", vec![])],
    );
    assert_conversion_paths(
        auth.variants[1].fields[0]
            .newtype_wrapper
            .as_deref()
            .expect("struct payload metadata"),
        &[("test_crate::Credential", vec![NewtypeContainer::Optional])],
    );

    for function_name in ["echo", "async_echo", "fallible_echo"] {
        let function = surface
            .functions
            .iter()
            .find(|item| item.name == function_name)
            .expect("function");
        assert_conversion_paths(
            function.params[0]
                .newtype_wrapper
                .as_deref()
                .expect("parameter metadata"),
            &[("test_crate::Credential", vec![])],
        );
        assert_conversion_paths(
            function.return_newtype_wrapper.as_deref().expect("return metadata"),
            &[("test_crate::Credential", vec![])],
        );
    }
    let optional = surface
        .functions
        .iter()
        .find(|item| item.name == "optional_echo")
        .expect("optional function");
    assert!(optional.params[0].optional);
    assert_conversion_paths(
        optional.params[0]
            .newtype_wrapper
            .as_deref()
            .expect("optional parameter metadata"),
        &[("test_crate::Credential", vec![NewtypeContainer::Optional])],
    );
    assert_conversion_paths(
        optional
            .return_newtype_wrapper
            .as_deref()
            .expect("optional return metadata"),
        &[("test_crate::Credential", vec![NewtypeContainer::Optional])],
    );

    let method = surface
        .types
        .iter()
        .find(|item| item.name == "Vault")
        .and_then(|item| item.methods.iter().find(|method| method.name == "echo"))
        .expect("method");
    assert_conversion_paths(
        method.params[0]
            .newtype_wrapper
            .as_deref()
            .expect("method parameter metadata"),
        &[("test_crate::Credential", vec![])],
    );
    assert_conversion_paths(
        method
            .return_newtype_wrapper
            .as_deref()
            .expect("method return metadata"),
        &[("test_crate::Credential", vec![])],
    );
}

#[test]
fn invalid_transparent_string_annotations_are_reported() {
    let cases = [
        ("transparent_string(from = \"from\")", "requires `into"),
        (
            "transparent_string(from = \"from\", into = \"into_inner\", extra = \"x\")",
            "unknown",
        ),
        ("transparent_string(from = 1, into = \"into_inner\")", "string literal"),
        (
            "transparent_string(from = \"not::an::ident\", into = \"into_inner\")",
            "identifier",
        ),
        (
            "transparent_string(from = \"from\", from = \"new\", into = \"into_inner\")",
            "duplicate",
        ),
    ];
    for (annotation, expected_reason) in cases {
        let source = format!("#[derive(Clone)] #[alef({annotation})] pub struct Credential(String);");
        let surface = extract_from_source(&source);
        assert!(
            surface
                .unsupported_public_items
                .iter()
                .any(|item| item.reason.contains(expected_reason)),
            "expected `{expected_reason}` diagnostic for {annotation}: {:?}",
            surface.unsupported_public_items
        );
    }
}

#[test]
fn invalid_transparent_string_shape_and_methods_are_reported() {
    let cases = [
        (
            "#[derive(Clone)] #[alef(transparent_string(from = \"from\", into = \"into_inner\"))] pub struct Credential { value: String }",
            "single-field tuple struct",
        ),
        (
            "#[derive(Clone)] #[alef(transparent_string(from = \"from\", into = \"into_inner\"))] pub struct Credential(u64);",
            "inner String",
        ),
        (
            "#[alef(transparent_string(from = \"from\", into = \"into_inner\"))] pub struct Credential(String);",
            "requires Clone",
        ),
        (
            r#"#[derive(Clone)]
               #[alef(transparent_string(from = "from", into = "into_inner"))]
               pub struct Credential(String);
               impl Credential { pub fn from(value: String) -> Self { Self(value) } }"#,
            "into_inner",
        ),
        (
            r#"#[derive(Clone)]
               #[alef(transparent_string(from = "from", into = "into_inner"))]
               pub struct Credential(String);
               impl Credential {
                   pub fn from(value: u64) -> Self { Self(value.to_string()) }
                   pub fn into_inner(&self) -> String { self.0.clone() }
               }"#,
            "must be a public synchronous",
        ),
    ];
    for (source, expected_reason) in cases {
        let surface = extract_from_source(source);
        assert!(
            surface
                .unsupported_public_items
                .iter()
                .any(|item| item.reason.contains(expected_reason)),
            "expected `{expected_reason}` diagnostic: {:?}",
            surface.unsupported_public_items
        );
    }
}

#[test]
fn duplicate_transparent_string_annotations_are_reported() {
    let surface = extract_from_source(
        r#"
        #[derive(Clone)]
        #[alef(transparent_string(from = "from", into = "into_inner"))]
        #[alef(transparent_string(from = "from", into = "into_inner"))]
        pub struct Credential(String);
        "#,
    );
    assert!(
        surface
            .unsupported_public_items
            .iter()
            .any(|item| item.reason.contains("duplicate"))
    );
}

#[test]
fn ambiguous_short_wrapper_names_are_reported_without_removing_either_type() {
    let surface = extract_from_source(
        r#"
        pub mod first {
            #[derive(Clone)]
            #[alef(transparent_string(from = "from", into = "into_inner"))]
            pub struct Secret(String);
            impl Secret {
                pub fn from(value: String) -> Self { Self(value) }
                pub fn into_inner(self) -> String { self.0 }
            }
        }
        pub mod second {
            #[derive(Clone)]
            #[alef(transparent_string(from = "from", into = "into_inner"))]
            pub struct Secret(String);
            impl Secret {
                pub fn from(value: String) -> Self { Self(value) }
                pub fn into_inner(self) -> String { self.0 }
            }
        }
        pub struct Pair {
            pub first: first::Secret,
            pub second: second::Secret,
        }
        "#,
    );

    assert_eq!(surface.types.iter().filter(|typ| typ.name == "Secret").count(), 2);
    assert!(surface.unsupported_public_items.iter().any(|item| {
        item.reason.contains("ambiguous transparent newtype name")
            && item.reason.contains("test_crate::first::Secret")
            && item.reason.contains("test_crate::second::Secret")
    }));
}

#[test]
fn wrapper_name_collision_with_ordinary_struct_is_reported() {
    let surface = extract_from_source(
        r#"
        pub mod first {
            #[derive(Clone)]
            #[alef(transparent_string(from = "from", into = "into_inner"))]
            pub struct Secret(String);
            impl Secret {
                pub fn from(value: String) -> Self { Self(value) }
                pub fn into_inner(self) -> String { self.0 }
            }
        }
        pub mod second {
            pub struct Secret { pub value: String }
        }
        pub fn reveal(value: first::Secret) -> String { value.into_inner() }
        "#,
    );

    assert_eq!(surface.types.iter().filter(|typ| typ.name == "Secret").count(), 2);
    assert!(surface.unsupported_public_items.iter().any(|item| {
        item.item_path == "test_crate::first::Secret"
            && item.reason.contains("test_crate::second::Secret")
            && item.reason.contains("ambiguous transparent newtype name")
    }));
}

#[test]
fn wrapper_name_collision_with_enum_is_reported() {
    let surface = extract_from_source(
        r#"
        pub mod first {
            #[derive(Clone)]
            #[alef(transparent_string(from = "from", into = "into_inner"))]
            pub struct Secret(String);
            impl Secret {
                pub fn from(value: String) -> Self { Self(value) }
                pub fn into_inner(self) -> String { self.0 }
            }
        }
        pub mod second {
            pub enum Secret { Missing }
        }
        pub fn reveal(value: first::Secret) -> String { value.into_inner() }
        "#,
    );

    assert!(
        surface
            .types
            .iter()
            .any(|typ| typ.rust_path == "test_crate::first::Secret")
    );
    assert!(
        surface
            .enums
            .iter()
            .any(|item| item.rust_path == "test_crate::second::Secret")
    );
    assert!(surface.unsupported_public_items.iter().any(|item| {
        item.item_path == "test_crate::first::Secret"
            && item.reason.contains("test_crate::second::Secret")
            && item.reason.contains("ambiguous transparent newtype name")
    }));
}

#[test]
fn borrowed_or_wrapped_transparent_string_methods_are_rejected() {
    let cases = [
        r#"pub fn from(value: &str) -> Self { Self(value.to_string()) }
            pub fn into_inner(self) -> String { self.0 }"#,
        r#"pub fn from(value: &mut String) -> Self { Self(value.clone()) }
            pub fn into_inner(self) -> String { self.0 }"#,
        r#"pub fn from(value: std::borrow::Cow<'static, str>) -> Self { Self(value.into_owned()) }
            pub fn into_inner(self) -> String { self.0 }"#,
        r#"pub fn from(value: String) -> Self { Self(value) }
            pub fn into_inner(self) -> &'static str { "redacted" }"#,
        r#"pub fn from(value: String) -> Self { Self(value) }
            pub fn into_inner(self) -> std::borrow::Cow<'static, str> { self.0.into() }"#,
    ];

    for methods in cases {
        let source = format!(
            r#"#[derive(Clone)]
                #[alef(transparent_string(from = "from", into = "into_inner"))]
                pub struct Credential(String);
                impl Credential {{ {methods} }}"#
        );
        let surface = extract_from_source(&source);
        assert!(
            surface
                .unsupported_public_items
                .iter()
                .any(|item| item.reason.contains("must be a public synchronous")),
            "expected an exact-signature diagnostic: {:?}",
            surface.unsupported_public_items
        );
    }
}
