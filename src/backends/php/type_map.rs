use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::PrimitiveType;
use ahash::AHashSet;
use std::borrow::Cow;

/// TypeMapper for ext-php-rs bindings.
/// PHP integers are signed, so U64/Usize/Isize map to i64.
/// JSON is handled as String.
/// Enum named types map to String (ext-php-rs does not support Rust enums as PHP
/// types; they are represented as string constants instead).
pub struct PhpMapper {
    /// Names of unit-variant enum types. These are mapped to `String`.
    pub enum_names: AHashSet<String>,
    /// Names of tagged data enums (struct-variant). These are mapped to their own
    /// flat PHP class (same name) instead of `String`.
    pub data_enum_names: AHashSet<String>,
    /// Names of untagged data enums (`#[serde(untagged)]` with at least one
    /// data variant — e.g. `Single(String) | Multiple(Vec<String>)`).  These
    /// cannot be lowered to `String` (the wire shape may be a string OR an
    /// array OR an object) so they are mapped to `serde_json::Value` in the
    /// PHP binding struct, with conversion to the typed core enum done in
    /// `From<BindingT> for CoreT` via `serde_json::from_value`.
    pub untagged_data_enum_names: AHashSet<String>,
    /// Names of externally-tagged data enums (within enum_names). These have
    /// at least one variant with fields, requiring serde_json serialization on return.
    pub json_string_enum_names: AHashSet<String>,
}

impl TypeMapper for PhpMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "u8",
            PrimitiveType::U16 => "u16",
            PrimitiveType::U32 => "u32",
            PrimitiveType::U64 => "i64",
            PrimitiveType::I8 => "i8",
            PrimitiveType::I16 => "i16",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::F32 => "f32",
            PrimitiveType::F64 => "f64",
            PrimitiveType::Usize => "i64",
            PrimitiveType::Isize => "i64",
        })
    }

    /// Map `serde_json::Value` to itself in the PHP binding.
    /// We previously lowered JSON to `String`, but `serde::Deserialize` on the binding
    /// struct then chokes on incoming JSON objects/arrays whose target field is `String`
    /// (e.g. a tool's `parameters` schema).  Keeping the field as `serde_json::Value`
    /// lets `from_json` accept any wire shape; PHP-side access goes through a
    /// JSON-string getter (see `gen_struct_methods_impl`).
    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("serde_json::Value")
    }

    /// Map bytes type to Vec<u8> (PHP strings are binary-safe).
    /// ext-php-rs receives PHP binary strings and can work with them as Vec<u8>.
    /// From impls will convert core bytes::Bytes → binding Vec<u8> seamlessly.
    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("Vec<u8>")
    }

    /// Map enum types to their PHP representation.
    /// - Unit-variant enums → `String` (paired with generated string constants).
    /// - Tagged data enums (struct variants) → their own flat PHP class name.
    /// - Struct (class) types pass through unchanged.
    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.data_enum_names.contains(name) {
            Cow::Borrowed(name)
        } else if self.untagged_data_enum_names.contains(name) {
            Cow::Borrowed("serde_json::Value")
        } else if self.enum_names.contains(name) {
            Cow::Borrowed("String")
        } else {
            Cow::Borrowed(name)
        }
    }

    /// Duration maps to i64 in PHP (PHP integers are always signed).
    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("i64")
    }

    fn error_wrapper(&self) -> &str {
        "PhpResult"
    }
}
