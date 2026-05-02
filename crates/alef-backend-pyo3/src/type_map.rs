use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{PrimitiveType, TypeRef};
use std::borrow::Cow;

/// TypeMapper for PyO3 bindings — uses Rust defaults except for Json and trait types.
///
/// Trait types cannot cross the PyO3 FFI boundary as bare trait names (E0782) or as
/// `Arc<dyn Trait>` (not `#[pyclass]`-compatible). They are mapped to `Py<PyAny>` so
/// that Python objects implementing the trait protocol can be passed through.
pub struct Pyo3Mapper {
    /// Names of types in the IR that are trait definitions (`TypeDef::is_trait == true`).
    /// When a `TypeRef::Named(name)` matches one of these, the generated parameter type
    /// becomes `Py<PyAny>` instead of the bare name, avoiding E0782.
    pub trait_type_names: AHashSet<String>,
}

impl Pyo3Mapper {
    /// Create a mapper with no trait type awareness (used when trait info is unavailable).
    pub fn new() -> Self {
        Self {
            trait_type_names: AHashSet::new(),
        }
    }
}

impl Default for Pyo3Mapper {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeMapper for Pyo3Mapper {
    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String") // JSON as string, user deserializes
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.trait_type_names.contains(name) {
            // Trait objects cannot be used as bare types (E0782) and cannot cross the
            // PyO3 FFI boundary as `Arc<dyn Trait>`. Map to `Arc<Py<PyAny>>` so Python
            // objects implementing the trait protocol can be passed through as opaque handles.
            // Arc wrapping makes the type Clone (Py<PyAny> is not Clone, but Arc is).
            // This allows the binding struct to derive Clone without issue.
            Cow::Borrowed("Arc<Py<PyAny>>")
        } else {
            Cow::Borrowed(name)
        }
    }

    fn error_wrapper(&self) -> &str {
        "PyResult"
    }
}

/// Maps a TypeRef to its Python representation for .pyi stubs.
pub fn python_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => "int".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "float".to_string(),
        },
        TypeRef::String | TypeRef::Char => "str".to_string(),
        TypeRef::Bytes => "bytes".to_string(),
        TypeRef::Optional(inner) => format!("{} | None", python_type(inner)),
        TypeRef::Vec(inner) => format!("list[{}]", python_type(inner)),
        TypeRef::Map(k, v) => {
            format!("dict[{}, {}]", python_type(k), python_type(v))
        }
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "str".to_string(),
        TypeRef::Json => "dict[str, Any]".to_string(),
        TypeRef::Unit => "None".to_string(),
        TypeRef::Duration => "int".to_string(),
    }
}
