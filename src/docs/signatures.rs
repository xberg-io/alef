use crate::core::config::Language;
use crate::core::ir::{FunctionDef, MethodDef, TypeRef};
use crate::docs::naming::{field_name, func_name, to_camel_case, type_name};
use crate::docs::type_mapping::doc_type;
use heck::{ToPascalCase, ToSnakeCase};

pub(crate) fn render_function_signature(func: &FunctionDef, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python => render_python_fn_sig(func, ffi_prefix),
        Language::Node | Language::Wasm => render_typescript_fn_sig(func, ffi_prefix),
        Language::Go => render_go_fn_sig(func, ffi_prefix),
        Language::Java => render_java_fn_sig(func, ffi_prefix),
        Language::Ruby => render_ruby_fn_sig(func),
        Language::Ffi | Language::C | Language::Jni => render_c_fn_sig(func, ffi_prefix),
        Language::Php => render_php_fn_sig(func, ffi_prefix),
        Language::Elixir => render_elixir_fn_sig(func),
        Language::R => render_r_fn_sig(func),
        Language::Csharp => render_csharp_fn_sig(func, ffi_prefix),
        Language::Rust => render_rust_fn_sig(func, ffi_prefix),
        Language::Kotlin | Language::KotlinAndroid => render_kotlin_fn_sig(func, ffi_prefix),
        Language::Swift => render_swift_fn_sig(func, ffi_prefix),
        Language::Dart => render_dart_fn_sig(func, ffi_prefix),
        Language::Zig => render_zig_fn_sig(func, ffi_prefix),
        Language::Gleam => {
            format!("// Phase 1: {lang} backend signature generation")
        }
    }
}

pub(crate) fn render_python_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Python, ffi_prefix);
            if p.optional {
                format!("{pname}: {pty} = None")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Python, ffi_prefix);
    // Python bindings wrap async Rust functions in sync Python functions,
    // so always emit `def` (never `async def`) for the public API.
    format!("def {}({}) -> {}", name, params.join(", "), ret)
}

pub(crate) fn render_typescript_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Node, ffi_prefix);
            if p.optional {
                format!("{pname}?: {pty}")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Node, ffi_prefix);
    if func.is_async {
        format!("function {}({}): Promise<{}>", name, params.join(", "), ret)
    } else {
        format!("function {}({}): {}", name, params.join(", "), ret)
    }
}

pub(crate) fn render_go_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_pascal_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Go, ffi_prefix);
            format!("{pname} {pty}")
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Go, ffi_prefix);
    if func.error_type.is_some() {
        if ret.is_empty() {
            // Result<(), E> → func Foo() error
            format!("func {}({}) error", name, params.join(", "))
        } else {
            format!("func {}({}) ({}, error)", name, params.join(", "), ret)
        }
    } else if ret.is_empty() {
        format!("func {}({})", name, params.join(", "))
    } else {
        format!("func {}({}) {}", name, params.join(", "), ret)
    }
}

pub(crate) fn render_java_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    let ret = doc_type(&func.return_type, Language::Java, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Java, ffi_prefix);
            format!("{pty} {pname}")
        })
        .collect();
    let throws = func
        .error_type
        .as_ref()
        .map(|e| format!(" throws {}", type_name(e, Language::Java, ffi_prefix)))
        .unwrap_or_default();
    format!("public static {} {}({}){}", ret, name, params.join(", "), throws)
}

pub(crate) fn render_ruby_fn_sig(func: &FunctionDef) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            if p.optional { format!("{pname}: nil") } else { pname }
        })
        .collect();
    format!("def self.{}({})", name, params.join(", "))
}

pub(crate) fn render_c_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let prefix = ffi_prefix.to_snake_case();
    let name = format!("{}_{}", prefix, func.name.to_snake_case());
    let ret = doc_type(&func.return_type, Language::Ffi, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Ffi, ffi_prefix);
            format!("{pty} {pname}")
        })
        .collect();
    // For Named types (structs), return a pointer; for primitives/strings, return directly
    let ret_str = match &func.return_type {
        TypeRef::Named(_) => format!("{}*", ret),
        TypeRef::Unit => "void".to_string(),
        _ => ret,
    };
    format!("{} {}({});", ret_str, name, params.join(", "))
}

pub(crate) fn render_php_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = format!("${}", to_camel_case(&p.name));
            let pty = doc_type(&p.ty, Language::Php, ffi_prefix);
            if p.optional {
                format!("?{pty} {pname} = null")
            } else {
                format!("{pty} {pname}")
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Php, ffi_prefix);
    format!("public static function {}({}): {}", name, params.join(", "), ret)
}

pub(crate) fn render_elixir_fn_sig(func: &FunctionDef) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func.params.iter().map(|p| p.name.to_snake_case()).collect();
    format!(
        "@spec {}({}) :: {{:ok, term()}} | {{:error, term()}}\ndef {}({})",
        name,
        params.join(", "),
        name,
        params.join(", ")
    )
}

pub(crate) fn render_r_fn_sig(func: &FunctionDef) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            if p.optional { format!("{pname} = NULL") } else { pname }
        })
        .collect();
    format!("{}({})", name, params.join(", "))
}

pub(crate) fn render_csharp_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_pascal_case();
    let ret = doc_type(&func.return_type, Language::Csharp, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Csharp, ffi_prefix);
            if p.optional {
                format!("{pty}? {pname} = null")
            } else {
                format!("{pty} {pname}")
            }
        })
        .collect();
    if func.is_async {
        let async_name = if name.ends_with("Async") {
            name.clone()
        } else {
            format!("{name}Async")
        };
        let task_ret = if ret == "void" {
            "Task".to_string()
        } else {
            format!("Task<{ret}>")
        };
        format!("public static async {} {}({})", task_ret, async_name, params.join(", "))
    } else {
        format!("public static {} {}({})", ret, name, params.join(", "))
    }
}

pub(crate) fn render_rust_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Rust, ffi_prefix);
            if p.optional {
                format!("{pname}: Option<{pty}>")
            } else {
                // Use references for String and Vec types in parameters
                match &p.ty {
                    TypeRef::String | TypeRef::Char => format!("{pname}: &str"),
                    TypeRef::Bytes => format!("{pname}: &[u8]"),
                    _ => format!("{pname}: {pty}"),
                }
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Rust, ffi_prefix);
    let error_part = if let Some(err) = &func.error_type {
        let err_ty = type_name(err, Language::Rust, ffi_prefix);
        if ret == "()" {
            format!(" -> Result<(), {err_ty}>")
        } else {
            format!(" -> Result<{ret}, {err_ty}>")
        }
    } else if ret == "()" {
        String::new()
    } else {
        format!(" -> {ret}")
    };
    if func.is_async {
        format!("pub async fn {}({}){}", name, params.join(", "), error_part)
    } else {
        format!("pub fn {}({}){}", name, params.join(", "), error_part)
    }
}

pub(crate) fn render_kotlin_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    let ret = doc_type(&func.return_type, Language::Kotlin, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Kotlin, ffi_prefix);
            if p.optional {
                format!("{pname}: {pty}? = null")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let throws = func
        .error_type
        .as_ref()
        .map(|e| format!("@Throws({}::class)\n", type_name(e, Language::Kotlin, ffi_prefix)))
        .unwrap_or_default();
    let ret_part = if ret == "Unit" {
        String::new()
    } else {
        format!(": {ret}")
    };
    format!("{throws}fun {name}({}){ret_part}", params.join(", "))
}

pub(crate) fn render_swift_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    let ret = doc_type(&func.return_type, Language::Swift, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Swift, ffi_prefix);
            if p.optional {
                format!("{pname}: {pty}? = nil")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let throws = if func.error_type.is_some() { " throws" } else { "" };
    let ret_part = if ret == "Void" {
        String::new()
    } else {
        format!(" -> {ret}")
    };
    format!("public static func {name}({}){throws}{ret_part}", params.join(", "))
}

pub(crate) fn render_dart_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    let ret = doc_type(&func.return_type, Language::Dart, ffi_prefix);
    let required: Vec<String> = func
        .params
        .iter()
        .filter(|p| !p.optional)
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Dart, ffi_prefix);
            format!("{pty} {pname}")
        })
        .collect();
    let optional: Vec<String> = func
        .params
        .iter()
        .filter(|p| p.optional)
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Dart, ffi_prefix);
            format!("{pty}? {pname}")
        })
        .collect();
    let mut all_params = required;
    if !optional.is_empty() {
        all_params.push(format!("[{}]", optional.join(", ")));
    }
    format!("{ret} {name}({})", all_params.join(", "))
}

pub(crate) fn render_zig_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_snake_case();
    let ret = doc_type(&func.return_type, Language::Zig, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Zig, ffi_prefix);
            if p.optional {
                format!("{pname}: ?{pty}")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let ret_str = if let Some(err) = &func.error_type {
        let err_ty = type_name(err, Language::Zig, ffi_prefix);
        if ret == "void" {
            format!("{err_ty}!void")
        } else {
            format!("{err_ty}!{ret}")
        }
    } else {
        ret
    };
    format!("pub fn {name}({}) {ret_str}", params.join(", "))
}

pub(crate) fn render_method_signature(
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    let name = func_name(&method.name, lang, ffi_prefix);
    let ret = doc_type(&method.return_type, lang, ffi_prefix);

    match lang {
        Language::Python => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = field_name(&p.name, lang);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pname}: {pty}")
                })
                .collect();
            if method.is_static {
                format!("@staticmethod\ndef {}({}) -> {}", name, params.join(", "), ret)
            } else {
                let mut all_params = vec!["self".to_string()];
                all_params.extend(params);
                format!("def {}({}) -> {}", name, all_params.join(", "), ret)
            }
        }
        Language::Node | Language::Wasm => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = field_name(&p.name, lang);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pname}: {pty}")
                })
                .collect();
            if method.is_static {
                format!("static {}({}): {}", name, params.join(", "), ret)
            } else {
                format!("{}({}): {}", name, params.join(", "), ret)
            }
        }
        Language::Ruby => {
            let params: Vec<String> = method.params.iter().map(|p| p.name.to_snake_case()).collect();
            if method.is_static {
                format!("def self.{}({})", name, params.join(", "))
            } else {
                format!("def {}({})", name, params.join(", "))
            }
        }
        Language::Go => {
            // Go methods: func (receiver *TypeName) MethodName(params) ReturnType
            let go_receiver_type = type_name(type_name_str, Language::Go, ffi_prefix);
            let receiver = format!("o *{go_receiver_type}");
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pname} {pty}")
                })
                .collect();
            if method.error_type.is_some() {
                if ret.is_empty() {
                    format!("func ({receiver}) {}({}) error", name, params.join(", "))
                } else {
                    format!("func ({receiver}) {}({}) ({}, error)", name, params.join(", "), ret)
                }
            } else if ret.is_empty() {
                format!("func ({receiver}) {}({})", name, params.join(", "))
            } else {
                format!("func ({receiver}) {}({}) {}", name, params.join(", "), ret)
            }
        }
        Language::Java => {
            // Java: avoid `default` reserved keyword
            let java_name = if name == "default" {
                "defaultOptions".to_string()
            } else {
                name.clone()
            };
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            let throws = method
                .error_type
                .as_ref()
                .map(|e| format!(" throws {}", type_name(e, lang, ffi_prefix)))
                .unwrap_or_default();
            if method.is_static {
                format!("public static {} {}({}){}", ret, java_name, params.join(", "), throws)
            } else {
                format!("public {} {}({}){}", ret, java_name, params.join(", "), throws)
            }
        }
        Language::Csharp => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            if method.is_async {
                let async_name = if name.ends_with("Async") {
                    name.clone()
                } else {
                    format!("{name}Async")
                };
                let task_ret = if ret == "void" {
                    "Task".to_string()
                } else {
                    format!("Task<{ret}>")
                };
                format!("public async {} {}({})", task_ret, async_name, params.join(", "))
            } else {
                format!("public {} {}({})", ret, name, params.join(", "))
            }
        }
        Language::Php => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = format!("${}", to_camel_case(&p.name));
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            if method.is_static {
                format!("public static function {}({}): {}", name, params.join(", "), ret)
            } else {
                format!("public function {}({}): {}", name, params.join(", "), ret)
            }
        }
        Language::Elixir => {
            let params: Vec<String> = method.params.iter().map(|p| p.name.to_snake_case()).collect();
            format!("def {}({})", name, params.join(", "))
        }
        Language::R => {
            let params: Vec<String> = method.params.iter().map(|p| p.name.to_snake_case()).collect();
            format!("{}({})", name, params.join(", "))
        }
        Language::Ffi | Language::C | Language::Jni => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_snake_case();
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            format!("{} {}({});", ret, name, params.join(", "))
        }
        Language::Rust => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_snake_case();
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: Option<{pty}>")
                    } else {
                        match &p.ty {
                            TypeRef::String | TypeRef::Char => format!("{pname}: &str"),
                            TypeRef::Bytes => format!("{pname}: &[u8]"),
                            _ => format!("{pname}: {pty}"),
                        }
                    }
                })
                .collect();
            if method.is_static {
                if ret == "()" {
                    format!("pub fn {}({})", name, params.join(", "))
                } else {
                    format!("pub fn {}({}) -> {}", name, params.join(", "), ret)
                }
            } else {
                let mut all_params = vec!["&self".to_string()];
                all_params.extend(params);
                if ret == "()" {
                    format!("pub fn {}({})", name, all_params.join(", "))
                } else {
                    format!("pub fn {}({}) -> {}", name, all_params.join(", "), ret)
                }
            }
        }
        Language::Kotlin | Language::KotlinAndroid => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: {pty}? = null")
                    } else {
                        format!("{pname}: {pty}")
                    }
                })
                .collect();
            let throws = method
                .error_type
                .as_ref()
                .map(|e| format!("@Throws({}::class)\n", type_name(e, lang, ffi_prefix)))
                .unwrap_or_default();
            let ret_part = if ret == "Unit" {
                String::new()
            } else {
                format!(": {ret}")
            };
            if method.is_static {
                format!("{throws}@JvmStatic\nfun {name}({}){ret_part}", params.join(", "))
            } else {
                format!("{throws}fun {name}({}){ret_part}", params.join(", "))
            }
        }
        Language::Swift => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: {pty}? = nil")
                    } else {
                        format!("{pname}: {pty}")
                    }
                })
                .collect();
            let throws = if method.error_type.is_some() { " throws" } else { "" };
            let ret_part = if ret == "Void" {
                String::new()
            } else {
                format!(" -> {ret}")
            };
            if method.is_static {
                format!("public static func {name}({}){throws}{ret_part}", params.join(", "))
            } else {
                format!("public func {name}({}){throws}{ret_part}", params.join(", "))
            }
        }
        Language::Dart => {
            let required: Vec<String> = method
                .params
                .iter()
                .filter(|p| !p.optional)
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            let optional: Vec<String> = method
                .params
                .iter()
                .filter(|p| p.optional)
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty}? {pname}")
                })
                .collect();
            let mut all_params = required;
            if !optional.is_empty() {
                all_params.push(format!("[{}]", optional.join(", ")));
            }
            let static_kw = if method.is_static { "static " } else { "" };
            format!("{static_kw}{ret} {name}({})", all_params.join(", "))
        }
        Language::Zig => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_snake_case();
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: ?{pty}")
                    } else {
                        format!("{pname}: {pty}")
                    }
                })
                .collect();
            let ret_str = if let Some(err) = &method.error_type {
                let err_ty = type_name(err, lang, ffi_prefix);
                if ret == "void" {
                    format!("{err_ty}!void")
                } else {
                    format!("{err_ty}!{ret}")
                }
            } else {
                ret
            };
            let receiver_ty = type_name(type_name_str, lang, ffi_prefix);
            let mut all_params = if method.is_static {
                Vec::new()
            } else {
                vec![format!("self: *const {receiver_ty}")]
            };
            all_params.extend(params);
            format!("pub fn {name}({}) {ret_str}", all_params.join(", "))
        }
        Language::Gleam => {
            format!("// Phase 1: {lang} backend method signature generation")
        }
    }
}

#[cfg(test)]
#[path = "signatures/tests.rs"]
mod tests;
