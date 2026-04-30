use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::TypeRef;
use std::borrow::Cow;

/// TypeMapper for Rustler/Elixir NIFs — default Rust types with String for Json and Map.
pub struct RustlerMapper;

impl TypeMapper for RustlerMapper {
    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }

    /// Rustler wraps errors as `Result<T, String>`.
    fn wrap_return(&self, base: &str, has_error: bool) -> String {
        if has_error {
            format!("Result<{base}, String>")
        } else {
            base.to_string()
        }
    }

    /// All map types encode as a plain `String` in the Rustler NIF binding layer.
    /// `HashMap` cannot cross the NIF boundary directly, so maps are serialized
    /// via `format!("{:?}", ...)` in core→binding conversions.
    fn map_type(&self, ty: &TypeRef) -> String {
        if matches!(ty, TypeRef::Map(_, _)) {
            return "String".to_string();
        }
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
}
