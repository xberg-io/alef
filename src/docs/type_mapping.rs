use crate::core::config::Language;
use crate::core::ir::{PrimitiveType, TypeRef};
use crate::docs::naming::type_name;

/// ~keep The C ABI's scalar generational-handle type name, mirroring
/// `backends/ffi/type_map.rs` (`TypeRef::Named(_) => "AlefHandle"`). That module is
/// private and under concurrent edit for the handle-ABI rollout, so this is a
/// deliberate literal duplicate rather than an import -- keep the two in sync by hand
/// if the ffi backend ever renames the handle type. `docs::examples` carries the same
/// duplicate for the same reason; the two must stay byte-identical.
pub(crate) const FFI_HANDLE_TYPE_NAME: &str = "AlefHandle";

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
            Language::Ffi | Language::C | Language::Jni => "const char*".to_string(),
            Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart => "String".to_string(),
            Language::Gleam => "String".to_string(),
            Language::Zig => "[]const u8".to_string(),
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
            Language::Ffi | Language::C | Language::Jni => "const uint8_t*".to_string(),
            Language::Kotlin | Language::KotlinAndroid => "ByteArray".to_string(),
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
                // ~keep The internal `...Rs` FFI-adjacent class genuinely returns
                // `Optional<T>` (see `java_return_type` in backends/java/type_map.rs), but
                // that is not what a caller sees: the public facade unwraps it and returns
                // `@Nullable T` instead (backends/java/gen_bindings/facade.rs's `return_type`
                // computation), and record fields do the same (gen_bindings/types/records.rs).
                // Documenting `Optional<T>` here was describing the wrong layer. Delegate to
                // the facade's own `render_nullable_type` so the two can never drift again.
                Language::Java => {
                    let boxed = java_boxed_type(inner);
                    crate::backends::java::gen_bindings::helpers::render_nullable_type(&boxed, true)
                }
                Language::Csharp => format!("{inner_ty}?"),
                Language::Ruby => format!("{inner_ty}?"),
                Language::Php => format!("?{inner_ty}"),
                Language::Elixir => format!("{inner_ty} | nil"),
                Language::R => format!("{inner_ty} or NULL"),
                Language::Rust => format!("Option<{inner_ty}>"),
                // ~keep Named types cross the C ABI as a scalar `AlefHandle`, never a
                // pointer -- wrapping one in another `*` here would be as wrong as the
                // signature-side bug this guards against. Types that already render as
                // pointers (strings, bytes, JSON, maps) are nullable in place; doubling
                // the `*` for those was the `const char**`-vs-header `const char *`
                // divergence. Jni is folded into the pointer arm below, and that is wrong for
                // it: the JNI backend emits Rust, where an optional crosses as `jlong` (0 for
                // absent), `jbyteArray`, or JSON over `jstring` -- never a pointer. See the
                // note in docs::examples::sample_param_value.
                Language::Ffi | Language::C
                    if matches!(inner.as_ref(), TypeRef::Named(_)) || inner_ty.ends_with('*') =>
                {
                    inner_ty
                }
                Language::Ffi | Language::C | Language::Jni => format!("{inner_ty}*"),
                Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart => format!("{inner_ty}?"),
                Language::Gleam => format!("Option({inner_ty})"),
                Language::Zig => format!("?{inner_ty}"),
            }
        }
        TypeRef::Vec(inner) => match lang {
            Language::Java => {
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
                    // ~keep A `Vec<T>` crosses the C ABI as one JSON-encoded string, never an
                    // array of `T` -- `FfiParamMapper::vec`/`FfiReturnMapper::vec`
                    // (backends/ffi/type_map.rs) return `*const`/`*mut std::ffi::c_char` for
                    // every element type, including `Vec<Named>`, with no per-element
                    // branching at all. `{inner_ty}*` was doubly wrong for a batch-of-handles
                    // parameter: it named a real C array of `AlefHandle` values that the
                    // header never declares, and for any other element type it also missed
                    // that a Vec is JSON regardless of what it holds. Jni is grouped in here
                    // for the same reason the Optional/String/Path/Json arms already are --
                    // still the wrong ABI for it, tracked separately (see the note in
                    // `docs::examples::sample_param_value`), but no more wrong than before.
                    Language::Ffi | Language::C | Language::Jni => "const char*".to_string(),
                    Language::Java | Language::Csharp => unreachable!(),
                    Language::Kotlin | Language::KotlinAndroid | Language::Dart => format!("List<{inner_ty}>"),
                    Language::Swift => format!("[{inner_ty}]"),
                    Language::Gleam => format!("List({inner_ty})"),
                    Language::Zig => format!("[]const {inner_ty}"),
                }
            }
        },
        TypeRef::Map(k, v) => {
            if lang == Language::Java {
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
                // ~keep Same JSON-string authority as the `Vec` arm above:
                // `FfiParamMapper::map`/`FfiReturnMapper::map` (backends/ffi/type_map.rs)
                // return `*const`/`*mut std::ffi::c_char` unconditionally, never a `void*` to
                // some C map structure the header does not declare.
                Language::Ffi | Language::C | Language::Jni => "const char*".to_string(),
                Language::Kotlin | Language::KotlinAndroid => format!("Map<{kty}, {vty}>"),
                Language::Swift => format!("[{kty}: {vty}]"),
                Language::Dart => format!("Map<{kty}, {vty}>"),
                Language::Gleam => format!("Dict({kty}, {vty})"),
                Language::Zig => format!("std.StringHashMap({vty})"),
            }
        }
        TypeRef::Named(name) if name.starts_with('(') && name.ends_with(')') => {
            let inner = &name[1..name.len() - 1];
            let rendered: Vec<String> = inner
                .split(',')
                .map(|part| {
                    let trimmed = part.trim();
                    match trimmed {
                        "usize" | "u64" | "u32" | "u16" | "u8" | "i64" | "i32" | "i16" | "i8" | "isize" => {
                            // ~keep Same usize/isize-vs-u64/i64 distinction as the primary
                            // `doc_primitive` Swift arm below -- see its comment.
                            let swift_name = match trimmed {
                                "u64" => "UInt64",
                                "usize" => "UInt",
                                "u32" => "UInt32",
                                "u16" => "UInt16",
                                "u8" => "UInt8",
                                "i64" => "Int64",
                                "isize" => "Int",
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
                                Language::Ffi | Language::C | Language::Jni => "uint64_t".to_string(),
                                Language::Kotlin | Language::KotlinAndroid => "Long".to_string(),
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
                            Language::Ffi | Language::C | Language::Jni => "const char*".to_string(),
                            Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart => {
                                "String".to_string()
                            }
                            Language::Gleam => "String".to_string(),
                            Language::Zig => "[]const u8".to_string(),
                        },
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
                                Language::Ffi | Language::C | Language::Jni => "const char**".to_string(),
                                Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart => {
                                    "List<String>".to_string()
                                }
                                Language::Gleam => "List(String)".to_string(),
                                Language::Zig => "[]const []const u8".to_string(),
                            }
                        }
                        other => {
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
                Language::Ffi | Language::C | Language::Jni => "void*".to_string(),
                Language::Kotlin | Language::KotlinAndroid => format!("Pair<{}>", rendered.join(", ")),
                Language::Swift => format!("({})", rendered.join(", ")),
                Language::Dart => format!("({})", rendered.join(", ")),
                Language::Gleam => format!("#({})", rendered.join(", ")),
                Language::Zig => format!("struct {{ {} }}", rendered.join(", ")),
            }
        }
        // ~keep Every Named type crosses the C ABI as the scalar `AlefHandle` token, not
        // a pointer to a struct named after the Rust type -- see FFI_HANDLE_TYPE_NAME.
        // Jni is a reachable target that falls through to the per-type rendering below, which
        // is wrong for it as well: the JNI backend passes an opaque type as a `jlong` handle
        // and a non-opaque one as JSON over `jstring`, so neither the per-type name nor
        // `AlefHandle` describes it. See docs::examples::sample_param_value.
        TypeRef::Named(_) if matches!(lang, Language::Ffi | Language::C) => {
            type_name(FFI_HANDLE_TYPE_NAME, lang, ffi_prefix)
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
            Language::Ffi | Language::C | Language::Jni => "const char*".to_string(),
            Language::Kotlin | Language::KotlinAndroid => "Path".to_string(),
            Language::Swift => "URL".to_string(),
            Language::Dart => "String".to_string(),
            Language::Gleam => "String".to_string(),
            Language::Zig => "[]const u8".to_string(),
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
            Language::Ffi | Language::C | Language::Jni => "void".to_string(),
            Language::Kotlin | Language::KotlinAndroid => "Unit".to_string(),
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
            // ~keep Same JSON-string authority as the `Vec`/`Map` arms above:
            // `FfiParamMapper::json`/`FfiReturnMapper::json` (backends/ffi/type_map.rs) return
            // `*const`/`*mut std::ffi::c_char`, not a `void*` to some opaque C value.
            Language::Ffi | Language::C | Language::Jni => "const char*".to_string(),
            Language::Kotlin | Language::KotlinAndroid => "Any".to_string(),
            Language::Swift => "String".to_string(),
            Language::Dart => "String".to_string(),
            Language::Gleam => "String".to_string(),
            Language::Zig => "[]const u8".to_string(),
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
            Language::Ffi | Language::C | Language::Jni => "uint64_t".to_string(),
            Language::Kotlin | Language::KotlinAndroid => "Duration".to_string(),
            Language::Swift => "TimeInterval".to_string(),
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
            // ~keep Matches GoMapper (backends/go/type_map.rs) exactly: usize/isize are
            // platform-native width in Rust, and Go's own `uint`/`int` are the matching
            // platform-native-width types -- but the two are not interchangeable with each
            // other. Collapsing them to a single "int" was documenting `uint` fields as
            // signed.
            PrimitiveType::Usize => "uint".to_string(),
            PrimitiveType::Isize => "int".to_string(),
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
            // ~keep Matches CsharpMapper (backends/csharp/type_map.rs) exactly. `nuint`/
            // `nint` are C#'s *native-pointer-width* types -- 32-bit on a 32-bit runtime --
            // which is not what the emitted binding uses; the real generated source is the
            // fixed-width `ulong`/`long`, matching Rust's own 64-bit-in-practice usize/isize.
            // Documenting `nuint` here was a genuine type error, not a cosmetic mismatch.
            PrimitiveType::Usize => "ulong".to_string(),
            PrimitiveType::Isize => "long".to_string(),
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
        Language::Ffi | Language::C | Language::Jni => match p {
            // ~keep Rust `bool` is not passed across the C ABI directly -- the FFI backend's
            // `c_primitive` (backends/ffi/type_map.rs) maps `PrimitiveType::Bool` to `i32` for
            // both params and returns, which cbindgen renders as `int32_t` (see the `I32` arm
            // below). Jni is a reachable target that falls through to plain `bool`, and that is
            // wrong for it too -- the JNI backend uses `jboolean`
            // (backends/jni/gen_shims/type_helpers.rs:64), which is neither `bool` nor
            // `int32_t`. See docs::examples::sample_param_value.
            PrimitiveType::Bool if matches!(lang, Language::Ffi | Language::C) => "int32_t".to_string(),
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
        Language::Kotlin | Language::KotlinAndroid => match p {
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
            PrimitiveType::U64 => "UInt64".to_string(),
            PrimitiveType::I8 => "Int8".to_string(),
            PrimitiveType::I16 => "Int16".to_string(),
            PrimitiveType::I32 => "Int32".to_string(),
            PrimitiveType::I64 => "Int64".to_string(),
            // ~keep Matches SwiftMapper (backends/swift/type_map.rs) exactly: Swift has a
            // native platform-width `UInt`/`Int` distinct from the fixed-width
            // `UInt64`/`Int64` Rust's u64/i64 map to. Collapsing usize/isize into the
            // fixed-width names was the same family of error as C#'s `nuint`/`ulong` mixup.
            PrimitiveType::Usize => "UInt".to_string(),
            PrimitiveType::Isize => "Int".to_string(),
            PrimitiveType::F32 => "Float".to_string(),
            PrimitiveType::F64 => "Double".to_string(),
        },
        Language::Dart => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "double".to_string(),
            // ~keep flutter_rust_bridge widens usize/isize to its own portable
            // `PlatformInt64` typedef rather than a bare `int` -- Rust's usize/isize vary in
            // width across compile targets (32-bit vs 64-bit), the same portability hazard
            // C#'s `nuint` has. This substitution happens inside frb itself, downstream of
            // alef's own codegen (DartMapper in backends/dart/type_map.rs has no override
            // for it either), so there is no in-repo canonical function to delegate to here.
            PrimitiveType::Usize | PrimitiveType::Isize => "PlatformInt64".to_string(),
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
        _ => doc_type(ty, Language::Java, ""),
    }
}

#[cfg(test)]
mod tests;
