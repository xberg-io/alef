use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::PrimitiveType;
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

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    /// Map enum types to their PHP representation.
    /// - Unit-variant enums → `String` (paired with generated string constants).
    /// - Tagged data enums (struct variants) → their own flat PHP class name.
    /// - Struct (class) types pass through unchanged.
    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.data_enum_names.contains(name) {
            // Data enum: maps to the flat PHP class with the same name.
            Cow::Borrowed(name)
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
