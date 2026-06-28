use super::{EmitContext, python_field_type};
use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::AHashSet;

/// Build the three name-sets `python_field_type` consults: plain enums, data enums, and the
/// subset of data enums that also accept a bare string tag (those with a unit variant).
fn make_sets<'a>(
    enum_names: &[&'a str],
    data_enum_names: &[&'a str],
    str_coercible: &[&'a str],
) -> (AHashSet<&'a str>, AHashSet<&'a str>, AHashSet<&'a str>) {
    (
        enum_names.iter().copied().collect(),
        data_enum_names.iter().copied().collect(),
        str_coercible.iter().copied().collect(),
    )
}

/// `Map<String, Named("ExtractionPattern")>` in OptionsModule context resolves to the bare
/// data-enum class name (imported from the native module) — a payload-only enum, so no `| str`.
#[test]
fn test_map_named_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &[]);
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("ExtractionPattern".to_string())),
    );
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "dict[str, ExtractionPattern]");
}

/// `Map<String, Named("ExtractionPattern")>` in NativeStub context resolves to the
/// native PyO3 class — also bare name (no `_native.` prefix needed in a .pyi file that
/// IS the native module). The `| str` widening never applies to the stub.
#[test]
fn test_map_named_data_enum_native_stub() {
    let (enum_names, data_enum_names, str_coercible) =
        make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &["ExtractionPattern"]);
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("ExtractionPattern".to_string())),
    );
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(result, "dict[str, ExtractionPattern]");
}

/// `Vec<Named("Message")>` in OptionsModule context uses the bare data-enum class name.
#[test]
fn test_vec_named_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["Message"], &["Message"], &[]);
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "list[Message]");
}

/// `Vec<Named("Message")>` in NativeStub context uses the bare native-class name.
#[test]
fn test_vec_named_data_enum_native_stub() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["Message"], &["Message"], &[]);
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(result, "list[Message]");
}

/// `Optional<Named("ExtractionPattern")>` in OptionsModule context appends `| None`.
#[test]
fn test_optional_named_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &[]);
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ExtractionPattern".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "ExtractionPattern | None");
}

/// `Optional<Named("ExtractionPattern")>` in NativeStub context appends `| None`.
#[test]
fn test_optional_named_data_enum_native_stub() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &[]);
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ExtractionPattern".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(result, "ExtractionPattern | None");
}

/// A data enum with a unit (tag-only) variant is widened to `<Class> | str` in OptionsModule
/// so the bare string tag (and string defaults like `= "native"`) type-check, while the
/// NativeStub keeps the class-only form.
#[test]
fn test_str_coercible_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) =
        make_sets(&["ImageOutputFormat"], &["ImageOutputFormat"], &["ImageOutputFormat"]);
    let ty = TypeRef::Named("ImageOutputFormat".to_string());
    let options = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    let native = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(options, "ImageOutputFormat | str");
    assert_eq!(native, "ImageOutputFormat");
}

/// The `| str` widening reaches inside containers: `Optional<ImageOutputFormat>` becomes
/// `ImageOutputFormat | str | None`.
#[test]
fn test_str_coercible_data_enum_optional() {
    let (enum_names, data_enum_names, str_coercible) =
        make_sets(&["ImageOutputFormat"], &["ImageOutputFormat"], &["ImageOutputFormat"]);
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ImageOutputFormat".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "ImageOutputFormat | str | None");
}

/// A payload-only data enum (no unit variant, e.g. EmbeddingModelType) stays class-only — the
/// flattened `str | int | LlmConfig` alias is gone, and a bare string is NOT a valid value.
#[test]
fn test_payload_only_data_enum_class_only() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["EmbeddingModelType"], &["EmbeddingModelType"], &[]);
    let ty = TypeRef::Named("EmbeddingModelType".to_string());
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "EmbeddingModelType");
}

/// Plain (non-data) enum field always uses `EnumName | str` regardless of context.
#[test]
fn test_plain_enum_field_both_contexts() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["HeadingStyle"], &[], &[]);
    let ty = TypeRef::Named("HeadingStyle".to_string());
    let options = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    let native = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(options, "HeadingStyle | str");
    assert_eq!(native, "HeadingStyle | str");
}

/// Primitive types are unaffected by context.
#[test]
fn test_primitive_unaffected_by_context() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&[], &[], &[]);
    let ty = TypeRef::Primitive(PrimitiveType::Bool);
    let options = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    let native = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(options, "bool");
    assert_eq!(native, "bool");
}
