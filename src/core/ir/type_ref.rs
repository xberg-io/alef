use serde::{Deserialize, Serialize};

/// Reference to a type, with enough info for codegen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TypeRef {
    Primitive(PrimitiveType),
    String,
    /// Rust `char` — single Unicode character. Binding layer represents as single-char string.
    Char,
    Bytes,
    Optional(Box<TypeRef>),
    Vec(Box<TypeRef>),
    Map(Box<TypeRef>, Box<TypeRef>),
    Named(String),
    Path,
    #[default]
    Unit,
    Json,
    Duration,
}

impl TypeRef {
    /// Returns true if this type reference contains `Named(name)` at any depth.
    pub fn references_named(&self, name: &str) -> bool {
        match self {
            Self::Named(n) => n == name,
            Self::Optional(inner) | Self::Vec(inner) => inner.references_named(name),
            Self::Map(k, v) => k.references_named(name) || v.references_named(name),
            _ => false,
        }
    }
}

/// Rust primitive types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PrimitiveType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Usize,
    Isize,
}
