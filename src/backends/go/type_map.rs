use std::borrow::Cow;

use crate::codegen::c_consumer;
use crate::codegen::naming::{field_uses_duration_map_wire, go_type_name};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FieldDef, PrimitiveType, TypeRef};

/// TypeMapper for Go bindings.
///
/// Maps Rust types to idiomatic Go types:
/// - Integers use Go's explicit-width types (uint8, int32, etc.)
/// - usize/isize map to uint/int (platform-native width)
/// - `Optional<T>` becomes `*T` (nullable pointer)
/// - `Vec<T>` becomes `[]T`
/// - `Map<K,V>` becomes `map[K]V`
/// - JSON becomes json.RawMessage
/// - Unit becomes "" (void in Go — no type in return position)
/// - Duration becomes uint64 (milliseconds)
pub struct GoMapper;

impl TypeMapper for GoMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "uint8",
            PrimitiveType::U16 => "uint16",
            PrimitiveType::U32 => "uint32",
            PrimitiveType::U64 => "uint64",
            PrimitiveType::I8 => "int8",
            PrimitiveType::I16 => "int16",
            PrimitiveType::I32 => "int32",
            PrimitiveType::I64 => "int64",
            PrimitiveType::F32 => "float32",
            PrimitiveType::F64 => "float64",
            PrimitiveType::Usize => "uint",
            PrimitiveType::Isize => "int",
        })
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("string")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("[]byte")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("string")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("json.RawMessage")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("uint64")
    }

    fn optional(&self, inner: &str) -> String {
        format!("*{inner}")
    }

    fn vec(&self, inner: &str) -> String {
        format!("[]{inner}")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("map[{key}]{value}")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Owned(go_type_name(name))
    }

    fn error_wrapper(&self) -> &str {
        "error"
    }
}

/// Exported Go identifier for the millisecond-based `Duration` wire type (see
/// `duration_millis_type.jinja`). The shape serde *derives* for `std::time::Duration` is
/// `{"secs":u64,"nanos":u32}`, not a plain number, so a struct field serialized through
/// `encoding/json` against the real Rust wire shape cannot be a bare `uint64`/`*uint64` —
/// see [`go_struct_field_type`]. Scalar function/method parameters are unaffected: those
/// cross the FFI boundary as a direct `uint64` argument that the Rust glue converts with
/// `Duration::from_millis`, never through JSON, so [`GoMapper::duration`] stays `uint64`. ~keep
pub const GO_DURATION_MILLIS_TYPE: &str = "DurationMillis";

/// Maps a `TypeRef` to its Go **struct field** type, special-casing `Duration` to
/// [`GO_DURATION_MILLIS_TYPE`] instead of the bare `uint64` [`go_type`] returns.
///
/// Use this (not [`go_type`]) for any field emitted with a `json:"..."` tag — struct
/// fields round-trip through `encoding/json` against Rust's `serde` shape, which a plain
/// integer cannot represent for a `Duration`. Scalar FFI parameters should keep using
/// [`go_type`] directly.
pub fn go_struct_field_type(ty: &TypeRef) -> Cow<'static, str> {
    if matches!(ty, TypeRef::Duration) {
        Cow::Borrowed(GO_DURATION_MILLIS_TYPE)
    } else {
        go_type(ty)
    }
}

/// Optional/pointer counterpart of [`go_struct_field_type`] — see [`go_optional_type`].
pub fn go_optional_struct_field_type(ty: &TypeRef) -> Cow<'static, str> {
    if matches!(ty, TypeRef::Duration) {
        Cow::Owned(format!("*{GO_DURATION_MILLIS_TYPE}"))
    } else {
        go_optional_type(ty)
    }
}

/// Field-aware form of [`go_struct_field_type`]: emitters must call this, not the type-only
/// function, for anything carrying a `json:"..."` tag.
///
/// [`go_struct_field_type`] rewrites `Duration` to [`GO_DURATION_MILLIS_TYPE`] on the premise
/// that the field's bytes are whatever serde *derives*. A field with `#[serde(with = "...")]`
/// or `serialize_with` breaks that premise — the widespread `duration_ms` convention writes a
/// bare millisecond integer, so imposing the `{"secs","nanos"}` object would make every Go-side
/// construction fail Rust deserialization with `invalid type: map, expected u64`. Alef cannot
/// know what an arbitrary codec emits, so it declines to wrap and leaves the underlying scalar
/// (`uint64`), which is both the pre-existing behavior and correct for that convention.
///
/// The decision itself lives in [`crate::codegen::naming::field_uses_duration_map_wire`], the
/// single predicate every backend that special-cases `Duration`'s derive shape must consult —
/// see that function's doc for why C# and Java need the identical check. ~keep
pub fn go_field_type(field: &FieldDef) -> Cow<'static, str> {
    if field_uses_duration_map_wire(field) {
        go_struct_field_type(&field.ty)
    } else {
        go_type(&field.ty)
    }
}

/// Optional/pointer counterpart of [`go_field_type`].
pub fn go_optional_field_type(field: &FieldDef) -> Cow<'static, str> {
    if field_uses_duration_map_wire(field) {
        go_optional_struct_field_type(&field.ty)
    } else {
        go_optional_type(&field.ty)
    }
}

/// Returns the emitted C name of alef's scalar generational handle type, as cbindgen names
/// it in the generated header (e.g. `HTMAlefHandle`).
///
/// `backends::ffi::type_map::c_param_optional`/`c_return_optional` map every
/// `TypeRef::Named` value to this handle unconditionally — never to an opaque pointer — so
/// every Go local, parameter, or comparison for a value that started as `TypeRef::Named`
/// must use this C type name and compare against `0`, not `nil`.
///
/// The prefix must go through [`c_consumer::export_type_prefix`], not a local `.to_uppercase()`:
/// that is the same shouty-snake-case cbindgen actually applies to its `[export] prefix`, and a
/// plain uppercase disagrees whenever the prefix has an internal word boundary (`SampleCore` ->
/// `SAMPLE_CORE` vs `SAMPLECORE`). ~keep
pub fn alef_handle_c_type(ffi_prefix: &str) -> String {
    format!("{}AlefHandle", c_consumer::export_type_prefix(ffi_prefix))
}

/// Maps a TypeRef to its Go type representation.
/// Used for non-optional types in general contexts.
///
/// Delegates to [`GoMapper`] for exhaustive TypeRef handling.
pub fn go_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(GoMapper.map_type(ty))
}

/// Maps a TypeRef to its optional Go type representation (pointer for option).
///
/// If the type is already `Optional`, delegates to `go_type` (which produces `*T`).
/// Slices (`Vec<T>`, `Bytes`) and maps are already reference types in Go — they
/// are not wrapped in a pointer because `*[]T` and `*map[K]V` are unidiomatic
/// and unnecessary.
/// String types (String, Char, Path) are wrapped in pointer: `*string`.
/// All other non-reference types are wrapped in a pointer: `*T`.
pub fn go_optional_type(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => go_type(ty),
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Named(_)
        | TypeRef::Primitive(_)
        | TypeRef::Duration
        | TypeRef::Unit => Cow::Owned(format!("*{}", GoMapper.map_type(ty))),
    }
}

/// Return-position counterpart of [`go_optional_type`].
///
/// A C function returns one value, so `Option<Option<T>>` reaches Go carrying a single level of
/// nullability: `backends::ffi::type_map::c_return_optional` recurses straight into the inner
/// option, which makes `None` and `Some(None)` indistinguishable at the boundary. The Go return
/// *expression* (`gen_bindings::types::mapping::go_return_expr`) collapses the same way and
/// yields one pointer, so declaring `**T` here would name a type nothing in the generated file
/// can produce — the declaration and the expression would not even agree with each other. ~keep
pub fn go_return_type(ty: &TypeRef) -> Cow<'static, str> {
    go_optional_type(collapsed_return_shape(ty))
}

fn collapsed_return_shape(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Optional(_)) => collapsed_return_shape(inner),
        _ => ty,
    }
}

/// Returns the Go zero-value expression for a return-type, used in `return <zero>, fmt.Errorf(...)`
/// early exits.
///
/// Must stay in sync with the return-signature logic in `gen_bindings::methods` and
/// `gen_bindings::functions`: scalar primitives and Duration stay as value types and
/// need an explicit zero literal (`0`, `false`); scalar types (String, Char, Path, Json)
/// also stay as value types and use empty string `""`; everything else (Named, Vec, Map,
/// Bytes, Optional) is emitted as a pointer or reference type whose zero is `nil`.
pub fn go_zero_value(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
        TypeRef::Primitive(_) | TypeRef::Duration => "0".to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
        TypeRef::Json => "nil".to_string(),
        TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Optional(_)
        | TypeRef::Named(_)
        | TypeRef::Unit => "nil".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitives() {
        let m = GoMapper;
        assert_eq!(m.primitive(&PrimitiveType::Bool), "bool");
        assert_eq!(m.primitive(&PrimitiveType::U8), "uint8");
        assert_eq!(m.primitive(&PrimitiveType::U16), "uint16");
        assert_eq!(m.primitive(&PrimitiveType::U32), "uint32");
        assert_eq!(m.primitive(&PrimitiveType::U64), "uint64");
        assert_eq!(m.primitive(&PrimitiveType::I8), "int8");
        assert_eq!(m.primitive(&PrimitiveType::I16), "int16");
        assert_eq!(m.primitive(&PrimitiveType::I32), "int32");
        assert_eq!(m.primitive(&PrimitiveType::I64), "int64");
        assert_eq!(m.primitive(&PrimitiveType::F32), "float32");
        assert_eq!(m.primitive(&PrimitiveType::F64), "float64");
        assert_eq!(m.primitive(&PrimitiveType::Usize), "uint");
        assert_eq!(m.primitive(&PrimitiveType::Isize), "int");
    }

    #[test]
    fn test_string_and_char() {
        assert_eq!(GoMapper.map_type(&TypeRef::String), "string");
        assert_eq!(GoMapper.map_type(&TypeRef::Char), "string");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(GoMapper.map_type(&TypeRef::Bytes), "[]byte");
    }

    #[test]
    fn alef_handle_c_type_uses_shouty_snake_prefix_not_plain_uppercase() {
        // `SampleCore` is load-bearing: it has no separator of its own, so shouty-snake-casing
        // it (`SAMPLE_CORE`) and plain-uppercasing it (`SAMPLECORE`) genuinely disagree. A
        // prefix like `sample_core` already contains its own underscore, so both formulas
        // would produce the same string and this test would pass even against the pre-fix
        // `.to_uppercase()` formula. ~keep
        let expected = format!("{}AlefHandle", c_consumer::export_type_prefix("SampleCore"));
        assert_eq!(expected, "SAMPLE_COREAlefHandle");

        assert_eq!(alef_handle_c_type("SampleCore"), expected);
        // Negative control: this is exactly what the pre-fix `ffi_prefix.to_uppercase()`
        // formula produced.
        assert_ne!(alef_handle_c_type("SampleCore"), "SAMPLECOREAlefHandle");
    }

    #[test]
    fn test_path() {
        assert_eq!(GoMapper.map_type(&TypeRef::Path), "string");
    }

    #[test]
    fn test_json() {
        assert_eq!(GoMapper.map_type(&TypeRef::Json), "json.RawMessage");
    }

    #[test]
    fn test_unit() {
        assert_eq!(GoMapper.map_type(&TypeRef::Unit), "");
    }

    #[test]
    fn test_duration() {
        assert_eq!(GoMapper.map_type(&TypeRef::Duration), "uint64");
    }

    #[test]
    fn test_optional() {
        assert_eq!(
            GoMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "*string"
        );
    }

    #[test]
    fn test_vec() {
        assert_eq!(GoMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::String))), "[]string");
    }

    #[test]
    fn test_map() {
        assert_eq!(
            GoMapper.map_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32))
            )),
            "map[string]int32"
        );
    }

    #[test]
    fn test_go_type_delegate() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
        assert_eq!(go_type(&ty).as_ref(), GoMapper.map_type(&ty));
    }

    #[test]
    fn test_go_optional_type_already_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(go_optional_type(&ty), go_type(&ty));
    }

    #[test]
    fn test_go_optional_type_non_optional() {
        assert_eq!(go_optional_type(&TypeRef::String), "*string");
    }

    #[test]
    fn should_collapse_a_nested_option_to_one_pointer_in_return_position() {
        let nested = TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::Primitive(
            PrimitiveType::I64,
        )))));
        assert_eq!(go_optional_type(&nested), "**int64");
        assert_eq!(go_return_type(&nested), "*int64");

        let triple = TypeRef::Optional(Box::new(nested));
        assert_eq!(go_return_type(&triple), "*int64");
    }

    #[test]
    fn should_leave_a_single_option_untouched_in_return_position() {
        for ty in [
            TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
            TypeRef::Optional(Box::new(TypeRef::Duration)),
            TypeRef::Optional(Box::new(TypeRef::String)),
            TypeRef::String,
            TypeRef::Vec(Box::new(TypeRef::String)),
        ] {
            assert_eq!(go_return_type(&ty), go_optional_type(&ty), "{ty:?} was rewritten");
        }
    }

    #[test]
    fn test_go_optional_type_vec_not_pointer() {
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        assert_eq!(go_optional_type(&ty), "[]string");
    }

    #[test]
    fn test_go_optional_type_bytes_not_pointer() {
        assert_eq!(go_optional_type(&TypeRef::Bytes), "[]byte");
    }

    #[test]
    fn test_go_optional_type_map_not_pointer() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        assert_eq!(go_optional_type(&ty), "map[string]string");
    }

    #[test]
    fn test_go_struct_field_type_duration_uses_wire_safe_type() {
        assert_eq!(go_struct_field_type(&TypeRef::Duration), "DurationMillis");
    }

    #[test]
    fn test_go_struct_field_type_non_duration_delegates_to_go_type() {
        assert_eq!(go_struct_field_type(&TypeRef::String), go_type(&TypeRef::String));
    }

    #[test]
    fn test_go_optional_struct_field_type_duration_is_pointer_to_wire_safe_type() {
        assert_eq!(go_optional_struct_field_type(&TypeRef::Duration), "*DurationMillis");
    }

    #[test]
    fn test_go_optional_struct_field_type_non_duration_delegates_to_go_optional_type() {
        assert_eq!(
            go_optional_struct_field_type(&TypeRef::String),
            go_optional_type(&TypeRef::String)
        );
    }

    #[test]
    fn test_go_type_duration_unaffected_scalar_param_stays_uint64() {
        // Scalar FFI parameters must keep the bare `uint64` mapping — only struct fields
        // (via `go_struct_field_type`) switch to the wire-safe `DurationMillis` type. ~keep
        assert_eq!(GoMapper.map_type(&TypeRef::Duration), "uint64");
    }

    fn duration_field(serde_with: Option<&str>) -> FieldDef {
        FieldDef {
            name: "timeout".to_string(),
            ty: TypeRef::Duration,
            serde_with: serde_with.map(str::to_string),
            serde_skip_serializing_if: false,
            serde_skip: false,
            ..Default::default()
        }
    }

    #[test]
    fn duration_field_without_serde_with_keeps_the_derived_object_shape() {
        assert_eq!(
            go_field_type(&duration_field(None)),
            "DurationMillis",
            "a plain Duration field is serialized by serde's derive, so it must round-trip \
             the {{\"secs\",\"nanos\"}} object shape"
        );
    }

    #[test]
    fn duration_field_with_serde_with_falls_back_to_a_bare_integer() {
        assert_eq!(
            go_field_type(&duration_field(Some("duration_ms"))),
            "uint64",
            "`#[serde(with = \"duration_ms\")]` writes a bare millisecond integer; emitting \
             DurationMillis would send an object and fail Rust deserialization with \
             `invalid type: map, expected u64`"
        );
    }

    #[test]
    fn optional_duration_field_without_serde_with_keeps_the_derived_object_shape() {
        assert_eq!(go_optional_field_type(&duration_field(None)), "*DurationMillis");
    }

    #[test]
    fn optional_duration_field_with_serde_with_falls_back_to_a_bare_integer() {
        assert_eq!(
            go_optional_field_type(&duration_field(Some("option_duration_ms"))),
            "*uint64",
            "the optional path must honor the custom codec exactly like the required path"
        );
    }

    #[test]
    fn serde_with_on_a_non_duration_field_changes_nothing() {
        let field = FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            serde_with: Some("custom".to_string()),
            serde_skip_serializing_if: false,
            serde_skip: false,
            ..Default::default()
        };
        assert_eq!(
            go_field_type(&field),
            go_type(&TypeRef::String),
            "serde_with only suppresses the Duration derive-shape wrapper; no other type is \
             special-cased, so the mapping must be untouched"
        );
    }
}
