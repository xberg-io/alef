use crate::{doc_type, field_name, func_name, to_camel_case, type_name};
use alef_core::config::Language;
use alef_core::ir::{FunctionDef, MethodDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};

pub(crate) fn render_function_signature(func: &FunctionDef, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python => render_python_fn_sig(func, ffi_prefix),
        Language::Node | Language::Wasm => render_typescript_fn_sig(func, ffi_prefix),
        Language::Go => render_go_fn_sig(func, ffi_prefix),
        Language::Java => render_java_fn_sig(func, ffi_prefix),
        Language::Ruby => render_ruby_fn_sig(func),
        Language::Ffi => render_c_fn_sig(func, ffi_prefix),
        Language::Php => render_php_fn_sig(func, ffi_prefix),
        Language::Elixir => render_elixir_fn_sig(func),
        Language::R => render_r_fn_sig(func),
        Language::Csharp => render_csharp_fn_sig(func, ffi_prefix),
        Language::Rust => render_rust_fn_sig(func, ffi_prefix),
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
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
        Language::Ffi => {
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
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            format!("// Phase 1: {lang} backend method signature generation")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_PREFIX, make_function, make_method, make_param};
    use alef_core::config::Language;
    use alef_core::ir::{PrimitiveType, TypeRef};

    // ---------------------------------------------------------------------------
    // render_method_signature — Python
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_python_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
        assert_eq!(sig, "def get_text(self, page: int) -> str");
    }

    #[test]
    fn test_render_method_signature_python_async() {
        let method = make_method("process", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
        assert_eq!(sig, "def process(self) -> str");
    }

    #[test]
    fn test_render_method_signature_python_static() {
        let method = make_method(
            "create",
            vec![make_param("name", TypeRef::String, false)],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
        assert_eq!(sig, "@staticmethod\ndef create(name: str) -> Document");
    }

    #[test]
    fn test_render_method_signature_python_optional_return() {
        let method = make_method(
            "find",
            vec![make_param("query", TypeRef::String, false)],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Python, TEST_PREFIX);
        assert_eq!(sig, "def find(self, query: str) -> str | None");
    }

    #[test]
    fn test_render_method_signature_python_with_error_type() {
        let method = make_method(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            false,
            Some("ParseError"),
        );
        let sig = render_method_signature(&method, "Parser", Language::Python, TEST_PREFIX);
        assert_eq!(sig, "def parse(self, source: str) -> Ast");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — Node / TypeScript
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_node_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
        assert_eq!(sig, "getText(page: number): string");
    }

    #[test]
    fn test_render_method_signature_node_async() {
        let method = make_method("process", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
        assert_eq!(sig, "process(): string");
    }

    #[test]
    fn test_render_method_signature_node_static() {
        let method = make_method(
            "create",
            vec![make_param("name", TypeRef::String, false)],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
        assert_eq!(sig, "static create(name: string): Document");
    }

    #[test]
    fn test_render_method_signature_node_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Node, TEST_PREFIX);
        assert_eq!(sig, "find(): string | null");
    }

    #[test]
    fn test_render_method_signature_node_with_error_type() {
        let method = make_method(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            false,
            Some("ParseError"),
        );
        let sig = render_method_signature(&method, "Parser", Language::Node, TEST_PREFIX);
        assert_eq!(sig, "parse(source: string): Ast");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — Rust
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_rust_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Rust, TEST_PREFIX);
        assert_eq!(sig, "pub fn get_text(&self, page: u32) -> String");
    }

    #[test]
    fn test_render_method_signature_rust_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Rust, TEST_PREFIX);
        assert_eq!(sig, "pub fn fetch(&self) -> String");
    }

    #[test]
    fn test_render_method_signature_rust_static() {
        let method = make_method(
            "new",
            vec![make_param("name", TypeRef::String, false)],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Rust, TEST_PREFIX);
        assert_eq!(sig, "pub fn new(name: &str) -> Document");
    }

    #[test]
    fn test_render_method_signature_rust_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::Named("Node".to_string()))),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Tree", Language::Rust, TEST_PREFIX);
        assert_eq!(sig, "pub fn find(&self) -> Option<Node>");
    }

    #[test]
    fn test_render_method_signature_rust_with_error_type() {
        let method = make_method(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            false,
            Some("ParseError"),
        );
        let sig = render_method_signature(&method, "Parser", Language::Rust, TEST_PREFIX);
        assert_eq!(sig, "pub fn parse(&self, source: &str) -> Ast");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — Go
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_go_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Go, TEST_PREFIX);
        assert_eq!(sig, "func (o *Document) GetText(page uint32) string");
    }

    #[test]
    fn test_render_method_signature_go_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Go, TEST_PREFIX);
        assert_eq!(sig, "func (o *Client) Fetch() string");
    }

    #[test]
    fn test_render_method_signature_go_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Go, TEST_PREFIX);
        assert_eq!(sig, "func (o *Corpus) Find() *string");
    }

    #[test]
    fn test_render_method_signature_go_with_error_type() {
        let method = make_method(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            false,
            Some("ParseError"),
        );
        let sig = render_method_signature(&method, "Parser", Language::Go, TEST_PREFIX);
        assert_eq!(sig, "func (o *Parser) Parse(source string) (Ast, error)");
    }

    #[test]
    fn test_render_method_signature_go_error_type_unit_return() {
        let method = make_method("save", vec![], TypeRef::Unit, false, false, Some("IoError"));
        let sig = render_method_signature(&method, "File", Language::Go, TEST_PREFIX);
        assert_eq!(sig, "func (o *File) Save() error");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — Ruby
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_ruby_sync_with_params() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Ruby, TEST_PREFIX);
        assert_eq!(sig, "def get_text(page)");
    }

    #[test]
    fn test_render_method_signature_ruby_static() {
        let method = make_method(
            "create",
            vec![make_param("name", TypeRef::String, false)],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Ruby, TEST_PREFIX);
        assert_eq!(sig, "def self.create(name)");
    }

    #[test]
    fn test_render_method_signature_ruby_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Ruby, TEST_PREFIX);
        assert_eq!(sig, "def fetch()");
    }

    #[test]
    fn test_render_method_signature_ruby_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Ruby, TEST_PREFIX);
        assert_eq!(sig, "def find()");
    }

    #[test]
    fn test_render_method_signature_ruby_with_error_type() {
        let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
        let sig = render_method_signature(&method, "Parser", Language::Ruby, TEST_PREFIX);
        assert_eq!(sig, "def parse()");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — PHP
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_php_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Php, TEST_PREFIX);
        assert_eq!(sig, "public function getText(int $page): string");
    }

    #[test]
    fn test_render_method_signature_php_static() {
        let method = make_method(
            "create",
            vec![make_param("name", TypeRef::String, false)],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Php, TEST_PREFIX);
        assert_eq!(sig, "public static function create(string $name): Document");
    }

    #[test]
    fn test_render_method_signature_php_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Php, TEST_PREFIX);
        assert_eq!(sig, "public function find(): ?string");
    }

    #[test]
    fn test_render_method_signature_php_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Php, TEST_PREFIX);
        assert_eq!(sig, "public function fetch(): string");
    }

    #[test]
    fn test_render_method_signature_php_with_error_type() {
        let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
        let sig = render_method_signature(&method, "Parser", Language::Php, TEST_PREFIX);
        assert_eq!(sig, "public function parse(): string");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — Java
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_java_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
        assert_eq!(sig, "public String getText(int page)");
    }

    #[test]
    fn test_render_method_signature_java_static() {
        let method = make_method(
            "create",
            vec![make_param("name", TypeRef::String, false)],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
        assert_eq!(sig, "public static Document create(String name)");
    }

    #[test]
    fn test_render_method_signature_java_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Java, TEST_PREFIX);
        assert_eq!(sig, "public Optional<String> find()");
    }

    #[test]
    fn test_render_method_signature_java_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Java, TEST_PREFIX);
        assert_eq!(sig, "public String fetch()");
    }

    #[test]
    fn test_render_method_signature_java_with_error_type() {
        let method = make_method(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            false,
            Some("ParseError"),
        );
        let sig = render_method_signature(&method, "Parser", Language::Java, TEST_PREFIX);
        assert_eq!(sig, "public Ast parse(String source) throws ParseError");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — C#
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_csharp_sync_with_params_and_return() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Csharp, TEST_PREFIX);
        assert_eq!(sig, "public string GetText(uint page)");
    }

    #[test]
    fn test_render_method_signature_csharp_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Csharp, TEST_PREFIX);
        assert_eq!(sig, "public async Task<string> FetchAsync()");
    }

    #[test]
    fn test_render_method_signature_csharp_async_already_suffixed() {
        let method = make_method("fetch_async", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Csharp, TEST_PREFIX);
        assert_eq!(sig, "public async Task<string> FetchAsync()");
    }

    #[test]
    fn test_render_method_signature_csharp_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Csharp, TEST_PREFIX);
        assert_eq!(sig, "public string? Find()");
    }

    #[test]
    fn test_render_method_signature_csharp_with_error_type() {
        let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
        let sig = render_method_signature(&method, "Parser", Language::Csharp, TEST_PREFIX);
        assert_eq!(sig, "public string Parse()");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — Elixir
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_elixir_sync_with_params() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Elixir, TEST_PREFIX);
        assert_eq!(sig, "def get_text(page)");
    }

    #[test]
    fn test_render_method_signature_elixir_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::Elixir, TEST_PREFIX);
        assert_eq!(sig, "def fetch()");
    }

    #[test]
    fn test_render_method_signature_elixir_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::Elixir, TEST_PREFIX);
        assert_eq!(sig, "def find()");
    }

    #[test]
    fn test_render_method_signature_elixir_with_error_type() {
        let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
        let sig = render_method_signature(&method, "Parser", Language::Elixir, TEST_PREFIX);
        assert_eq!(sig, "def parse()");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — R
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_r_sync_with_params() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::R, TEST_PREFIX);
        assert_eq!(sig, "get_text(page)");
    }

    #[test]
    fn test_render_method_signature_r_async() {
        let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
        let sig = render_method_signature(&method, "Client", Language::R, TEST_PREFIX);
        assert_eq!(sig, "fetch()");
    }

    #[test]
    fn test_render_method_signature_r_optional_return() {
        let method = make_method(
            "find",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Corpus", Language::R, TEST_PREFIX);
        assert_eq!(sig, "find()");
    }

    #[test]
    fn test_render_method_signature_r_with_error_type() {
        let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
        let sig = render_method_signature(&method, "Parser", Language::R, TEST_PREFIX);
        assert_eq!(sig, "parse()");
    }

    // ---------------------------------------------------------------------------
    // render_method_signature — WASM (shares Node rendering)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_method_signature_wasm_sync() {
        let method = make_method(
            "get_text",
            vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::String,
            false,
            false,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Wasm, TEST_PREFIX);
        assert_eq!(sig, "getText(page: number): string");
    }

    #[test]
    fn test_render_method_signature_wasm_static() {
        let method = make_method(
            "create",
            vec![],
            TypeRef::Named("Document".to_string()),
            false,
            true,
            None,
        );
        let sig = render_method_signature(&method, "Document", Language::Wasm, TEST_PREFIX);
        assert_eq!(sig, "static create(): Document");
    }

    // ---------------------------------------------------------------------------
    // render_python_fn_sig
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_python_fn_sig_basic() {
        let func = make_function(
            "convert",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::String,
            false,
            None,
        );
        let sig = render_python_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "def convert(source: str) -> str");
    }

    #[test]
    fn test_render_python_fn_sig_async() {
        let func = make_function("fetch", vec![], TypeRef::String, true, None);
        let sig = render_python_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "def fetch() -> str");
    }

    #[test]
    fn test_render_python_fn_sig_optional_param() {
        let func = make_function(
            "search",
            vec![
                make_param("query", TypeRef::String, false),
                make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
            ],
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            None,
        );
        let sig = render_python_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "def search(query: str, limit: int = None) -> list[str]");
    }

    #[test]
    fn test_render_python_fn_sig_complex_return_type() {
        let func = make_function(
            "get_mapping",
            vec![],
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32)),
            ),
            false,
            None,
        );
        let sig = render_python_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "def get_mapping() -> dict[str, int]");
    }

    // ---------------------------------------------------------------------------
    // render_rust_fn_sig
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_rust_fn_sig_basic() {
        let func = make_function(
            "convert",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::String,
            false,
            None,
        );
        let sig = render_rust_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "pub fn convert(source: &str) -> String");
    }

    #[test]
    fn test_render_rust_fn_sig_async() {
        let func = make_function("fetch", vec![], TypeRef::String, true, None);
        let sig = render_rust_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "pub async fn fetch() -> String");
    }

    #[test]
    fn test_render_rust_fn_sig_optional_param() {
        let func = make_function(
            "search",
            vec![
                make_param("query", TypeRef::String, false),
                make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
            ],
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            None,
        );
        let sig = render_rust_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "pub fn search(query: &str, limit: Option<u32>) -> Vec<String>");
    }

    #[test]
    fn test_render_rust_fn_sig_error_type_with_return() {
        let func = make_function(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            Some("ParseError"),
        );
        let sig = render_rust_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "pub fn parse(source: &str) -> Result<Ast, ParseError>");
    }

    #[test]
    fn test_render_rust_fn_sig_error_type_unit_return() {
        let func = make_function("save", vec![], TypeRef::Unit, false, Some("IoError"));
        let sig = render_rust_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "pub fn save() -> Result<(), IoError>");
    }

    // ---------------------------------------------------------------------------
    // render_go_fn_sig
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_go_fn_sig_basic() {
        let func = make_function(
            "convert",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::String,
            false,
            None,
        );
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Convert(source string) string");
    }

    #[test]
    fn test_render_go_fn_sig_async() {
        let func = make_function("fetch", vec![], TypeRef::String, true, None);
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Fetch() string");
    }

    #[test]
    fn test_render_go_fn_sig_optional_param() {
        let func = make_function(
            "search",
            vec![make_param("limit", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            None,
        );
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Search(limit uint32) []string");
    }

    #[test]
    fn test_render_go_fn_sig_error_type_with_return() {
        let func = make_function(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            Some("ParseError"),
        );
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Parse(source string) (Ast, error)");
    }

    #[test]
    fn test_render_go_fn_sig_error_type_unit_return() {
        let func = make_function("save", vec![], TypeRef::Unit, false, Some("IoError"));
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Save() error");
    }

    // ---------------------------------------------------------------------------
    // render_java_fn_sig
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_java_fn_sig_basic() {
        let func = make_function(
            "convert",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::String,
            false,
            None,
        );
        let sig = render_java_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static String convert(String source)");
    }

    #[test]
    fn test_render_java_fn_sig_async() {
        let func = make_function("fetch", vec![], TypeRef::String, true, None);
        let sig = render_java_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static String fetch()");
    }

    #[test]
    fn test_render_java_fn_sig_optional_param() {
        let func = make_function(
            "search",
            vec![make_param("limit", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            None,
        );
        let sig = render_java_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static List<String> search(int limit)");
    }

    #[test]
    fn test_render_java_fn_sig_error_type() {
        let func = make_function(
            "parse",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::Named("Ast".to_string()),
            false,
            Some("ParseError"),
        );
        let sig = render_java_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static Ast parse(String source) throws ParseError");
    }

    // ---------------------------------------------------------------------------
    // render_csharp_fn_sig
    // ---------------------------------------------------------------------------

    #[test]
    fn test_render_csharp_fn_sig_basic() {
        let func = make_function(
            "convert",
            vec![make_param("source", TypeRef::String, false)],
            TypeRef::String,
            false,
            None,
        );
        let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static string Convert(string source)");
    }

    #[test]
    fn test_render_csharp_fn_sig_async() {
        let func = make_function("fetch", vec![], TypeRef::String, true, None);
        let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static async Task<string> FetchAsync()");
    }

    #[test]
    fn test_render_csharp_fn_sig_optional_param() {
        let func = make_function(
            "search",
            vec![
                make_param("query", TypeRef::String, false),
                make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
            ],
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            None,
        );
        let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
        assert_eq!(
            sig,
            "public static List<string> Search(string query, uint? limit = null)"
        );
    }

    #[test]
    fn test_render_csharp_fn_sig_complex_return_type() {
        let func = make_function(
            "get_mapping",
            vec![],
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32)),
            ),
            false,
            None,
        );
        let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "public static Dictionary<string, int> GetMapping()");
    }

    // ---------------------------------------------------------------------------
    // render_param_list via render_function_signature — parameter formatting
    // ---------------------------------------------------------------------------

    #[test]
    fn test_param_list_python_optional_uses_none_default() {
        let func = make_function(
            "run",
            vec![
                make_param("input", TypeRef::String, false),
                make_param("config", TypeRef::Named("Config".to_string()), true),
            ],
            TypeRef::Unit,
            false,
            None,
        );
        let sig = render_python_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "def run(input: str, config: Config = None) -> None");
    }

    #[test]
    fn test_param_list_node_optional_uses_question_mark() {
        let func = make_function(
            "run",
            vec![
                make_param("input", TypeRef::String, false),
                make_param("config", TypeRef::Named("Config".to_string()), true),
            ],
            TypeRef::Unit,
            false,
            None,
        );
        let sig = render_typescript_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "function run(input: string, config?: Config): void");
    }

    #[test]
    fn test_param_list_go_no_optional_syntax() {
        let func = make_function(
            "run",
            vec![make_param("input", TypeRef::String, false)],
            TypeRef::Unit,
            false,
            None,
        );
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Run(input string)");
    }

    #[test]
    fn test_param_list_rust_string_params_use_refs() {
        let func = make_function(
            "process",
            vec![
                make_param("name", TypeRef::String, false),
                make_param("initial", TypeRef::Char, false),
                make_param("data", TypeRef::Bytes, false),
            ],
            TypeRef::Unit,
            false,
            None,
        );
        let sig = render_rust_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "pub fn process(name: &str, initial: &str, data: &[u8])");
    }

    #[test]
    fn test_param_list_php_uses_dollar_prefix() {
        let func = make_function(
            "search",
            vec![
                make_param("query", TypeRef::String, false),
                make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
            ],
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            None,
        );
        let sig = render_php_fn_sig(&func, TEST_PREFIX);
        assert_eq!(
            sig,
            "public static function search(string $query, ?int $limit = null): array<string>"
        );
    }
}
