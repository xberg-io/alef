use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::AHashSet;
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
        Cow::Borrowed("String")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.trait_type_names.contains(name) {
            Cow::Borrowed("PyVisitorRef")
        } else if name == "Value" {
            Cow::Borrowed("serde_json::Value")
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

/// Maps a TypeRef to its Python representation for a value the host *returns* to a trait
/// bridge — a `Protocol` method the caller implements and PyO3 extracts from.
///
/// Numeric sequences widen to `Iterable[...]`. PyO3's `Vec<T>` extraction accepts any object
/// passing `PySequence_Check`, not just `list`, so annotating these as `list[...]` understates
/// the boundary and forces array-based implementations (NumPy and friends) through a `.tolist()`
/// the bridge never needed. Parameters keep [`python_type`], which describes what the bridge
/// actually passes in.
///
/// Only numeric leaves widen. `Iterable[str]` would admit a bare `str` — `str` is iterable and
/// yields `str` — which PyO3 explicitly rejects (`Can't extract \`str\` to \`Vec\``), so widening
/// a `Vec<String>` return would delete a static check that catches a real mistake. `str` is not
/// an `Iterable[float]`, so the numeric case has no such hole, and it is the only case with an
/// array form to accommodate.
pub fn python_callback_return_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Vec(inner) if has_numeric_leaf(inner) => {
            format!("Iterable[{}]", python_callback_return_type(inner))
        }
        TypeRef::Optional(inner) => format!("{} | None", python_callback_return_type(inner)),
        other => python_type(other),
    }
}

/// True when `ty` is a numeric scalar, or nests down to one through `Vec`.
fn has_numeric_leaf(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_) => true,
        TypeRef::Vec(inner) => has_numeric_leaf(inner),
        _ => false,
    }
}
