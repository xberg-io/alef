//! Language-specific default value emission for trait bridge method returns.
//!
//! This module provides per-language implementations of default values for types
//! returned by trait methods. When a trait bridge emits a stub method that returns
//! a type, it needs to return a sensible default value for each language.
//!
//! The `LanguageDefaults` trait abstracts this to allow each language backend to
//! specify how to construct defaults for all TypeRef variants.

use crate::backends::go::type_map::GoMapper;
use crate::codegen::type_mapper::TypeMapper as _;
use crate::core::ir::{PrimitiveType, TypeRef};

/// Trait for emitting language-native default values given a type reference.
pub trait LanguageDefaults: Send + Sync {
    /// Emit the language's default value expression for the given type.
    ///
    /// Returns a string that is a valid expression in the target language,
    /// constructing an empty or default instance of the type. For example:
    ///
    /// - Rust: `Default::default()`, `String::new()`, `Ok(Default::default())`
    /// - Python: `ExtractionResult()`, `[]`, `None`
    /// - TypeScript: `new ExtractionResult()`, `[]`, `null`
    /// - Go: `&ExtractionResult{}`, `[]string{}`, `nil`
    ///
    /// The returned expression should never panic/crash at runtime, even if
    /// called multiple times, and should represent a valid "empty" state
    /// for the type.
    fn emit_default(&self, ty: &TypeRef) -> String;
}

/// Get the language-specific defaults emitter.
pub fn language_defaults(language: &str) -> Box<dyn LanguageDefaults> {
    match language {
        "rust" => Box::new(RustDefaults),
        "python" => Box::new(PythonDefaults),
        "typescript" | "wasm" | "node" => Box::new(TypeScriptDefaults),
        "go" => Box::new(GoDefaults),
        "java" => Box::new(JavaDefaults),
        "kotlin" => Box::new(KotlinDefaults),
        "kotlin_android" => Box::new(KotlinDefaults),
        "csharp" => Box::new(CSharpDefaults),
        "php" => Box::new(PhpDefaults),
        "ruby" => Box::new(RubyDefaults),
        "elixir" => Box::new(ElixirDefaults),
        "gleam" => Box::new(GleamDefaults),
        "r" => Box::new(RDefaults),
        "c" => Box::new(CDefaults),
        "zig" => Box::new(ZigDefaults),
        "dart" => Box::new(DartDefaults),
        "swift" => Box::new(SwiftDefaults),
        _ => Box::new(RustDefaults),
    }
}

fn prim_default(p: &PrimitiveType) -> (&'static str, &'static str) {
    match p {
        PrimitiveType::Bool => ("false", "False"),
        PrimitiveType::I8
        | PrimitiveType::I16
        | PrimitiveType::I32
        | PrimitiveType::I64
        | PrimitiveType::U8
        | PrimitiveType::U16
        | PrimitiveType::U32
        | PrimitiveType::U64
        | PrimitiveType::Isize
        | PrimitiveType::Usize => ("0", "0"),
        PrimitiveType::F32 | PrimitiveType::F64 => ("0.0", "0.0"),
    }
}

struct RustDefaults;
impl LanguageDefaults for RustDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => prim_default(p).0.to_string(),
            TypeRef::String => "String::new()".to_string(),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(..) => "std::collections::HashMap::new()".to_string(),
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Named(name) => format!("{}::default()", name),
            TypeRef::Unit => "()".to_string(),
            TypeRef::Json => "serde_json::json!({{}})".to_string(),
            TypeRef::Duration => "std::time::Duration::from_secs(0)".to_string(),
            TypeRef::Char => "'\\0'".to_string(),
            TypeRef::Path => "Default::default()".to_string(),
        }
    }
}

struct PythonDefaults;
impl LanguageDefaults for PythonDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => {
                let (_, python_val) = prim_default(p);
                python_val.to_string()
            }
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "b\"\"".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "{}".to_string(),
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Named(name) => format!("{}()", name),
            TypeRef::Unit => "None".to_string(),
            TypeRef::Json => "{}".to_string(),
            TypeRef::Duration => "0.0".to_string(),
            TypeRef::Char => "\"\"".to_string(),
            TypeRef::Path => "None".to_string(),
        }
    }
}

struct TypeScriptDefaults;
impl LanguageDefaults for TypeScriptDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "new Uint8Array()".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "{}".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(name) => format!("new {}()", name),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => "{}".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "\"\"".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct GoDefaults;
impl LanguageDefaults for GoDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "[]byte{}".to_string(),
            TypeRef::Vec(inner) => format!("([]{})(nil)", GoMapper.map_type(inner)),
            TypeRef::Map(..) => "nil".to_string(),
            TypeRef::Optional(_) => "nil".to_string(),
            TypeRef::Named(name) => format!("{}{{}}", name),
            TypeRef::Unit => "nil".to_string(),
            TypeRef::Json => "json.RawMessage(nil)".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "rune(0)".to_string(),
            TypeRef::Path => "nil".to_string(),
        }
    }
}

struct JavaDefaults;
impl LanguageDefaults for JavaDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "new byte[]{}".to_string(),
            TypeRef::Vec(_) => "new java.util.ArrayList<>()".to_string(),
            TypeRef::Map(..) => "new java.util.HashMap<>()".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(name) => format!("new {}()", name),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => "new JSONObject()".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "'\\0'".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct KotlinDefaults;
impl LanguageDefaults for KotlinDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "byteArrayOf()".to_string(),
            TypeRef::Vec(_) => "emptyList()".to_string(),
            TypeRef::Map(..) => "emptyMap()".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(name) => format!("{}()", name),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => "JSONObject()".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "'\\0'".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct CSharpDefaults;
impl LanguageDefaults for CSharpDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "Array.Empty<byte>()".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "new Dictionary<string, object>()".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(name) => format!("new {}()", name),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => "new JObject()".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "'\\0'".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct PhpDefaults;
impl LanguageDefaults for PhpDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "''".to_string(),
            TypeRef::Bytes => "''".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "[]".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(name) => format!("new {}()", name),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => "[]".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "''".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct RubyDefaults;
impl LanguageDefaults for RubyDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "''".to_string(),
            TypeRef::Bytes => "''".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "{}".to_string(),
            TypeRef::Optional(_) => "nil".to_string(),
            TypeRef::Named(name) => format!("{}.new", name),
            TypeRef::Unit => "nil".to_string(),
            TypeRef::Json => "{}".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "''".to_string(),
            TypeRef::Path => "nil".to_string(),
        }
    }
}

struct ElixirDefaults;
impl LanguageDefaults for ElixirDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "<<>>".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "%{}".to_string(),
            TypeRef::Optional(_) => "nil".to_string(),
            TypeRef::Named(_name) => "%{}".to_string(),
            TypeRef::Unit => "nil".to_string(),
            TypeRef::Json => "%{}".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "\"\"".to_string(),
            TypeRef::Path => "nil".to_string(),
        }
    }
}

struct GleamDefaults;
impl LanguageDefaults for GleamDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "False".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "<<>>".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "dict.new()".to_string(),
            TypeRef::Optional(_) => "Nil".to_string(),
            TypeRef::Named(name) => name.clone(),
            TypeRef::Unit => "Nil".to_string(),
            TypeRef::Json => "dict.new()".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "\"\"".to_string(),
            TypeRef::Path => "Nil".to_string(),
        }
    }
}

struct RDefaults;
impl LanguageDefaults for RDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "FALSE".to_string(),
            TypeRef::Primitive(_) => "0L".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "raw(NULL)".to_string(),
            TypeRef::Vec(_) => "c()".to_string(),
            TypeRef::Map(..) => "list()".to_string(),
            TypeRef::Optional(_) => "NULL".to_string(),
            TypeRef::Named(name) => format!("{}()", name),
            TypeRef::Unit => "NULL".to_string(),
            TypeRef::Json => "list()".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "\"\"".to_string(),
            TypeRef::Path => "NULL".to_string(),
        }
    }
}

struct CDefaults;
impl LanguageDefaults for CDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "NULL".to_string(),
            TypeRef::Vec(_) => "NULL".to_string(),
            TypeRef::Map(..) => "NULL".to_string(),
            TypeRef::Optional(_) => "NULL".to_string(),
            TypeRef::Named(name) => format!("{{ 0 }}  /* zero-init {} */", name),
            TypeRef::Unit => "NULL".to_string(),
            TypeRef::Json => "NULL".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "'\\0'".to_string(),
            TypeRef::Path => "NULL".to_string(),
        }
    }
}

struct ZigDefaults;
impl LanguageDefaults for ZigDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "\"\"".to_string(),
            TypeRef::Vec(_) => "&.{}".to_string(),
            TypeRef::Map(..) => ".{}".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(_name) => "undefined".to_string(),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => ".{}".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Char => "'\\0'".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct DartDefaults;
impl LanguageDefaults for DartDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "''".to_string(),
            TypeRef::Bytes => "Uint8List(0)".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "{}".to_string(),
            TypeRef::Optional(_) => "null".to_string(),
            TypeRef::Named(name) => format!("{}()", name),
            TypeRef::Unit => "null".to_string(),
            TypeRef::Json => "{}".to_string(),
            TypeRef::Duration => "const Duration(seconds: 0)".to_string(),
            TypeRef::Char => "''".to_string(),
            TypeRef::Path => "null".to_string(),
        }
    }
}

struct SwiftDefaults;
impl LanguageDefaults for SwiftDefaults {
    fn emit_default(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "0".to_string(),
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes => "Data()".to_string(),
            TypeRef::Vec(_) => "[]".to_string(),
            TypeRef::Map(..) => "[:]".to_string(),
            TypeRef::Optional(_) => "nil".to_string(),
            TypeRef::Named(name) => format!("{}()", name),
            TypeRef::Unit => "()".to_string(),
            TypeRef::Json => "[:]".to_string(),
            TypeRef::Duration => ".zero".to_string(),
            TypeRef::Char => "\"\"".to_string(),
            TypeRef::Path => "nil".to_string(),
        }
    }
}
