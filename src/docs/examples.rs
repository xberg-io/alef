use crate::core::config::Language;
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeRef};
use crate::docs::naming::{func_name, lang_code_fence, type_name};
use crate::docs::template_env;
use crate::docs::type_mapping::doc_type;
use heck::ToSnakeCase;

pub(crate) fn render_function_example(func: &FunctionDef, lang: Language, ffi_prefix: &str) -> String {
    if let Some(example) = authored_example_block(&func.doc, lang) {
        return example;
    }
    let call = function_call_expression(func, lang, ffi_prefix);
    render_example_block(
        lang,
        render_call_statement(
            &call,
            &func.return_type,
            func.error_type.is_some(),
            func.is_async,
            lang,
            ffi_prefix,
        ),
    )
}

pub(crate) fn render_method_example(method: &MethodDef, owner_type: &str, lang: Language, ffi_prefix: &str) -> String {
    if let Some(example) = authored_example_block(&method.doc, lang) {
        return example;
    }
    let call = method_call_expression(method, owner_type, lang, ffi_prefix);
    render_example_block(
        lang,
        render_call_statement(
            &call,
            &method.return_type,
            method.error_type.is_some(),
            method.is_async,
            lang,
            ffi_prefix,
        ),
    )
}

fn authored_example_block(doc: &str, lang: Language) -> Option<String> {
    let sections = crate::codegen::doc_emission::parse_rustdoc_sections(doc);
    let example = sections.example.as_deref()?.trim();
    if example.is_empty() || !example_language_matches(example, lang) {
        return None;
    }
    let body = crate::codegen::doc_emission::replace_fence_lang(example, lang_code_fence(lang));
    let mut out = String::new();
    out.push_str("**Example:**\n\n");
    out.push_str(&body);
    out.push('\n');
    out.push('\n');
    Some(out)
}

fn example_language_matches(example: &str, lang: Language) -> bool {
    let Some(fence_lang) = first_fence_lang(example) else {
        return lang == Language::Rust;
    };
    if rust_fence(&fence_lang) {
        return lang == Language::Rust;
    }
    let target = lang_code_fence(lang);
    fence_lang == target || compatible_alias(&fence_lang, target)
}

fn first_fence_lang(example: &str) -> Option<String> {
    for line in example.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            let tag = rest.split(',').next().unwrap_or("").trim();
            return Some(tag.to_ascii_lowercase());
        }
    }
    None
}

fn rust_fence(tag: &str) -> bool {
    tag.is_empty()
        || tag == "rust"
        || tag.starts_with("rust,")
        || matches!(tag, "no_run" | "ignore" | "should_panic" | "compile_fail")
        || tag.starts_with("edition")
}

fn compatible_alias(source: &str, target: &str) -> bool {
    matches!(
        (source, target),
        ("py", "python")
            | ("js", "typescript")
            | ("javascript", "typescript")
            | ("ts", "typescript")
            | ("tsx", "typescript")
            | ("c", "c")
            | ("h", "c")
            | ("ex", "elixir")
            | ("exs", "elixir")
            | ("kt", "kotlin")
            | ("kts", "kotlin")
            | ("cs", "csharp")
    )
}

fn render_example_block(lang: Language, body: String) -> String {
    let mut out = String::new();
    out.push_str("**Example:**\n\n");
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code_fence(lang), body => body },
    ));
    out.push('\n');
    out
}

fn function_call_expression(func: &FunctionDef, lang: Language, ffi_prefix: &str) -> String {
    let name = func_name(&func.name, lang, ffi_prefix);
    let args = render_args(&func.params, lang, ffi_prefix);
    match lang {
        Language::Elixir => format!("{name}({args})"),
        Language::Php => format!("{name}({args})"),
        _ => format!("{name}({args})"),
    }
}

fn method_call_expression(method: &MethodDef, owner_type: &str, lang: Language, ffi_prefix: &str) -> String {
    let name = func_name(&method.name, lang, ffi_prefix);
    let args = render_args(&method.params, lang, ffi_prefix);
    if method.is_static {
        return static_method_call(owner_type, &name, &args, lang, ffi_prefix);
    }
    let receiver = match lang {
        Language::Go => "instance",
        Language::Rust => "instance",
        Language::C | Language::Ffi | Language::Jni => "instance",
        _ => "instance",
    };
    match lang {
        Language::Php => format!("${receiver}->{name}({args})"),
        Language::C | Language::Ffi | Language::Jni => {
            if args.is_empty() {
                format!("{name}({receiver})")
            } else {
                format!("{name}({receiver}, {args})")
            }
        }
        _ => format!("{receiver}.{name}({args})"),
    }
}

fn static_method_call(owner_type: &str, name: &str, args: &str, lang: Language, ffi_prefix: &str) -> String {
    let owner = type_name(owner_type, lang, ffi_prefix);
    match lang {
        Language::Ruby => format!("{owner}.{name}({args})"),
        Language::Php => format!("{owner}::{name}({args})"),
        Language::Rust => format!("{owner}::{name}({args})"),
        Language::C | Language::Ffi | Language::Jni => format!("{name}({args})"),
        _ => format!("{owner}.{name}({args})"),
    }
}

fn render_args(params: &[ParamDef], lang: Language, ffi_prefix: &str) -> String {
    params
        .iter()
        .map(|param| {
            let value = sample_param_value(param, lang, ffi_prefix);
            match lang {
                Language::Ruby if param.optional => format!("{}: {value}", param.name.to_snake_case()),
                Language::Python if param.optional => format!("{}={value}", param.name.to_snake_case()),
                _ => value,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn sample_param_value(param: &ParamDef, lang: Language, ffi_prefix: &str) -> String {
    if matches!(lang, Language::Ffi | Language::C | Language::Jni)
        && let TypeRef::Named(name) = &param.ty
    {
        if param.optional || param.is_ref {
            return "NULL".to_string();
        }

        let ty = type_name(name, lang, ffi_prefix);
        return format!("({ty}){{0}}");
    }

    sample_value(&param.ty, lang, ffi_prefix)
}

fn render_call_statement(
    call: &str,
    return_type: &TypeRef,
    fallible: bool,
    is_async: bool,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    let returns_value = !matches!(return_type, TypeRef::Unit);
    match lang {
        Language::Python => {
            if returns_value {
                format!("result = {call}")
            } else {
                call.to_string()
            }
        }
        Language::Node | Language::Wasm => {
            let awaited = if is_async {
                format!("await {call}")
            } else {
                call.to_string()
            };
            if returns_value {
                format!("const result = {awaited};")
            } else {
                format!("{awaited};")
            }
        }
        Language::Ruby => {
            if returns_value {
                format!("result = {call}")
            } else {
                call.to_string()
            }
        }
        Language::Php => {
            if returns_value {
                format!("$result = {call};")
            } else {
                format!("{call};")
            }
        }
        Language::Elixir => {
            if returns_value {
                format!("{{:ok, result}} = {call}")
            } else {
                format!(":ok = {call}")
            }
        }
        Language::Go => render_go_call_statement(call, returns_value, fallible),
        Language::Java => {
            if returns_value {
                format!("var result = {call};")
            } else {
                format!("{call};")
            }
        }
        Language::Csharp => {
            let awaited = if is_async {
                format!("await {call}")
            } else {
                call.to_string()
            };
            if returns_value {
                format!("var result = {awaited};")
            } else {
                format!("{awaited};")
            }
        }
        Language::Ffi | Language::C | Language::Jni => {
            if returns_value {
                let declaration = c_result_declaration(return_type, lang, ffi_prefix);
                format!("{declaration} = {call};")
            } else {
                format!("{call};")
            }
        }
        Language::R => {
            if returns_value {
                format!("result <- {call}")
            } else {
                call.to_string()
            }
        }
        Language::Rust => {
            let awaited = if is_async {
                format!("{call}.await")
            } else {
                call.to_string()
            };
            let expr = if fallible { format!("{awaited}?") } else { awaited };
            if returns_value {
                format!("let result = {expr};")
            } else {
                format!("{expr};")
            }
        }
        Language::Kotlin | Language::KotlinAndroid => {
            if returns_value {
                format!("val result = {call}")
            } else {
                call.to_string()
            }
        }
        Language::Swift => {
            let expr = if fallible {
                format!("try {call}")
            } else {
                call.to_string()
            };
            if returns_value {
                format!("let result = {expr}")
            } else {
                expr
            }
        }
        Language::Dart => {
            let awaited = if is_async {
                format!("await {call}")
            } else {
                call.to_string()
            };
            if returns_value {
                format!("final result = {awaited};")
            } else {
                format!("{awaited};")
            }
        }
        Language::Gleam => {
            if returns_value {
                format!("let result = {call}")
            } else {
                call.to_string()
            }
        }
        Language::Zig => {
            let expr = if fallible {
                format!("try {call}")
            } else {
                call.to_string()
            };
            if returns_value {
                format!("const result = {expr};")
            } else {
                format!("{expr};")
            }
        }
    }
}

fn c_result_declaration(return_type: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    match return_type {
        TypeRef::Named(_) => format!("{} *result", doc_type(return_type, lang, ffi_prefix)),
        TypeRef::String | TypeRef::Char => "const char *result".to_string(),
        TypeRef::Bytes => "const uint8_t *result".to_string(),
        TypeRef::Unit => "void result".to_string(),
        _ => format!("{} result", doc_type(return_type, lang, ffi_prefix)),
    }
}

fn render_go_call_statement(call: &str, returns_value: bool, fallible: bool) -> String {
    match (returns_value, fallible) {
        (true, true) => format!("result, err := {call}\nif err != nil {{\n    return err\n}}"),
        (false, true) => format!("if err := {call}; err != nil {{\n    return err\n}}"),
        (true, false) => format!("result := {call}"),
        (false, false) => call.to_string(),
    }
}

fn sample_value(ty: &TypeRef, lang: Language, ffi_prefix: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => "\"value\"".to_string(),
        TypeRef::Bytes => sample_bytes_value(lang),
        TypeRef::Primitive(primitive) => sample_primitive_value(primitive, lang),
        TypeRef::Optional(_) => null_literal(lang),
        TypeRef::Vec(_) => empty_list_literal(lang),
        TypeRef::Map(_, _) => empty_map_literal(lang),
        TypeRef::Named(name) => sample_named_value(name, lang, ffi_prefix),
        TypeRef::Unit => unit_literal(lang),
        TypeRef::Json => empty_map_literal(lang),
        TypeRef::Duration => sample_duration_value(lang),
    }
}

fn sample_bytes_value(lang: Language) -> String {
    match lang {
        Language::Python => "b\"data\"".to_string(),
        Language::Node => "Buffer.from(\"data\")".to_string(),
        Language::Wasm => "new Uint8Array([100, 97, 116, 97])".to_string(),
        Language::Go => "[]byte(\"data\")".to_string(),
        Language::Java => "\"data\".getBytes()".to_string(),
        Language::Csharp => "System.Text.Encoding.UTF8.GetBytes(\"data\")".to_string(),
        Language::Ruby | Language::Php => "\"data\"".to_string(),
        Language::Elixir => "<<100, 97, 116, 97>>".to_string(),
        Language::R => "charToRaw(\"data\")".to_string(),
        Language::Rust => "b\"data\"".to_string(),
        Language::Ffi | Language::C | Language::Jni => "(const uint8_t *)\"data\"".to_string(),
        Language::Kotlin | Language::KotlinAndroid => "\"data\".toByteArray()".to_string(),
        Language::Swift => "Data(\"data\".utf8)".to_string(),
        Language::Dart => "Uint8List.fromList([100, 97, 116, 97])".to_string(),
        Language::Gleam => "<<\"data\":utf8>>".to_string(),
        Language::Zig => "\"data\"".to_string(),
    }
}

fn sample_primitive_value(primitive: &PrimitiveType, lang: Language) -> String {
    match primitive {
        PrimitiveType::Bool => match lang {
            Language::Python => "True".to_string(),
            Language::R => "TRUE".to_string(),
            _ => "true".to_string(),
        },
        PrimitiveType::F32 | PrimitiveType::F64 => "0.5".to_string(),
        _ => "42".to_string(),
    }
}

fn sample_named_value(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let ty = type_name(name, lang, ffi_prefix);
    match lang {
        Language::Python | Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart => {
            format!("{ty}()")
        }
        Language::Node | Language::Wasm | Language::Java | Language::Csharp | Language::Php => format!("new {ty}()"),
        Language::Ruby => format!("{ty}.new"),
        Language::Elixir | Language::R => "%{{}}".to_string(),
        Language::Go => format!("{ty}{{}}"),
        Language::Rust => format!("{ty}::default()"),
        Language::Ffi | Language::C | Language::Jni => "NULL".to_string(),
        Language::Gleam => "todo".to_string(),
        Language::Zig => ".{}".to_string(),
    }
}

fn sample_duration_value(lang: Language) -> String {
    match lang {
        Language::Python | Language::Node | Language::Wasm | Language::Ruby | Language::Php | Language::R => {
            "1.0".to_string()
        }
        Language::Go => "time.Second".to_string(),
        Language::Java => "Duration.ofSeconds(1)".to_string(),
        Language::Csharp => "TimeSpan.FromSeconds(1)".to_string(),
        Language::Elixir => "1000".to_string(),
        Language::Rust => "std::time::Duration::from_secs(1)".to_string(),
        Language::Ffi | Language::C | Language::Jni => "1000".to_string(),
        Language::Kotlin | Language::KotlinAndroid => "1.seconds".to_string(),
        Language::Swift => ".seconds(1)".to_string(),
        Language::Dart => "Duration(seconds: 1)".to_string(),
        Language::Gleam | Language::Zig => "1000".to_string(),
    }
}

fn empty_list_literal(lang: Language) -> String {
    match lang {
        Language::Go => "nil".to_string(),
        Language::Java => "List.of()".to_string(),
        Language::Csharp => "new List<object>()".to_string(),
        Language::Elixir => "[]".to_string(),
        Language::Rust => "vec![]".to_string(),
        Language::Ffi | Language::C | Language::Jni => "NULL".to_string(),
        Language::Swift | Language::Dart | Language::Kotlin | Language::KotlinAndroid => "[]".to_string(),
        Language::Gleam => "[]".to_string(),
        Language::Zig => "&[_]u8{}".to_string(),
        _ => "[]".to_string(),
    }
}

fn empty_map_literal(lang: Language) -> String {
    match lang {
        Language::Python => "{}".to_string(),
        Language::Node | Language::Wasm => "{}".to_string(),
        Language::Go => "nil".to_string(),
        Language::Java => "Map.of()".to_string(),
        Language::Csharp => "new Dictionary<string, object>()".to_string(),
        Language::Ruby | Language::Php | Language::R => "[]".to_string(),
        Language::Elixir => "%{}".to_string(),
        Language::Rust => "std::collections::HashMap::new()".to_string(),
        Language::Ffi | Language::C | Language::Jni => "NULL".to_string(),
        Language::Kotlin | Language::KotlinAndroid => "emptyMap()".to_string(),
        Language::Swift => "[:]".to_string(),
        Language::Dart => "{}".to_string(),
        Language::Gleam => "dict.new()".to_string(),
        Language::Zig => ".{}".to_string(),
    }
}

fn null_literal(lang: Language) -> String {
    match lang {
        Language::Python | Language::Rust => "None".to_string(),
        Language::Ruby | Language::Elixir | Language::Go => "nil".to_string(),
        Language::R | Language::Ffi | Language::C | Language::Jni => "NULL".to_string(),
        Language::Zig => "null".to_string(),
        _ => "null".to_string(),
    }
}

fn unit_literal(lang: Language) -> String {
    match lang {
        Language::Python | Language::Rust => "None".to_string(),
        Language::Ruby | Language::Elixir | Language::Go => "nil".to_string(),
        Language::R | Language::Ffi | Language::C | Language::Jni => "NULL".to_string(),
        Language::Swift => "()".to_string(),
        Language::Zig => "{}".to_string(),
        _ => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ParamDef;

    fn param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    fn function() -> FunctionDef {
        FunctionDef {
            name: "parse_document".to_string(),
            rust_path: "mylib::parse_document".to_string(),
            original_rust_path: String::new(),
            params: vec![param("input", TypeRef::String)],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("DemoError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn function_example_uses_async_typescript_await() {
        let rendered = render_function_example(&function(), Language::Node, "Demo");
        assert!(rendered.contains("const result = await parseDocument(\"value\");"));
    }

    #[test]
    fn function_example_uses_rust_try_and_await() {
        let rendered = render_function_example(&function(), Language::Rust, "Demo");
        assert!(rendered.contains("let result = parse_document(\"value\").await?;"));
    }

    #[test]
    fn function_example_uses_go_error_handling() {
        let rendered = render_function_example(&function(), Language::Go, "Demo");
        assert!(rendered.contains("result, err := ParseDocument(\"value\")"));
        assert!(rendered.contains("if err != nil"));
    }

    #[test]
    fn function_example_uses_c_return_type() {
        let rendered = render_function_example(&function(), Language::C, "Demo");
        assert!(rendered.contains("const char *result = demo_parse_document(\"value\");"));
        assert!(!rendered.contains("void *result"));
    }

    #[test]
    fn function_example_uses_c_zeroed_struct_for_by_value_named_param() {
        let mut function = function();
        function.params = vec![param("config", TypeRef::Named("ClientConfig".to_string()))];
        function.return_type = TypeRef::Unit;
        let rendered = render_function_example(&function, Language::C, "Demo");
        assert!(rendered.contains("demo_parse_document((DemoClientConfig){0});"));
        assert!(!rendered.contains("demo_parse_document(NULL);"));
    }

    #[test]
    fn function_example_preserves_matching_authored_example() {
        let mut function = function();
        function.doc =
            "Parse a document.\n\n# Examples\n\n```python\nresult = parse_document(\"file.pdf\")\n```".to_string();
        let rendered = render_function_example(&function, Language::Python, "Demo");
        assert!(rendered.contains("result = parse_document(\"file.pdf\")"));
        assert!(!rendered.contains("parse_document(\"value\")"));
    }

    #[test]
    fn function_example_ignores_non_matching_authored_example() {
        let mut function = function();
        function.doc =
            "Parse a document.\n\n# Examples\n\n```python\nresult = parse_document(\"file.pdf\")\n```".to_string();
        let rendered = render_function_example(&function, Language::Node, "Demo");
        assert!(rendered.contains("const result = await parseDocument(\"value\");"));
        assert!(!rendered.contains("file.pdf"));
    }
}
