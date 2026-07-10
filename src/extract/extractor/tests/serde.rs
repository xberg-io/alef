use super::*;

#[test]
fn test_has_serde_via_derive_still_detected() {
    // Regression: types using #[derive(Serialize, Deserialize)] must still get has_serde=true.
    let source = r#"
        #[derive(Clone, serde::Serialize, serde::Deserialize)]
        pub struct Config {
            pub name: String,
            pub timeout: u64,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(surface.types[0].has_serde, "derive-based serde must still be detected");
}

#[test]
fn test_has_serde_via_manual_impls_detected() {
    let source = r#"
        #[derive(Clone, Debug)]
        pub struct NodeContext {
            pub tag_name: String,
            pub depth: usize,
        }

        impl serde::Serialize for NodeContext {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                unimplemented!()
            }
        }

        impl<'de> serde::Deserialize<'de> for NodeContext {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(
        surface.types[0].has_serde,
        "manual impl Serialize + impl Deserialize must set has_serde=true"
    );
}

#[test]
fn test_has_serde_with_lifetime_parameterised_manual_impls() {
    let source = r#"
        #[derive(Clone, Debug)]
        pub struct Foo {
            pub value: String,
        }

        impl serde::Serialize for Foo {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                unimplemented!()
            }
        }

        impl<'de> serde::Deserialize<'de> for Foo {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(
        surface.types[0].has_serde,
        "lifetime-parameterised manual serde impls must set has_serde=true"
    );
}

#[test]
fn test_has_serde_only_serialize_not_set() {
    let source = r#"
        #[derive(Clone, Debug)]
        pub struct Foo {
            pub value: String,
        }

        impl serde::Serialize for Foo {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(
        !surface.types[0].has_serde,
        "only Serialize without Deserialize must not set has_serde"
    );
}

#[test]
fn test_extract_function_since_annotation_is_populated() {
    let source = r#"
        #[alef(since = "1.0.0")]
        pub fn new_api() {}
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].version.since.as_deref(), Some("1.0.0"));
    assert!(surface.functions[0].version.deprecated.is_none());
}
