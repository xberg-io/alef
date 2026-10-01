use crate::core::config::Language;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeRef};
use crate::docs::naming::{func_name, lang_code_fence, method_name, type_name};
use crate::docs::template_env;
use crate::docs::type_mapping::doc_type;
use heck::ToSnakeCase;

/// ~keep The C ABI's scalar generational-handle type name, mirroring
/// `backends/ffi/type_map.rs` (`TypeRef::Named(_) => "AlefHandle"`). That module
/// is private and under concurrent edit for the handle-ABI rollout, so this is a
/// deliberate literal duplicate rather than an import — keep the two in sync by
/// hand if the ffi backend ever renames the handle type.
const FFI_HANDLE_TYPE_NAME: &str = "AlefHandle";

#[cfg(test)]
pub(crate) fn render_function_example(
    func: &FunctionDef,
    lang: Language,
    ffi_prefix: &str,
    crate_name: &str,
) -> String {
    render_function_example_with_api(func, lang, ffi_prefix, crate_name, &ApiSurface::default())
}

pub(crate) fn render_function_example_with_api(
    func: &FunctionDef,
    lang: Language,
    ffi_prefix: &str,
    crate_name: &str,
    api: &ApiSurface,
) -> String {
    if let Some(example) = authored_example_block(&func.doc, lang) {
        return example;
    }
    let call = function_call_expression(func, lang, ffi_prefix, crate_name, api);
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

#[allow(dead_code)]
pub(crate) fn render_method_example(method: &MethodDef, owner_type: &str, lang: Language, ffi_prefix: &str) -> String {
    render_method_example_with_override(method, owner_type, lang, ffi_prefix, None)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MethodExampleOverride {
    pub(crate) body: String,
}

pub(crate) fn render_method_example_with_override(
    method: &MethodDef,
    owner_type: &str,
    lang: Language,
    ffi_prefix: &str,
    example_override: Option<&MethodExampleOverride>,
) -> String {
    if let Some(example) = authored_example_block(&method.doc, lang) {
        return example;
    }
    if let Some(example_override) = example_override {
        return render_example_block(lang, example_override.body.clone());
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

fn function_call_expression(
    func: &FunctionDef,
    lang: Language,
    ffi_prefix: &str,
    crate_name: &str,
    api: &ApiSurface,
) -> String {
    let name = func_name(&func.name, lang, ffi_prefix);
    let args = render_args_with_api(&func.params, lang, ffi_prefix, api);
    // ~keep Every alef free function becomes a `public static` member of one wrapper class in
    // the emitted C# (`gen_wrapper_class` in backends/csharp/gen_bindings/mod.rs), never a
    // bare top-level function -- C# has no free functions. `csharp_wrapper_class_name` is the
    // same helper the C# backend and `readme::generate_readme` use to name that class, so the
    // example call site cannot spell a class the binding does not actually emit. The async
    // suffix must be applied to `name` before qualifying it, matching
    // `render_csharp_fn_sig`'s naming.
    if lang == Language::Csharp {
        let class_name = crate::codegen::naming::csharp_wrapper_class_name(crate_name, "");
        let member_name = crate::docs::naming::csharp_async_member_name(&name, func.is_async);
        return format!("{class_name}.{member_name}({args})");
    }
    format!("{name}({args})")
}

fn method_call_expression(method: &MethodDef, owner_type: &str, lang: Language, ffi_prefix: &str) -> String {
    let name = method_name(owner_type, &method.name, lang, ffi_prefix);
    // ~keep Same rule as `function_call_expression` and `render_method_signature`'s C# arm: an
    // async opaque method's C# name always carries the `Async` suffix, whether it is an
    // instance method or a static factory (`gen_opaque_method` in
    // backends/csharp/gen_bindings/types/opaque.rs applies it to both).
    let name = if lang == Language::Csharp {
        crate::docs::naming::csharp_async_member_name(&name, method.is_async)
    } else {
        name
    };
    let args = render_args(&method.params, lang, ffi_prefix);
    if method.is_static {
        return static_method_call(method, owner_type, &name, &args, lang, ffi_prefix);
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

fn static_method_call(
    method: &MethodDef,
    owner_type: &str,
    name: &str,
    args: &str,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    let owner = type_name(owner_type, lang, ffi_prefix);
    match lang {
        Language::Ruby => format!("{owner}.{name}({args})"),
        Language::Php => format!("{owner}::{name}({args})"),
        Language::Rust => format!("{owner}::{name}({args})"),
        Language::C | Language::Ffi | Language::Jni => format!("{name}({args})"),
        // ~keep Go has no static members: `gen_method_wrapper` renders
        // `method_signature_static.jinja` -- `func {{ receiver_type }}{{ method_name }}(...)` --
        // a package-level function whose name concatenates type and method. The `{owner}.{name}`
        // fallthrough produced `DownloadManager.New(...)`, which contradicted the
        // `func DownloadManagerNew(...)` signature printed directly above it on the same page.
        Language::Go => format!("{owner}{name}({args})"),
        // ~keep Zig's static shape is the same free-function-with-type-suffix as the signature
        // arm documents (`opaque_static_signature.jinja`:
        // `pub fn {{ method_snake }}_{{ type_snake }}(...)`), so the call site must not use
        // member syntax either. `name` is already snake_case here (`func_name`'s Zig arm).
        Language::Zig => format!("{name}_{}({args})", owner.to_snake_case()),
        // ~keep The one shape the C# backend promotes to a real constructor
        // (`is_static_constructor` -> `opaque_static_constructor_signature.jinja`) is invoked
        // with `new`, not as a member of the type. Same predicate as the signature arm, so the
        // example and the signature above it cannot disagree.
        Language::Csharp if crate::docs::signatures::is_csharp_static_constructor(method, owner_type) => {
            format!("new {owner}({args})")
        }
        _ => format!("{owner}.{name}({args})"),
    }
}

fn render_args(params: &[ParamDef], lang: Language, ffi_prefix: &str) -> String {
    render_args_with_api(params, lang, ffi_prefix, &ApiSurface::default())
}

fn render_args_with_api(params: &[ParamDef], lang: Language, ffi_prefix: &str, api: &ApiSurface) -> String {
    params
        .iter()
        .map(|param| {
            let value = sample_param_value(param, lang, ffi_prefix, api);
            match lang {
                Language::Ruby if param.optional => format!("{}: {value}", param.name.to_snake_case()),
                Language::Python if param.optional => format!("{}={value}", param.name.to_snake_case()),
                _ => value,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn sample_param_value(param: &ParamDef, lang: Language, ffi_prefix: &str, api: &ApiSurface) -> String {
    let mut value = crate::docs::rust_param_samples::rust_original_param_sample(param, lang, api)
        .unwrap_or_else(|| sample_value(&param.ty, lang, ffi_prefix));
    // ~keep The Rust page documents the crate itself, so a borrowed param has to be *passed*
    // borrowed or the example will not compile against the signature printed directly above it.
    // String/Char/Path/Bytes are excluded because their samples are already borrow-shaped
    // literals (`"value"` is a `&str`, `b"data"` a `&[u8; N]`). For `Option<&T>`, borrow the
    // inner sample before wrapping it in `Some`, matching `rust_param_type`'s source shape.
    let is_borrow_shaped_literal = param.original_type.is_none()
        && matches!(
            &param.ty,
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Bytes
        );
    if lang == Language::Rust && param.is_ref && !is_borrow_shaped_literal {
        let borrow = if param.is_mut { "&mut " } else { "&" };
        value = format!("{borrow}{value}");
    }
    if lang == Language::Rust && param.optional {
        value = format!("Some({value})");
    }

    if matches!(lang, Language::Ffi | Language::C) && matches!(&param.ty, TypeRef::Named(_)) {
        // ~keep Every Named-type param is a scalar `AlefHandle` (uint64_t) in the C
        // ABI, whether it's passed by value, by ref, or is optional — there is no
        // struct left to zero-initialize and no pointer left to null out. `0` is
        // the only value that both compiles and matches the reserved "no handle"
        // sentinel used by `ffi_null_return_value` in
        // backends/ffi/gen_bindings/helpers.rs.
        return "0".to_string();
    }

    // ~keep Jni IS reachable from this docs pipeline — the arms below run against real
    // configs. `"jni"` deserializes (core/config/extras.rs:30); core/config/new_config.rs:159
    // explicitly accepts it in `languages`, erroring only when `kotlin_android` is absent;
    // bin_cli/helpers.rs:231-237 (`resolve_languages_inner`, behind `resolve_doc_languages`)
    // returns `config.languages` verbatim with no Jni filter; and docs/mod.rs:56 iterates that
    // list straight into `generate_lang_doc`. So `languages = ["kotlin_android", "jni"]` — a
    // legal config, and the pairing backends/jni/mod.rs:15-18 says is the only way the backend
    // is driven — renders these arms today.
    //
    // What they render is wrong, and not merely stale. The JNI backend emits *Rust*: a
    // `<crate>-jni` crate of `pub unsafe extern "system" fn Java_<pkg>_<Bridge>_<method>` shims
    // built on the `jni` crate (backends/jni/mod.rs:1-18). Its boundary types are JNI's, not
    // C's: opaque handles are `jlong` (gen_shims/function_shims.rs:212,
    // gen_shims/method_shims.rs:95), strings and paths are `JString`
    // (gen_shims/function_shims.rs:137-153,192-208), bools are `jboolean` and every other
    // shape is JSON over `jstring` (gen_shims/type_helpers.rs:9,24,57,64). Errors do not
    // return a sentinel at all — the shim calls `env.throw_new` and returns a type-specific
    // zero (`()`, `false`, `0`, `std::ptr::null_mut()`; gen_shims/type_helpers.rs:29-42,
    // templates/runtime_helpers.rs.jinja:32-42) — and a `0` handle *param* is rejected with a
    // thrown exception (templates/opaque_handle_unmarshal.rs.jinja:1-4), so nothing here is
    // nullable in the C sense. It emits no C at all, so every C-shaped token these arms
    // produce — `NULL`, the `({ty}){0}` compound literal below, `const char*`, `bool`,
    // `void*`, per-type struct names — documents an ABI that has never existed for this
    // backend. `docs::naming::lang_slug` compounds it by mapping Jni to `"c"`,
    // so the page is titled "C API Reference" and written to `api-c.md`, the same path the real
    // C/Ffi page uses. These arms are left as-is rather than half-migrated to the handle ABI,
    // which would be just as fictional; fixing them means modelling the JNI shim surface (or
    // declining to emit a Jni page) and is tracked separately.
    if matches!(lang, Language::Jni)
        && let TypeRef::Named(name) = &param.ty
    {
        if param.optional || param.is_ref {
            return "NULL".to_string();
        }

        let ty = type_name(name, lang, ffi_prefix);
        return format!("({ty}){{0}}");
    }

    value
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
        // ~keep Named-type returns are a scalar `AlefHandle`, not a pointer to a
        // struct named after the Rust type — see FFI_HANDLE_TYPE_NAME. Jni is a reachable
        // target that falls through to the typed-pointer arm below, which is wrong for it:
        // a JNI shim returns `jlong` and the Kotlin caller holds a `Long`. See the note in
        // sample_param_value.
        TypeRef::Named(_) if matches!(lang, Language::Ffi | Language::C) => {
            format!("{} result", type_name(FFI_HANDLE_TYPE_NAME, lang, ffi_prefix))
        }
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
        Language::Elixir | Language::R => "%{}".to_string(),
        Language::Go => format!("{ty}{{}}"),
        Language::Rust => format!("{ty}::default()"),
        // ~keep Same scalar-handle reasoning as sample_param_value. In practice the Ffi/C arm
        // is unreachable today — sample_param_value intercepts every Named-type param before
        // falling through to sample_value — but it is fixed for defense-in-depth and
        // consistency with the sentinel scheme. The Jni arm is reachable and its `NULL` is
        // wrong: the value is a `jlong` handle, and `NULL` is not a token the JNI backend
        // emits anywhere. See the note in sample_param_value.
        Language::Ffi | Language::C => "0".to_string(),
        Language::Jni => "NULL".to_string(),
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
        let rendered = render_function_example(&function(), Language::Node, "Demo", "Demo");
        assert!(rendered.contains("const result = await parseDocument(\"value\");"));
    }

    #[test]
    fn function_example_uses_rust_try_and_await() {
        let rendered = render_function_example(&function(), Language::Rust, "Demo", "Demo");
        assert!(rendered.contains("let result = parse_document(\"value\").await?;"));
    }

    #[test]
    fn function_example_wraps_optional_rust_string_and_u32_samples() {
        let mut function = function();
        let mut name = param("name", TypeRef::String);
        name.optional = true;
        let mut limit = param("limit", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32));
        limit.optional = true;
        function.params = vec![name, limit];

        let rendered = render_function_example(&function, Language::Rust, "Demo", "Demo");

        assert!(rendered.contains("parse_document(Some(\"value\"), Some(42)).await?"));
    }

    #[test]
    fn function_example_uses_go_error_handling() {
        let rendered = render_function_example(&function(), Language::Go, "Demo", "Demo");
        assert!(rendered.contains("result, err := ParseDocument(\"value\")"));
        assert!(rendered.contains("if err != nil"));
    }

    #[test]
    fn function_example_uses_c_return_type() {
        let rendered = render_function_example(&function(), Language::C, "Demo", "Demo");
        assert!(rendered.contains("const char *result = demo_parse_document(\"value\");"));
        assert!(!rendered.contains("void *result"));
    }

    #[test]
    fn function_example_uses_c_scalar_handle_zero_for_by_value_named_param() {
        let mut function = function();
        function.params = vec![param("config", TypeRef::Named("ClientConfig".to_string()))];
        function.return_type = TypeRef::Unit;
        let rendered = render_function_example(&function, Language::C, "Demo", "Demo");
        assert!(rendered.contains("demo_parse_document(0);"));
        assert!(!rendered.contains("demo_parse_document(NULL);"));
        assert!(!rendered.contains("(DEMOClientConfig){0}"));
    }

    #[test]
    fn function_example_uses_c_scalar_handle_zero_for_optional_named_param() {
        let mut function = function();
        let mut config_param = param("options", TypeRef::Named("ClientConfig".to_string()));
        config_param.optional = true;
        function.params = vec![config_param];
        function.return_type = TypeRef::Unit;
        let rendered = render_function_example(&function, Language::C, "Demo", "Demo");
        assert!(rendered.contains("demo_parse_document(0);"));
        assert!(!rendered.contains("demo_parse_document(NULL);"));
    }

    #[test]
    fn function_example_uses_c_alef_handle_type_for_named_return() {
        let mut function = function();
        function.return_type = TypeRef::Named("ConversionResult".to_string());
        let rendered = render_function_example(&function, Language::C, "Demo", "Demo");
        assert!(rendered.contains("DEMOAlefHandle result = demo_parse_document(\"value\");"));
        assert!(!rendered.contains("DEMOConversionResult *result"));
    }

    #[test]
    fn function_example_c_optional_string_param_still_uses_null() {
        let mut function = function();
        function.params = vec![param("note", TypeRef::Optional(Box::new(TypeRef::String)))];
        let rendered = render_function_example(&function, Language::C, "Demo", "Demo");
        assert!(rendered.contains("demo_parse_document(NULL)"));
    }

    #[test]
    fn function_example_preserves_matching_authored_example() {
        let mut function = function();
        function.doc =
            "Parse a document.\n\n# Examples\n\n```python\nresult = parse_document(\"file.pdf\")\n```".to_string();
        let rendered = render_function_example(&function, Language::Python, "Demo", "Demo");
        assert!(rendered.contains("result = parse_document(\"file.pdf\")"));
        assert!(!rendered.contains("parse_document(\"value\")"));
    }

    #[test]
    fn function_example_ignores_non_matching_authored_example() {
        let mut function = function();
        function.doc =
            "Parse a document.\n\n# Examples\n\n```python\nresult = parse_document(\"file.pdf\")\n```".to_string();
        let rendered = render_function_example(&function, Language::Node, "Demo", "Demo");
        assert!(rendered.contains("const result = await parseDocument(\"value\");"));
        assert!(!rendered.contains("file.pdf"));
    }

    fn method(name: &str, is_async: bool, is_static: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async,
            is_static,
            error_type: None,
            doc: String::new(),
            receiver: None,
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// ~keep Reproduces a real-consumer symptom: the C# binding wraps every free function as a
    /// `public static` member of one wrapper class (`gen_wrapper_class` in
    /// backends/csharp/gen_bindings/mod.rs) and appends `Async` to every async member
    /// (`gen_bindings/methods/wrappers.rs`) -- C# has no free functions at all. The docs example
    /// for a free function used to call the bare, un-suffixed name with no receiver, which is
    /// not code the binding this page documents can even compile.
    #[test]
    fn function_example_csharp_uses_wrapper_class_and_async_suffix() {
        let rendered = render_function_example(&function(), Language::Csharp, "Demo", "sample-parser-rs");
        assert!(
            rendered.contains("var result = await SampleParserConverter.ParseDocumentAsync(\"value\");"),
            "{rendered}"
        );
    }

    /// ~keep Control: a sync C# free function must NOT gain the `Async` suffix or an `await` --
    /// guards against an over-eager fix that suffixes every C# call regardless of `is_async`.
    #[test]
    fn function_example_csharp_sync_function_omits_async_suffix_and_await() {
        let mut sync_function = function();
        sync_function.is_async = false;
        let rendered = render_function_example(&sync_function, Language::Csharp, "Demo", "sample-parser-rs");
        assert!(
            rendered.contains("var result = SampleParserConverter.ParseDocument(\"value\");"),
            "{rendered}"
        );
        assert!(!rendered.contains("Async"), "{rendered}");
        assert!(!rendered.contains("await"), "{rendered}");
    }

    #[test]
    fn method_example_csharp_instance_method_uses_async_suffix() {
        let async_method = method("process_document", true, false);
        let rendered = render_method_example(&async_method, "DocumentProcessor", Language::Csharp, "Demo");
        assert!(
            rendered.contains("var result = await instance.ProcessDocumentAsync();"),
            "{rendered}"
        );
    }

    #[test]
    fn method_example_csharp_static_method_uses_async_suffix() {
        let async_method = method("from_bytes", true, true);
        let rendered = render_method_example(&async_method, "DocumentProcessor", Language::Csharp, "Demo");
        assert!(
            rendered.contains("var result = await DocumentProcessor.FromBytesAsync();"),
            "{rendered}"
        );
    }

    /// ~keep The dominant defect shape in this pipeline is two independent renderers deriving
    /// the same fact differently -- here, the C# member name for an async function. Pins the
    /// example call site and the signature line to name the SAME emitted member for the SAME
    /// `FunctionDef`, rather than asserting each against a literal that could drift
    /// independently the way they already had.
    #[test]
    fn function_example_csharp_agrees_with_signature_on_async_member_name() {
        let func = function();
        let signature = crate::docs::signatures::render_function_signature(
            &func,
            Language::Csharp,
            "Demo",
            "sample-parser-rs",
            &crate::core::ir::ApiSurface::default(),
        );
        let example = render_function_example(&func, Language::Csharp, "Demo", "sample-parser-rs");
        assert!(signature.contains("ParseDocumentAsync"), "{signature}");
        assert!(example.contains("ParseDocumentAsync"), "{example}");
    }
}
