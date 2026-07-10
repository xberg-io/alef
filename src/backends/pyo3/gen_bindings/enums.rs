//! Shared enum types and string-utility helpers used across the gen_bindings sub-modules.

/// Selects which Python module context a type annotation is being emitted for.
///
/// The same IR type can resolve to different Python names depending on the target
/// module: in `options.py`, a data-enum `Named` type refers to the locally defined
/// union type alias; in `_native.pyi`, it refers to the PyO3 class exposed by the
/// native extension.  Callers pass `EmitContext` so that `python_field_type` can
/// produce the correct annotation for each context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EmitContext {
    /// Emitting type annotations inside `options.py` (dataclass / TypedDict fields).
    /// `Named(DataEnum)` resolves to the locally defined union type alias.
    OptionsModule,
    /// Emitting type annotations inside a `_native.pyi` stub.
    /// `Named(DataEnum)` resolves to the native PyO3 class with the same name.
    #[allow(dead_code)]
    NativeStub,
}

/// Wrapping shape of a parameter type whose leaf is a named type.
#[derive(Debug, Clone, Copy)]
pub(super) enum Wrapping {
    /// `T`
    Plain,
    /// `Option<T>`
    Optional,
    /// `Vec<T>`
    Vec,
    /// `Option<Vec<T>>`
    OptionalVec,
}

/// Sanitize a Rust doc comment string for use in Python docstrings.
///
/// Replaces Unicode characters that trigger ruff RUF001/RUF002 lint errors with
/// their closest ASCII equivalents. Must be applied to every doc string before
/// it is emitted into a Python source file or stub.
pub(crate) fn sanitize_python_doc(s: &str) -> String {
    s.replace('\u{2013}', "-")
        .replace('\u{2014}', "--")
        .replace('\u{00D7}', "x")
        .replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"")
}

/// Convert a CamelCase class name to a human-readable docstring sentence.
///
/// Examples: `AuthenticationError` → `"Authentication error."`,
/// `SampleLlmError` → `"Sample llm error."`
pub(crate) fn class_name_to_docstring(name: &str) -> String {
    use heck::ToSnakeCase;
    let snake = name.to_snake_case();
    let sentence = snake.replace('_', " ");
    let mut chars = sentence.chars();
    let capitalized = match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    };
    format!("{}.", capitalized)
}

#[cfg(test)]
mod tests {
    use super::{EmitContext, Wrapping, class_name_to_docstring, sanitize_python_doc};

    /// sanitize_python_doc replaces en-dash with ASCII hyphen.
    #[test]
    fn sanitize_python_doc_replaces_en_dash() {
        let input = "foo \u{2013} bar";
        assert_eq!(sanitize_python_doc(input), "foo - bar");
    }

    /// sanitize_python_doc replaces em-dash with double hyphen.
    #[test]
    fn sanitize_python_doc_replaces_em_dash() {
        let input = "foo \u{2014} bar";
        assert_eq!(sanitize_python_doc(input), "foo -- bar");
    }

    /// class_name_to_docstring converts CamelCase to "Sentence case.".
    #[test]
    fn class_name_to_docstring_converts_camel_case() {
        assert_eq!(class_name_to_docstring("AuthenticationError"), "Authentication error.");
    }

    /// EmitContext variants are distinct.
    #[test]
    fn emit_context_variants_are_distinct() {
        assert_ne!(EmitContext::OptionsModule, EmitContext::NativeStub);
    }

    /// Wrapping::Plain is copy-able (required by callers).
    #[test]
    fn wrapping_plain_is_copy() {
        let w = Wrapping::Plain;
        let _w2 = w;
        let _w3 = w;
    }
}
