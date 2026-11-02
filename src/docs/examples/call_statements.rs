use super::{c_result_declaration, render_go_call_statement};
use crate::core::config::Language;
use crate::core::ir::TypeRef;

pub(super) fn render_call_statement(
    call: &str,
    return_type: &TypeRef,
    fallible: bool,
    is_async: bool,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    let returns_value = !matches!(return_type, TypeRef::Unit);
    match lang {
        Language::Python | Language::Ruby => bind_expression(call, returns_value, "result = ", ""),
        Language::Node | Language::Wasm | Language::Csharp | Language::Dart => {
            render_awaited_call(call, returns_value, is_async, lang)
        }
        Language::Php => bind_expression(call, returns_value, "$result = ", ";"),
        Language::Elixir => {
            let pattern = if returns_value { "{:ok, result}" } else { ":ok" };
            format!("{pattern} = {call}")
        }
        Language::Go => render_go_call_statement(call, returns_value, fallible),
        Language::Java => bind_expression(call, returns_value, "var result = ", ";"),
        Language::Ffi | Language::C | Language::Jni => {
            let prefix = if returns_value {
                format!("{} = ", c_result_declaration(return_type, lang, ffi_prefix))
            } else {
                String::new()
            };
            bind_expression(call, returns_value, &prefix, ";")
        }
        Language::R => bind_expression(call, returns_value, "result <- ", ""),
        Language::Rust => render_rust_call(call, returns_value, fallible, is_async),
        Language::Kotlin | Language::KotlinAndroid => bind_expression(call, returns_value, "val result = ", ""),
        Language::Swift | Language::Zig => render_try_call(call, returns_value, fallible, lang),
        Language::Gleam => bind_expression(call, returns_value, "let result = ", ""),
    }
}

fn bind_expression(expression: &str, returns_value: bool, binding: &str, terminator: &str) -> String {
    if returns_value {
        format!("{binding}{expression}{terminator}")
    } else {
        format!("{expression}{terminator}")
    }
}

fn render_awaited_call(call: &str, returns_value: bool, is_async: bool, lang: Language) -> String {
    let expression = if is_async {
        format!("await {call}")
    } else {
        call.to_string()
    };
    let binding = match lang {
        Language::Csharp => "var result = ",
        Language::Dart => "final result = ",
        _ => "const result = ",
    };
    bind_expression(&expression, returns_value, binding, ";")
}

fn render_rust_call(call: &str, returns_value: bool, fallible: bool, is_async: bool) -> String {
    let awaited = if is_async {
        format!("{call}.await")
    } else {
        call.to_string()
    };
    let expression = if fallible { format!("{awaited}?") } else { awaited };
    bind_expression(&expression, returns_value, "let result = ", ";")
}

fn render_try_call(call: &str, returns_value: bool, fallible: bool, lang: Language) -> String {
    let expression = if fallible {
        format!("try {call}")
    } else {
        call.to_string()
    };
    let (binding, terminator) = if lang == Language::Swift {
        ("let result = ", "")
    } else {
        ("const result = ", ";")
    };
    bind_expression(&expression, returns_value, binding, terminator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::PrimitiveType;

    const EXPECTED_CALLS: &[(Language, [&str; 8])] = &[
        (
            Language::Python,
            [
                "run()",
                "run()",
                "run()",
                "run()",
                "result = run()",
                "result = run()",
                "result = run()",
                "result = run()",
            ],
        ),
        (
            Language::Node,
            [
                "run();",
                "run();",
                "await run();",
                "await run();",
                "const result = run();",
                "const result = run();",
                "const result = await run();",
                "const result = await run();",
            ],
        ),
        (
            Language::Wasm,
            [
                "run();",
                "run();",
                "await run();",
                "await run();",
                "const result = run();",
                "const result = run();",
                "const result = await run();",
                "const result = await run();",
            ],
        ),
        (
            Language::Ruby,
            [
                "run()",
                "run()",
                "run()",
                "run()",
                "result = run()",
                "result = run()",
                "result = run()",
                "result = run()",
            ],
        ),
        (
            Language::Php,
            [
                "run();",
                "run();",
                "run();",
                "run();",
                "$result = run();",
                "$result = run();",
                "$result = run();",
                "$result = run();",
            ],
        ),
        (
            Language::Elixir,
            [
                ":ok = run()",
                ":ok = run()",
                ":ok = run()",
                ":ok = run()",
                "{:ok, result} = run()",
                "{:ok, result} = run()",
                "{:ok, result} = run()",
                "{:ok, result} = run()",
            ],
        ),
        (
            Language::Go,
            [
                "run()",
                "if err := run(); err != nil {\n    return err\n}",
                "run()",
                "if err := run(); err != nil {\n    return err\n}",
                "result := run()",
                "result, err := run()\nif err != nil {\n    return err\n}",
                "result := run()",
                "result, err := run()\nif err != nil {\n    return err\n}",
            ],
        ),
        (
            Language::Java,
            [
                "run();",
                "run();",
                "run();",
                "run();",
                "var result = run();",
                "var result = run();",
                "var result = run();",
                "var result = run();",
            ],
        ),
        (
            Language::Csharp,
            [
                "run();",
                "run();",
                "await run();",
                "await run();",
                "var result = run();",
                "var result = run();",
                "var result = await run();",
                "var result = await run();",
            ],
        ),
        (
            Language::Ffi,
            [
                "run();",
                "run();",
                "run();",
                "run();",
                "int32_t result = run();",
                "int32_t result = run();",
                "int32_t result = run();",
                "int32_t result = run();",
            ],
        ),
        (
            Language::C,
            [
                "run();",
                "run();",
                "run();",
                "run();",
                "int32_t result = run();",
                "int32_t result = run();",
                "int32_t result = run();",
                "int32_t result = run();",
            ],
        ),
        (
            Language::Jni,
            [
                "run();",
                "run();",
                "run();",
                "run();",
                "int32_t result = run();",
                "int32_t result = run();",
                "int32_t result = run();",
                "int32_t result = run();",
            ],
        ),
        (
            Language::R,
            [
                "run()",
                "run()",
                "run()",
                "run()",
                "result <- run()",
                "result <- run()",
                "result <- run()",
                "result <- run()",
            ],
        ),
        (
            Language::Rust,
            [
                "run();",
                "run()?;",
                "run().await;",
                "run().await?;",
                "let result = run();",
                "let result = run()?;",
                "let result = run().await;",
                "let result = run().await?;",
            ],
        ),
        (
            Language::Kotlin,
            [
                "run()",
                "run()",
                "run()",
                "run()",
                "val result = run()",
                "val result = run()",
                "val result = run()",
                "val result = run()",
            ],
        ),
        (
            Language::KotlinAndroid,
            [
                "run()",
                "run()",
                "run()",
                "run()",
                "val result = run()",
                "val result = run()",
                "val result = run()",
                "val result = run()",
            ],
        ),
        (
            Language::Swift,
            [
                "run()",
                "try run()",
                "run()",
                "try run()",
                "let result = run()",
                "let result = try run()",
                "let result = run()",
                "let result = try run()",
            ],
        ),
        (
            Language::Dart,
            [
                "run();",
                "run();",
                "await run();",
                "await run();",
                "final result = run();",
                "final result = run();",
                "final result = await run();",
                "final result = await run();",
            ],
        ),
        (
            Language::Gleam,
            [
                "run()",
                "run()",
                "run()",
                "run()",
                "let result = run()",
                "let result = run()",
                "let result = run()",
                "let result = run()",
            ],
        ),
        (
            Language::Zig,
            [
                "run();",
                "try run();",
                "run();",
                "try run();",
                "const result = run();",
                "const result = try run();",
                "const result = run();",
                "const result = try run();",
            ],
        ),
    ];

    #[test]
    fn call_statements_preserve_all_languages_and_async_error_return_combinations() {
        for (lang, expected) in EXPECTED_CALLS {
            let mut index = 0;
            for return_type in [TypeRef::Unit, TypeRef::Primitive(PrimitiveType::I32)] {
                for is_async in [false, true] {
                    for fallible in [false, true] {
                        assert_eq!(
                            render_call_statement("run()", &return_type, fallible, is_async, *lang, "Demo"),
                            expected[index],
                            "{lang:?}, {return_type:?}, async={is_async}, fallible={fallible}",
                        );
                        index += 1;
                    }
                }
            }
            assert_eq!(index, expected.len());
        }
    }
}
