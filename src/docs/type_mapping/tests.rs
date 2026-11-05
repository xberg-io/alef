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

/// ~keep A `Vec<Named>` (e.g. a batch-extraction parameter taking several handles) must
/// document the same JSON `const char*` the C ABI actually declares, not a real array of the
/// element type. `FfiParamMapper::vec`/`FfiReturnMapper::vec` (backends/ffi/type_map.rs)
/// return `*const`/`*mut std::ffi::c_char` for a `Vec` regardless of its element type -- there
/// is no per-element branch in either mapper. Reproduces a real-consumer symptom: the docs
/// showed `AlefHandle*` (a real handle array) for a batch parameter the emitted header takes
/// as one JSON string.
#[test]
fn test_doc_type_vec_of_named_ffi_is_json_string_not_a_handle_array() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("DocumentHandle".to_string())));
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "const char*");
    assert_eq!(doc_type(&ty, Language::C, TEST_PREFIX), "const char*");
}

/// ~keep Control: a `Vec<Primitive>` must ALSO document as the JSON `const char*` the ABI
/// declares -- guards against a fix that special-cased only the `Named`-element case and left
/// every other element type still rendering the old `{inner_ty}*` array shape.
#[test]
fn test_doc_type_vec_of_primitive_ffi_is_also_json_string() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "const char*");
}

/// ~keep Same JSON-string authority as the `Vec` case: `FfiParamMapper::map`/
/// `FfiReturnMapper::map` return `*const`/`*mut std::ffi::c_char`, not the `void*` this page
/// used to document.
#[test]
fn test_doc_type_map_ffi_is_json_string_not_void_pointer() {
    let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "const char*");
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
    assert_eq!(doc_type(&TypeRef::String, Language::Zig, TEST_PREFIX), "[]const u8");
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
    assert_eq!(doc_type(&TypeRef::Path, Language::Zig, TEST_PREFIX), "[]const u8");
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
    assert_eq!(doc_type(&TypeRef::Json, Language::Ffi, TEST_PREFIX), "const char*");
    assert_eq!(doc_type(&TypeRef::Json, Language::Kotlin, TEST_PREFIX), "Any");
    assert_eq!(doc_type(&TypeRef::Json, Language::Swift, TEST_PREFIX), "String");
    assert_eq!(doc_type(&TypeRef::Json, Language::Dart, TEST_PREFIX), "String");
    assert_eq!(doc_type(&TypeRef::Json, Language::Gleam, TEST_PREFIX), "String");
    assert_eq!(doc_type(&TypeRef::Json, Language::Zig, TEST_PREFIX), "[]const u8");
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
    assert_eq!(
        doc_type(&TypeRef::Duration, Language::Swift, TEST_PREFIX),
        "TimeInterval"
    );
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
    // ~keep Every Named type crosses the C ABI as the scalar `AlefHandle` token, never a
    // pointer to a struct named after the Rust type -- see FFI_HANDLE_TYPE_NAME.
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HTMAlefHandle");
}

#[test]
fn test_doc_type_named_without_path() {
    let ty = TypeRef::Named("ParseOptions".to_string());
    assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ParseOptions");
    assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "ParseOptions");
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HTMAlefHandle");
}

#[test]
fn test_doc_type_named_ffi_uses_same_handle_name_regardless_of_concrete_type() {
    // ~keep Two different Rust types must render as the identical scalar handle token in
    // C -- there is no per-type struct left to distinguish them by name.
    let a = TypeRef::Named("ClientConfig".to_string());
    let b = TypeRef::Named("ConversionResult".to_string());
    assert_eq!(doc_type(&a, Language::Ffi, TEST_PREFIX), "HTMAlefHandle");
    assert_eq!(doc_type(&b, Language::Ffi, TEST_PREFIX), "HTMAlefHandle");
}

#[test]
fn test_doc_type_named_jni_currently_emits_per_type_c_name() {
    // ~keep Characterisation only: this pins what the Jni arm currently emits, NOT what it
    // ought to emit. Jni is reachable and `HTMParseOptions` is wrong for it -- the JNI backend
    // passes an opaque type as `jlong` and a non-opaque one as JSON over `jstring`, so
    // `AlefHandle` would be wrong here too. See docs::examples::sample_param_value.
    let ty = TypeRef::Named("ParseOptions".to_string());
    assert_eq!(doc_type(&ty, Language::Jni, TEST_PREFIX), "HTMParseOptions");
}

#[test]
fn test_doc_type_optional_named_ffi_is_bare_scalar_handle_not_a_pointer() {
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string())));
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HTMAlefHandle");
    assert!(!doc_type(&ty, Language::Ffi, TEST_PREFIX).contains('*'));
}

#[test]
fn test_doc_type_optional_string_ffi_is_single_pointer_not_double() {
    // ~keep Regression: `const char*` wrapped in Optional's Ffi arm used to add a second
    // `*`, producing `const char**` in the parameters table while the emitted header
    // declares the param `const char *` (single pointer, nullable in place).
    let ty = TypeRef::Optional(Box::new(TypeRef::String));
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "const char*");
}

#[test]
fn test_doc_type_optional_string_jni_currently_emits_c_double_pointer() {
    // ~keep Characterisation only: pins what the Jni arm currently emits, NOT what it ought to
    // emit. `const char**` is wrong for a backend that emits Rust and crosses an optional
    // string as `jstring`. See docs::examples::sample_param_value.
    let ty = TypeRef::Optional(Box::new(TypeRef::String));
    assert_eq!(doc_type(&ty, Language::Jni, TEST_PREFIX), "const char**");
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
    assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "const char*");
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
    // Java never wraps in `Optional` here either -- the inner Optional's boxed/nullable
    // formula applies at any nesting depth, same as the top-level case.
    assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "List<@Nullable String>");
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
    // Java: the public facade unwraps Optional and annotates with @Nullable -- it never
    // hands the caller an `Optional<T>` (see the Java-specific regression tests below).
    assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "@Nullable ParseOptions");
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
        // usize is Go's `uint`, not `int` -- see GoMapper (backends/go/type_map.rs).
        (PrimitiveType::Usize, "uint"),
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
fn test_doc_type_go_usize_is_uint_not_int_regression() {
    // ~keep Regression for the doc emitter documenting a Go `uint` field as `int` --
    // GoMapper (backends/go/type_map.rs) is the canonical source and disagrees with `int`.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::Usize), Language::Go, TEST_PREFIX),
        "uint"
    );
    // Positive control: isize was already correct and must stay so.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::Isize), Language::Go, TEST_PREFIX),
        "int"
    );
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
        // usize/isize are CsharpMapper's `ulong`/`long` (backends/csharp/type_map.rs), not
        // the native-pointer-width `nuint`/`nint` -- `nuint` is 32-bit on a 32-bit runtime.
        (PrimitiveType::Usize, "ulong"),
        (PrimitiveType::Isize, "long"),
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
fn test_doc_type_csharp_usize_is_ulong_not_nuint_regression() {
    // ~keep Regression: 11 real signatures (LanguageCount, StartByte, EndByte,
    // ChildCount, ...) were documented as `nuint` while the emitted C# source declares
    // `ulong` -- a genuine type error, not cosmetic, since `nuint` is 32-bit on a 32-bit
    // runtime. CsharpMapper (backends/csharp/type_map.rs) is the canonical source.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::Usize), Language::Csharp, TEST_PREFIX),
        "ulong"
    );
    // Positive control: u64 was already correct and must land on the same type usize now
    // does, since CsharpMapper maps both to `ulong`.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::U64), Language::Csharp, TEST_PREFIX),
        "ulong"
    );
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
        // ~keep `bool` is not FFI-safe in the emitted ABI -- `c_primitive`
        // (backends/ffi/type_map.rs) maps it to `i32`, same as `PrimitiveType::I32`,
        // which cbindgen renders `int32_t`.
        (PrimitiveType::Bool, "int32_t"),
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
fn test_doc_type_bool_jni_currently_emits_c_bool() {
    // ~keep Characterisation only: pins what the Jni arm currently emits, NOT what it ought to
    // emit. Jni is reachable, and neither `bool` nor `int32_t` is right -- the JNI backend uses
    // `jboolean` (backends/jni/gen_shims/type_helpers.rs:64). See
    // docs::examples::sample_param_value.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::Bool), Language::Jni, TEST_PREFIX),
        "bool"
    );
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
fn test_doc_type_all_dart_primitives() {
    let cases: &[(PrimitiveType, &str)] = &[
        (PrimitiveType::Bool, "bool"),
        (PrimitiveType::U8, "int"),
        (PrimitiveType::I8, "int"),
        (PrimitiveType::U16, "int"),
        (PrimitiveType::I16, "int"),
        (PrimitiveType::U32, "int"),
        (PrimitiveType::I32, "int"),
        (PrimitiveType::U64, "int"),
        (PrimitiveType::I64, "int"),
        // usize/isize get flutter_rust_bridge's portable `PlatformInt64`, not a bare
        // `int` -- see the regression test below for why.
        (PrimitiveType::Usize, "PlatformInt64"),
        (PrimitiveType::Isize, "PlatformInt64"),
        (PrimitiveType::F32, "double"),
        (PrimitiveType::F64, "double"),
    ];
    for (prim, expected) in cases {
        assert_eq!(
            doc_type(&TypeRef::Primitive(prim.clone()), Language::Dart, TEST_PREFIX),
            *expected,
            "Dart primitive {prim:?}"
        );
    }
}

#[test]
fn test_doc_type_dart_usize_is_platform_int64_not_int_regression() {
    // ~keep Regression: 6 real fields were documented as `int` while flutter_rust_bridge
    // actually widens usize/isize to its own `PlatformInt64` typedef -- a portability
    // hazard identical in kind to C#'s `nuint` bug (Rust's usize/isize vary in width
    // across compile targets; a bare Dart `int` erases that and silently narrows on some
    // targets).
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::Usize), Language::Dart, TEST_PREFIX),
        "PlatformInt64"
    );
    // Positive control: u64 was already correct (plain `int`) and must not be swept up by
    // the usize/isize fix -- only the platform-width variants change.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::U64), Language::Dart, TEST_PREFIX),
        "int"
    );
}

#[test]
fn test_doc_type_all_swift_primitives() {
    let cases: &[(PrimitiveType, &str)] = &[
        (PrimitiveType::Bool, "Bool"),
        (PrimitiveType::U8, "UInt8"),
        (PrimitiveType::I8, "Int8"),
        (PrimitiveType::U16, "UInt16"),
        (PrimitiveType::I16, "Int16"),
        (PrimitiveType::U32, "UInt32"),
        (PrimitiveType::I32, "Int32"),
        (PrimitiveType::U64, "UInt64"),
        (PrimitiveType::I64, "Int64"),
        // usize/isize are Swift's platform-width `UInt`/`Int`, distinct from the
        // fixed-width `UInt64`/`Int64` u64/i64 map to -- see SwiftMapper
        // (backends/swift/type_map.rs).
        (PrimitiveType::Usize, "UInt"),
        (PrimitiveType::Isize, "Int"),
        (PrimitiveType::F32, "Float"),
        (PrimitiveType::F64, "Double"),
    ];
    for (prim, expected) in cases {
        assert_eq!(
            doc_type(&TypeRef::Primitive(prim.clone()), Language::Swift, TEST_PREFIX),
            *expected,
            "Swift primitive {prim:?}"
        );
    }
}

#[test]
fn test_doc_type_swift_usize_is_uint_not_uint64_regression() {
    // ~keep Regression: collapsing usize into the same bucket as u64 documented a
    // platform-width Swift `UInt` field as the fixed-width `UInt64` -- same family of
    // error as C#'s `nuint`/`ulong` mixup, just inverted (fixed-width name for a
    // platform-width field instead of the other way around).
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::Usize), Language::Swift, TEST_PREFIX),
        "UInt"
    );
    // Positive control: u64 was already correct and must stay UInt64, not regress to UInt.
    assert_eq!(
        doc_type(&TypeRef::Primitive(PrimitiveType::U64), Language::Swift, TEST_PREFIX),
        "UInt64"
    );
}

#[test]
fn test_doc_type_java_optional_record_field_shape_is_nullable_not_optional() {
    // ~keep Regression for 26 record-field errors: Java docs said `Optional<T>`; the
    // emitted record field is `@Nullable T` (backends/java/gen_bindings/types/records.rs).
    let ty = TypeRef::Optional(Box::new(TypeRef::String));
    assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "@Nullable String");
}

#[test]
fn test_doc_type_java_optional_boxes_primitive_before_annotating() {
    // A Java primitive (`int`) cannot itself be `@Nullable` -- the boxed type must be
    // used, exactly as the real facade/record codegen does via `java_boxed_type`.
    let ty = TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32)));
    assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "@Nullable Integer");
}

#[test]
fn test_doc_type_java_optional_function_return_shape_matches_public_facade_not_internal_rs_layer() {
    // ~keep Regression for the layer-selection bug: 9 top-level functions were documented
    // with the internal `...Rs` FFI-adjacent class's real `Optional<String>` return type
    // instead of the public facade callers actually use, which unwraps to `@Nullable
    // String` (backends/java/gen_bindings/facade.rs's `return_type` computation, e.g.
    // TreeSitterLanguagePack.java's unwrap of the Rs layer).
    let ty = TypeRef::Optional(Box::new(TypeRef::String));
    let facade_doc = doc_type(&ty, Language::Java, TEST_PREFIX);
    assert_eq!(facade_doc, "@Nullable String");
    assert!(
        !facade_doc.contains("Optional"),
        "must document the public facade's unwrapped return, not the internal Rs layer's \
         Optional<T>: {facade_doc}"
    );
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
