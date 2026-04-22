//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::{
    ApiSurface, DefaultValue, EnumDef, ErrorDef, FieldDef, FunctionDef, MethodDef, PrimitiveType, TypeDef, TypeRef,
};
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate API reference documentation for the given languages.
///
/// Produces one `api-{lang}.md` per language, plus shared `configuration.md`,
/// `types.md`, and `errors.md` files written into `output_dir`.
pub fn generate_docs(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
    output_dir: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();
    let ffi_prefix = &config.ffi_prefix().to_pascal_case();

    for &lang in languages {
        files.push(generate_lang_doc(api, config, lang, output_dir, ffi_prefix)?);
    }

    files.push(generate_configuration_doc(api, config, output_dir)?);
    files.push(generate_types_doc(api, output_dir)?);
    files.push(generate_errors_doc(api, output_dir)?);

    // Post-process: ensure trailing newline and wrap bare URLs (MD034)
    for file in &mut files {
        // Wrap bare http(s) URLs in angle brackets to satisfy MD034
        file.content = wrap_bare_urls(&file.content);
        // Ensure POSIX trailing newline
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

// ---------------------------------------------------------------------------
// Per-language doc page
// ---------------------------------------------------------------------------

fn generate_lang_doc(
    api: &ApiSurface,
    config: &AlefConfig,
    lang: Language,
    output_dir: &str,
    ffi_prefix: &str,
) -> anyhow::Result<GeneratedFile> {
    let lang_display = lang_display_name(lang);
    let version = &api.version;
    let lang_slug = lang_slug(lang);

    let mut out = String::with_capacity(8192);

    // Front matter
    let _ = writeln!(out, "---\ntitle: \"{lang_display} API Reference\"\n---\n");

    // Title
    let _ = writeln!(
        out,
        "## {lang_display} API Reference <span class=\"version-badge\">v{version}</span>\n"
    );

    // --- Functions section ---
    let public_fns: Vec<&FunctionDef> = api.functions.iter().collect();
    if !public_fns.is_empty() {
        out.push_str("### Functions\n\n");
        for func in &public_fns {
            out.push_str(&render_function(func, lang, config, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Types section ---
    // Order: ConversionOptions, ConversionResult, then rest alphabetical
    // Skip opaque types and *Update types in main section
    let mut types_to_doc: Vec<&TypeDef> = api.types.iter().filter(|t| !is_update_type(&t.name)).collect();

    // Sort: ConversionOptions first, ConversionResult second, rest alphabetical
    types_to_doc.sort_by(|a, b| type_sort_key(&a.name).cmp(&type_sort_key(&b.name)));

    if !types_to_doc.is_empty() {
        out.push_str("### Types\n\n");
        for ty in &types_to_doc {
            out.push_str(&render_type(ty, lang, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Enums section ---
    if !api.enums.is_empty() {
        out.push_str("### Enums\n\n");
        for en in &api.enums {
            out.push_str(&render_enum(en, lang, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Errors section ---
    if !api.errors.is_empty() {
        out.push_str("### Errors\n\n");
        for err in &api.errors {
            out.push_str(&render_error(err, lang, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    let path = PathBuf::from(format!("{output_dir}/api-{lang_slug}.md"));

    Ok(GeneratedFile {
        path,
        content: out,
        generated_header: false,
    })
}

// ---------------------------------------------------------------------------
// Function rendering
// ---------------------------------------------------------------------------

fn render_function(
    func: &FunctionDef,
    lang: Language,
    _config: &AlefConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let fn_name = func_name(&func.name, lang, ffi_prefix);

    let _ = writeln!(out, "#### {fn_name}()\n");

    // Extract parameter descriptions from the RAW doc string BEFORE cleaning
    let param_docs = extract_param_docs(&func.doc);

    if !func.doc.is_empty() {
        out.push_str(&clean_doc(&func.doc, lang));
        out.push('\n');
        out.push('\n');
    }

    // Signature
    out.push_str("**Signature:**\n\n");
    let lang_code = lang_code_fence(lang);
    let sig = render_function_signature(func, lang, ffi_prefix);
    let _ = writeln!(out, "```{lang_code}\n{sig}\n```\n");

    // Parameters table
    if !func.params.is_empty() {
        out.push_str("**Parameters:**\n\n");
        out.push_str("| Name | Type | Required | Description |\n");
        out.push_str("|------|------|----------|-------------|\n");
        for param in &func.params {
            let pname = field_name(&param.name, lang);
            let pty = doc_type_with_optional(&param.ty, lang, param.optional, ffi_prefix);
            let required = if param.optional { "No" } else { "Yes" };
            let pdoc = param_docs
                .get(param.name.as_str())
                .map(|s| {
                    let s = s.replace('|', "\\|");
                    // Clean Rust syntax from param descriptions
                    let s = s.replace("::", ".");
                    s.replace("ConversionOptions.default()", "default options")
                })
                .unwrap_or_else(|| generate_param_description(&param.name, &param.ty));
            let _ = writeln!(out, "| `{pname}` | `{pty}` | {required} | {pdoc} |");
        }
        out.push('\n');
    }

    // Return type
    let ret_ty = doc_type(&func.return_type, lang, ffi_prefix);
    let _ = write!(out, "**Returns:** `{ret_ty}`");
    out.push('\n');
    out.push('\n');

    // Errors
    if let Some(err) = &func.error_type {
        let error_phrase = format_error_phrase(err, lang);
        let _ = writeln!(out, "**Errors:** {error_phrase}\n");
    }

    let _ = api; // api is available for future use in function rendering
    out
}

fn render_function_signature(func: &FunctionDef, lang: Language, ffi_prefix: &str) -> String {
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
    }
}

fn render_python_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

fn render_typescript_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

fn render_go_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

fn render_java_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

fn render_ruby_fn_sig(func: &FunctionDef) -> String {
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

fn render_c_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

fn render_php_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

fn render_elixir_fn_sig(func: &FunctionDef) -> String {
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

fn render_r_fn_sig(func: &FunctionDef) -> String {
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

fn render_csharp_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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
        format!(
            "public static async Task<{}> {}({})",
            ret,
            async_name,
            params.join(", ")
        )
    } else {
        format!("public static {} {}({})", ret, name, params.join(", "))
    }
}

fn render_rust_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
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

// ---------------------------------------------------------------------------
// Type rendering
// ---------------------------------------------------------------------------

fn render_type(ty: &TypeDef, lang: Language, api: &ApiSurface, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let tname = type_name(&ty.name, lang, ffi_prefix);

    let _ = writeln!(out, "#### {tname}\n");

    let doc = clean_doc(&ty.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    // Fields table (only for non-opaque types or opaque types with documented fields)
    if !ty.is_opaque && !ty.fields.is_empty() {
        out.push_str("| Field | Type | Default | Description |\n");
        out.push_str("|-------|------|---------|-------------|\n");
        for field in &ty.fields {
            let fname = field_name(&field.name, lang);
            let fty = doc_type_with_optional(&field.ty, lang, field.optional, ffi_prefix);
            let fdefault = format_field_default(field, lang, api, ffi_prefix);
            let fdoc = {
                let raw = clean_doc_inline(&field.doc, lang);
                if raw.is_empty() {
                    generate_field_description(&field.name, &field.ty)
                } else {
                    raw
                }
            };
            let _ = writeln!(out, "| `{fname}` | `{fty}` | {fdefault} | {fdoc} |");
        }
        out.push('\n');
    }

    // Methods (called "Functions" in Elixir)
    if !ty.methods.is_empty() {
        let methods_heading = if lang == Language::Elixir {
            "Functions"
        } else {
            "Methods"
        };
        let _ = writeln!(out, "##### {methods_heading}\n");
        for method in &ty.methods {
            out.push_str(&render_method(method, &ty.name, lang, ffi_prefix));
        }
    }

    out
}

fn render_method(method: &MethodDef, type_name_str: &str, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let mname = func_name(&method.name, lang, ffi_prefix);

    let _ = writeln!(out, "###### {mname}()\n");

    let doc = clean_doc(&method.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let lang_code = lang_code_fence(lang);
    let sig = render_method_signature(method, type_name_str, lang, ffi_prefix);
    out.push_str("**Signature:**\n\n");
    let _ = writeln!(out, "```{lang_code}\n{sig}\n```\n");

    out
}

fn render_method_signature(method: &MethodDef, type_name_str: &str, lang: Language, ffi_prefix: &str) -> String {
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
                format!("public async Task<{}> {}({})", ret, async_name, params.join(", "))
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
    }
}

// ---------------------------------------------------------------------------
// Enum rendering
// ---------------------------------------------------------------------------

fn render_enum(en: &EnumDef, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&en.name, lang, ffi_prefix);

    let _ = writeln!(out, "#### {ename}\n");

    let doc = clean_doc(&en.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    out.push_str("| Value | Description |\n");
    out.push_str("|-------|-------------|\n");
    for variant in &en.variants {
        let vname = enum_variant_name(&variant.name, lang, ffi_prefix);
        let mut vdoc = if !variant.doc.is_empty() {
            clean_doc_inline(&variant.doc, lang)
        } else {
            generate_enum_variant_description(&variant.name)
        };
        // Append field info for data variants
        if !variant.fields.is_empty() {
            let fields_desc: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let fname = field_name(&f.name, lang);
                    let fty = doc_type(&f.ty, lang, ffi_prefix);
                    format!("`{fname}`: `{fty}`")
                })
                .collect();
            vdoc = format!("{vdoc} — Fields: {}", fields_desc.join(", "));
        }
        let _ = writeln!(out, "| `{vname}` | {vdoc} |");
    }
    out.push('\n');

    out
}

// ---------------------------------------------------------------------------
// Error rendering
// ---------------------------------------------------------------------------

fn render_error(err: &ErrorDef, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&err.name, lang, ffi_prefix);

    let _ = writeln!(out, "#### {ename}\n");

    let doc = clean_doc(&err.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    // For Node/WASM, note that errors are plain Error objects
    if matches!(lang, Language::Node | Language::Wasm) {
        out.push_str("Errors are thrown as plain `Error` objects with descriptive messages.\n\n");
    }

    // For Python, render as exception class hierarchy
    if lang == Language::Python {
        let _ = writeln!(out, "**Base class:** `{ename}(Exception)`\n");
        out.push_str("| Exception | Description |\n");
        out.push_str("|-----------|-------------|\n");
        for variant in &err.variants {
            let vname = variant.name.to_pascal_case();
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, lang)
            } else if let Some(tmpl) = &variant.message_template {
                clean_doc_inline(tmpl, lang)
            } else {
                generate_error_variant_description(&variant.name)
            };
            let _ = writeln!(out, "| `{vname}({ename})` | {vdoc} |");
        }
    } else {
        out.push_str("| Variant | Description |\n");
        out.push_str("|---------|-------------|\n");
        for variant in &err.variants {
            let vname = enum_variant_name(&variant.name, lang, ffi_prefix);
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, lang)
            } else if let Some(tmpl) = &variant.message_template {
                clean_doc_inline(tmpl, lang)
            } else {
                generate_error_variant_description(&variant.name)
            };
            let _ = writeln!(out, "| `{vname}` | {vdoc} |");
        }
    }
    out.push('\n');

    out
}

// ---------------------------------------------------------------------------
// Configuration page
// ---------------------------------------------------------------------------

fn generate_configuration_doc(
    api: &ApiSurface,
    _config: &AlefConfig,
    output_dir: &str,
) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Configuration Reference\"\n---\n\n");
    out.push_str("## Configuration Reference\n\n");
    out.push_str("This page documents all configuration types and their defaults across all languages.\n\n");

    // Collect config-like types (Config, Options, Settings suffixes, or types with Default)
    let config_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| {
            (t.name.ends_with("Config") || t.name.ends_with("Options") || t.name.ends_with("Settings") || t.has_default)
                && !t.is_opaque
                && !is_update_type(&t.name)
        })
        .collect();

    for ty in config_types {
        let _ = writeln!(out, "### {}\n", ty.name);
        let doc = clean_doc(&ty.doc, Language::Python);
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

        if !ty.fields.is_empty() {
            out.push_str("| Field | Type | Default | Description |\n");
            out.push_str("|-------|------|---------|-------------|\n");
            for field in &ty.fields {
                let fty = doc_type_with_optional(&field.ty, Language::Python, field.optional, "");
                let fdefault = format_field_default(field, Language::Python, api, "");
                let fdoc = {
                    let raw = clean_doc_inline(&field.doc, Language::Python);
                    if raw.is_empty() {
                        generate_field_description(&field.name, &field.ty)
                    } else {
                        raw
                    }
                };
                let _ = writeln!(out, "| `{}` | `{}` | {} | {} |", field.name, fty, fdefault, fdoc);
            }
            out.push('\n');
        }

        out.push_str("---\n\n");
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/configuration.md")),
        content: out,
        generated_header: false,
    })
}

// ---------------------------------------------------------------------------
// Types reference page
// ---------------------------------------------------------------------------

/// Categorize a type by name/path patterns into a documentation group.
fn categorize_type(ty: &TypeDef) -> &'static str {
    let name = &ty.name;
    if name.ends_with("Result") || name.contains("Result") {
        "Result Types"
    } else if name.contains("Metadata") || name.ends_with("Meta") {
        "Metadata Types"
    } else if name.ends_with("Config") || name.ends_with("Options") || name.ends_with("Settings") || ty.has_default {
        "Configuration Types"
    } else if name.contains("Node") || name.contains("Table") || name.contains("Grid") || name.contains("Document") {
        "Document Structure"
    } else if name.contains("Ocr") || name.contains("Tesseract") || name.contains("Paddle") {
        "OCR Types"
    } else {
        "Other Types"
    }
}

fn generate_types_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Types Reference\"\n---\n\n");
    out.push_str("## Types Reference\n\n");
    out.push_str("All types defined by the library, grouped by category. Types are shown using Rust as the canonical representation.\n\n");

    // Collect non-update types
    let types_to_doc: Vec<&TypeDef> = api.types.iter().filter(|t| !is_update_type(&t.name)).collect();

    if types_to_doc.is_empty() {
        out.push_str("No types defined.\n");
        return Ok(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}/types.md")),
            content: out,
            generated_header: false,
        });
    }

    // Define category order
    let category_order = [
        "Result Types",
        "Configuration Types",
        "Metadata Types",
        "Document Structure",
        "OCR Types",
        "Other Types",
    ];

    // Group types by category
    let mut groups: std::collections::HashMap<&str, Vec<&TypeDef>> = std::collections::HashMap::new();
    for ty in &types_to_doc {
        let cat = categorize_type(ty);
        groups.entry(cat).or_default().push(ty);
    }

    // Render each category in order
    for &cat in &category_order {
        let Some(types) = groups.get(cat) else {
            continue;
        };
        let _ = writeln!(out, "### {cat}\n");

        if cat == "Configuration Types" {
            out.push_str("See [Configuration Reference](configuration.md) for detailed defaults and language-specific representations.\n\n");
        }

        for ty in types {
            let _ = writeln!(out, "#### {}\n", ty.name);

            let doc = clean_doc(&ty.doc, Language::Python);
            if !doc.is_empty() {
                out.push_str(&doc);
                out.push('\n');
                out.push('\n');
            }

            if ty.is_opaque {
                out.push_str("*Opaque type — fields are not directly accessible.*\n\n");
            } else if !ty.fields.is_empty() {
                out.push_str("| Field | Type | Default | Description |\n");
                out.push_str("|-------|------|---------|-------------|\n");
                for field in &ty.fields {
                    // Use Rust-style type representation as canonical
                    let fty = format_type_ref_rust(&field.ty, field.optional);
                    // Use the typed default (consistent with per-language pages)
                    // falling back to the raw string default.
                    let fdefault = format_field_default(field, Language::Rust, api, "");
                    let fdoc = {
                        let raw = clean_doc_inline(&field.doc, Language::Rust);
                        if raw.is_empty() {
                            generate_field_description(&field.name, &field.ty)
                        } else {
                            raw
                        }
                    };
                    let _ = writeln!(out, "| `{}` | `{}` | {} | {} |", field.name, fty, fdefault, fdoc);
                }
                out.push('\n');
            }

            out.push_str("---\n\n");
        }
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/types.md")),
        content: out,
        generated_header: false,
    })
}

/// Format a TypeRef as a Rust-like canonical type string (language-neutral).
fn format_type_ref_rust(ty: &TypeRef, optional: bool) -> String {
    let base = match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "Duration".to_string(),
        TypeRef::Primitive(p) => match p {
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
        TypeRef::Optional(inner) => {
            return format!("Option<{}>", format_type_ref_rust(inner, false));
        }
        TypeRef::Vec(inner) => {
            return format!("Vec<{}>", format_type_ref_rust(inner, false));
        }
        TypeRef::Map(k, v) => {
            return format!(
                "HashMap<{}, {}>",
                format_type_ref_rust(k, false),
                format_type_ref_rust(v, false)
            );
        }
        TypeRef::Named(name) => name.rsplit("::").next().unwrap_or(name).to_string(),
    };
    if optional && !matches!(ty, TypeRef::Optional(_)) {
        format!("Option<{base}>")
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// Errors page
// ---------------------------------------------------------------------------

fn generate_errors_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Error Reference\"\n---\n\n");
    out.push_str("## Error Reference\n\n");
    out.push_str("All error types thrown by the library across all languages.\n\n");

    for err in &api.errors {
        let _ = writeln!(out, "### {}\n", err.name);

        let doc = clean_doc(&err.doc, Language::Python);
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

        out.push_str("| Variant | Message | Description |\n");
        out.push_str("|---------|---------|-------------|\n");
        for variant in &err.variants {
            let tmpl = variant.message_template.as_deref().unwrap_or("").replace('|', "\\|");
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, Language::Python)
            } else {
                generate_error_variant_description(&variant.name)
            };
            let _ = writeln!(out, "| `{}` | {} | {} |", variant.name, tmpl, vdoc);
        }
        out.push('\n');
        out.push_str("---\n\n");
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/errors.md")),
        content: out,
        generated_header: false,
    })
}

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

/// Map an IR TypeRef to the idiomatic type string for a given language.
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
            Language::Ffi => "const char*".to_string(),
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
            Language::Ffi => "const uint8_t*".to_string(),
        },
        TypeRef::Primitive(p) => doc_primitive(p, lang),
        TypeRef::Optional(inner) => {
            let inner_ty = doc_type(inner, lang, ffi_prefix);
            match lang {
                Language::Python => format!("{inner_ty} | None"),
                Language::Node | Language::Wasm => format!("{inner_ty} | null"),
                Language::Go => format!("*{inner_ty}"),
                Language::Java => {
                    let boxed = java_boxed_type(inner);
                    format!("Optional<{boxed}>")
                }
                Language::Csharp => format!("{inner_ty}?"),
                Language::Ruby => format!("{inner_ty}?"),
                Language::Php => format!("?{inner_ty}"),
                Language::Elixir => format!("{inner_ty} | nil"),
                Language::R => format!("{inner_ty} or NULL"),
                Language::Rust => format!("Option<{inner_ty}>"),
                Language::Ffi => format!("{inner_ty}*"),
            }
        }
        TypeRef::Vec(inner) => {
            match lang {
                Language::Java => {
                    // Java generics can't use primitives — box them
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
                        Language::Ffi => format!("{inner_ty}*"),
                        Language::Java | Language::Csharp => unreachable!(),
                    }
                }
            }
        }
        TypeRef::Map(k, v) => {
            if lang == Language::Java {
                // Java generics require boxed types
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
                Language::Ffi => "void*".to_string(),
            }
        }
        TypeRef::Named(name) if name.starts_with('(') && name.ends_with(')') => {
            // Tuple type encoded as Named("(A, B)") — render idiomatically per language
            let inner = &name[1..name.len() - 1];
            let rendered: Vec<String> = inner
                .split(',')
                .map(|part| {
                    let trimmed = part.trim();
                    match trimmed {
                        "usize" | "u64" | "u32" | "u16" | "u8" | "i64" | "i32" | "i16" | "i8" | "isize" => match lang {
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
                            Language::Ffi => "uint64_t".to_string(),
                        },
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
                            Language::Ffi => "const char*".to_string(),
                        },
                        // Slice of strings — &[&str], &'static [&'static str], Vec<String>, etc.
                        // Also covers compacted IR forms like &'static[&'staticstr]
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
                                Language::Ffi => "const char**".to_string(),
                            }
                        }
                        other => {
                            // For Rust, preserve the raw type token rather than
                            // PascalCasing it — Rust type names are already correct.
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
                Language::Ffi => "void*".to_string(),
            }
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
            Language::Ffi => "const char*".to_string(),
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
            Language::Ffi => "void".to_string(),
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
            Language::Ffi => "void*".to_string(),
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
            Language::Ffi => "uint64_t".to_string(),
        },
    }
}

fn doc_primitive(p: &PrimitiveType, lang: Language) -> String {
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
            PrimitiveType::Usize | PrimitiveType::Isize => "int".to_string(),
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
            PrimitiveType::Usize => "nuint".to_string(),
            PrimitiveType::Isize => "nint".to_string(),
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
        Language::Ffi => match p {
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
    }
}

/// Return the boxed (object) type for Java generics.
///
/// Java generics cannot use primitive types (`int`, `long`, etc.); they require
/// the corresponding wrapper classes (`Integer`, `Long`, etc.).
fn java_boxed_type(ty: &TypeRef) -> String {
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
        // Non-primitive types are already reference types in Java
        _ => doc_type(ty, Language::Java, ""),
    }
}

// ---------------------------------------------------------------------------
// Naming conventions
// ---------------------------------------------------------------------------

/// Get the display name for a language.
fn lang_display_name(lang: Language) -> &'static str {
    match lang {
        Language::Python => "Python",
        Language::Node => "TypeScript",
        Language::Ruby => "Ruby",
        Language::Php => "PHP",
        Language::Elixir => "Elixir",
        Language::Go => "Go",
        Language::Java => "Java",
        Language::Csharp => "C#",
        Language::Ffi => "C",
        Language::Wasm => "WebAssembly",
        Language::R => "R",
        Language::Rust => "Rust",
    }
}

/// Get the slug used in file names (e.g. `typescript` for `Node`).
fn lang_slug(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "c",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
    }
}

/// Get the code fence language identifier.
fn lang_code_fence(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node | Language::Wasm => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "c",
        Language::R => "r",
        Language::Rust => "rust",
    }
}

/// Convert a Rust type name to the idiomatic name for the target language.
fn type_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    // Strip module path prefix if present
    let short = name.rsplit("::").next().unwrap_or(name);
    match lang {
        Language::Python
        | Language::Node
        | Language::Wasm
        | Language::Ruby
        | Language::Go
        | Language::Java
        | Language::Csharp
        | Language::Php
        | Language::Elixir
        | Language::R
        | Language::Rust => short.to_pascal_case(),
        Language::Ffi => {
            // C: prefix with configured FFI prefix (PascalCase) and PascalCase type name
            format!("{}{}", ffi_prefix, short.to_pascal_case())
        }
    }
}

/// Convert a Rust function name to the idiomatic name for the target language.
fn func_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let base = match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::R | Language::Rust => name.to_snake_case(),
        Language::Node | Language::Wasm | Language::Java | Language::Php => to_camel_case(name),
        Language::Csharp | Language::Go => name.to_pascal_case(),
        Language::Ffi => format!("{}_{}", ffi_prefix.to_snake_case(), name.to_snake_case()),
    };
    // Handle reserved keywords
    match (lang, base.as_str()) {
        (Language::Java, "default") => "defaultOptions".to_string(),
        (Language::Csharp, "Default") => "CreateDefault".to_string(),
        _ => base,
    }
}

/// Convert a Rust field name to the idiomatic name for the target language.
fn field_name(name: &str, lang: Language) -> String {
    match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::R | Language::Ffi | Language::Rust => {
            name.to_snake_case()
        }
        // Go and C# exported fields/properties are PascalCase
        Language::Go | Language::Csharp => name.to_pascal_case(),
        Language::Node | Language::Wasm | Language::Java | Language::Php => to_camel_case(name),
    }
}

/// Convert a Rust enum variant name to the idiomatic name for the target language.
fn enum_variant_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    // Special-case acronym variants that don't split cleanly
    if name == "RDFa" {
        return match lang {
            Language::Python | Language::Java => "RDFA".to_string(),
            Language::Ruby | Language::Elixir => "rdfa".to_string(),
            Language::R => "rdfa".to_string(),
            Language::Ffi => format!("{}_{}", ffi_prefix.to_shouty_snake_case(), "RDFA"),
            _ => "RDFa".to_string(),
        };
    }
    match lang {
        Language::Python => {
            // Python: UPPER_SNAKE_CASE
            name.to_shouty_snake_case()
        }
        Language::Java => {
            // Java: UPPER_SNAKE_CASE
            name.to_shouty_snake_case()
        }
        Language::Ruby | Language::Elixir => {
            // Ruby/Elixir: :snake_atom style
            name.to_snake_case()
        }
        Language::Go | Language::Node | Language::Wasm | Language::Csharp | Language::Php => name.to_pascal_case(),
        Language::R => name.to_snake_case(),
        // Rust: PascalCase enum variants
        Language::Rust => name.to_pascal_case(),
        Language::Ffi => format!("{}_{}", ffi_prefix.to_shouty_snake_case(), name.to_shouty_snake_case()),
    }
}

/// Convert snake_case or PascalCase to camelCase.
fn to_camel_case(s: &str) -> String {
    let pascal = s.to_upper_camel_case();
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Default value formatting
// ---------------------------------------------------------------------------

fn format_field_default(field: &FieldDef, lang: Language, api: &ApiSurface, ffi_prefix: &str) -> String {
    if let Some(typed) = &field.typed_default {
        return format_typed_default(typed, &field.ty, lang, api, ffi_prefix, field.optional);
    }
    if let Some(raw) = &field.default {
        if !raw.is_empty() {
            return format!("`{raw}`");
        }
    }
    if field.optional {
        return match lang {
            Language::Python => "`None`".to_string(),
            Language::Node | Language::Wasm => "`null`".to_string(),
            Language::Go => "`nil`".to_string(),
            Language::Java => "`null`".to_string(),
            Language::Csharp => "`null`".to_string(),
            Language::Ruby => "`nil`".to_string(),
            Language::Php => "`null`".to_string(),
            Language::Elixir => "`nil`".to_string(),
            Language::R => "`NULL`".to_string(),
            Language::Rust => "`None`".to_string(),
            Language::Ffi => "`NULL`".to_string(),
        };
    }
    "—".to_string()
}

fn format_typed_default(
    val: &DefaultValue,
    field_ty: &TypeRef,
    lang: Language,
    api: &ApiSurface,
    ffi_prefix: &str,
    optional: bool,
) -> String {
    match val {
        DefaultValue::BoolLiteral(b) => match lang {
            Language::Python => format!("`{}`", if *b { "True" } else { "False" }),
            _ => format!("`{b}`"),
        },
        DefaultValue::StringLiteral(s) => format!("`\"{s}\"`"),
        DefaultValue::IntLiteral(n) => {
            // Duration fields store defaults as milliseconds; show with unit label
            if matches!(field_ty, TypeRef::Duration)
                || matches!(field_ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Duration))
            {
                return format!("`{n}ms`");
            }
            format!("`{n}`")
        }
        DefaultValue::FloatLiteral(f) => format!("`{f}`"),
        DefaultValue::EnumVariant(v) => {
            // v is something like "HeadingStyle::Atx" or just "Atx"
            let parts: Vec<&str> = v.splitn(2, "::").collect();
            if parts.len() == 2 {
                let enum_type = type_name(parts[0], lang, ffi_prefix);
                let variant = enum_variant_name(parts[1], lang, ffi_prefix);
                format!("`{}`", format_enum_variant_ref(&enum_type, &variant, lang, ffi_prefix))
            } else {
                // Bare variant name — resolve the enum type from the field type
                let enum_type_name_str = match field_ty {
                    TypeRef::Named(n) => Some(n.as_str()),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(type_str) = enum_type_name_str {
                    let etype = type_name(type_str, lang, ffi_prefix);
                    let variant = enum_variant_name(v, lang, ffi_prefix);
                    format!("`{}`", format_enum_variant_ref(&etype, &variant, lang, ffi_prefix))
                } else {
                    format!("`{v}`")
                }
            }
        }
        DefaultValue::Empty => {
            // Duration fields with Empty default: the actual value could not be parsed.
            // Show a language-neutral placeholder rather than None/null.
            let inner_for_dur = match field_ty {
                TypeRef::Optional(inner) => Some(inner.as_ref()),
                other => Some(other),
            };
            if matches!(inner_for_dur, Some(TypeRef::Duration)) {
                return match lang {
                    Language::Rust => "`Duration::default()`".to_string(),
                    _ => "`0ms`".to_string(),
                };
            }

            // If the field type is a Named enum, resolve to its default (or first) variant.
            // But only for non-optional fields — optional enum fields default to None/null.
            if !optional {
                if let TypeRef::Named(type_name_str) = field_ty {
                    if let Some(enum_def) = api.enums.iter().find(|e| &e.name == type_name_str) {
                        let variant = enum_def
                            .variants
                            .iter()
                            .find(|v| v.is_default)
                            .or_else(|| enum_def.variants.first());
                        if let Some(v) = variant {
                            let etype = type_name(type_name_str, lang, ffi_prefix);
                            let vname = enum_variant_name(&v.name, lang, ffi_prefix);
                            return format!("`{}`", format_enum_variant_ref(&etype, &vname, lang, ffi_prefix));
                        }
                    }
                }
            }
            // Non-enum Empty: depends on field type
            // Unwrap Optional wrapper to get inner type for collection/map detection
            let inner_ty = match field_ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            if matches!(inner_ty, TypeRef::Vec(_)) {
                return match lang {
                    Language::Python => "`[]`".to_string(),
                    Language::Node | Language::Wasm => "`[]`".to_string(),
                    Language::Go => "`nil`".to_string(),
                    Language::Java => "`Collections.emptyList()`".to_string(),
                    Language::Csharp => {
                        let elem_ty = if let TypeRef::Vec(elem) = inner_ty {
                            doc_type(elem, lang, ffi_prefix)
                        } else {
                            String::new()
                        };
                        format!("`new List<{elem_ty}>()`")
                    }
                    Language::Ruby | Language::Elixir => "`[]`".to_string(),
                    Language::Php => "`[]`".to_string(),
                    Language::Rust => "`vec![]`".to_string(),
                    Language::Ffi => "`NULL`".to_string(),
                    Language::R => "`list()`".to_string(),
                };
            }
            if matches!(inner_ty, TypeRef::Map(_, _)) {
                return match lang {
                    Language::Python | Language::Ruby | Language::Php => "`{}`".to_string(),
                    Language::Node | Language::Wasm => "`{}`".to_string(),
                    Language::Go => "`nil`".to_string(),
                    Language::Elixir => "`%{}`".to_string(),
                    Language::Java => "`Collections.emptyMap()`".to_string(),
                    Language::Csharp => {
                        if let TypeRef::Map(k, v) = inner_ty {
                            let kty = doc_type(k, lang, ffi_prefix);
                            let vty = doc_type(v, lang, ffi_prefix);
                            format!("`new Dictionary<{kty}, {vty}>()`")
                        } else {
                            "`new Dictionary<>()`".to_string()
                        }
                    }
                    Language::Rust => "`HashMap::new()`".to_string(),
                    Language::Ffi => "`NULL`".to_string(),
                    Language::R => "`list()`".to_string(),
                };
            }
            // Non-collection Empty: only show null for optional fields
            if !optional {
                return "—".to_string();
            }
            match lang {
                Language::Python => "`None`".to_string(),
                Language::Node | Language::Wasm => "`null`".to_string(),
                Language::Go => "`nil`".to_string(),
                Language::Java => "`null`".to_string(),
                Language::Csharp => "`null`".to_string(),
                Language::Ruby => "`nil`".to_string(),
                Language::Php => "`null`".to_string(),
                Language::Elixir => "`nil`".to_string(),
                Language::R => "`NULL`".to_string(),
                Language::Rust => "`Default::default()`".to_string(),
                Language::Ffi => "`NULL`".to_string(),
            }
        }
        DefaultValue::None => {
            if !optional {
                return "—".to_string();
            }
            match lang {
                Language::Python => "`None`".to_string(),
                Language::Node | Language::Wasm => "`null`".to_string(),
                Language::Go => "`nil`".to_string(),
                Language::Java => "`null`".to_string(),
                Language::Csharp => "`null`".to_string(),
                Language::Ruby => "`nil`".to_string(),
                Language::Php => "`null`".to_string(),
                Language::Elixir => "`nil`".to_string(),
                Language::R => "`NULL`".to_string(),
                Language::Rust => "`None`".to_string(),
                Language::Ffi => "`NULL`".to_string(),
            }
        }
    }
}

/// Format an enum variant reference: `TypeName.VARIANT` or `:atom` style per language.
fn format_enum_variant_ref(enum_type: &str, variant: &str, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python => format!("{enum_type}.{variant}"),
        Language::Node | Language::Wasm => format!("{enum_type}.{variant}"),
        Language::Go => format!("{enum_type}.{variant}"),
        Language::Java => format!("{enum_type}.{variant}"),
        Language::Csharp => format!("{enum_type}.{variant}"),
        Language::Ruby => format!(":{variant}"),
        Language::Php => format!("{enum_type}::{variant}"),
        Language::Elixir => format!(":{variant}"),
        Language::R => format!("\"{variant}\""),
        Language::Rust => format!("{enum_type}::{variant}"),
        Language::Ffi => format!(
            "{}_{}",
            ffi_prefix.to_shouty_snake_case(),
            variant.to_shouty_snake_case()
        ),
    }
}

/// Format the error/exception phrase for a function that can fail.
fn format_error_phrase(error_type: &str, lang: Language) -> String {
    let short = error_type.rsplit("::").next().unwrap_or(error_type);
    match lang {
        Language::Python => {
            let ename = short.to_pascal_case();
            format!("Raises `{ename}`.")
        }
        Language::Go => "Returns `error`.".to_string(),
        Language::Java => {
            let ename = short.to_pascal_case();
            let ename = if ename.ends_with("Exception") {
                ename
            } else {
                format!("{ename}Exception")
            };
            format!("Throws `{ename}`.")
        }
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
        Language::Ffi => "Returns `NULL` on error.".to_string(),
        Language::R => "Stops with error message.".to_string(),
        Language::Rust => {
            let ename = short.to_pascal_case();
            format!("Returns `Err({ename})`.")
        }
    }
}

/// Like `doc_type` but wraps in the nullable form when `optional` is true.
fn doc_type_with_optional(ty: &TypeRef, lang: Language, optional: bool, ffi_prefix: &str) -> String {
    // If the type is already Optional<T>, don't double-wrap
    if optional && !matches!(ty, TypeRef::Optional(_)) {
        let inner = doc_type(ty, lang, ffi_prefix);
        return match lang {
            Language::Python => format!("{inner} | None"),
            Language::Node | Language::Wasm => format!("{inner} | null"),
            Language::Go => format!("*{inner}"),
            Language::Java => format!("Optional<{}>", java_boxed_type(ty)),
            Language::Csharp => format!("{inner}?"),
            Language::Ruby => format!("{inner}?"),
            Language::Php => format!("?{inner}"),
            Language::Elixir => format!("{inner} | nil"),
            Language::R => format!("{inner} or NULL"),
            Language::Rust => format!("Option<{inner}>"),
            Language::Ffi => format!("{inner}*"),
        };
    }
    doc_type(ty, lang, ffi_prefix)
}

// ---------------------------------------------------------------------------
// Doc string utilities
// ---------------------------------------------------------------------------

/// Rust doc section headers that should be stripped for all non-Rust output.
const RUST_ONLY_SECTIONS: &[&str] = &["example", "examples", "arguments", "fields"];

/// Wrap bare `http://` and `https://` URLs in angle brackets to satisfy MD034.
/// Skips URLs already inside markdown links `[...](url)` or angle brackets `<url>`.
fn wrap_bare_urls(text: &str) -> String {
    let url_re = regex::Regex::new(r"(https?://[^\s)>\]]+)").unwrap();
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for mat in url_re.find_iter(text) {
        let start = mat.start();
        // Check character before the URL
        let preceding = if start > 0 { text.as_bytes()[start - 1] } else { b' ' };
        // Skip if already inside parens (markdown link) or angle brackets
        if preceding == b'(' || preceding == b'<' {
            continue;
        }
        result.push_str(&text[last_end..start]);
        result.push('<');
        result.push_str(mat.as_str());
        result.push('>');
        last_end = mat.end();
    }
    result.push_str(&text[last_end..]);
    result
}

/// Clean up Rust doc strings for Markdown output.
///
/// - Strips `# Example`, `# Arguments`, `# Fields` sections (Rust-specific)
/// - Strips code blocks containing Rust-specific syntax
/// - Converts `` [`Foo`](Self::bar) `` → `` `Foo` ``
/// - Converts bare `` [`Foo`] `` → `` `Foo` ``
/// - Converts `# Errors` / `# Returns` headings to bold inline text
/// - Converts `Foo::bar()` Rust path syntax to `Foo.bar()` in prose
fn clean_doc(doc: &str, lang: Language) -> String {
    if doc.is_empty() {
        return String::new();
    }

    // Strip Rust-specific sections and their code blocks
    let doc = strip_rust_sections(doc);

    // Convert Rust-style links
    let doc = rust_links_to_plain(&doc);

    // Convert `# Errors` / `# Returns` headings to bold inline text
    // These are Rust doc conventions that render as H1 headings, which is wrong
    let doc = convert_doc_headings_to_bold(&doc);

    // Convert Rust path syntax `Foo::bar()` → `Foo.bar()` (or `Foo::bar()` for PHP) in prose
    let doc = rust_paths_to_dot_notation(&doc, lang);

    // Replace Rust-centric terminology
    let doc = replace_rust_terminology(&doc, lang);

    doc.trim().to_string()
}

/// Convert `# Errors` and `# Returns` section headings to bold inline text.
fn convert_doc_headings_to_bold(doc: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !in_code_block && line.starts_with('#') {
            let heading_text = line.trim_start_matches('#').trim();
            let lower = heading_text.to_lowercase();
            if lower == "errors"
                || lower == "returns"
                || lower == "panics"
                || lower == "safety"
                || lower == "notes"
                || lower == "note"
            {
                let _ = writeln!(out, "**{heading_text}:**");
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Replace Rust-centric terminology with language-neutral equivalents.
fn replace_rust_terminology(doc: &str, lang: Language) -> String {
    let doc = doc
        .replace("this crate", "this library")
        .replace("in this crate", "in this library")
        .replace("for this crate", "for this library")
        .replace(
            "Panic caught during conversion to prevent unwinding across FFI boundaries",
            "Internal error caught during conversion",
        );

    // Replace OutputFormat.None references with language-neutral phrasing
    let doc = doc.replace(
        "None when `output_format` is set to `OutputFormat.None`",
        "null/nil when in extraction-only mode",
    );

    // Replace `None` backtick references with the language-idiomatic null
    let none_replacement = match lang {
        Language::Go | Language::Ruby | Language::Elixir => "`nil`",
        Language::Java | Language::Node | Language::Wasm | Language::Csharp | Language::Php => "`null`",
        Language::Python | Language::Rust => "`None`", // keep as-is for Python and Rust
        Language::R | Language::Ffi => "`NULL`",
    };
    let doc = doc.replace("`None`", none_replacement);

    // For Python, normalise boolean literals in prose: `true` → `True`, `false` → `False`
    if lang == Language::Python {
        let doc = doc.replace("`true`", "`True`").replace("`false`", "`False`");
        return doc;
    }

    // For non-Python languages, normalise Rust/Python boolean literals: `True` → `true`, `False` → `false`
    if lang != Language::Rust {
        let doc = doc.replace("`True`", "`true`").replace("`False`", "`false`");
        return doc;
    }

    doc
}

/// Replace Rust `Foo::bar()` path notation with `Foo.bar()` in prose (outside code blocks).
///
/// For PHP, static method calls use `::` so we keep that separator.
fn rust_paths_to_dot_notation(doc: &str, lang: Language) -> String {
    // PHP uses `::` for static method calls; other languages use `.`
    let sep = if lang == Language::Php { "::" } else { "." };
    let mut out = String::new();
    let mut in_code_block = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Replace `Foo::bar` patterns in prose
        // Common Rust-isms: Default::default(), ConversionOptions::default(), ConversionOptions::builder()
        let line = line
            .replace("Default::default()", "the default constructor")
            .replace("::", sep);
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Inline version that also strips newlines for use in table cells.
fn clean_doc_inline(doc: &str, lang: Language) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let cleaned = clean_doc(doc, lang);
    // Collapse to single line for table cells
    cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        // Escape pipe characters in table cells
        .replace('|', "\\|")
}

/// Strip Rust-specific doc sections (`# Example`, `# Arguments`, `# Fields`).
///
/// Also strips fenced code blocks that contain Rust-specific syntax
/// (use statements, unwrap(), assert!, etc.) regardless of which section they appear in.
fn strip_rust_sections(doc: &str) -> String {
    let mut out = String::new();
    let mut skip_section = false;
    let mut in_code_block = false;
    let mut code_block_buf = String::new();

    for line in doc.lines() {
        // Track code block boundaries
        if line.trim_start().starts_with("```") {
            if in_code_block {
                // End of code block — decide whether to emit it
                in_code_block = false;
                if !skip_section && !is_rust_code_block(&code_block_buf) {
                    out.push_str(&code_block_buf);
                    out.push_str(line);
                    out.push('\n');
                }
                code_block_buf.clear();
                continue;
            } else {
                in_code_block = true;
                if !skip_section {
                    code_block_buf.push_str(line);
                    code_block_buf.push('\n');
                }
                continue;
            }
        }

        if in_code_block {
            if !skip_section {
                code_block_buf.push_str(line);
                code_block_buf.push('\n');
            }
            continue;
        }

        // Outside code block: check for section headers
        if line.starts_with('#') {
            let header_text = line.trim_start_matches('#').trim().to_lowercase();
            if RUST_ONLY_SECTIONS.contains(&header_text.as_str()) {
                skip_section = true;
                continue;
            } else {
                // Any other section header ends the skip
                skip_section = false;
            }
        }

        if skip_section {
            // Blank lines and list items are part of the section — keep skipping
            let trimmed = line.trim();
            let is_section_content = trimmed.is_empty()
                || trimmed.starts_with('*')
                || trimmed.starts_with('-')
                || trimmed.starts_with('+')
                || trimmed.starts_with("  ") // indented continuation
                || trimmed.starts_with('\t');
            if is_section_content {
                continue;
            }
            // Non-list, non-blank line ends the skip
            skip_section = false;
        }

        // Skip lines that are clearly Rust-specific (unfenced imports/assertions)
        if is_rust_specific_line(line) {
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

/// Returns true if a code block's content contains Rust-specific patterns.
fn is_rust_code_block(content: &str) -> bool {
    // Opening fence line may declare "rust" or "no_run" etc.
    let first_line = content.lines().next().unwrap_or("");
    let fence_lang = first_line.trim_start_matches('`').trim().to_lowercase();
    if matches!(fence_lang.as_str(), "rust" | "rust,no_run" | "rust,ignore" | "") {
        // Check if content looks like Rust
        for line in content.lines().skip(1) {
            if line.starts_with("use ")
                || line.contains("unwrap()")
                || line.contains("assert!")
                || line.contains("assert_eq!")
                || line.contains("Vec::new()")
                || line.contains("Default::default()")
                || line.contains("::new(")
                || line.contains(".to_string()")
                || line.contains("html_to_markdown")
                || line.contains("r#\"")
            {
                return true;
            }
        }
    }
    false
}

/// Returns true if a plain (non-fenced) line is Rust-specific and should be removed.
fn is_rust_specific_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("# use ") || trimmed.starts_with("use ") && trimmed.ends_with(';')
}

/// Extract parameter descriptions from a `# Arguments` section in a doc string.
///
/// Parses lines like `* name - description` or `* name: description`.
/// Returns a map of parameter name → description.
fn extract_param_docs(doc: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut in_args = false;
    let mut in_code_block = false;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        if line.starts_with('#') {
            let header = line.trim_start_matches('#').trim().to_lowercase();
            in_args = matches!(header.as_str(), "arguments" | "args" | "parameters" | "params");
            continue;
        }

        if in_args {
            // Match "* `param_name` - description" or "* param_name - description"
            // or "* param_name: description"
            let trimmed = line.trim_start_matches(['*', '-', ' ']);
            // Try " - " separator first (3 chars), then ": " (2 chars)
            let parsed = trimmed
                .find(" - ")
                .map(|pos| (pos, 3))
                .or_else(|| trimmed.find(": ").map(|pos| (pos, 2)));
            if let Some((sep_pos, sep_len)) = parsed {
                let raw_name = trimmed[..sep_pos].trim();
                // Strip surrounding backticks if present (e.g. `` `html` `` → `html`)
                let param_name = raw_name.trim_matches('`');
                let desc = trimmed[sep_pos + sep_len..].trim();
                if !param_name.is_empty() && !desc.is_empty() {
                    map.insert(param_name.to_string(), desc.to_string());
                }
            }
        }
    }

    map
}

/// Convert `` [`text`](path) `` and bare `` [`text`] `` patterns to `` `text` ``.
fn rust_links_to_plain(doc: &str) -> String {
    // Pattern 1: [`text`](anything) → `text`
    // Pattern 2: [`text`] → `text`  (bare doc links)
    let mut result = String::with_capacity(doc.len());
    let chars: Vec<char> = doc.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Look for [`
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '`' {
            // Find closing `]`
            let start = i + 1; // position of opening `
            let mut j = start;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() {
                let text: String = chars[start..j].iter().collect();
                // Check if followed by `(` (linked form) or not (bare form)
                if j + 1 < chars.len() && chars[j + 1] == '(' {
                    // Linked form: find closing `)`
                    let mut k = j + 2;
                    while k < chars.len() && chars[k] != ')' {
                        k += 1;
                    }
                    if k < chars.len() {
                        result.push_str(&text);
                        i = k + 1;
                        continue;
                    }
                } else {
                    // Bare form: [`text`] — emit just the text
                    result.push_str(&text);
                    i = j + 1;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Ordering helpers
// ---------------------------------------------------------------------------

fn type_sort_key(name: &str) -> (u8, &str) {
    match name {
        "ConversionOptions" => (0, name),
        "ConversionResult" => (1, name),
        _ => (2, name),
    }
}

fn is_update_type(name: &str) -> bool {
    name.ends_with("Update")
}

/// Generate a human-readable description for an enum variant from its name.
///
/// Handles both PascalCase (`SingleColumn` → "Single column") and
/// SCREAMING_CASE (`CODE_BLOCK` → "Code block element") variant names.
fn generate_enum_variant_description(variant_name: &str) -> String {
    if variant_name.is_empty() {
        return String::new();
    }

    // Well-known variant names with specific descriptions
    match variant_name {
        "TEXT" => return "Text format".to_string(),
        "MARKDOWN" => return "Markdown format".to_string(),
        "HTML" | "Html" => return "Preserve as HTML `<mark>` tags".to_string(),
        "JSON" => return "JSON format".to_string(),
        "CSV" => return "CSV format".to_string(),
        "XML" => return "XML format".to_string(),
        "PDF" => return "PDF format".to_string(),
        "PLAIN" => return "Plain text format".to_string(),
        _ => {}
    }

    // Detect SCREAMING_CASE: all uppercase/underscores/digits
    let is_screaming = variant_name
        .chars()
        .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit());

    let words: Vec<String> = if is_screaming {
        // SCREAMING_CASE: split on underscores and lowercase
        variant_name
            .split('_')
            .filter(|s| !s.is_empty())
            .map(|w| w.to_lowercase())
            .collect()
    } else {
        // PascalCase: split on uppercase boundaries
        let mut parts = Vec::new();
        let mut current = String::new();
        let chars: Vec<char> = variant_name.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if c.is_uppercase() && !current.is_empty() {
                // Check for acronym runs (e.g., "OSD" in "AutoOsd" stays together
                // only if next char is also uppercase or we're at the end)
                let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
                if next_is_lower && current.len() > 1 && current.chars().all(|ch| ch.is_uppercase()) {
                    // End of acronym run: split off all but last char
                    let last = current.pop().expect("current is non-empty");
                    if !current.is_empty() {
                        parts.push(current);
                    }
                    current = String::new();
                    current.push(last);
                } else {
                    parts.push(current);
                    current = String::new();
                }
            }
            current.push(c);
        }
        if !current.is_empty() {
            parts.push(current);
        }
        parts.into_iter().map(|w| w.to_lowercase()).collect()
    };

    if words.is_empty() {
        return String::new();
    }

    // Determine suffix based on category heuristics
    let joined = words.join(" ");
    let suffix = determine_enum_variant_suffix(&joined, is_screaming);

    // Capitalize first letter
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => {
            let capitalized = first.to_uppercase().to_string() + chars.as_str();
            if suffix.is_empty() {
                capitalized
            } else {
                format!("{capitalized} {suffix}")
            }
        }
        None => String::new(),
    }
}

/// Determine an appropriate suffix for an enum variant description.
fn determine_enum_variant_suffix(readable: &str, is_screaming: bool) -> &'static str {
    // Format-like variants
    let format_words = [
        "text", "markdown", "html", "json", "csv", "xml", "pdf", "yaml", "toml", "docx", "xlsx", "pptx", "rtf",
        "latex", "rst", "asciidoc", "epub",
    ];
    for w in &format_words {
        if readable == *w {
            return "format";
        }
    }

    // Element-like variants (common in SCREAMING_CASE block-level names)
    let element_words = [
        "heading",
        "paragraph",
        "blockquote",
        "table",
        "figure",
        "caption",
        "footnote",
        "header",
        "footer",
        "section",
        "title",
        "subtitle",
        "image",
    ];
    for w in &element_words {
        if readable == *w {
            return "element";
        }
    }

    // If it already ends with a category word, no suffix needed
    let no_suffix_endings = [
        "format", "mode", "type", "level", "style", "strategy", "method", "state", "status", "error", "element",
        "block", "list", "model",
    ];
    for ending in &no_suffix_endings {
        if readable.ends_with(ending) {
            return "";
        }
    }

    // SCREAMING_CASE compound names ending in list/block often describe elements
    if is_screaming && (readable.contains("list") || readable.contains("block") || readable.contains("item")) {
        return "";
    }

    // No category suffix applicable
    ""
}

/// Generate a human-readable description for an error variant from its PascalCase name.
///
/// Splits PascalCase into words and forms a sentence like "IO errors" or "Parsing errors".
fn generate_error_variant_description(variant_name: &str) -> String {
    // Split PascalCase into words
    let mut words = Vec::new();
    let mut current = String::new();
    for c in variant_name.chars() {
        if c.is_uppercase() && !current.is_empty() {
            words.push(current);
            current = String::new();
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }

    if words.is_empty() {
        return String::new();
    }

    // Join into readable form and add "errors" suffix
    let readable = words.join(" ").to_lowercase();
    // Capitalize first letter
    let mut chars = readable.chars();
    match chars.next() {
        Some(first) => {
            let capitalized = first.to_uppercase().to_string() + chars.as_str();
            format!("{capitalized} errors")
        }
        None => String::new(),
    }
}

/// Generate a human-readable field description from its name and type
/// when no explicit doc comment exists on a struct field.
fn generate_field_description(field_name: &str, type_ref: &TypeRef) -> String {
    // Well-known field names with specific descriptions
    match field_name {
        "content" => return "The extracted text content".to_string(),
        "mime_type" => return "The detected MIME type".to_string(),
        "metadata" => return "Document metadata".to_string(),
        "tables" => return "Tables extracted from the document".to_string(),
        "images" => return "Images extracted from the document".to_string(),
        "pages" => return "Per-page content".to_string(),
        "chunks" => return "Text chunks for chunking/embedding".to_string(),
        "elements" => return "Semantic document elements".to_string(),
        "name" => return "The name".to_string(),
        "path" => return "File path".to_string(),
        "description" => return "Human-readable description".to_string(),
        "version" => return "Version string".to_string(),
        "id" => return "Unique identifier".to_string(),
        "enabled" => return "Whether this feature is enabled".to_string(),
        "size" => return "Size in bytes".to_string(),
        "count" => return "Number of items".to_string(),
        _ => {}
    }

    // Prefix-based patterns
    if let Some(rest) = field_name.strip_suffix("_count") {
        let readable = rest.replace('_', " ");
        let pluralized = if readable.ends_with('s') {
            readable
        } else {
            format!("{readable}s")
        };
        return format!("Number of {pluralized}");
    }
    if let Some(rest) = field_name.strip_prefix("is_") {
        let readable = rest.replace('_', " ");
        return format!("Whether {readable}");
    }
    if let Some(rest) = field_name.strip_prefix("has_") {
        let readable = rest.replace('_', " ");
        return format!("Whether {readable}");
    }
    if let Some(rest) = field_name.strip_prefix("max_") {
        let readable = rest.replace('_', " ");
        return format!("Maximum {readable}");
    }
    if let Some(rest) = field_name.strip_prefix("min_") {
        let readable = rest.replace('_', " ");
        return format!("Minimum {readable}");
    }

    // For named types, use the type name for extra context
    if let TypeRef::Named(type_name) = type_ref {
        let readable_type = type_name.chars().enumerate().fold(String::new(), |mut acc, (i, c)| {
            if c.is_uppercase() && i > 0 {
                acc.push(' ');
                acc.push(c.to_ascii_lowercase());
            } else if i == 0 {
                acc.push(c.to_ascii_lowercase());
            } else {
                acc.push(c);
            }
            acc
        });
        // If the field name matches the type (e.g. field "metadata" of type "Metadata"),
        // we already handled it above, so this provides context for other combos.
        let readable_name = snake_to_readable(field_name);
        return format!("{readable_name} ({readable_type})");
    }

    // Default: convert snake_case to readable text
    snake_to_readable(field_name)
}

/// Convert a `snake_case` identifier to `Readable text` (capitalize first letter).
fn snake_to_readable(name: &str) -> String {
    let readable = name.replace('_', " ");
    let mut chars = readable.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Generate a human-readable parameter description from its name and type
/// when no explicit doc comment or `# Arguments` entry exists.
fn generate_param_description(name: &str, ty: &TypeRef) -> String {
    // Derive a readable noun phrase from the parameter name by splitting on underscores
    // and joining with spaces (e.g. "mime_type" → "MIME type", "config" → "configuration").
    let article = match name {
        // Common names that benefit from a specific description
        "config" | "configuration" => return "The configuration options".to_string(),
        "options" | "opts" => return "The options to use".to_string(),
        "path" | "file_path" => return "Path to the file".to_string(),
        "content" | "contents" => return "The content to process".to_string(),
        "input" => return "The input data".to_string(),
        "output" => return "The output destination".to_string(),
        "url" => return "The URL to fetch".to_string(),
        "timeout" => return "Timeout duration".to_string(),
        "callback" | "cb" => return "Callback function".to_string(),
        _ => "The",
    };

    // For named types, use the type name for context
    let type_hint = match ty {
        TypeRef::Named(type_name) => {
            // Convert PascalCase type name to readable form
            type_name.chars().enumerate().fold(String::new(), |mut acc, (i, c)| {
                if c.is_uppercase() && i > 0 {
                    acc.push(' ');
                    acc.push(c.to_ascii_lowercase());
                } else if i == 0 {
                    acc.push(c.to_ascii_lowercase());
                } else {
                    acc.push(c);
                }
                acc
            })
        }
        _ => {
            // For non-named types, use the param name as description
            name.replace('_', " ")
        }
    };

    format!("{article} {type_hint}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::PrimitiveType;

    const TEST_PREFIX: &str = "Htm";

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
    fn test_enum_variant_name_python() {
        assert_eq!(enum_variant_name("Atx", Language::Python, TEST_PREFIX), "ATX");
        assert_eq!(
            enum_variant_name("SnakeCase", Language::Python, TEST_PREFIX),
            "SNAKE_CASE"
        );
    }

    #[test]
    fn test_enum_variant_name_java() {
        assert_eq!(enum_variant_name("Atx", Language::Java, TEST_PREFIX), "ATX");
    }

    #[test]
    fn test_enum_variant_name_ffi() {
        assert_eq!(enum_variant_name("Atx", Language::Ffi, TEST_PREFIX), "HTM_ATX");
    }

    #[test]
    fn test_type_name_ffi_uses_prefix() {
        assert_eq!(
            type_name("ConversionOptions", Language::Ffi, "Kreuzberg"),
            "KreuzbergConversionOptions"
        );
        assert_eq!(
            type_name("ConversionResult", Language::Ffi, "Kreuzberg"),
            "KreuzbergConversionResult"
        );
    }

    #[test]
    fn test_func_name_ffi_uses_prefix() {
        assert_eq!(func_name("convert", Language::Ffi, "Kreuzberg"), "kreuzberg_convert");
    }

    #[test]
    fn test_enum_variant_name_ffi_uses_prefix() {
        assert_eq!(enum_variant_name("Atx", Language::Ffi, "Kreuzberg"), "KREUZBERG_ATX");
    }

    #[test]
    fn test_clean_doc_strips_examples() {
        let doc = "Does something.\n\n# Examples\n\n```rust\nfoo();\n```\n";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(!cleaned.contains("Examples"));
        assert!(!cleaned.contains("foo()"));
        assert!(cleaned.contains("Does something"));
    }

    #[test]
    fn test_clean_doc_strips_arguments() {
        let doc = "Does something.\n\n# Arguments\n\n* html - The HTML string\n\nMore text.";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(!cleaned.contains("Arguments"));
        assert!(!cleaned.contains("html - The HTML string"));
        assert!(cleaned.contains("Does something"));
        assert!(cleaned.contains("More text"));
    }

    #[test]
    fn test_clean_doc_rust_links() {
        let doc = "See [`field`](Self::field) for details.";
        let cleaned = clean_doc(doc, Language::Python);
        assert_eq!(cleaned, "See `field` for details.");
    }

    #[test]
    fn test_clean_doc_bare_rust_links() {
        let doc = "See [`ConversionOptions`] for details.";
        let cleaned = clean_doc(doc, Language::Python);
        assert_eq!(cleaned, "See `ConversionOptions` for details.");
    }

    #[test]
    fn test_extract_param_docs() {
        let doc = "Convert HTML to Markdown.\n\n# Arguments\n\n* html - The HTML string to convert\n* options - Conversion options\n";
        let params = extract_param_docs(doc);
        assert_eq!(
            params.get("html").map(String::as_str),
            Some("The HTML string to convert")
        );
        assert_eq!(params.get("options").map(String::as_str), Some("Conversion options"));
    }

    #[test]
    fn test_field_name_go_pascal_case() {
        assert_eq!(field_name("heading_style", Language::Go), "HeadingStyle");
        assert_eq!(field_name("list_indent_type", Language::Go), "ListIndentType");
    }

    #[test]
    fn test_is_update_type() {
        assert!(is_update_type("ConversionOptionsUpdate"));
        assert!(!is_update_type("ConversionOptions"));
    }

    #[test]
    fn test_type_sort_key_ordering() {
        assert!(type_sort_key("ConversionOptions") < type_sort_key("ConversionResult"));
        assert!(type_sort_key("ConversionResult") < type_sort_key("SomeOtherType"));
    }

    #[test]
    fn test_func_name_conventions() {
        assert_eq!(func_name("convert", Language::Python, TEST_PREFIX), "convert");
        assert_eq!(func_name("convert_html", Language::Node, TEST_PREFIX), "convertHtml");
        assert_eq!(func_name("convert_html", Language::Go, TEST_PREFIX), "ConvertHtml");
        assert_eq!(func_name("convert", Language::Ffi, TEST_PREFIX), "htm_convert");
    }

    #[test]
    fn test_type_name_ffi_prefix() {
        assert_eq!(
            type_name("ConversionOptions", Language::Ffi, TEST_PREFIX),
            "HtmConversionOptions"
        );
        assert_eq!(
            type_name("ConversionResult", Language::Ffi, TEST_PREFIX),
            "HtmConversionResult"
        );
    }

    #[test]
    fn test_generate_docs_empty_api() {
        let api = ApiSurface {
            crate_name: "test".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        use alef_core::config::*;
        let config = AlefConfig {
            crate_config: CrateConfig {
                name: "test".to_string(),
                sources: vec![],
                version_from: "Cargo.toml".to_string(),
                core_import: None,
                workspace_root: None,
                skip_core_import: false,
                features: vec![],
                path_mappings: std::collections::HashMap::new(),
                auto_path_mappings: Default::default(),
                extra_dependencies: Default::default(),
                source_crates: vec![],
                error_type: None,
                error_constructor: None,
            },
            languages: vec![Language::Python],
            exclude: ExcludeConfig::default(),
            include: IncludeConfig::default(),
            output: OutputConfig::default(),
            python: None,
            node: None,
            ruby: None,
            php: None,
            elixir: None,
            wasm: None,
            ffi: None,
            go: None,
            java: None,
            csharp: None,
            r: None,
            scaffold: None,
            readme: None,
            lint: None,
            test: None,
            custom_files: None,
            adapters: vec![],
            custom_modules: CustomModulesConfig::default(),
            custom_registrations: CustomRegistrationsConfig::default(),
            opaque_types: std::collections::HashMap::new(),
            generate: GenerateConfig::default(),
            generate_overrides: std::collections::HashMap::new(),
            dto: Default::default(),
            sync: None,
            e2e: None,
            trait_bridges: vec![],
        };

        let files = generate_docs(&api, &config, &[Language::Python], "docs").unwrap();
        // 1 lang + configuration.md + types.md + errors.md
        assert_eq!(files.len(), 4);
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("Python API Reference"));
        assert!(lang_file.content.contains("v0.1.0"));
    }

    #[test]
    fn test_generate_field_description_known_names() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("content", &ty), "The extracted text content");
        assert_eq!(generate_field_description("mime_type", &ty), "The detected MIME type");
        assert_eq!(generate_field_description("metadata", &ty), "Document metadata");
        assert_eq!(
            generate_field_description("tables", &ty),
            "Tables extracted from the document"
        );
        assert_eq!(
            generate_field_description("images", &ty),
            "Images extracted from the document"
        );
        assert_eq!(generate_field_description("pages", &ty), "Per-page content");
        assert_eq!(
            generate_field_description("chunks", &ty),
            "Text chunks for chunking/embedding"
        );
        assert_eq!(
            generate_field_description("elements", &ty),
            "Semantic document elements"
        );
        assert_eq!(generate_field_description("name", &ty), "The name");
        assert_eq!(generate_field_description("path", &ty), "File path");
        assert_eq!(
            generate_field_description("description", &ty),
            "Human-readable description"
        );
        assert_eq!(generate_field_description("version", &ty), "Version string");
        assert_eq!(generate_field_description("id", &ty), "Unique identifier");
        assert_eq!(
            generate_field_description("enabled", &ty),
            "Whether this feature is enabled"
        );
        assert_eq!(generate_field_description("size", &ty), "Size in bytes");
        assert_eq!(generate_field_description("count", &ty), "Number of items");
    }

    #[test]
    fn test_generate_field_description_prefix_patterns() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("row_count", &ty), "Number of rows");
        assert_eq!(generate_field_description("is_valid", &ty), "Whether valid");
        assert_eq!(generate_field_description("has_errors", &ty), "Whether errors");
        assert_eq!(generate_field_description("max_retries", &ty), "Maximum retries");
        assert_eq!(generate_field_description("min_confidence", &ty), "Minimum confidence");
        assert_eq!(generate_field_description("is_ocr_enabled", &ty), "Whether ocr enabled");
    }

    #[test]
    fn test_generate_field_description_named_type() {
        let ty = TypeRef::Named("ExtractionConfig".to_string());
        assert_eq!(generate_field_description("config", &ty), "Config (extraction config)");
    }

    #[test]
    fn test_generate_field_description_fallback_snake_case() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("column_types", &ty), "Column types");
        assert_eq!(generate_field_description("output_format", &ty), "Output format");
    }

    #[test]
    fn test_snake_to_readable() {
        assert_eq!(snake_to_readable("row_count"), "Row count");
        assert_eq!(snake_to_readable("column_types"), "Column types");
        assert_eq!(snake_to_readable("x"), "X");
        assert_eq!(snake_to_readable(""), "");
    }

    #[test]
    fn test_generate_enum_variant_description_well_known() {
        assert_eq!(generate_enum_variant_description("TEXT"), "Text format");
        assert_eq!(generate_enum_variant_description("MARKDOWN"), "Markdown format");
        assert_eq!(
            generate_enum_variant_description("HTML"),
            "Preserve as HTML `<mark>` tags"
        );
        assert_eq!(generate_enum_variant_description("JSON"), "JSON format");
        assert_eq!(generate_enum_variant_description("PDF"), "PDF format");
        assert_eq!(generate_enum_variant_description("PLAIN"), "Plain text format");
    }

    #[test]
    fn test_generate_enum_variant_description_screaming_case() {
        assert_eq!(generate_enum_variant_description("CODE_BLOCK"), "Code block");
        assert_eq!(generate_enum_variant_description("ORDERED_LIST"), "Ordered list");
        assert_eq!(generate_enum_variant_description("BULLET_LIST"), "Bullet list");
        assert_eq!(generate_enum_variant_description("HEADING"), "Heading element");
    }

    #[test]
    fn test_generate_enum_variant_description_pascal_case() {
        assert_eq!(generate_enum_variant_description("SingleColumn"), "Single column");
        assert_eq!(generate_enum_variant_description("AutoOsd"), "Auto osd");
    }

    #[test]
    fn test_generate_enum_variant_description_empty() {
        assert_eq!(generate_enum_variant_description(""), "");
    }

    // Regression tests for GitHub issue #5: whitespace between `'static` and the
    // following type name or bracket was being stripped, producing `&'staticstr`
    // instead of `&'static str` and `&'static[&'staticstr]` instead of
    // `&'static [&'static str]`.

    #[test]
    fn test_doc_type_rust_static_str_in_named_tuple() {
        // A tuple type whose element is encoded as a Named with a static str.
        // The Named string arrives from alef-extract's type_to_string, which now
        // preserves the space after `'static`.
        let ty = TypeRef::Named("(&'static str)".to_string());
        // For Rust output the raw name is passed through unchanged.
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "(&'static str)");
    }

    #[test]
    fn test_doc_type_named_static_str_renders_correctly_for_non_rust() {
        // When a Named type encodes a two-element tuple where one element is `&'static str`,
        // it should map to the idiomatic string type for each language.
        let ty = TypeRef::Named("(&'static str, u32)".to_string());
        // The inner element `&'static str` is recognised by the string-type arm.
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "tuple[str, int]");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "[string, number]");
    }

    #[test]
    fn test_doc_type_static_slice_in_tuple_element_rust() {
        // The slice-of-strings arm covers `&'static [&'static str]` tokens.
        // After the whitespace fix, the Named string is `&'static [&'static str]`
        // (with spaces preserved); the arm detects `[&` and maps correctly.
        let ty = TypeRef::Named("(&'static [&'static str], u32)".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "tuple[list[str], int]");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "([]string, int)");
    }

    // ---------------------------------------------------------------------------
    // Helpers for building test IR objects
    // ---------------------------------------------------------------------------

    fn make_param(name: &str, ty: TypeRef, optional: bool) -> alef_core::ir::ParamDef {
        alef_core::ir::ParamDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
        }
    }

    fn make_method(
        name: &str,
        params: Vec<alef_core::ir::ParamDef>,
        return_type: TypeRef,
        is_async: bool,
        is_static: bool,
        error_type: Option<&str>,
    ) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async,
            is_static,
            error_type: error_type.map(str::to_string),
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    fn make_function(
        name: &str,
        params: Vec<alef_core::ir::ParamDef>,
        return_type: TypeRef,
        is_async: bool,
        error_type: Option<&str>,
    ) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            params,
            return_type,
            is_async,
            error_type: error_type.map(str::to_string),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }
    }

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
        // Python bindings wrap async; the signature still uses `def`, not `async def`
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
        // error_type is not reflected in the Python method signature itself
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
        // Node async is expressed at callsite (Promise); signature stays the same
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
        // Rust async is not reflected in method signatures (rendered same as sync)
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
        // error_type is not part of the Rust method signature in this renderer
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
        // Ruby signatures don't include return types
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
        // PHP uses `?T` nullable prefix syntax for Optional types
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
        // error_type not reflected in C# method signatures
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
        // Python signatures always use `def`, async is transparent at the Python level
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
        // Go has no async keyword; async Rust functions become regular Go functions
        let func = make_function("fetch", vec![], TypeRef::String, true, None);
        let sig = render_go_fn_sig(&func, TEST_PREFIX);
        assert_eq!(sig, "func Fetch() string");
    }

    #[test]
    fn test_render_go_fn_sig_optional_param() {
        // Go optional params are represented as pointers
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
        // Go has no optional syntax; all params are required
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
        // String and Char params in Rust should be rendered as &str
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

    // ---------------------------------------------------------------------------
    // Helper — minimal FieldDef construction for default-value tests
    // ---------------------------------------------------------------------------

    fn make_field(name: &str, ty: TypeRef, optional: bool, typed_default: Option<DefaultValue>) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    fn empty_api() -> ApiSurface {
        ApiSurface {
            crate_name: "test".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    // ---------------------------------------------------------------------------
    // doc_type — comprehensive coverage of TypeRef variants × languages
    // ---------------------------------------------------------------------------

    #[test]
    fn test_doc_type_char_maps_like_string() {
        // Char should map identically to String for every language.
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
    }

    #[test]
    fn test_doc_type_named_strips_module_path() {
        let ty = TypeRef::Named("my_crate::types::OutputFormat".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "OutputFormat");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "OutputFormat");
        // FFI prefixes the short name with the configured prefix
        assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HtmOutputFormat");
    }

    #[test]
    fn test_doc_type_named_without_path() {
        let ty = TypeRef::Named("ConversionOptions".to_string());
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Ffi, TEST_PREFIX), "HtmConversionOptions");
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
        // Java boxes primitives when used as Map value type arguments
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
        // Vec<Option<String>>
        let ty = TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::String))));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "list[str | None]");
        assert_eq!(doc_type(&ty, Language::Node, TEST_PREFIX), "Array<string | null>");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "[]*string");
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "List<Optional<String>>");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "Vec<Option<String>>");
    }

    #[test]
    fn test_doc_type_nested_map_string_to_vec_u32() {
        // Map<String, Vec<u32>>
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
        // Java boxes Vec inner primitives too
        assert_eq!(doc_type(&ty, Language::Java, TEST_PREFIX), "Map<String, List<Integer>>");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "HashMap<String, Vec<u32>>");
    }

    #[test]
    fn test_doc_type_optional_of_named_all_languages() {
        // Option<ConversionOptions>
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("ConversionOptions".to_string())));
        assert_eq!(doc_type(&ty, Language::Python, TEST_PREFIX), "ConversionOptions | None");
        assert_eq!(
            doc_type(&ty, Language::Java, TEST_PREFIX),
            "Optional<ConversionOptions>"
        );
        assert_eq!(doc_type(&ty, Language::Csharp, TEST_PREFIX), "ConversionOptions?");
        assert_eq!(doc_type(&ty, Language::Go, TEST_PREFIX), "*ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Rust, TEST_PREFIX), "Option<ConversionOptions>");
        assert_eq!(doc_type(&ty, Language::Ruby, TEST_PREFIX), "ConversionOptions?");
        assert_eq!(doc_type(&ty, Language::Php, TEST_PREFIX), "?ConversionOptions");
        assert_eq!(doc_type(&ty, Language::Elixir, TEST_PREFIX), "ConversionOptions | nil");
        assert_eq!(doc_type(&ty, Language::R, TEST_PREFIX), "ConversionOptions or NULL");
    }

    // ---------------------------------------------------------------------------
    // doc_type — all primitives for key languages
    // ---------------------------------------------------------------------------

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

    // ---------------------------------------------------------------------------
    // doc_type_with_optional
    // ---------------------------------------------------------------------------

    #[test]
    fn test_doc_type_with_optional_true_wraps_correctly() {
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Python, true, TEST_PREFIX),
            "str | None"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Node, true, TEST_PREFIX),
            "string | null"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Go, true, TEST_PREFIX),
            "*string"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Csharp, true, TEST_PREFIX),
            "string?"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ruby, true, TEST_PREFIX),
            "String?"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Php, true, TEST_PREFIX),
            "?string"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Elixir, true, TEST_PREFIX),
            "String.t() | nil"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::R, true, TEST_PREFIX),
            "character or NULL"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Rust, true, TEST_PREFIX),
            "Option<String>"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ffi, true, TEST_PREFIX),
            "const char**"
        );
    }

    #[test]
    fn test_doc_type_with_optional_false_is_identity() {
        // optional=false must not wrap — same result as doc_type
        for lang in [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Java,
            Language::Rust,
        ] {
            assert_eq!(
                doc_type_with_optional(&TypeRef::String, lang, false, TEST_PREFIX),
                doc_type(&TypeRef::String, lang, TEST_PREFIX),
                "optional=false should be identity for {lang:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_with_optional_does_not_double_wrap_already_optional_type() {
        // If the inner type is already Optional<T>, optional=true must not nest again
        let already_optional = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(
            doc_type_with_optional(&already_optional, Language::Python, true, TEST_PREFIX),
            "str | None"
        );
        assert_eq!(
            doc_type_with_optional(&already_optional, Language::Rust, true, TEST_PREFIX),
            "Option<String>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_i32() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::I32),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "Optional<Integer>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_bool() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::Bool),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "Optional<Boolean>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_f64() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::F64),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "Optional<Double>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_non_primitive_not_double_boxed() {
        // String is already a reference type; java_boxed_type returns "String"
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Java, true, TEST_PREFIX),
            "Optional<String>"
        );
    }

    // ---------------------------------------------------------------------------
    // java_boxed_type
    // ---------------------------------------------------------------------------

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
        // Non-primitive types are already reference types in Java.
        assert_eq!(java_boxed_type(&TypeRef::String), "String");
        assert_eq!(java_boxed_type(&TypeRef::Bytes), "byte[]");
        assert_eq!(
            java_boxed_type(&TypeRef::Named("ConversionOptions".to_string())),
            "ConversionOptions"
        );
        assert_eq!(java_boxed_type(&TypeRef::Duration), "Duration");
    }

    // ---------------------------------------------------------------------------
    // determine_enum_variant_suffix
    // ---------------------------------------------------------------------------

    #[test]
    fn test_determine_enum_variant_suffix_format_words() {
        for word in ["text", "markdown", "html", "json", "csv", "xml", "pdf", "yaml"] {
            assert_eq!(
                determine_enum_variant_suffix(word, false),
                "format",
                "expected 'format' suffix for '{word}'"
            );
        }
    }

    #[test]
    fn test_determine_enum_variant_suffix_element_words() {
        for word in [
            "heading",
            "paragraph",
            "blockquote",
            "table",
            "figure",
            "caption",
            "footnote",
            "header",
            "footer",
            "section",
            "title",
            "image",
        ] {
            assert_eq!(
                determine_enum_variant_suffix(word, false),
                "element",
                "expected 'element' suffix for '{word}'"
            );
        }
    }

    #[test]
    fn test_determine_enum_variant_suffix_no_suffix_when_ending_matches_category_word() {
        // Already ends with a recognised category word — no extra suffix
        let no_suffix_cases = [
            "extraction mode",
            "output format",
            "heading style",
            "retry strategy",
            "connection state",
            "error status",
            "dom element",
            "code block",
            "ordered list",
            "language model",
        ];
        for word in no_suffix_cases {
            assert_eq!(
                determine_enum_variant_suffix(word, false),
                "",
                "expected empty suffix for '{word}'"
            );
        }
    }

    #[test]
    fn test_determine_enum_variant_suffix_screaming_with_list_block_item() {
        // SCREAMING compound names containing "list"/"block"/"item" get no suffix
        assert_eq!(determine_enum_variant_suffix("bullet list", true), "");
        assert_eq!(determine_enum_variant_suffix("code block", true), "");
        assert_eq!(determine_enum_variant_suffix("list item", true), "");
    }

    #[test]
    fn test_determine_enum_variant_suffix_unknown_word_returns_empty() {
        // Generic words that don't match any heuristic → empty suffix
        assert_eq!(determine_enum_variant_suffix("single column", false), "");
        assert_eq!(determine_enum_variant_suffix("auto osd", false), "");
        assert_eq!(determine_enum_variant_suffix("left", false), "");
    }

    // ---------------------------------------------------------------------------
    // format_field_default / format_typed_default
    // ---------------------------------------------------------------------------

    #[test]
    fn test_format_default_bool_literal_python_uses_capitalised_form() {
        let api = empty_api();
        let field_true = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(true)),
        );
        let field_false = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(false)),
        );
        assert_eq!(
            format_field_default(&field_true, Language::Python, &api, TEST_PREFIX),
            "`True`"
        );
        assert_eq!(
            format_field_default(&field_false, Language::Python, &api, TEST_PREFIX),
            "`False`"
        );
    }

    #[test]
    fn test_format_default_bool_literal_non_python_uses_lowercase_form() {
        let api = empty_api();
        let field_true = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(true)),
        );
        for lang in [Language::Rust, Language::Java, Language::Go, Language::Node] {
            assert_eq!(
                format_field_default(&field_true, lang, &api, TEST_PREFIX),
                "`true`",
                "bool literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_string_literal_all_languages_produce_quoted_form() {
        let api = empty_api();
        let field = make_field(
            "name",
            TypeRef::String,
            false,
            Some(DefaultValue::StringLiteral("hello".to_string())),
        );
        for lang in [
            Language::Python,
            Language::Rust,
            Language::Java,
            Language::Go,
            Language::Node,
        ] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`\"hello\"`",
                "string literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_int_literal() {
        let api = empty_api();
        let field = make_field(
            "count",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            Some(DefaultValue::IntLiteral(42)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java, Language::Node] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`42`",
                "int literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_int_literal_on_duration_field_shows_ms_suffix() {
        let api = empty_api();
        let field = make_field(
            "timeout",
            TypeRef::Duration,
            false,
            Some(DefaultValue::IntLiteral(5000)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java, Language::Go] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`5000ms`",
                "duration field should show ms suffix for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_float_literal() {
        let api = empty_api();
        let field = make_field(
            "confidence",
            TypeRef::Primitive(PrimitiveType::F32),
            false,
            Some(DefaultValue::FloatLiteral(0.85)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`0.85`",
                "float literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_enum_variant_qualified_python_and_rust() {
        let api = empty_api();
        let field = make_field(
            "style",
            TypeRef::Named("HeadingStyle".to_string()),
            false,
            Some(DefaultValue::EnumVariant("HeadingStyle::Atx".to_string())),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`HeadingStyle.ATX`"
        );
        // Rust: PascalCase variant preserved
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`HeadingStyle::Atx`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`HeadingStyle.ATX`"
        );
        assert_eq!(
            format_field_default(&field, Language::Ruby, &api, TEST_PREFIX),
            "`:atx`"
        );
        // PHP: PascalCase variant, :: separator
        assert_eq!(
            format_field_default(&field, Language::Php, &api, TEST_PREFIX),
            "`HeadingStyle::Atx`"
        );
    }

    #[test]
    fn test_format_default_empty_vec_field() {
        let api = empty_api();
        let field = make_field(
            "items",
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            Some(DefaultValue::Empty),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`[]`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`vec![]`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`Collections.emptyList()`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Csharp, &api, TEST_PREFIX),
            "`new List<string>()`"
        );
        assert_eq!(format_field_default(&field, Language::R, &api, TEST_PREFIX), "`list()`");
        assert_eq!(format_field_default(&field, Language::Ruby, &api, TEST_PREFIX), "`[]`");
        assert_eq!(
            format_field_default(&field, Language::Elixir, &api, TEST_PREFIX),
            "`[]`"
        );
        assert_eq!(format_field_default(&field, Language::Ffi, &api, TEST_PREFIX), "`NULL`");
    }

    #[test]
    fn test_format_default_empty_map_field() {
        let api = empty_api();
        let field = make_field(
            "attributes",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            false,
            Some(DefaultValue::Empty),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`{}`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`HashMap::new()`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`Collections.emptyMap()`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Elixir, &api, TEST_PREFIX),
            "`%{}`"
        );
        assert_eq!(
            format_field_default(&field, Language::Csharp, &api, TEST_PREFIX),
            "`new Dictionary<string, string>()`"
        );
    }

    #[test]
    fn test_format_default_none_on_optional_field() {
        let api = empty_api();
        let field = make_field("label", TypeRef::String, true, Some(DefaultValue::None));
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`None`"
        );
        assert_eq!(
            format_field_default(&field, Language::Node, &api, TEST_PREFIX),
            "`null`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`None`"
        );
        assert_eq!(format_field_default(&field, Language::Ffi, &api, TEST_PREFIX), "`NULL`");
        assert_eq!(format_field_default(&field, Language::R, &api, TEST_PREFIX), "`NULL`");
    }

    #[test]
    fn test_format_default_none_on_non_optional_field_returns_dash() {
        // DefaultValue::None on a required field should produce "—"
        let api = empty_api();
        let field = make_field(
            "count",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            Some(DefaultValue::None),
        );
        assert_eq!(format_field_default(&field, Language::Python, &api, TEST_PREFIX), "—");
    }

    #[test]
    fn test_format_default_empty_duration_shows_zero_ms_for_non_rust() {
        let api = empty_api();
        let field = make_field("timeout", TypeRef::Duration, false, Some(DefaultValue::Empty));
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`0ms`"
        );
        assert_eq!(format_field_default(&field, Language::Java, &api, TEST_PREFIX), "`0ms`");
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`0ms`");
    }

    #[test]
    fn test_format_default_empty_duration_rust_shows_duration_default() {
        let api = empty_api();
        let field = make_field("timeout", TypeRef::Duration, false, Some(DefaultValue::Empty));
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`Duration::default()`"
        );
    }

    // ---------------------------------------------------------------------------
    // clean_doc — additional coverage
    // ---------------------------------------------------------------------------

    #[test]
    fn test_clean_doc_empty_string_all_languages() {
        for lang in [Language::Python, Language::Go, Language::Node, Language::Rust] {
            assert_eq!(clean_doc("", lang), "", "empty doc for {lang:?} must stay empty");
        }
    }

    #[test]
    fn test_clean_doc_multiline_prose_all_paragraphs_preserved() {
        let doc = "First line.\n\nSecond paragraph.\n\nThird paragraph.";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(cleaned.contains("First line."));
        assert!(cleaned.contains("Second paragraph."));
        assert!(cleaned.contains("Third paragraph."));
    }

    #[test]
    fn test_clean_doc_none_becomes_nil_for_go_ruby_elixir() {
        let doc = "Returns `None` when nothing is found.";
        assert_eq!(clean_doc(doc, Language::Go), "Returns `nil` when nothing is found.");
        assert_eq!(clean_doc(doc, Language::Ruby), "Returns `nil` when nothing is found.");
        assert_eq!(clean_doc(doc, Language::Elixir), "Returns `nil` when nothing is found.");
    }

    #[test]
    fn test_clean_doc_none_becomes_null_for_node_java_csharp_php() {
        let doc = "Returns `None` on failure.";
        assert_eq!(clean_doc(doc, Language::Node), "Returns `null` on failure.");
        assert_eq!(clean_doc(doc, Language::Java), "Returns `null` on failure.");
        assert_eq!(clean_doc(doc, Language::Csharp), "Returns `null` on failure.");
        assert_eq!(clean_doc(doc, Language::Php), "Returns `null` on failure.");
    }

    #[test]
    fn test_clean_doc_none_stays_none_for_python_and_rust() {
        let doc = "Returns `None` when empty.";
        assert_eq!(clean_doc(doc, Language::Python), "Returns `None` when empty.");
        assert_eq!(clean_doc(doc, Language::Rust), "Returns `None` when empty.");
    }

    #[test]
    fn test_clean_doc_none_becomes_null_uppercase_for_r_and_ffi() {
        let doc = "Returns `None` when empty.";
        assert_eq!(clean_doc(doc, Language::R), "Returns `NULL` when empty.");
        assert_eq!(clean_doc(doc, Language::Ffi), "Returns `NULL` when empty.");
    }

    #[test]
    fn test_clean_doc_python_booleans_capitalised() {
        let doc = "Pass `true` to enable or `false` to disable.";
        let cleaned = clean_doc(doc, Language::Python);
        assert_eq!(cleaned, "Pass `True` to enable or `False` to disable.");
    }

    #[test]
    fn test_clean_doc_non_python_booleans_lowercase_unchanged() {
        let doc = "Pass `true` to enable or `false` to disable.";
        assert_eq!(clean_doc(doc, Language::Go), doc);
        assert_eq!(clean_doc(doc, Language::Node), doc);
        assert_eq!(clean_doc(doc, Language::Java), doc);
    }

    #[test]
    fn test_clean_doc_rust_path_becomes_dot_notation_for_python() {
        let doc = "Call `Foo::bar()` to create one.";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(cleaned.contains("Foo.bar()"), "expected dot notation: {cleaned}");
        assert!(!cleaned.contains("Foo::bar()"));
    }

    #[test]
    fn test_clean_doc_rust_path_stays_double_colon_for_php() {
        let doc = "Call `Foo::bar()` to create one.";
        let cleaned = clean_doc(doc, Language::Php);
        assert!(cleaned.contains("Foo::bar()"), "PHP keeps :: notation: {cleaned}");
    }

    #[test]
    fn test_clean_doc_non_rust_code_block_preserved() {
        let doc = "Example:\n\n```python\nresult = convert(html)\n```\n";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(cleaned.contains("```python"));
        assert!(cleaned.contains("result = convert(html)"));
    }

    #[test]
    fn test_clean_doc_rust_code_block_stripped() {
        let doc = "Example:\n\n```rust\nuse foo::Bar;\nBar::new().unwrap();\n```\n\nAfter block.";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(!cleaned.contains("use foo::Bar"), "Rust use statement must be stripped");
        assert!(cleaned.contains("After block."));
    }

    #[test]
    fn test_clean_doc_errors_section_heading_becomes_bold() {
        let doc = "Summary.\n\n# Errors\n\nMay fail.\n";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(cleaned.contains("**Errors:**"), "heading must become bold: {cleaned}");
        assert!(!cleaned.contains("# Errors"), "raw # heading must be gone: {cleaned}");
    }

    #[test]
    fn test_clean_doc_returns_section_heading_becomes_bold() {
        let doc = "Summary.\n\n# Returns\n\nSome value.\n";
        let cleaned = clean_doc(doc, Language::Python);
        assert!(cleaned.contains("**Returns:**"));
        assert!(!cleaned.contains("# Returns"));
    }

    #[test]
    fn test_clean_doc_crate_references_replaced_with_library() {
        let doc = "Available in this crate as a public API.";
        assert_eq!(
            clean_doc(doc, Language::Python),
            "Available in this library as a public API."
        );
    }

    #[test]
    fn test_clean_doc_inline_code_spans_survive_for_rust() {
        let doc = "Use `None` or `false` to skip.";
        let cleaned = clean_doc(doc, Language::Rust);
        assert!(cleaned.contains("`None`"));
        assert!(cleaned.contains("`false`"));
    }

    // ---------------------------------------------------------------------------
    // clean_doc_inline — coverage
    // ---------------------------------------------------------------------------

    #[test]
    fn test_clean_doc_inline_empty_string() {
        assert_eq!(clean_doc_inline("", Language::Python), "");
        assert_eq!(clean_doc_inline("", Language::Go), "");
    }

    #[test]
    fn test_clean_doc_inline_collapses_multiline_to_single_line() {
        let doc = "First sentence.\nSecond sentence.";
        let result = clean_doc_inline(doc, Language::Python);
        assert!(!result.contains('\n'), "inline output must be single-line: {result}");
        assert!(result.contains("First sentence."));
        assert!(result.contains("Second sentence."));
    }

    #[test]
    fn test_clean_doc_inline_escapes_pipe_for_table_cells() {
        let doc = "Value between 0 | 1.";
        let result = clean_doc_inline(doc, Language::Python);
        assert!(result.contains("\\|"), "pipe must be escaped: {result}");
        assert!(!result.contains(" | "), "unescaped pipe must not remain: {result}");
    }

    #[test]
    fn test_clean_doc_inline_applies_language_terminology() {
        let doc = "Returns `None` when empty.";
        assert_eq!(clean_doc_inline(doc, Language::Go), "Returns `nil` when empty.");
        assert_eq!(clean_doc_inline(doc, Language::Node), "Returns `null` when empty.");
    }

    #[test]
    fn test_clean_doc_inline_strips_argument_sections() {
        let doc = "Summary.\n\n# Arguments\n\n* foo - bar\n";
        let result = clean_doc_inline(doc, Language::Python);
        assert!(!result.contains("Arguments"));
        assert!(!result.contains("foo - bar"));
        assert!(result.contains("Summary."));
    }

    #[test]
    fn test_clean_doc_inline_filters_blank_only_lines() {
        let doc = "\n\n  \n\nActual content.\n\n  \n";
        let result = clean_doc_inline(doc, Language::Python);
        assert_eq!(result, "Actual content.");
    }

    // ---------------------------------------------------------------------------
    // wrap_bare_urls — coverage
    // ---------------------------------------------------------------------------

    #[test]
    fn test_wrap_bare_urls_plain_https() {
        let text = "See https://example.com for details.";
        assert_eq!(wrap_bare_urls(text), "See <https://example.com> for details.");
    }

    #[test]
    fn test_wrap_bare_urls_plain_http() {
        let text = "Visit http://example.com today.";
        assert_eq!(wrap_bare_urls(text), "Visit <http://example.com> today.");
    }

    #[test]
    fn test_wrap_bare_urls_skips_already_angle_bracketed() {
        let text = "See <https://example.com> already wrapped.";
        assert_eq!(wrap_bare_urls(text), text);
    }

    #[test]
    fn test_wrap_bare_urls_skips_markdown_link_url() {
        let text = "See [docs](https://example.com/docs) for more.";
        assert_eq!(wrap_bare_urls(text), text);
    }

    #[test]
    fn test_wrap_bare_urls_multiple_bare_urls() {
        let text = "A: https://a.com B: https://b.com";
        assert_eq!(wrap_bare_urls(text), "A: <https://a.com> B: <https://b.com>");
    }

    #[test]
    fn test_wrap_bare_urls_mixed_bare_and_already_wrapped() {
        let text = "Visit <https://wrapped.com> or https://bare.com";
        assert_eq!(
            wrap_bare_urls(text),
            "Visit <https://wrapped.com> or <https://bare.com>"
        );
    }

    #[test]
    fn test_wrap_bare_urls_url_at_start_of_string() {
        let text = "https://example.com is the homepage.";
        assert_eq!(wrap_bare_urls(text), "<https://example.com> is the homepage.");
    }

    #[test]
    fn test_wrap_bare_urls_url_at_end_of_string() {
        let text = "Homepage: https://example.com";
        assert_eq!(wrap_bare_urls(text), "Homepage: <https://example.com>");
    }

    #[test]
    fn test_wrap_bare_urls_no_urls() {
        let text = "No links here, just prose.";
        assert_eq!(wrap_bare_urls(text), text);
    }

    #[test]
    fn test_wrap_bare_urls_empty_string() {
        assert_eq!(wrap_bare_urls(""), "");
    }

    // ---------------------------------------------------------------------------
    // generate_field_description — additional patterns
    // ---------------------------------------------------------------------------

    #[test]
    fn test_generate_field_description_count_suffix_already_plural() {
        // "errors" already ends with 's' — must not double-pluralise
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("errors_count", &ty), "Number of errors");
    }

    #[test]
    fn test_generate_field_description_count_suffix_singular_words() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("page_count", &ty), "Number of pages");
        assert_eq!(generate_field_description("word_count", &ty), "Number of words");
    }

    #[test]
    fn test_generate_field_description_is_prefix_multi_word() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("is_read_only", &ty), "Whether read only");
        assert_eq!(generate_field_description("is_active", &ty), "Whether active");
    }

    #[test]
    fn test_generate_field_description_has_prefix() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("has_metadata", &ty), "Whether metadata");
        assert_eq!(
            generate_field_description("has_ocr_support", &ty),
            "Whether ocr support"
        );
    }

    #[test]
    fn test_generate_field_description_at_suffix_falls_back_to_snake_readable() {
        // _at fields have no dedicated pattern — snake_to_readable is the fallback
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("created_at", &ty), "Created at");
        assert_eq!(generate_field_description("updated_at", &ty), "Updated at");
    }

    #[test]
    fn test_generate_field_description_max_compound_name() {
        let ty = TypeRef::String;
        // max_retries has no _count suffix, so the max_ prefix pattern fires
        assert_eq!(generate_field_description("max_retries", &ty), "Maximum retries");
        // max_size likewise
        assert_eq!(generate_field_description("max_size", &ty), "Maximum size");
    }

    #[test]
    fn test_generate_field_description_primitive_type_uses_name_fallback() {
        // Primitive types do not inject type-name context — falls to snake_to_readable
        assert_eq!(
            generate_field_description("confidence", &TypeRef::Primitive(PrimitiveType::F64)),
            "Confidence"
        );
    }

    // ---------------------------------------------------------------------------
    // generate_docs — integration tests beyond the empty-API case
    // ---------------------------------------------------------------------------

    fn make_test_config() -> AlefConfig {
        use alef_core::config::*;
        AlefConfig {
            crate_config: CrateConfig {
                name: "mylib".to_string(),
                sources: vec![],
                version_from: "Cargo.toml".to_string(),
                core_import: None,
                workspace_root: None,
                skip_core_import: false,
                features: vec![],
                path_mappings: std::collections::HashMap::new(),
                auto_path_mappings: Default::default(),
                extra_dependencies: Default::default(),
                source_crates: vec![],
                error_type: None,
                error_constructor: None,
            },
            languages: vec![Language::Python],
            exclude: ExcludeConfig::default(),
            include: IncludeConfig::default(),
            output: OutputConfig::default(),
            python: None,
            node: None,
            ruby: None,
            php: None,
            elixir: None,
            wasm: None,
            ffi: None,
            go: None,
            java: None,
            csharp: None,
            r: None,
            scaffold: None,
            readme: None,
            lint: None,
            test: None,
            custom_files: None,
            adapters: vec![],
            custom_modules: CustomModulesConfig::default(),
            custom_registrations: CustomRegistrationsConfig::default(),
            opaque_types: std::collections::HashMap::new(),
            generate: GenerateConfig::default(),
            generate_overrides: std::collections::HashMap::new(),
            dto: Default::default(),
            sync: None,
            e2e: None,
            trait_bridges: vec![],
        }
    }

    fn make_minimal_api(version: &str) -> ApiSurface {
        ApiSurface {
            crate_name: "mylib".to_string(),
            version: version.to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn test_generate_docs_produces_one_file_per_language_plus_three_shared() {
        let api = make_minimal_api("1.2.3");
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python, Language::Node], "out").unwrap();
        // 2 language files + configuration.md + types.md + errors.md
        assert_eq!(files.len(), 5);
        let paths: Vec<&str> = files.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert!(paths.iter().any(|p| p.contains("api-python")));
        assert!(paths.iter().any(|p| p.contains("api-typescript")));
        assert!(paths.iter().any(|p| p.contains("configuration")));
        assert!(paths.iter().any(|p| p.contains("types")));
        assert!(paths.iter().any(|p| p.contains("errors")));
    }

    #[test]
    fn test_generate_docs_all_output_files_end_with_newline() {
        let api = make_minimal_api("0.1.0");
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        for file in &files {
            assert!(
                file.content.ends_with('\n'),
                "file {:?} must end with trailing newline",
                file.path
            );
        }
    }

    #[test]
    fn test_generate_docs_output_dir_prefix_in_all_paths() {
        let api = make_minimal_api("0.1.0");
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "custom/output/dir").unwrap();
        for file in &files {
            assert!(
                file.path.to_str().unwrap().starts_with("custom/output/dir"),
                "all paths must be under output_dir: {:?}",
                file.path
            );
        }
    }

    #[test]
    fn test_generate_docs_with_function_renders_signature_and_params() {
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "convert_html".to_string(),
                rust_path: "mylib::convert_html".to_string(),
                original_rust_path: String::new(),
                params: vec![make_param("html", TypeRef::String, false)],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Converts HTML to plain text.".to_string(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("convert_html()"));
        assert!(lang_file.content.contains("Converts HTML to plain text."));
        assert!(lang_file.content.contains("**Signature:**"));
        assert!(lang_file.content.contains("**Parameters:**"));
    }

    #[test]
    fn test_generate_docs_with_enum_renders_python_screaming_case_variants() {
        use alef_core::ir::EnumVariant;
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![EnumDef {
                name: "OutputFormat".to_string(),
                rust_path: "mylib::OutputFormat".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Markdown".to_string(),
                        fields: vec![],
                        doc: "Markdown output.".to_string(),
                        is_default: true,
                        serde_rename: None,
                    },
                    EnumVariant {
                        name: "Plain".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                ],
                doc: "The output format.".to_string(),
                cfg: None,
                serde_tag: None,
                serde_rename_all: None,
            }],
            errors: vec![],
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("OutputFormat"));
        assert!(
            lang_file.content.contains("MARKDOWN"),
            "Python variant must be SCREAMING_SNAKE"
        );
        assert!(lang_file.content.contains("PLAIN"));
    }

    #[test]
    fn test_generate_docs_with_type_renders_fields_and_doc() {
        use alef_core::ir::CoreWrapper;
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "ConversionOptions".to_string(),
                rust_path: "mylib::ConversionOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "max_length".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: true,
                    default: None,
                    doc: "Maximum output length.".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                doc: "Options for the conversion.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("ConversionOptions"));
        assert!(lang_file.content.contains("max_length"));
        assert!(lang_file.content.contains("Maximum output length."));
    }

    #[test]
    fn test_generate_docs_with_error_appears_in_lang_page_and_errors_md() {
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![ErrorDef {
                name: "ConversionError".to_string(),
                rust_path: "mylib::ConversionError".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    alef_core::ir::ErrorVariant {
                        name: "InvalidInput".to_string(),
                        message_template: Some("Invalid input: {0}".to_string()),
                        fields: vec![],
                        has_source: false,
                        has_from: false,
                        is_unit: false,
                        doc: String::new(),
                    },
                    alef_core::ir::ErrorVariant {
                        name: "IoError".to_string(),
                        message_template: None,
                        fields: vec![],
                        has_source: false,
                        has_from: false,
                        is_unit: true,
                        doc: "An I/O error occurred.".to_string(),
                    },
                ],
                doc: "Errors from the conversion API.".to_string(),
            }],
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("ConversionError"));
        assert!(lang_file.content.contains("InvalidInput"));
        assert!(lang_file.content.contains("IoError"));

        let errors_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("errors"))
            .unwrap();
        assert!(errors_file.content.contains("ConversionError"));
        assert!(errors_file.content.contains("Invalid input: {0}"));
    }

    #[test]
    fn test_generate_docs_multiple_languages_produce_correct_slugs() {
        let api = make_minimal_api("0.1.0");
        let config = make_test_config();
        let langs = [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Java,
            Language::Ruby,
        ];
        let expected_slugs = ["api-python", "api-typescript", "api-go", "api-java", "api-ruby"];
        let files = generate_docs(&api, &config, &langs, "docs/api").unwrap();
        // 5 lang files + 3 shared
        assert_eq!(files.len(), 8);
        for slug in &expected_slugs {
            assert!(
                files.iter().any(|f| f.path.to_str().unwrap().contains(slug)),
                "expected file with slug {slug}"
            );
        }
    }

    #[test]
    fn test_generate_docs_post_processing_wraps_bare_urls() {
        // A bare URL in a function doc string must be angle-bracket-wrapped in output
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "fetch".to_string(),
                rust_path: "mylib::fetch".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Fetches from https://example.com directly.".to_string(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(
            lang_file.content.contains("<https://example.com>"),
            "bare URL must be wrapped by post-processing: {}",
            lang_file.content
        );
    }
}
