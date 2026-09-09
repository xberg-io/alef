use serde::{Deserialize, Serialize};

/// Reference to a type, with enough info for codegen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TypeRef {
    Primitive(PrimitiveType),
    String,
    /// Rust `char` — single Unicode character. Binding layer represents as single-char string. ~keep
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
    /// Whether this return type crosses the C boundary through the byte-buffer out-param
    /// convention — trailing `(uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap)` plus an
    /// `int32_t` status return — rather than as a direct return value.
    ///
    /// This is the single source of truth for that ABI question, and it lives on the IR rather
    /// than in a backend because every backend must answer it identically or emit a declaration
    /// that disagrees with the header. It previously existed as several hand-restated copies,
    /// which disagreed:
    ///
    /// - the FFI backend, which emits the header and therefore defines the truth, matched `Bytes`
    ///   and `Optional<Bytes>` with no fallibility condition
    /// - C# matched bare `Bytes` only, so an `Optional<Bytes>` return fell through to the *string*
    ///   template: six native parameters declared as three, an `int32_t` status declared
    ///   pointer-width, and a call that dereferenced and freed a small integer as a pointer
    /// - Java additionally required `error_type.is_some()`, so an infallible bytes return missed
    ///   the same way
    /// - Go derives its call through cgo, which type-checks against the real header, which is why
    ///   Go was the one backend that could not get this wrong
    ///
    /// `Optional<Bytes>` shares one C signature with bare `Bytes`, encoding `None` as
    /// `*out_ptr == NULL`; absence rides on the pointer, not on a length or status sentinel. The
    /// optional wrapper therefore cannot change the answer, and a predicate that inspects only the
    /// outer constructor is wrong by construction. ~keep
    pub fn returns_bytes_out_params(&self) -> bool {
        match self {
            Self::Bytes => true,
            Self::Optional(inner) => matches!(inner.as_ref(), Self::Bytes),
            _ => false,
        }
    }

    /// Render this type as the Rust source text it stands for.
    ///
    /// Unlike the per-language mappers this performs no normalization and no naming policy: a
    /// `Named` leaf is emitted verbatim, including names that are not part of the binding
    /// surface. That is the point — it is used to capture a type *before* the sanitizer
    /// rewrites unbindable leaves to `String`, so the Rust-facing surfaces can still show what
    /// the source declared. ~keep
    pub fn rust_source_display(&self) -> String {
        match self {
            Self::Primitive(p) => p.rust_source_display().to_string(),
            Self::String => "String".to_string(),
            Self::Char => "char".to_string(),
            Self::Bytes => "Vec<u8>".to_string(),
            Self::Optional(inner) => format!("Option<{}>", inner.rust_source_display()),
            Self::Vec(inner) => format!("Vec<{}>", inner.rust_source_display()),
            Self::Map(key, value) => {
                format!(
                    "HashMap<{}, {}>",
                    key.rust_source_display(),
                    value.rust_source_display()
                )
            }
            Self::Named(name) => name.clone(),
            Self::Path => "PathBuf".to_string(),
            Self::Unit => "()".to_string(),
            Self::Json => "serde_json::Value".to_string(),
            Self::Duration => "Duration".to_string(),
        }
    }

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

impl PrimitiveType {
    pub fn rust_source_display(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Usize => "usize",
            Self::Isize => "isize",
        }
    }
}
