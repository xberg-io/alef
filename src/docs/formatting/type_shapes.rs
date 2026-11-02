use crate::core::config::Language;
use crate::core::ir::{PrimitiveType, TypeRef};
use crate::docs::type_mapping::{doc_type, java_boxed_type};
use heck::ToPascalCase;

/// ~keep The C ABI's on-error return value depends on the function's return shape, not a
/// single fixed sentinel -- traced from `backends/ffi/gen_bindings/functions/orchestration.rs`
/// (`:223-229`/`:760-765`, `gen_function_wrapper_footer` `:615-617`) and `helpers.rs`
/// (`:193-232`): a fallible `()` return, or any `Bytes` result (always delivered via
/// out-param + status), reports failure as `-1`; a `TypeRef::Named` handle reports it as
/// the integer `0` -- never `NULL`, there is no pointer to compare a `uint64_t` handle
/// against, which is exactly the confusion this fix exists to remove; the remaining
/// heap-allocated shapes (String/Char/Path/Json/Vec/Map) really do return a null pointer;
/// numeric primitives and `Duration` report `0` (`0.0` for floats). Must stay in lockstep
/// with the `int32_t`/`-1` text in `function_render.rs`'s `push_returns_with_override` --
/// see `ffi_status_code.rs`'s cross-check tests for why the two must not drift apart.
fn ffi_error_return_phrase(return_type: &TypeRef) -> String {
    match return_type {
        TypeRef::Unit | TypeRef::Bytes => "Returns `-1` on error.".to_string(),
        TypeRef::Named(_) => "Returns the sentinel handle `0` on error.".to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Returns `NULL` on error.".to_string()
        }
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "Returns `0.0` on error.".to_string(),
        TypeRef::Primitive(_) | TypeRef::Duration => "Returns `0` on error.".to_string(),
        TypeRef::Optional(inner) => ffi_error_return_phrase(inner),
    }
}

/// Format the error/exception phrase for a function that can fail.
pub(crate) fn format_error_phrase(error_type: &str, return_type: &TypeRef, lang: Language, crate_name: &str) -> String {
    let short = error_type.rsplit("::").next().unwrap_or(error_type);
    match lang {
        Language::Python => {
            let ename = short.to_pascal_case();
            format!("Raises `{ename}`.")
        }
        Language::Go => "Returns `error`.".to_string(),
        Language::Java => java_error_phrase(crate_name),
        Language::Node | Language::Wasm => "Throws `Error` with a descriptive message.".to_string(),
        Language::Ruby => {
            let ename = short.to_pascal_case();
            format!("Raises `{ename}`.")
        }
        Language::Csharp => {
            let ename = short.to_pascal_case();
            format!("Throws `{ename}`.")
        }
        Language::Elixir => "Returns `{:error, reason}`".to_string(),
        Language::Php => {
            let ename = short.to_pascal_case();
            format!("Throws `{ename}`.")
        }
        Language::Ffi | Language::C => ffi_error_return_phrase(return_type),
        // ~keep Jni is reachable (see docs::examples::sample_param_value) and this fixed `NULL`
        // phrase is wrong for it: a JNI shim signals failure by throwing
        // `<Bridge>Exception` via `env.throw_new`, then returns a type-specific zero — `()`,
        // `false`, `0`, or `std::ptr::null_mut()` (backends/jni/gen_shims/type_helpers.rs:29-42,
        // templates/runtime_helpers.rs.jinja:32-42). "Returns `NULL`" is wrong for unit,
        // primitive, and handle returns alike, and buries the exception the caller must catch.
        Language::Jni => "Returns `NULL` on error.".to_string(),
        Language::R => "Stops with error message.".to_string(),
        Language::Rust => {
            let ename = short.to_pascal_case();
            format!("Returns `Err({ename})`.")
        }
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => {
            let ename = short.to_pascal_case();
            format!("Throws `{ename}`.")
        }
    }
}

/// Like `doc_type` but wraps in the nullable form when `optional` is true.
pub(crate) fn doc_type_with_optional(ty: &TypeRef, lang: Language, optional: bool, ffi_prefix: &str) -> String {
    if optional && !matches!(ty, TypeRef::Optional(_)) {
        let inner = doc_type(ty, lang, ffi_prefix);
        return match lang {
            Language::Python => format!("{inner} | None"),
            Language::Node | Language::Wasm => format!("{inner} | null"),
            Language::Go => format!("*{inner}"),
            Language::Java => java_optional_type(ty),
            Language::Csharp => format!("{inner}?"),
            Language::Ruby => format!("{inner}?"),
            Language::Php => format!("?{inner}"),
            Language::Elixir => format!("{inner} | nil"),
            Language::R => format!("{inner} or NULL"),
            Language::Rust => format!("Option<{inner}>"),
            // ~keep Mirrors type_mapping.rs's `TypeRef::Optional` arm: a Named type crosses
            // the C ABI as a scalar `AlefHandle`, never a pointer, so wrapping it in another
            // `*` here fabricates a pointer that doesn't exist. Types that already render as
            // pointers (strings, bytes, JSON, maps) are nullable in place -- doubling the `*`
            // for those was the `const char**`-vs-header `const char *` divergence. Jni falls
            // into the pointer arm below, which is wrong for it: the JNI backend emits Rust,
            // where an optional is `jlong`/`jbyteArray`/`jstring`, never a pointer. See
            // docs::examples::sample_param_value.
            Language::Ffi | Language::C if matches!(ty, TypeRef::Named(_)) || inner.ends_with('*') => inner,
            Language::Ffi | Language::C | Language::Jni => format!("{inner}*"),
            Language::Kotlin
            | Language::KotlinAndroid
            | Language::Swift
            | Language::Dart
            | Language::Gleam
            | Language::Zig => {
                format!("{inner}?")
            }
        };
    }
    doc_type(ty, lang, ffi_prefix)
}

// ~keep The class is named by `backends::java::naming::exception_class_name`, the same
// derivation the signature's `throws` clause (`signatures.rs`) and the streaming
// override (`streaming.rs`) use. Pascal-casing the error type's own short name here
// used to name a *different* class -- one the domain-error hierarchy happens to
// declare (`codegen/error_gen/host_langs.rs`) but that no method actually throws.
fn java_error_phrase(crate_name: &str) -> String {
    let ename = crate::backends::java::naming::exception_class_name(crate_name);
    format!("Throws `{ename}`.")
}

// ~keep The public facade unwraps the internal `...Rs` `Optional<T>` and returns
// `@Nullable T` instead, calling `render_nullable_type` for both parameters and
// returns (backends/java/gen_bindings/facade.rs:37,62). Delegating here, the same
// way type_mapping.rs's `TypeRef::Optional` arm does, is what keeps this copy from
// drifting off the facade; matching its output by hand is what produced the drift
// this arm fixes. Record fields land on the same `@Nullable T` shape but do NOT
// route through `render_nullable_type` -- gen_bindings/types/records.rs:87-155
// hand-rolls the annotation from `has_nullable`/`nullable_at_leading_pos` -- so
// that third copy remains free to drift and is not guarded by this delegation.
fn java_optional_type(ty: &TypeRef) -> String {
    let boxed = java_boxed_type(ty);
    crate::backends::java::gen_bindings::helpers::render_nullable_type(&boxed, true)
}
