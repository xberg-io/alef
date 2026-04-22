use alef_core::ir::{PrimitiveType, TypeRef};
use std::borrow::Cow;

/// Trait for mapping IR types to language-specific type strings.
/// Backends implement only what differs from the Rust default.
pub trait TypeMapper {
    /// Map a primitive type. Default: Rust type names.
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "u8",
            PrimitiveType::U16 => "u16",
            PrimitiveType::U32 => "u32",
            PrimitiveType::U64 => "u64",
            PrimitiveType::I8 => "i8",
            PrimitiveType::I16 => "i16",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::F32 => "f32",
            PrimitiveType::F64 => "f64",
            PrimitiveType::Usize => "usize",
            PrimitiveType::Isize => "isize",
        })
    }

    /// Map a string type. Default: "String"
    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    /// Map a bytes type. Default: "Vec<u8>"
    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("Vec<u8>")
    }

    /// Map a path type. Default: "String"
    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    /// Map a JSON type. Default: "serde_json::Value"
    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("serde_json::Value")
    }

    /// Map a unit type. Default: "()"
    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("()")
    }

    /// Map a duration type. Default: "u64" (seconds)
    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("u64")
    }

    /// Map an optional type. Default: "Option<T>"
    fn optional(&self, inner: &str) -> String {
        format!("Option<{inner}>")
    }

    /// Map a vec type. Default: "Vec<T>"
    fn vec(&self, inner: &str) -> String {
        format!("Vec<{inner}>")
    }

    /// Map a map type. Default: "HashMap<K, V>"
    fn map(&self, key: &str, value: &str) -> String {
        format!("HashMap<{key}, {value}>")
    }

    /// Map a named type. Default: identity.
    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(name)
    }

    /// Map a full TypeRef. Typically not overridden.
    fn map_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => self.primitive(p).into_owned(),
            TypeRef::String | TypeRef::Char => self.string().into_owned(),
            TypeRef::Bytes => self.bytes().into_owned(),
            TypeRef::Path => self.path().into_owned(),
            TypeRef::Json => self.json().into_owned(),
            TypeRef::Unit => self.unit().into_owned(),
            TypeRef::Optional(inner) => self.optional(&self.map_type(inner)),
            TypeRef::Vec(inner) => self.vec(&self.map_type(inner)),
            TypeRef::Map(k, v) => self.map(&self.map_type(k), &self.map_type(v)),
            TypeRef::Named(name) => self.named(name).into_owned(),
            TypeRef::Duration => self.duration().into_owned(),
        }
    }

    /// The error wrapper type for this language. e.g. "PyResult", "napi::Result", "PhpResult"
    fn error_wrapper(&self) -> &str;

    /// Wrap a return type with error handling if needed.
    fn wrap_return(&self, base: &str, has_error: bool) -> String {
        if has_error {
            format!("{}<{base}>", self.error_wrapper())
        } else {
            base.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal concrete implementation of `TypeMapper` using the default Rust mappings.
    struct RustMapper;

    impl TypeMapper for RustMapper {
        fn error_wrapper(&self) -> &str {
            "Result"
        }
    }

    // -------------------------------------------------------------------------
    // map_type — default (Rust) mappings for every TypeRef variant
    // -------------------------------------------------------------------------

    #[test]
    fn test_map_type_primitive_bool() {
        assert_eq!(RustMapper.map_type(&TypeRef::Primitive(PrimitiveType::Bool)), "bool");
    }

    #[test]
    fn test_map_type_primitive_integers() {
        let mapper = RustMapper;
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::U8)), "u8");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::U16)), "u16");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::U32)), "u32");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::U64)), "u64");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::I8)), "i8");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::I16)), "i16");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::I32)), "i32");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::I64)), "i64");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::Usize)), "usize");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::Isize)), "isize");
    }

    #[test]
    fn test_map_type_primitive_floats() {
        let mapper = RustMapper;
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::F32)), "f32");
        assert_eq!(mapper.map_type(&TypeRef::Primitive(PrimitiveType::F64)), "f64");
    }

    #[test]
    fn test_map_type_string_and_char() {
        let mapper = RustMapper;
        assert_eq!(mapper.map_type(&TypeRef::String), "String");
        assert_eq!(mapper.map_type(&TypeRef::Char), "String");
    }

    #[test]
    fn test_map_type_bytes() {
        assert_eq!(RustMapper.map_type(&TypeRef::Bytes), "Vec<u8>");
    }

    #[test]
    fn test_map_type_path() {
        assert_eq!(RustMapper.map_type(&TypeRef::Path), "String");
    }

    #[test]
    fn test_map_type_json() {
        assert_eq!(RustMapper.map_type(&TypeRef::Json), "serde_json::Value");
    }

    #[test]
    fn test_map_type_unit() {
        assert_eq!(RustMapper.map_type(&TypeRef::Unit), "()");
    }

    #[test]
    fn test_map_type_duration() {
        assert_eq!(RustMapper.map_type(&TypeRef::Duration), "u64");
    }

    #[test]
    fn test_map_type_named_identity() {
        assert_eq!(RustMapper.map_type(&TypeRef::Named("MyConfig".to_string())), "MyConfig");
    }

    #[test]
    fn test_map_type_optional_wraps_inner() {
        assert_eq!(
            RustMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32)))),
            "Option<u32>"
        );
    }

    #[test]
    fn test_map_type_optional_nested() {
        // Option<Option<String>>
        let ty = TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::String))));
        assert_eq!(RustMapper.map_type(&ty), "Option<Option<String>>");
    }

    #[test]
    fn test_map_type_vec_wraps_inner() {
        assert_eq!(
            RustMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::String))),
            "Vec<String>"
        );
    }

    #[test]
    fn test_map_type_vec_of_named() {
        assert_eq!(
            RustMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string())))),
            "Vec<Item>"
        );
    }

    #[test]
    fn test_map_type_map_string_to_u32() {
        assert_eq!(
            RustMapper.map_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::U32))
            )),
            "HashMap<String, u32>"
        );
    }

    #[test]
    fn test_map_type_nested_vec_in_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        assert_eq!(RustMapper.map_type(&ty), "Option<Vec<String>>");
    }

    // -------------------------------------------------------------------------
    // wrap_return — default implementation
    // -------------------------------------------------------------------------

    #[test]
    fn test_wrap_return_no_error_passes_through() {
        assert_eq!(RustMapper.wrap_return("String", false), "String");
    }

    #[test]
    fn test_wrap_return_with_error_wraps_in_error_wrapper() {
        assert_eq!(RustMapper.wrap_return("String", true), "Result<String>");
    }

    #[test]
    fn test_wrap_return_unit_with_error() {
        assert_eq!(RustMapper.wrap_return("()", true), "Result<()>");
    }

    // -------------------------------------------------------------------------
    // Overriding individual methods affects map_type output
    // -------------------------------------------------------------------------

    struct CustomMapper;

    impl TypeMapper for CustomMapper {
        fn json(&self) -> Cow<'static, str> {
            Cow::Borrowed("JsValue")
        }

        fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
            Cow::Owned(format!("Js{name}"))
        }

        fn vec(&self, inner: &str) -> String {
            if inner.starts_with("Vec<") {
                "JsValue".to_string()
            } else {
                format!("Vec<{inner}>")
            }
        }

        fn error_wrapper(&self) -> &str {
            "JsResult"
        }
    }

    #[test]
    fn test_custom_mapper_json_override() {
        assert_eq!(CustomMapper.map_type(&TypeRef::Json), "JsValue");
    }

    #[test]
    fn test_custom_mapper_named_override() {
        assert_eq!(
            CustomMapper.map_type(&TypeRef::Named("Config".to_string())),
            "JsConfig"
        );
    }

    #[test]
    fn test_custom_mapper_nested_vec_override() {
        // Vec<Vec<String>> → outer vec gets inner "Vec<String>", which starts with "Vec<" → JsValue
        let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        assert_eq!(CustomMapper.map_type(&ty), "JsValue");
    }

    #[test]
    fn test_custom_mapper_wrap_return_with_error() {
        assert_eq!(CustomMapper.wrap_return("String", true), "JsResult<String>");
    }
}
