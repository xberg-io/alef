use super::*;

// ==============================================================================
// ==============================================================================

#[test]
fn test_wrap_return_with_mutex_self_opaque_plain() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        false,
        false,
    );

    assert_eq!(result, "Self { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_with_mutex_self_opaque_mutex_type() {
    let opaque_types = AHashSet::new();
    let mut mutex_types = AHashSet::new();
    mutex_types.insert("MyType".to_string());

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        false,
        false,
    );

    assert_eq!(result, "Self { inner: Arc::new(std::sync::Mutex::new(result)) }");
}

#[test]
fn test_wrap_return_with_mutex_other_opaque_type() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("OtherType".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("OtherType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "OtherType { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_with_mutex_non_opaque_named_uses_into() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("SomeType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_with_mutex_string_returns_ref_uses_into() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::String,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true,
        false,
    );

    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_with_mutex_string_owned_passthrough() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::String,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_with_mutex_returns_cow_owned_named() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("SomeType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        true,
    );

    assert_eq!(result, "result.into_owned().into()");
}

#[test]
fn test_wrap_return_with_mutex_duration() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Duration,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.as_millis() as u64");
}

#[test]
fn test_wrap_return_with_mutex_optional_opaque() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Handle".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Named("Handle".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.map(|v| Handle { inner: Arc::new(v) })");
}

#[test]
fn test_wrap_return_with_mutex_vec_opaque() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Item".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result,
        "result.into_iter().map(|v| Item { inner: Arc::new(v) }).collect()"
    );
}

#[test]
fn test_apply_return_newtype_unwrap_none() {
    let result = binding_helpers::apply_return_newtype_unwrap("result", &None);
    assert_eq!(result, "result", "None wrapper should pass through unchanged");
}

#[test]
fn test_apply_return_newtype_unwrap_some() {
    let result = binding_helpers::apply_return_newtype_unwrap("result", &Some("NodeIndex".to_string()));
    assert_eq!(result, "(result).0", "Some wrapper should unwrap with .0");
}

#[test]
fn test_apply_return_newtype_unwrap_complex_expr() {
    let result = binding_helpers::apply_return_newtype_unwrap("self.inner.method(args)", &Some("W".to_string()));
    assert_eq!(
        result, "(self.inner.method(args)).0",
        "complex expression wrapped in parens then .0"
    );
}

#[test]
fn test_wrap_return_with_mutex_opaque_self_with_mutex() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyType".to_string());
    let mut mutex_types = AHashSet::new();
    mutex_types.insert("MyType".to_string());

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        false,
        false,
    );

    assert_eq!(
        result, "Self { inner: Arc::new(std::sync::Mutex::new(result)) }",
        "mutex opaque self-return should use Mutex::new"
    );
}

#[test]
fn test_wrap_return_with_mutex_other_opaque_with_mutex() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("OtherType".to_string());
    let mut mutex_types = AHashSet::new();
    mutex_types.insert("OtherType".to_string());

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("OtherType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result, "OtherType { inner: Arc::new(std::sync::Mutex::new(result)) }",
        "mutex cross-type opaque return should use Mutex::new"
    );
}

#[test]
fn test_wrap_return_with_mutex_returns_ref_owned() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyType".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        true,
        false,
    );

    assert!(
        result.contains("result.clone()"),
        "returns_ref should clone before wrapping"
    );
    assert!(result.contains("Self { inner: Arc::new"), "should still wrap in Self");
}

#[test]
fn test_wrap_return_with_mutex_returns_cow_opaque_self() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyType".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        &mutex_types,
        true,
        false,
        true,
    );

    assert!(
        result.contains("result.into_owned()"),
        "returns_cow should call .into_owned()"
    );
    assert!(result.contains("Self { inner: Arc::new"), "should wrap in Self");
}

#[test]
fn test_wrap_return_with_mutex_json_conversion() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Json,
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result.to_string()", "Json should serialize to string");
}

#[test]
fn test_wrap_return_with_mutex_optional_non_opaque_named() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Named("Config".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result, "result.map(Into::into)",
        "Optional non-opaque Named should map with Into::into"
    );
}

#[test]
fn test_wrap_return_with_mutex_optional_opaque_returns_ref() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Handle".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Named("Handle".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true,
        false,
    );

    assert!(
        result.contains(".clone()"),
        "returns_ref on optional opaque should clone"
    );
    assert!(
        result.contains("Handle { inner: Arc::new("),
        "should wrap Handle in Arc"
    );
}

#[test]
fn test_wrap_return_with_mutex_optional_string_returns_ref() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::String)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true,
        false,
    );

    assert_eq!(
        result, "result.map(Into::into)",
        "Optional String returns_ref should map Into::into"
    );
}

#[test]
fn test_wrap_return_with_mutex_optional_string_owned() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::String)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result", "Optional String owned should pass through");
}

#[test]
fn test_wrap_return_with_mutex_optional_duration() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Duration)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result, "result.map(|d| d.as_millis() as u64)",
        "Optional Duration should convert to millis"
    );
}

#[test]
fn test_wrap_return_with_mutex_optional_json() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Json)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result, "result.map(ToString::to_string)",
        "Optional Json should serialize via ToString"
    );
}

#[test]
fn test_wrap_return_with_mutex_optional_path() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Path)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result, "result.map(Into::into)",
        "Optional Path should convert via Into"
    );
}

#[test]
fn test_wrap_return_with_mutex_vec_non_opaque_named_ref() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::Named("Config".to_string()))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true,
        false,
    );

    assert_eq!(
        result, "result.iter().map(|v| v.clone().into()).collect()",
        "Vec non-opaque Named returns_ref should clone each element"
    );
}

#[test]
fn test_wrap_return_with_mutex_vec_string_returns_ref() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::String)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true,
        false,
    );

    assert_eq!(
        result, "result.iter().map(|s| s.to_string()).collect()",
        "Vec<String> returns_ref must hop through ToString (&&str doesn't impl From)"
    );
}

#[test]
fn test_wrap_return_with_mutex_vec_string_owned() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::String)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(result, "result", "Vec String owned should pass through");
}

#[test]
fn test_wrap_return_with_mutex_vec_path() {
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::Path)),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        false,
        false,
    );

    assert_eq!(
        result, "result.into_iter().map(Into::into).collect()",
        "Vec Path should convert via Into"
    );
}

#[test]
fn test_wrap_return_with_mutex_optional_vec_opaque_returns_ref() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Item".to_string());
    let mutex_types = AHashSet::new();

    let result = binding_helpers::wrap_return_with_mutex(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))))),
        "MyType",
        &opaque_types,
        &mutex_types,
        false,
        true,
        false,
    );

    assert!(
        result.contains(".clone()"),
        "returns_ref on optional vec opaque should clone elements"
    );
    assert!(
        result.contains("Item { inner: Arc::new("),
        "should wrap each element in Arc"
    );
}
