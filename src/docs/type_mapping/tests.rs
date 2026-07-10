use super::*;
use crate::core::config::Language;
use crate::docs::test_helpers::TEST_PREFIX;

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
    let vec_string = TypeRef::Vec(Box::new(TypeRef::String));
    assert_eq!(doc_type(&vec_string, Language::Swift, TEST_PREFIX), "[String]");
    assert_eq!(doc_type(&vec_string, Language::Dart, TEST_PREFIX), "List<String>");

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
    let ty = TypeRef::Named("ParseOptions".to_string());
    assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ParseOptions");
    assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "ParseOptions");
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HtmParseOptions");
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
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string())));
    assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ParseOptions | None");
    assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "Optional<ParseOptions>");
    assert_eq!(doc_type(&ty, Language::Csharp, TEST_PREFIX), "ParseOptions?");
    assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "*ParseOptions");
    assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "Option<ParseOptions>");
    assert_eq!(doc_type(&ty, Language::Ruby, TEST_PREFIX), "ParseOptions?");
    assert_eq!(doc_type(&ty, Language::Php, TEST_PREFIX), "?ParseOptions");
    assert_eq!(doc_type(&ty, Language::Elixir, TEST_PREFIX), "ParseOptions | nil");
    assert_eq!(doc_type(&ty, Language::R, TEST_PREFIX), "ParseOptions or NULL");
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
        java_boxed_type(&TypeRef::Named("ParseOptions".to_string())),
        "ParseOptions"
    );
    assert_eq!(java_boxed_type(&TypeRef::Duration), "Duration");
}
