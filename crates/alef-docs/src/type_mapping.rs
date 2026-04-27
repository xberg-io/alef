use crate::naming::type_name;
use alef_core::config::Language;
use alef_core::ir::{PrimitiveType, TypeRef};

pub fn doc_type(ty: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => match lang {
            Language::Python => "str".to_string(),
            Language::Node | Language::Wasm => "string".to_string(),
            Language::Go => "string".to_string(),
            Language::Java => "String".to_string(),
            Language::Csharp => "string".to_string(),
            Language::Ruby => "String".to_string(),
            Language::Php => "string".to_string(),
            Language::Elixir => "String.t()".to_string(),
            Language::R => "character".to_string(),
            Language::Rust => "String".to_string(),
            Language::Ffi => "const char*".to_string(),
            Language::Kotlin | Language::Swift | Language::Dart => "String".to_string(),
            Language::Gleam => "String".to_string(),
            Language::Zig => "[:0]const u8".to_string(),
        },
        TypeRef::Bytes => match lang {
            Language::Python => "bytes".to_string(),
            Language::Node | Language::Wasm => "Buffer".to_string(),
            Language::Go => "[]byte".to_string(),
            Language::Java => "byte[]".to_string(),
            Language::Csharp => "byte[]".to_string(),
            Language::Ruby => "String".to_string(),
            Language::Php => "string".to_string(),
            Language::Elixir => "binary()".to_string(),
            Language::R => "raw".to_string(),
            Language::Rust => "Vec<u8>".to_string(),
            Language::Ffi => "const uint8_t*".to_string(),
            Language::Kotlin => "ByteArray".to_string(),
            Language::Swift => "Data".to_string(),
            Language::Dart => "Uint8List".to_string(),
            Language::Gleam => "BitArray".to_string(),
            Language::Zig => "[]const u8".to_string(),
        },
        TypeRef::Primitive(p) => doc_primitive(p, lang),
        TypeRef::Optional(inner) => {
            let inner_ty = doc_type(inner, lang, ffi_prefix);
            match lang {
                Language::Python => format!("{inner_ty} | None"),
                Language::Node | Language::Wasm => format!("{inner_ty} | null"),
                Language::Go => format!("*{inner_ty}"),
                Language::Java => {
                    let boxed = java_boxed_type(inner);
                    format!("Optional<{boxed}>")
                }
                Language::Csharp => format!("{inner_ty}?"),
                Language::Ruby => format!("{inner_ty}?"),
                Language::Php => format!("?{inner_ty}"),
                Language::Elixir => format!("{inner_ty} | nil"),
                Language::R => format!("{inner_ty} or NULL"),
                Language::Rust => format!("Option<{inner_ty}>"),
                Language::Ffi => format!("{inner_ty}*"),
                Language::Kotlin | Language::Swift | Language::Dart => format!("{inner_ty}?"),
                Language::Gleam => format!("Option({inner_ty})"),
                Language::Zig => format!("?{inner_ty}"),
            }
        }
        TypeRef::Vec(inner) => {
            match lang {
                Language::Java => {
                    // Java generics can't use primitives — box them
                    let inner_ty = java_boxed_type(inner);
                    format!("List<{inner_ty}>")
                }
                Language::Csharp => {
                    let inner_ty = doc_type(inner, lang, ffi_prefix);
                    format!("List<{inner_ty}>")
                }
                _ => {
                    let inner_ty = doc_type(inner, lang, ffi_prefix);
                    match lang {
                        Language::Python => format!("list[{inner_ty}]"),
                        Language::Node | Language::Wasm => format!("Array<{inner_ty}>"),
                        Language::Go => format!("[]{inner_ty}"),
                        Language::Ruby => format!("Array<{inner_ty}>"),
                        Language::Php => format!("array<{inner_ty}>"),
                        Language::Elixir => format!("list({inner_ty})"),
                        Language::R => "list".to_string(),
                        Language::Rust => format!("Vec<{inner_ty}>"),
                        Language::Ffi => format!("{inner_ty}*"),
                        Language::Java | Language::Csharp => unreachable!(),
                        Language::Kotlin | Language::Dart => format!("List<{inner_ty}>"),
                        Language::Swift => format!("[{inner_ty}]"),
                        Language::Gleam => format!("List({inner_ty})"),
                        Language::Zig => format!("[]const {inner_ty}"),
                    }
                }
            }
        }
        TypeRef::Map(k, v) => {
            if lang == Language::Java {
                // Java generics require boxed types
                let kty = java_boxed_type(k);
                let vty = java_boxed_type(v);
                return format!("Map<{kty}, {vty}>");
            }
            let kty = doc_type(k, lang, ffi_prefix);
            let vty = doc_type(v, lang, ffi_prefix);
            match lang {
                Language::Python => format!("dict[{kty}, {vty}]"),
                Language::Node | Language::Wasm => format!("Record<{kty}, {vty}>"),
                Language::Go => format!("map[{kty}]{vty}"),
                Language::Java => format!("Map<{kty}, {vty}>"),
                Language::Csharp => format!("Dictionary<{kty}, {vty}>"),
                Language::Ruby => format!("Hash{{{kty}=>{vty}}}"),
                Language::Php => format!("array<{kty}, {vty}>"),
                Language::Elixir => "map()".to_string(),
                Language::R => "list".to_string(),
                Language::Rust => format!("HashMap<{kty}, {vty}>"),
                Language::Ffi => "void*".to_string(),
                Language::Kotlin => format!("Map<{kty}, {vty}>"),
                Language::Swift => format!("[{kty}: {vty}]"),
                Language::Dart => format!("Map<{kty}, {vty}>"),
                Language::Gleam => format!("Dict({kty}, {vty})"),
                Language::Zig => format!("std.StringHashMap({vty})"),
            }
        }
        TypeRef::Named(name) if name.starts_with('(') && name.ends_with(')') => {
            // Tuple type encoded as Named("(A, B)") — render idiomatically per language
            let inner = &name[1..name.len() - 1];
            let rendered: Vec<String> = inner
                .split(',')
                .map(|part| {
                    let trimmed = part.trim();
                    match trimmed {
                        "usize" | "u64" | "u32" | "u16" | "u8" | "i64" | "i32" | "i16" | "i8" | "isize" => {
                            // Swift preserves the signed/unsigned distinction; other
                            // languages collapse to a single integer type per their
                            // primitive convention.
                            let swift_name = match trimmed {
                                "u64" | "usize" => "UInt64",
                                "u32" => "UInt32",
                                "u16" => "UInt16",
                                "u8" => "UInt8",
                                "i64" | "isize" => "Int64",
                                "i32" => "Int32",
                                "i16" => "Int16",
                                "i8" => "Int8",
                                _ => "Int64",
                            };
                            match lang {
                                Language::Python => "int".to_string(),
                                Language::Node | Language::Wasm => "number".to_string(),
                                Language::Go => "int".to_string(),
                                Language::Java => "long".to_string(),
                                Language::Csharp => "long".to_string(),
                                Language::Ruby => "Integer".to_string(),
                                Language::Php => "int".to_string(),
                                Language::Elixir => "integer()".to_string(),
                                Language::R => "integer".to_string(),
                                Language::Rust => trimmed.to_string(),
                                Language::Ffi => "uint64_t".to_string(),
                                Language::Kotlin => "Long".to_string(),
                                Language::Swift => swift_name.to_string(),
                                Language::Dart => "int".to_string(),
                                Language::Gleam => "Int".to_string(),
                                Language::Zig => "i64".to_string(),
                            }
                        }
                        s @ ("str" | "&str" | "String" | "&'static str" | "&'staticstr") => match lang {
                            Language::Python => "str".to_string(),
                            Language::Node | Language::Wasm => "string".to_string(),
                            Language::Go => "string".to_string(),
                            Language::Java => "String".to_string(),
                            Language::Csharp => "string".to_string(),
                            Language::Ruby => "String".to_string(),
                            Language::Php => "string".to_string(),
                            Language::Elixir => "String.t()".to_string(),
                            Language::R => "character".to_string(),
                            Language::Rust => s.to_string(),
                            Language::Ffi => "const char*".to_string(),
                            Language::Kotlin | Language::Swift | Language::Dart => "String".to_string(),
                            Language::Gleam => "String".to_string(),
                            Language::Zig => "[]const u8".to_string(),
                        },
                        // Slice of strings — &[&str], &'static [&'static str], Vec<String>, etc.
                        // Also covers compacted IR forms like &'static[&'staticstr]
                        s if s.contains("[&")
                            || s.contains("[String")
                            || s.contains("Vec<&")
                            || s.contains("Vec<String")
                            || s.contains("staticstr") =>
                        {
                            match lang {
                                Language::Python => "list[str]".to_string(),
                                Language::Node | Language::Wasm => "string[]".to_string(),
                                Language::Go => "[]string".to_string(),
                                Language::Java => "List<String>".to_string(),
                                Language::Csharp => "List<string>".to_string(),
                                Language::Ruby => "Array<String>".to_string(),
                                Language::Php => "array<string>".to_string(),
                                Language::Elixir => "list(String.t())".to_string(),
                                Language::R => "list".to_string(),
                                Language::Rust => s.to_string(),
                                Language::Ffi => "const char**".to_string(),
                                Language::Kotlin | Language::Swift | Language::Dart => "List<String>".to_string(),
                                Language::Gleam => "List(String)".to_string(),
                                Language::Zig => "[]const []const u8".to_string(),
                            }
                        }
                        other => {
                            // For Rust, preserve the raw type token rather than
                            // PascalCasing it — Rust type names are already correct.
                            if lang == Language::Rust {
                                other.to_string()
                            } else {
                                type_name(other, lang, ffi_prefix)
                            }
                        }
                    }
                })
                .collect();
            match lang {
                Language::Python => format!("tuple[{}]", rendered.join(", ")),
                Language::Node | Language::Wasm => format!("[{}]", rendered.join(", ")),
                Language::Go => format!("({})", rendered.join(", ")),
                Language::Java => format!("Tuple<{}>", rendered.join(", ")),
                Language::Csharp => format!("({})", rendered.join(", ")),
                Language::Ruby => format!("[{}]", rendered.join(", ")),
                Language::Php => format!("array{{{}}}", rendered.join(", ")),
                Language::Elixir => format!("{{{}}}", rendered.join(", ")),
                Language::R => "list".to_string(),
                Language::Rust => format!("({})", rendered.join(", ")),
                Language::Ffi => "void*".to_string(),
                Language::Kotlin => format!("Pair<{}>", rendered.join(", ")),
                Language::Swift => format!("({})", rendered.join(", ")),
                Language::Dart => format!("({})", rendered.join(", ")),
                Language::Gleam => format!("#({})", rendered.join(", ")),
                Language::Zig => format!("struct {{ {} }}", rendered.join(", ")),
            }
        }
        TypeRef::Named(name) => type_name(name, lang, ffi_prefix),
        TypeRef::Path => match lang {
            Language::Python => "str".to_string(),
            Language::Node | Language::Wasm => "string".to_string(),
            Language::Go => "string".to_string(),
            Language::Java => "String".to_string(),
            Language::Csharp => "string".to_string(),
            Language::Ruby => "String".to_string(),
            Language::Php => "string".to_string(),
            Language::Elixir => "String.t()".to_string(),
            Language::R => "character".to_string(),
            Language::Rust => "PathBuf".to_string(),
            Language::Ffi => "const char*".to_string(),
            Language::Kotlin => "Path".to_string(),
            Language::Swift => "URL".to_string(),
            Language::Dart => "String".to_string(),
            Language::Gleam => "String".to_string(),
            Language::Zig => "[:0]const u8".to_string(),
        },
        TypeRef::Unit => match lang {
            Language::Python => "None".to_string(),
            Language::Node | Language::Wasm => "void".to_string(),
            Language::Go => "".to_string(),
            Language::Java => "void".to_string(),
            Language::Csharp => "void".to_string(),
            Language::Ruby => "nil".to_string(),
            Language::Php => "void".to_string(),
            Language::Elixir => ":ok".to_string(),
            Language::R => "NULL".to_string(),
            Language::Rust => "()".to_string(),
            Language::Ffi => "void".to_string(),
            Language::Kotlin => "Unit".to_string(),
            Language::Swift => "Void".to_string(),
            Language::Dart => "void".to_string(),
            Language::Gleam => "Nil".to_string(),
            Language::Zig => "void".to_string(),
        },
        TypeRef::Json => match lang {
            Language::Python => "dict[str, Any]".to_string(),
            Language::Node | Language::Wasm => "unknown".to_string(),
            Language::Go => "interface{}".to_string(),
            Language::Java => "Object".to_string(),
            Language::Csharp => "object".to_string(),
            Language::Ruby => "Object".to_string(),
            Language::Php => "mixed".to_string(),
            Language::Elixir => "term()".to_string(),
            Language::R => "list".to_string(),
            Language::Rust => "serde_json::Value".to_string(),
            Language::Ffi => "void*".to_string(),
            Language::Kotlin => "Any".to_string(),
            // Swift and Dart mappers return "String" — JSON is passed serialized.
            Language::Swift => "String".to_string(),
            Language::Dart => "String".to_string(),
            // Gleam and Zig backends serialize JSON as a string (the Mappers
            // return "String" / "[:0]const u8"); doc names must match.
            Language::Gleam => "String".to_string(),
            Language::Zig => "[:0]const u8".to_string(),
        },
        TypeRef::Duration => match lang {
            Language::Python => "float".to_string(),
            Language::Node | Language::Wasm => "number".to_string(),
            Language::Go => "time.Duration".to_string(),
            Language::Java => "Duration".to_string(),
            Language::Csharp => "TimeSpan".to_string(),
            Language::Ruby => "Float".to_string(),
            Language::Php => "float".to_string(),
            Language::Elixir => "integer()".to_string(),
            Language::R => "numeric".to_string(),
            Language::Rust => "std::time::Duration".to_string(),
            Language::Ffi => "uint64_t".to_string(),
            Language::Kotlin => "Duration".to_string(),
            Language::Swift => "Duration".to_string(),
            Language::Dart => "Duration".to_string(),
            Language::Gleam => "Int".to_string(),
            Language::Zig => "i64".to_string(),
        },
    }
}

pub(crate) fn doc_primitive(p: &PrimitiveType, lang: Language) -> String {
    match lang {
        Language::Python => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "float".to_string(),
            _ => "int".to_string(),
        },
        Language::Node | Language::Wasm => match p {
            PrimitiveType::Bool => "boolean".to_string(),
            _ => "number".to_string(),
        },
        Language::Go => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "uint8".to_string(),
            PrimitiveType::U16 => "uint16".to_string(),
            PrimitiveType::U32 => "uint32".to_string(),
            PrimitiveType::U64 => "uint64".to_string(),
            PrimitiveType::I8 => "int8".to_string(),
            PrimitiveType::I16 => "int16".to_string(),
            PrimitiveType::I32 => "int32".to_string(),
            PrimitiveType::I64 => "int64".to_string(),
            PrimitiveType::F32 => "float32".to_string(),
            PrimitiveType::F64 => "float64".to_string(),
            PrimitiveType::Usize | PrimitiveType::Isize => "int".to_string(),
        },
        Language::Java => match p {
            PrimitiveType::Bool => "boolean".to_string(),
            PrimitiveType::U8 | PrimitiveType::I8 => "byte".to_string(),
            PrimitiveType::U16 | PrimitiveType::I16 => "short".to_string(),
            PrimitiveType::U32 | PrimitiveType::I32 => "int".to_string(),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "long".to_string(),
            PrimitiveType::F32 => "float".to_string(),
            PrimitiveType::F64 => "double".to_string(),
        },
        Language::Csharp => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "byte".to_string(),
            PrimitiveType::U16 => "ushort".to_string(),
            PrimitiveType::U32 => "uint".to_string(),
            PrimitiveType::U64 => "ulong".to_string(),
            PrimitiveType::I8 => "sbyte".to_string(),
            PrimitiveType::I16 => "short".to_string(),
            PrimitiveType::I32 => "int".to_string(),
            PrimitiveType::I64 => "long".to_string(),
            PrimitiveType::Usize => "nuint".to_string(),
            PrimitiveType::Isize => "nint".to_string(),
            PrimitiveType::F32 => "float".to_string(),
            PrimitiveType::F64 => "double".to_string(),
        },
        Language::Ruby => match p {
            PrimitiveType::Bool => "Boolean".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_string(),
            _ => "Integer".to_string(),
        },
        Language::Php => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "float".to_string(),
            _ => "int".to_string(),
        },
        Language::Elixir => match p {
            PrimitiveType::Bool => "boolean()".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "float()".to_string(),
            _ => "integer()".to_string(),
        },
        Language::R => match p {
            PrimitiveType::Bool => "logical".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "numeric".to_string(),
            _ => "integer".to_string(),
        },
        Language::Ffi => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "uint8_t".to_string(),
            PrimitiveType::U16 => "uint16_t".to_string(),
            PrimitiveType::U32 => "uint32_t".to_string(),
            PrimitiveType::U64 => "uint64_t".to_string(),
            PrimitiveType::I8 => "int8_t".to_string(),
            PrimitiveType::I16 => "int16_t".to_string(),
            PrimitiveType::I32 => "int32_t".to_string(),
            PrimitiveType::I64 => "int64_t".to_string(),
            PrimitiveType::Usize => "uintptr_t".to_string(),
            PrimitiveType::Isize => "intptr_t".to_string(),
            PrimitiveType::F32 => "float".to_string(),
            PrimitiveType::F64 => "double".to_string(),
        },
        Language::Rust => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            PrimitiveType::Usize => "usize".to_string(),
            PrimitiveType::Isize => "isize".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
        },
        Language::Kotlin => match p {
            PrimitiveType::Bool => "Boolean".to_string(),
            PrimitiveType::U8 | PrimitiveType::I8 => "Byte".to_string(),
            PrimitiveType::U16 | PrimitiveType::I16 => "Short".to_string(),
            PrimitiveType::U32 | PrimitiveType::I32 => "Int".to_string(),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long".to_string(),
            PrimitiveType::F32 => "Float".to_string(),
            PrimitiveType::F64 => "Double".to_string(),
        },
        Language::Swift => match p {
            PrimitiveType::Bool => "Bool".to_string(),
            PrimitiveType::U8 => "UInt8".to_string(),
            PrimitiveType::U16 => "UInt16".to_string(),
            PrimitiveType::U32 => "UInt32".to_string(),
            PrimitiveType::U64 | PrimitiveType::Usize => "UInt64".to_string(),
            PrimitiveType::I8 => "Int8".to_string(),
            PrimitiveType::I16 => "Int16".to_string(),
            PrimitiveType::I32 => "Int32".to_string(),
            PrimitiveType::I64 | PrimitiveType::Isize => "Int64".to_string(),
            PrimitiveType::F32 => "Float".to_string(),
            PrimitiveType::F64 => "Double".to_string(),
        },
        Language::Dart => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "double".to_string(),
            _ => "int".to_string(),
        },
        Language::Gleam => match p {
            PrimitiveType::Bool => "Bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_string(),
            _ => "Int".to_string(),
        },
        Language::Zig => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            // ZigMapper deliberately uses fixed-width types for FFI stability.
            PrimitiveType::Usize => "u64".to_string(),
            PrimitiveType::Isize => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
        },
    }
}

/// Return the boxed (object) type for Java generics.
///
/// Java generics cannot use primitive types (`int`, `long`, etc.); they require
/// the corresponding wrapper classes (`Integer`, `Long`, etc.).
pub(crate) fn java_boxed_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Boolean".to_string(),
            PrimitiveType::U8 | PrimitiveType::I8 => "Byte".to_string(),
            PrimitiveType::U16 | PrimitiveType::I16 => "Short".to_string(),
            PrimitiveType::U32 | PrimitiveType::I32 => "Integer".to_string(),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long".to_string(),
            PrimitiveType::F32 => "Float".to_string(),
            PrimitiveType::F64 => "Double".to_string(),
        },
        // Non-primitive types are already reference types in Java
        _ => doc_type(ty, Language::Java, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TEST_PREFIX;
    use alef_core::config::Language;

    #[test]
    fn test_doc_type_string() {
        assert_eq!(doc_type(&TypeRef::String, Language::Python, TEST_PREFIX), "str");
        assert_eq!(doc_type(&TypeRef::String, Language::Node, TEST_PREFIX), "string");
        assert_eq!(doc_type(&TypeRef::String, Language::Java, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::String, Language::Ffi, TEST_PREFIX), "const char*");
    }

    #[test]
    fn test_doc_type_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "str | None");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "string | null");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "*string");
        assert_eq!(doc_type(&ty, Language::Csharp, TEST_PREFIX), "string?");
    }

    #[test]
    fn test_doc_type_vec() {
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "list[str]");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "Array<string>");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "[]string");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "List<String>");
    }

    #[test]
    fn test_doc_type_primitives() {
        assert_eq!(
            doc_type(&TypeRef::Primitive(PrimitiveType::Bool), Language::Python, TEST_PREFIX),
            "bool"
        );
        assert_eq!(
            doc_type(&TypeRef::Primitive(PrimitiveType::Bool), Language::Node, TEST_PREFIX),
            "boolean"
        );
        assert_eq!(
            doc_type(&TypeRef::Primitive(PrimitiveType::U64), Language::Node, TEST_PREFIX),
            "number"
        );
        assert_eq!(
            doc_type(&TypeRef::Primitive(PrimitiveType::F64), Language::Python, TEST_PREFIX),
            "float"
        );
        assert_eq!(
            doc_type(&TypeRef::Primitive(PrimitiveType::U32), Language::Ffi, TEST_PREFIX),
            "uint32_t"
        );
    }

    #[test]
    fn test_doc_type_rust_static_str_in_named_tuple() {
        let ty = TypeRef::Named("(&'static str)".to_string());
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "(&'static str)");
    }

    #[test]
    fn test_doc_type_named_static_str_renders_correctly_for_non_rust() {
        let ty = TypeRef::Named("(&'static str, u32)".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "tuple[str, int]");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "[string, number]");
    }

    #[test]
    fn test_doc_type_static_slice_in_tuple_element_rust() {
        let ty = TypeRef::Named("(&'static [&'static str], u32)".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "tuple[list[str], int]");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "([]string, int)");
    }

    #[test]
    fn test_doc_type_char_maps_like_string() {
        for lang in [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Ruby,
            Language::Php,
            Language::Elixir,
            Language::R,
            Language::Rust,
            Language::Ffi,
        ] {
            assert_eq!(
                doc_type(&TypeRef::Char, lang, TEST_PREFIX),
                doc_type(&TypeRef::String, lang, TEST_PREFIX),
                "Char != String for {lang:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_bytes_all_languages() {
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Python, TEST_PREFIX), "bytes");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Node, TEST_PREFIX), "Buffer");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Go, TEST_PREFIX), "[]byte");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Java, TEST_PREFIX), "byte[]");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Csharp, TEST_PREFIX), "byte[]");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Ruby, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Rust, TEST_PREFIX), "Vec<u8>");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Ffi, TEST_PREFIX), "const uint8_t*");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Kotlin, TEST_PREFIX), "ByteArray");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Swift, TEST_PREFIX), "Data");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Dart, TEST_PREFIX), "Uint8List");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Gleam, TEST_PREFIX), "BitArray");
        assert_eq!(doc_type(&TypeRef::Bytes, Language::Zig, TEST_PREFIX), "[]const u8");
    }

    #[test]
    fn test_doc_type_string_kotlin_gleam_zig() {
        assert_eq!(doc_type(&TypeRef::String, Language::Kotlin, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::String, Language::Gleam, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::String, Language::Zig, TEST_PREFIX), "[:0]const u8");
    }

    #[test]
    fn test_doc_type_unit_all_languages() {
        assert_eq!(doc_type(&TypeRef::Unit, Language::Python, TEST_PREFIX), "None");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Node, TEST_PREFIX), "void");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Go, TEST_PREFIX), "");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Java, TEST_PREFIX), "void");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Csharp, TEST_PREFIX), "void");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Ruby, TEST_PREFIX), "nil");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Php, TEST_PREFIX), "void");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Elixir, TEST_PREFIX), ":ok");
        assert_eq!(doc_type(&TypeRef::Unit, Language::R, TEST_PREFIX), "NULL");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Rust, TEST_PREFIX), "()");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Ffi, TEST_PREFIX), "void");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Kotlin, TEST_PREFIX), "Unit");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Gleam, TEST_PREFIX), "Nil");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Zig, TEST_PREFIX), "void");
    }

    #[test]
    fn test_doc_type_path_all_languages() {
        assert_eq!(doc_type(&TypeRef::Path, Language::Python, TEST_PREFIX), "str");
        assert_eq!(doc_type(&TypeRef::Path, Language::Node, TEST_PREFIX), "string");
        assert_eq!(doc_type(&TypeRef::Path, Language::Go, TEST_PREFIX), "string");
        assert_eq!(doc_type(&TypeRef::Path, Language::Java, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Path, Language::Csharp, TEST_PREFIX), "string");
        assert_eq!(doc_type(&TypeRef::Path, Language::Ruby, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Path, Language::Php, TEST_PREFIX), "string");
        assert_eq!(doc_type(&TypeRef::Path, Language::Elixir, TEST_PREFIX), "String.t()");
        assert_eq!(doc_type(&TypeRef::Path, Language::R, TEST_PREFIX), "character");
        assert_eq!(doc_type(&TypeRef::Path, Language::Rust, TEST_PREFIX), "PathBuf");
        assert_eq!(doc_type(&TypeRef::Path, Language::Ffi, TEST_PREFIX), "const char*");
        assert_eq!(doc_type(&TypeRef::Path, Language::Kotlin, TEST_PREFIX), "Path");
        assert_eq!(doc_type(&TypeRef::Path, Language::Gleam, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Path, Language::Zig, TEST_PREFIX), "[:0]const u8");
    }

    #[test]
    fn test_doc_type_json_all_languages() {
        assert_eq!(
            doc_type(&TypeRef::Json, Language::Python, TEST_PREFIX),
            "dict[str, Any]"
        );
        assert_eq!(doc_type(&TypeRef::Json, Language::Node, TEST_PREFIX), "unknown");
        assert_eq!(doc_type(&TypeRef::Json, Language::Go, TEST_PREFIX), "interface{}");
        assert_eq!(doc_type(&TypeRef::Json, Language::Java, TEST_PREFIX), "Object");
        assert_eq!(doc_type(&TypeRef::Json, Language::Csharp, TEST_PREFIX), "object");
        assert_eq!(doc_type(&TypeRef::Json, Language::Ruby, TEST_PREFIX), "Object");
        assert_eq!(doc_type(&TypeRef::Json, Language::Php, TEST_PREFIX), "mixed");
        assert_eq!(doc_type(&TypeRef::Json, Language::Elixir, TEST_PREFIX), "term()");
        assert_eq!(doc_type(&TypeRef::Json, Language::R, TEST_PREFIX), "list");
        assert_eq!(
            doc_type(&TypeRef::Json, Language::Rust, TEST_PREFIX),
            "serde_json::Value"
        );
        assert_eq!(doc_type(&TypeRef::Json, Language::Ffi, TEST_PREFIX), "void*");
        assert_eq!(doc_type(&TypeRef::Json, Language::Kotlin, TEST_PREFIX), "Any");
        // SwiftMapper, DartMapper, GleamMapper, and ZigMapper all serialize JSON
        // as a string at the FFI boundary; doc names must match the mappers.
        assert_eq!(doc_type(&TypeRef::Json, Language::Swift, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Json, Language::Dart, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Json, Language::Gleam, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Json, Language::Zig, TEST_PREFIX), "[:0]const u8");
    }

    #[test]
    fn test_doc_type_duration_all_languages() {
        assert_eq!(doc_type(&TypeRef::Duration, Language::Python, TEST_PREFIX), "float");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Node, TEST_PREFIX), "number");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Go, TEST_PREFIX), "time.Duration");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Java, TEST_PREFIX), "Duration");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Csharp, TEST_PREFIX), "TimeSpan");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Ruby, TEST_PREFIX), "Float");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Php, TEST_PREFIX), "float");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Elixir, TEST_PREFIX), "integer()");
        assert_eq!(doc_type(&TypeRef::Duration, Language::R, TEST_PREFIX), "numeric");
        assert_eq!(
            doc_type(&TypeRef::Duration, Language::Rust, TEST_PREFIX),
            "std::time::Duration"
        );
        assert_eq!(doc_type(&TypeRef::Duration, Language::Ffi, TEST_PREFIX), "uint64_t");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Kotlin, TEST_PREFIX), "Duration");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Swift, TEST_PREFIX), "Duration");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Dart, TEST_PREFIX), "Duration");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Gleam, TEST_PREFIX), "Int");
        assert_eq!(doc_type(&TypeRef::Duration, Language::Zig, TEST_PREFIX), "i64");
    }

    #[test]
    fn test_doc_type_swift_dart_vec_and_map() {
        // Swift uses `[T]` syntactic sugar; Dart uses `List<T>` like Kotlin.
        let vec_string = TypeRef::Vec(Box::new(TypeRef::String));
        assert_eq!(doc_type(&vec_string, Language::Swift, TEST_PREFIX), "[String]");
        assert_eq!(doc_type(&vec_string, Language::Dart, TEST_PREFIX), "List<String>");

        // Swift dict literal: `[K: V]`. Dart: `Map<K, V>`.
        let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        assert_eq!(doc_type(&map, Language::Swift, TEST_PREFIX), "[String: String]");
        assert_eq!(doc_type(&map, Language::Dart, TEST_PREFIX), "Map<String, String>");
    }

    #[test]
    fn test_doc_type_swift_dart_path_and_unit() {
        assert_eq!(doc_type(&TypeRef::Path, Language::Swift, TEST_PREFIX), "URL");
        assert_eq!(doc_type(&TypeRef::Path, Language::Dart, TEST_PREFIX), "String");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Swift, TEST_PREFIX), "Void");
        assert_eq!(doc_type(&TypeRef::Unit, Language::Dart, TEST_PREFIX), "void");
    }

    #[test]
    fn test_doc_type_named_strips_module_path() {
        let ty = TypeRef::Named("my_crate::types::OutputFormat".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HtmOutputFormat");
    }

    #[test]
    fn test_doc_type_named_without_path() {
        let ty = TypeRef::Named("ConversionOptions".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HtmConversionOptions");
    }

    #[test]
    fn test_doc_type_map_string_to_string_all_languages() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "dict[str, str]");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "Record<string, string>");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "map[string]string");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "Map<String, String>");
        assert_eq!(
            doc_type(&ty, Language::Csharp, TEST_PREFIX),
            "Dictionary<string, string>"
        );
        assert_eq!(doc_type(&ty, Language::Ruby, TEST_PREFIX), "Hash{String=>String}");
        assert_eq!(doc_type(&ty, Language::Php, TEST_PREFIX), "array<string, string>");
        assert_eq!(doc_type(&ty, Language::Elixir, TEST_PREFIX), "map()");
        assert_eq!(doc_type(&ty, Language::R, TEST_PREFIX), "list");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "HashMap<String, String>");
        assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "void*");
    }

    #[test]
    fn test_doc_type_map_with_primitive_value_java_boxes() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::I32)),
        );
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "Map<String, Integer>");
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "dict[str, int]");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "HashMap<String, i32>");
    }

    #[test]
    fn test_doc_type_nested_vec_of_optional_string() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::String))));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "list[str | None]");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "Array<string | null>");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "[]*string");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "List<Optional<String>>");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "Vec<Option<String>>");
    }

    #[test]
    fn test_doc_type_nested_map_string_to_vec_u32() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)))),
        );
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "dict[str, list[int]]");
        assert_eq!(
            doc_type(&ty, Language::Node, TEST_PREFIX),
            "Record<string, Array<number>>"
        );
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "map[string][]uint32");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "Map<String, List<Integer>>");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "HashMap<String, Vec<u32>>");
    }

    #[test]
    fn test_doc_type_optional_of_named_all_languages() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("ConversionOptions".to_string())));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ConversionOptions | None");
        assert_eq!(
            doc_type(&ty, Language::Java, TEST_PREFIX),
            "Optional<ConversionOptions>"
        );
        assert_eq!(doc_type(&ty, Language::Csharp, TEST_PREFIX), "ConversionOptions?");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "*ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "Option<ConversionOptions>");
        assert_eq!(doc_type(&ty, Language::Ruby, TEST_PREFIX), "ConversionOptions?");
        assert_eq!(doc_type(&ty, Language::Php, TEST_PREFIX), "?ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Elixir, TEST_PREFIX), "ConversionOptions | nil");
        assert_eq!(doc_type(&ty, Language::R, TEST_PREFIX), "ConversionOptions or NULL");
    }

    #[test]
    fn test_doc_type_all_go_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "bool"),
            (PrimitiveType::U8, "uint8"),
            (PrimitiveType::U16, "uint16"),
            (PrimitiveType::U32, "uint32"),
            (PrimitiveType::U64, "uint64"),
            (PrimitiveType::I8, "int8"),
            (PrimitiveType::I16, "int16"),
            (PrimitiveType::I32, "int32"),
            (PrimitiveType::I64, "int64"),
            (PrimitiveType::F32, "float32"),
            (PrimitiveType::F64, "float64"),
            (PrimitiveType::Usize, "int"),
            (PrimitiveType::Isize, "int"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Go, TEST_PREFIX),
                *expected,
                "Go primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_java_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "boolean"),
            (PrimitiveType::U8, "byte"),
            (PrimitiveType::I8, "byte"),
            (PrimitiveType::U16, "short"),
            (PrimitiveType::I16, "short"),
            (PrimitiveType::U32, "int"),
            (PrimitiveType::I32, "int"),
            (PrimitiveType::U64, "long"),
            (PrimitiveType::I64, "long"),
            (PrimitiveType::Usize, "long"),
            (PrimitiveType::Isize, "long"),
            (PrimitiveType::F32, "float"),
            (PrimitiveType::F64, "double"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Java, TEST_PREFIX),
                *expected,
                "Java primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_csharp_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "bool"),
            (PrimitiveType::U8, "byte"),
            (PrimitiveType::U16, "ushort"),
            (PrimitiveType::U32, "uint"),
            (PrimitiveType::U64, "ulong"),
            (PrimitiveType::I8, "sbyte"),
            (PrimitiveType::I16, "short"),
            (PrimitiveType::I32, "int"),
            (PrimitiveType::I64, "long"),
            (PrimitiveType::Usize, "nuint"),
            (PrimitiveType::Isize, "nint"),
            (PrimitiveType::F32, "float"),
            (PrimitiveType::F64, "double"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Csharp, TEST_PREFIX),
                *expected,
                "C# primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_rust_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "bool"),
            (PrimitiveType::U8, "u8"),
            (PrimitiveType::U16, "u16"),
            (PrimitiveType::U32, "u32"),
            (PrimitiveType::U64, "u64"),
            (PrimitiveType::I8, "i8"),
            (PrimitiveType::I16, "i16"),
            (PrimitiveType::I32, "i32"),
            (PrimitiveType::I64, "i64"),
            (PrimitiveType::Usize, "usize"),
            (PrimitiveType::Isize, "isize"),
            (PrimitiveType::F32, "f32"),
            (PrimitiveType::F64, "f64"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Rust, TEST_PREFIX),
                *expected,
                "Rust primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_ffi_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "bool"),
            (PrimitiveType::U8, "uint8_t"),
            (PrimitiveType::U16, "uint16_t"),
            (PrimitiveType::U32, "uint32_t"),
            (PrimitiveType::U64, "uint64_t"),
            (PrimitiveType::I8, "int8_t"),
            (PrimitiveType::I16, "int16_t"),
            (PrimitiveType::I32, "int32_t"),
            (PrimitiveType::I64, "int64_t"),
            (PrimitiveType::Usize, "uintptr_t"),
            (PrimitiveType::Isize, "intptr_t"),
            (PrimitiveType::F32, "float"),
            (PrimitiveType::F64, "double"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Ffi, TEST_PREFIX),
                *expected,
                "FFI primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_kotlin_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "Boolean"),
            (PrimitiveType::U8, "Byte"),
            (PrimitiveType::I8, "Byte"),
            (PrimitiveType::U16, "Short"),
            (PrimitiveType::I16, "Short"),
            (PrimitiveType::U32, "Int"),
            (PrimitiveType::I32, "Int"),
            (PrimitiveType::U64, "Long"),
            (PrimitiveType::I64, "Long"),
            (PrimitiveType::Usize, "Long"),
            (PrimitiveType::Isize, "Long"),
            (PrimitiveType::F32, "Float"),
            (PrimitiveType::F64, "Double"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Kotlin, TEST_PREFIX),
                *expected,
                "Kotlin primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_gleam_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "Bool"),
            (PrimitiveType::U8, "Int"),
            (PrimitiveType::U64, "Int"),
            (PrimitiveType::I32, "Int"),
            (PrimitiveType::Usize, "Int"),
            (PrimitiveType::F32, "Float"),
            (PrimitiveType::F64, "Float"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Gleam, TEST_PREFIX),
                *expected,
                "Gleam primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_all_zig_primitives() {
        // ZigMapper deliberately maps Usize/Isize to fixed-width u64/i64 for
        // FFI stability — pin that choice so docs and the mapper don't drift.
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "bool"),
            (PrimitiveType::U8, "u8"),
            (PrimitiveType::U16, "u16"),
            (PrimitiveType::U32, "u32"),
            (PrimitiveType::U64, "u64"),
            (PrimitiveType::I8, "i8"),
            (PrimitiveType::I16, "i16"),
            (PrimitiveType::I32, "i32"),
            (PrimitiveType::I64, "i64"),
            (PrimitiveType::Usize, "u64"),
            (PrimitiveType::Isize, "i64"),
            (PrimitiveType::F32, "f32"),
            (PrimitiveType::F64, "f64"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                doc_type(&TypeRef::Primitive(prim.clone()), Language::Zig, TEST_PREFIX),
                *expected,
                "Zig primitive {prim:?}"
            );
        }
    }

    #[test]
    fn test_java_boxed_type_all_primitives() {
        let cases: &[(PrimitiveType, &str)] = &[
            (PrimitiveType::Bool, "Boolean"),
            (PrimitiveType::U8, "Byte"),
            (PrimitiveType::I8, "Byte"),
            (PrimitiveType::U16, "Short"),
            (PrimitiveType::I16, "Short"),
            (PrimitiveType::U32, "Integer"),
            (PrimitiveType::I32, "Integer"),
            (PrimitiveType::U64, "Long"),
            (PrimitiveType::I64, "Long"),
            (PrimitiveType::Usize, "Long"),
            (PrimitiveType::Isize, "Long"),
            (PrimitiveType::F32, "Float"),
            (PrimitiveType::F64, "Double"),
        ];
        for (prim, expected) in cases {
            assert_eq!(
                java_boxed_type(&TypeRef::Primitive(prim.clone())),
                *expected,
                "boxed Java type for {prim:?}"
            );
        }
    }

    #[test]
    fn test_java_boxed_type_non_primitives_delegate_to_java_doc_type() {
        assert_eq!(java_boxed_type(&TypeRef::String), "String");
        assert_eq!(java_boxed_type(&TypeRef::Bytes), "byte[]");
        assert_eq!(
            java_boxed_type(&TypeRef::Named("ConversionOptions".to_string())),
            "ConversionOptions"
        );
        assert_eq!(java_boxed_type(&TypeRef::Duration), "Duration");
    }
}
