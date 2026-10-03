use crate::core::ir::{NewtypeContainer, NewtypeConversion, TypeRef};

use super::super::extract_from_source;

#[test]
fn marked_private_string_wrapper_is_resolved_with_explicit_conversion_metadata() {
    let surface = extract_from_source(
        r#"
        #[cfg_attr(alef, alef(transparent_string(from = "from", into = "into_inner")))]
        pub struct SecretString(String);

        pub struct Credentials {
            pub token: SecretString,
            pub optional: Option<SecretString>,
            pub headers: std::collections::HashMap<String, SecretString>,
        }

        pub enum Auth {
            Bearer(SecretString),
            Header { value: SecretString },
        }
        "#,
    );

    assert!(surface.types.iter().all(|typ| typ.name != "SecretString"));
    let credentials = surface
        .types
        .iter()
        .find(|typ| typ.name == "Credentials")
        .expect("Credentials must be extracted");

    let expected = [
        ("token", Vec::new()),
        ("optional", vec![NewtypeContainer::Optional]),
        ("headers", vec![NewtypeContainer::MapValue]),
    ];
    for (name, containers) in expected {
        let field = credentials
            .fields
            .iter()
            .find(|field| field.name == name)
            .expect("field must be extracted");
        let wrapper = field
            .newtype_wrapper
            .as_ref()
            .expect("wrapper metadata must survive resolution");
        assert_eq!(wrapper.rust_path(), "test_crate::SecretString");
        assert_eq!(wrapper.containers(), containers);
        assert_eq!(
            wrapper.conversion(),
            &NewtypeConversion::TransparentString {
                from: "from".to_string(),
                into: "into_inner".to_string(),
            }
        );
    }

    assert_eq!(credentials.fields[0].ty, TypeRef::String);
    assert!(matches!(credentials.fields[1].ty, TypeRef::Optional(_)));
    assert!(matches!(credentials.fields[2].ty, TypeRef::Map(_, _)));

    let auth = surface
        .enums
        .iter()
        .find(|enum_def| enum_def.name == "Auth")
        .expect("Auth must be extracted");
    for variant in &auth.variants {
        let wrapper = variant.fields[0]
            .newtype_wrapper
            .as_ref()
            .expect("enum payload wrapper metadata must survive resolution");
        assert_eq!(wrapper.rust_path(), "test_crate::SecretString");
        assert!(wrapper.containers().is_empty());
    }
}
