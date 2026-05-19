use crate::gen_bindings::enums::sanitize_python_doc;
use crate::type_map::python_type;
use alef_codegen::shared::binding_fields;
use alef_core::config::{AdapterPattern, Language, ResolvedCrateConfig, TraitBridgeConfig};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, EnumDef, FunctionDef, MethodDef, TypeDef, TypeRef};

/// Format a Rust doc string as a single-line Python `"""…"""` docstring,
/// indented for inclusion inside a class body. Returns `None` when `doc` is
/// empty so callers can skip emission.
fn pyi_docstring(doc: &str, indent: &str) -> Option<String> {
    let trimmed = doc.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Take the first paragraph (split on blank line) and join multi-line docs
    // with spaces so the stub stays a single line — easy to read in IDE hovers
    // and avoids escaping subtleties.
    let first_paragraph = trimmed.split("\n\n").next().unwrap_or(trimmed);
    let joined: String = first_paragraph
        .lines()
        .map(|l| l.trim().trim_start_matches("///").trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        return None;
    }
    let sanitized = sanitize_python_doc(&joined);
    // Escape embedded triple-double-quote sequences and backslashes that would
    // break the docstring boundary.
    let escaped = sanitized.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
    Some(format!("{indent}\"\"\"{escaped}\"\"\""))
}

/// Convert an identifier to a Python-safe name by escaping reserved keywords.
///
/// Delegates to the shared keyword list in `alef_core::keywords` so there is a single
/// source of truth for Python reserved words.  Use `resolve_field_name` on the config
/// when a per-field explicit rename is possible; this function handles the automatic
/// keyword-escape fallback for method names, enum variant names, etc.
fn python_safe_name(name: &str) -> String {
    alef_core::keywords::python_ident(name)
}

/// Check if a parameter name shadows a Python builtin (triggers ruff A002).
pub fn is_python_builtin_name(name: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "id",
        "type",
        "input",
        "hash",
        "format",
        "dir",
        "help",
        "list",
        "map",
        "filter",
        "range",
        "set",
        "dict",
        "str",
        "int",
        "float",
        "bool",
        "bytes",
        "tuple",
        "len",
        "max",
        "min",
        "sum",
        "abs",
        "all",
        "any",
        "print",
        "open",
        "next",
        "iter",
        "vars",
        "zip",
        "object",
        "property",
        "super",
        "staticmethod",
        "classmethod",
        "compile",
        "exec",
        "eval",
        "license",
        "credits",
        "copyright",
    ];
    BUILTINS.contains(&name)
}

/// For constructor parameters, use the enum type name for enum fields.
/// The enum stub has `__init__(self, value: int | str)` so callers can pass
/// either a raw string/int or an enum instance.
/// Data enum fields accept a `dict`.
fn constructor_param_type(ty: &TypeRef, api: &ApiSurface) -> String {
    use alef_codegen::generators::enum_has_data_variants;
    let enum_names: std::collections::HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let data_enum_names: std::collections::HashSet<String> = api
        .enums
        .iter()
        .filter(|e| enum_has_data_variants(e))
        .map(|e| e.name.clone())
        .collect();

    match ty {
        TypeRef::Named(name) if data_enum_names.contains(name) => name.clone(),
        TypeRef::Named(name) if enum_names.contains(name) => format!("{} | str", name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if data_enum_names.contains(name) => {
                format!("{} | None", name)
            }
            TypeRef::Named(name) if enum_names.contains(name) => format!("{} | str | None", name),
            _ => python_type(ty),
        },
        _ => python_type(ty),
    }
}

pub fn gen_stubs(api: &ApiSurface, trait_bridges: &[TraitBridgeConfig], config: &ResolvedCrateConfig) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut header_lines: Vec<String> = header.lines().map(str::to_string).collect();
    header_lines.push("".to_string());

    // Collect bridge param names so function stubs can emit `object | None` instead of
    // `str | None` for params that are sanitized trait bridge parameters.
    let bridge_param_names: std::collections::HashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // Build options-field-bridge lookup keyed by the options type name.
    // For each function whose params contain a value of one of these types, the PyO3
    // binding accepts an additional `{kwarg_name}: {type_alias} | None = None` kwarg
    // (e.g. ConversionOptions → `visitor: VisitorHandle | None = None`).
    // The stub `__init__` of the options type itself, and any module-level function
    // taking that type, must surface this kwarg so api.py's wrapper type-checks.
    let options_field_bridges: std::collections::HashMap<&str, (&str, Option<&str>)> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let param_name = b.param_name.as_deref()?;
            let type_alias = b.type_alias.as_deref();
            Some((options_type, (param_name, type_alias)))
        })
        .collect();

    // Build a streaming-adapter lookup: (owner_type, method_name) → item_type.
    // Used to override return types in method/function stubs with AsyncIterator[ItemType].
    // Key is (Option<owner_type>, adapter_name); value is the Python item type string.
    // When owner_type is None the adapter is a free function; otherwise it is a method.
    let streaming_return_types: std::collections::HashMap<(Option<String>, String), String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .map(|a| {
            let item = a.item_type.as_deref().unwrap_or("Any").to_string();
            ((a.owner_type.clone(), a.name.clone()), item)
        })
        .collect();

    // Collect capsule type names so we can skip their opaque class stubs and replace
    // any return/param type references with `Any` (they live in third-party packages).
    let capsule_names: std::collections::HashSet<&str> = config
        .python
        .as_ref()
        .map(|p| p.capsule_types.keys().map(String::as_str).collect())
        .unwrap_or_default();

    // Gate docstring emission behind config — ruff PYI021 flags docstrings in stub files.
    let emit_docstrings = config
        .python
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .is_some_and(|s| s.emit_docstrings);

    // Generate type stubs — collect opaque types separately so consecutive
    // one-liner class stubs are emitted without blank lines between them
    // (ruff strips those in .pyi files).
    let (opaque, non_opaque): (Vec<_>, Vec<_>) = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait)
        .partition(|typ| typ.is_opaque);

    let mut body_lines: Vec<String> = Vec::new();
    for typ in &non_opaque {
        body_lines.push(gen_type_stub(
            typ,
            api,
            config,
            &capsule_names,
            &options_field_bridges,
            emit_docstrings,
            &streaming_return_types,
        ));
        body_lines.push("".to_string());
    }

    // Opaque stubs: skip any type whose name is a capsule type (it lives in a third-party
    // package and must not be redeclared here).
    let opaque_non_capsule: Vec<_> = opaque
        .iter()
        .filter(|typ| !capsule_names.contains(typ.name.as_str()))
        .collect();
    if !opaque_non_capsule.is_empty() {
        for typ in &opaque_non_capsule {
            body_lines.push(gen_opaque_type_stub(typ, &capsule_names, &streaming_return_types));
        }
        body_lines.push("".to_string());
    }

    // Generate enum stubs
    for enum_def in &api.enums {
        body_lines.push(gen_enum_stub(enum_def, emit_docstrings));
        body_lines.push("".to_string());
    }

    // Generate function stubs — no blank lines between consecutive stubs (ruff strips them)
    for func in &api.functions {
        body_lines.push(gen_function_stub(
            func,
            &bridge_param_names,
            &capsule_names,
            &options_field_bridges,
            &streaming_return_types,
        ));
    }

    // Build the `from typing import …` line based on names actually referenced in the body,
    // so unused-import lint (F401) stays clean even when a particular API surface doesn't
    // need every helper.
    let body_joined = body_lines.join("\n");
    let used_typing: Vec<&str> = ["Any", "AsyncIterator", "Literal", "TypeAlias", "TypedDict"]
        .iter()
        .copied()
        .filter(|name| contains_word(&body_joined, name))
        .collect();
    let mut lines = header_lines;
    if !used_typing.is_empty() {
        lines.push(format!("from typing import {}", used_typing.join(", ")));
        lines.push("".to_string());
    }
    lines.extend(body_lines);

    lines.join("\n")
}

/// Return true when `text` contains `word` as a standalone identifier (not as a substring of
/// another identifier). Used to decide whether a `from typing import X` is actually referenced
/// by the generated stub body.
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = word.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = 0;
    while let Some(idx) = text[start..].find(word) {
        let pos = start + idx;
        let before_ok = pos == 0 || !is_ident(bytes[pos - 1]);
        let after_pos = pos + needle.len();
        let after_ok = after_pos == bytes.len() || !is_ident(bytes[after_pos]);
        if before_ok && after_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

/// Replace any standalone capsule type name in a Python type annotation string with `Any`.
///
/// Handles bare names (`Language` → `Any`), optional forms (`Language | None` → `Any`),
/// and list forms (`list[Language]` → `list[Any]`).  Matching is whole-word to avoid
/// touching unrelated identifiers that share a prefix.
fn substitute_capsule_type(type_str: &str, capsule_names: &std::collections::HashSet<&str>) -> String {
    let mut result = type_str.to_string();
    for name in capsule_names {
        // Replace `list[Name]` → `list[Any]`
        let list_pattern = format!("list[{name}]");
        if result.contains(&list_pattern) {
            result = result.replace(&list_pattern, "list[Any]");
            continue;
        }
        // Replace `Name | None` → `Any` (Any already subsumes None)
        let optional_pattern = format!("{name} | None");
        if result.contains(&optional_pattern) {
            result = result.replace(&optional_pattern, "Any");
            continue;
        }
        // Replace bare `Name` (whole-word) → `Any`
        let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let needle = name.as_bytes();
        let bytes = result.as_bytes();
        let mut out = String::new();
        let mut start = 0usize;
        while let Some(idx) = result[start..].find(name) {
            let pos = start + idx;
            let before_ok = pos == 0 || !is_ident(bytes[pos - 1]);
            let after_pos = pos + needle.len();
            let after_ok = after_pos == bytes.len() || !is_ident(bytes[after_pos]);
            if before_ok && after_ok {
                out.push_str(&result[start..pos]);
                out.push_str("Any");
                start = after_pos;
            } else {
                out.push_str(&result[start..=pos]);
                start = pos + 1;
            }
        }
        out.push_str(&result[start..]);
        result = out;
    }
    result
}

/// Generate a Python type stub for an opaque type (no fields, only methods).
fn gen_opaque_type_stub(
    typ: &TypeDef,
    capsule_names: &std::collections::HashSet<&str>,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("class {}:", typ.name));

    // Instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(
                method,
                false,
                capsule_names,
                Some(&typ.name),
                streaming_return_types,
            ));
        }
    }

    // Static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(
                method,
                true,
                capsule_names,
                Some(&typ.name),
                streaming_return_types,
            ));
        }
    }

    // If no methods at all, emit as a one-liner (ruff collapses `class Foo:\n    ...` to `class Foo: ...`)
    if typ.methods.is_empty() {
        return format!("class {}: ...", typ.name);
    }

    lines.join("\n")
}

/// Generate a Python type stub for a struct.
fn gen_type_stub(
    typ: &TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    capsule_names: &std::collections::HashSet<&str>,
    options_field_bridges: &std::collections::HashMap<&str, (&str, Option<&str>)>,
    emit_docstrings: bool,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("class {}:", typ.name));

    // Class-level docstring from Rust doc comment — gated behind emit_docstrings (ruff PYI021).
    if emit_docstrings {
        if let Some(docstring) = pyi_docstring(&typ.doc, "    ") {
            lines.push(docstring);
        }
    }

    // Add field type annotations.
    // Field names that are Python reserved keywords are shown with their escaped name
    // (e.g. `class_`) because that is the attribute name callers must use in Python.
    // The underlying `#[pyo3(get, name = "class")]` attribute on the Rust struct exposes
    // it as `obj.class_` (the escaped name), NOT as `obj.class`, because `class` is a
    // syntax error in a Python attribute access expression.  The stub must match.
    for field in binding_fields(&typ.fields) {
        let type_str = python_type(&field.ty);
        // Duration fields on has_default types are Option<u64> in PyO3, so annotate as int | None
        let is_optional_duration = typ.has_default && matches!(field.ty, TypeRef::Duration) && !field.optional;
        let field_type = if (is_optional_duration || field.optional) && !type_str.contains("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        // Resolve the field name: use config-driven rename if available, otherwise apply
        // automatic keyword escaping via python_safe_name.
        let stub_field_name = config
            .resolve_field_name(Language::Python, &typ.name, &field.name)
            .unwrap_or_else(|| field.name.clone());
        lines.push(format!("    {stub_field_name}: {field_type}"));
        // Field-level docstring follows the type annotation (PEP-style) — gated behind emit_docstrings.
        if emit_docstrings {
            if let Some(docstring) = pyi_docstring(&field.doc, "    ") {
                lines.push(docstring);
            }
        }
    }

    // Add __init__ signature
    lines.push(gen_type_init_stub(typ, api, config, options_field_bridges));

    // Add instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(
                method,
                false,
                capsule_names,
                Some(&typ.name),
                streaming_return_types,
            ));
        }
    }

    // Add static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(
                method,
                true,
                capsule_names,
                Some(&typ.name),
                streaming_return_types,
            ));
        }
    }

    lines.join("\n")
}

/// Generate __init__ signature stub for a struct.
fn gen_type_init_stub(
    typ: &TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    options_field_bridges: &std::collections::HashMap<&str, (&str, Option<&str>)>,
) -> String {
    // Partition fields into required (non-optional) and optional.
    //
    // When `typ.has_default` is true, the Rust binding uses
    // `config_constructor_parts_with_options` which wraps ALL fields in `Option<T>` with
    // `=None` defaults in the `#[pyo3(signature = (...))]` macro.  The `.pyi` stub must
    // match this, so every field is treated as optional.
    //
    // For non-has_default types, only fields explicitly marked `optional` (or Duration
    // fields on has_default types) go into the optional partition.
    let (required, optional): (Vec<_>, Vec<_>) =
        binding_fields(&typ.fields).filter(|f| f.cfg.is_none()).partition(|f| {
            if typ.has_default {
                // All fields are optional in the Rust signature — nothing is required.
                return false;
            }
            let is_optional_duration = matches!(f.ty, TypeRef::Duration) && !f.optional;
            !f.optional && !is_optional_duration
        });

    // Generate required params first, then optional params.
    // For constructor params, use str instead of enum types (PyO3 accepts any string).
    // Field names that are Python reserved keywords are emitted with their escaped name
    // (e.g. `class_`) so the generated `__init__` signature is valid Python syntax.
    let mut params: Vec<String> = required
        .iter()
        .map(|f| {
            let param_type = constructor_param_type(&f.ty, api);
            let param_name = config
                .resolve_field_name(Language::Python, &typ.name, &f.name)
                .unwrap_or_else(|| f.name.clone());
            format!("{param_name}: {param_type}")
        })
        .collect();

    params.extend(optional.iter().map(|f| {
        let type_str = constructor_param_type(&f.ty, api);
        let param_type = if !type_str.ends_with("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        let param_name = config
            .resolve_field_name(Language::Python, &typ.name, &f.name)
            .unwrap_or_else(|| f.name.clone());
        format!("{param_name}: {param_type} = None")
    }));

    // When this struct is the options-type of a trait bridge with `bind_via=OptionsField`,
    // the PyO3 `#[new]` constructor accepts an additional `{kwarg_name}: {type_alias} = None`
    // kwarg (e.g. `visitor: VisitorHandle | None = None`). The bridge field is cfg-gated in
    // the IR, so the partition above strips it, but the PyO3 macro keeps it via
    // `never_skip_cfg_field_names`. Surface it here so api.py callers type-check.
    if let Some((kwarg_name, type_alias)) = options_field_bridges.get(typ.name.as_str()) {
        let visitor_type = type_alias.unwrap_or("object");
        params.push(format!("{kwarg_name}: {visitor_type} | None = None"));
    }

    // If any parameter shadows a Python builtin we must use the multi-line form so we can
    // append `# noqa: A002` on those lines. The noqa suppression is not valid on a single-line
    // def, so force wrapping whenever a builtin-shadowing param is present.
    let has_builtin_param = params
        .iter()
        .any(|p| is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));
    let single_line = format!("    def __init__(self, {}) -> None: ...", params.join(", "));
    if single_line.len() <= 100 && !has_builtin_param {
        single_line
    } else {
        // Wrap parameters across multiple lines to stay within 100 chars.
        // For params that shadow Python builtins, append `# noqa: A002` AFTER the comma.
        let mut wrapped = String::from("    def __init__(\n");
        wrapped.push_str("        self,\n");
        for param in &params {
            let name = param.split(':').next().unwrap_or("").trim();
            if is_python_builtin_name(name) {
                wrapped.push_str(&crate::template_env::render(
                    "stub_param_wrapped_noqa.jinja",
                    minijinja::context! { param => param, indent => "        " },
                ));
            } else {
                wrapped.push_str(&crate::template_env::render(
                    "stub_param_wrapped.jinja",
                    minijinja::context! { param => param, indent => "        " },
                ));
            }
        }
        wrapped.push_str("    ) -> None: ...");
        wrapped
    }
}

/// Generate a method stub.
fn gen_method_stub(
    method: &MethodDef,
    is_static: bool,
    capsule_names: &std::collections::HashSet<&str>,
    owner_type: Option<&str>,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
) -> String {
    // Partition params into required (non-optional) and optional
    let (required, optional): (Vec<_>, Vec<_>) = method.params.iter().partition(|p| !p.optional);

    // Generate required params first, then optional params
    let mut params: Vec<String> = required
        .iter()
        .map(|p| {
            let param_type = substitute_capsule_type(&python_type(&p.ty), capsule_names);
            format!("{}: {}", p.name, param_type)
        })
        .collect();

    params.extend(optional.iter().map(|p| {
        let type_str = substitute_capsule_type(&python_type(&p.ty), capsule_names);
        let param_type = if !type_str.ends_with("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        format!("{}: {} = None", p.name, param_type)
    }));

    // Check whether this method has a streaming adapter. When it does, override the
    // return type with `AsyncIterator[ItemType]` so the stub matches the real async
    // iterator emitted by the Rust shim rather than the buffered placeholder type.
    let streaming_key = (owner_type.map(str::to_string), method.name.clone());
    let return_type = if let Some(item_type) = streaming_return_types.get(&streaming_key) {
        format!("AsyncIterator[{item_type}]")
    } else {
        substitute_capsule_type(&python_type(&method.return_type), capsule_names)
    };
    let indent = "    ";
    let safe_name = python_safe_name(&method.name);
    // pyo3 async methods return a Python awaitable (via `pyo3_async_runtimes::*::future_into_py`).
    // Emit `async def` in the .pyi stub so the `await _rust.method(...)` calls in the generated
    // `api.py` wrapper type-check correctly.
    let def_kw = if method.is_async { "async def" } else { "def" };

    // Force multi-line wrapping whenever a param shadows a Python builtin so we can
    // append `# noqa: A002` on those lines (the suppression is invalid on a single-line def).
    let has_builtin_param = params
        .iter()
        .any(|p| is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));

    let emit_params_wrapped = |prefix: &str, suffix: &str| -> String {
        let mut wrapped = format!("{prefix}\n");
        for param in &params {
            let name = param.split(':').next().unwrap_or("").trim();
            if is_python_builtin_name(name) {
                wrapped.push_str(&crate::template_env::render(
                    "stub_param_method_wrapped_noqa.jinja",
                    minijinja::context! { indent => indent, param => param },
                ));
            } else {
                wrapped.push_str(&crate::template_env::render(
                    "stub_param_method_wrapped.jinja",
                    minijinja::context! { indent => indent, param => param },
                ));
            }
        }
        wrapped.push_str(suffix);
        wrapped
    };

    if is_static {
        if params.is_empty() {
            format!(
                "{}@staticmethod\n{}{} {}() -> {}: ...",
                indent, indent, def_kw, safe_name, return_type
            )
        } else {
            let prefix = format!("{}@staticmethod\n{}{} {}(", indent, indent, def_kw, safe_name);
            let suffix = format!("{}) -> {}: ...", indent, return_type);
            // Check the def line (second line) for length
            let def_line = format!(
                "{}{} {}({}) -> {}: ...",
                indent,
                def_kw,
                safe_name,
                params.join(", "),
                return_type
            );
            if def_line.len() <= 100 && !has_builtin_param {
                format!(
                    "{}@staticmethod\n{}{} {}({}) -> {}: ...",
                    indent,
                    indent,
                    def_kw,
                    safe_name,
                    params.join(", "),
                    return_type
                )
            } else {
                emit_params_wrapped(&prefix, &suffix)
            }
        }
    } else if params.is_empty() {
        format!("{}{} {}(self) -> {}: ...", indent, def_kw, safe_name, return_type)
    } else {
        let single_line = format!(
            "{}{} {}(self, {}) -> {}: ...",
            indent,
            def_kw,
            safe_name,
            params.join(", "),
            return_type
        );
        if single_line.len() <= 100 && !has_builtin_param {
            single_line
        } else {
            let prefix = format!("{}{} {}(\n{}    self,", indent, def_kw, safe_name, indent);
            let suffix = format!("{}) -> {}: ...", indent, return_type);
            emit_params_wrapped(&prefix, &suffix)
        }
    }
}

/// Convert a Rust PascalCase variant name to `UPPER_SNAKE_CASE` for Python enum stubs.
///
/// Mirrors the logic in `alef-codegen::generators::enums::to_pyo3_screaming` so that
/// `.pyi` stub attribute names match the `#[pyo3(name = "...")]` rename emitted on the
/// Rust pyclass variant.  Handles leading-acronym names (e.g. `RDFa` → `RDFA`).
fn to_python_screaming(name: &str) -> String {
    use heck::ToShoutySnakeCase;
    let chars: Vec<char> = name.chars().collect();
    let upper_prefix_len = chars.iter().take_while(|c| c.is_uppercase()).count();
    if upper_prefix_len >= 2 && chars[upper_prefix_len..].iter().all(|c| c.is_lowercase() || *c == '_') {
        name.to_ascii_uppercase()
    } else {
        name.to_shouty_snake_case()
    }
}

/// Generate a Python enum stub.
fn gen_enum_stub(enum_def: &EnumDef, emit_docstrings: bool) -> String {
    use alef_codegen::generators::enum_has_data_variants;
    let mut lines = vec![];

    if enum_has_data_variants(enum_def) {
        // Data enums: emit a TypedDict per variant and a Union type alias.
        gen_data_enum_typeddicts(&mut lines, enum_def);
    } else {
        lines.push(format!("class {}:", enum_def.name));
        // Enum-level docstring — gated behind emit_docstrings (ruff PYI021).
        if emit_docstrings {
            if let Some(docstring) = pyi_docstring(&enum_def.doc, "    ") {
                lines.push(docstring);
            }
        }
        for variant in &enum_def.variants {
            // Emit UPPER_SNAKE_CASE attribute names to match the #[pyo3(name = "...")] rename
            // on the Rust pyclass variant (PEP 8: enum members are UPPER_SNAKE_CASE).
            lines.push(format!(
                "    {}: {} = ...",
                to_python_screaming(&variant.name),
                enum_def.name
            ));
            // Variant-level docstring — gated behind emit_docstrings (ruff PYI021).
            if emit_docstrings {
                if let Some(docstring) = pyi_docstring(&variant.doc, "    ") {
                    lines.push(docstring);
                }
            }
        }
        lines.push("    def __init__(self, value: int | str) -> None: ...".to_string());
    }

    lines.join("\n")
}

/// Generate TypedDicts for each variant of a data enum, plus a Union type alias.
fn gen_data_enum_typeddicts(lines: &mut Vec<String>, enum_def: &EnumDef) {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let rename_all = enum_def.serde_rename_all.as_deref();

    let mut variant_class_names = vec![];

    for variant in &enum_def.variants {
        let class_name = format!("{}{}Variant", enum_def.name, variant.name);
        variant_class_names.push(class_name.clone());

        // Compute the tag value (what appears in JSON)
        let tag_value = if let Some(rename) = &variant.serde_rename {
            rename.clone()
        } else {
            apply_rename_all(&variant.name, rename_all)
        };

        lines.push(format!("class {}(TypedDict):", class_name));

        // Tag field with Literal type
        lines.push(format!("    {}: Literal[\"{}\"]", tag_field, tag_value));

        // Data fields
        for field in &variant.fields {
            let field_type = python_type(&field.ty);
            let field_type = if field.optional && !field_type.contains("| None") {
                format!("{} | None", field_type)
            } else {
                field_type
            };
            lines.push(format!("    {}: {}", python_safe_name(&field.name), field_type));
        }

        // If no data fields, the TypedDict only has the tag
        if variant.fields.is_empty() && lines.last().is_some_and(|l| l.ends_with("):")) {
            // Empty body — TypedDict needs at least the tag field which is already added
        }

        lines.push("".to_string());
    }

    // Emit a class stub for the opaque pyo3 wrapper.
    // The wrapper exposes the serde tag as a readable attribute and delegates __str__
    // to the inner serde serialization. Variant TypedDicts above are kept for documentation.
    lines.push(format!("class {}:", enum_def.name));
    lines.push(format!("    {}: str", tag_field));
    // PYI029: __str__/__repr__ stubs are needed because the pyo3 wrapper implements them
    // via Display/Debug, and downstream callers rely on str(value) returning the serde tag.
    lines.push("    def __str__(self) -> str: ...  # noqa: PYI029".to_string());
    lines.push("    def __repr__(self) -> str: ...  # noqa: PYI029".to_string());
}

/// Apply serde rename_all strategy to a variant name.
fn apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") => {
            // PascalCase → snake_case
            let mut result = String::new();
            for (i, ch) in name.chars().enumerate() {
                if ch.is_uppercase() && i > 0 {
                    result.push('_');
                }
                result.push(ch.to_lowercase().next().unwrap_or(ch));
            }
            result
        }
        Some("camelCase") => {
            let mut chars = name.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
            }
        }
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        _ => name.to_string(), // No renaming or unknown strategy
    }
}

/// Generate a function stub.
///
/// `bridge_param_names` is the set of parameter names that are trait bridge params.
/// These are emitted as `object | None` instead of `str | None` because the IR
/// sanitizes their type to `String` (unable to represent `Rc<RefCell<dyn Trait>>`),
/// but callers pass arbitrary Python objects implementing the visitor protocol.
///
/// `capsule_names` is the set of type names that are capsule types (live in third-party
/// packages). Any occurrence of these names in param/return annotations is replaced with
/// `Any` so the stub stays free of third-party imports and mypy follows api.py's
/// more-precise annotations.
fn gen_function_stub(
    func: &FunctionDef,
    bridge_param_names: &std::collections::HashSet<&str>,
    capsule_names: &std::collections::HashSet<&str>,
    options_field_bridges: &std::collections::HashMap<&str, (&str, Option<&str>)>,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
) -> String {
    // Partition params into required (non-optional) and optional
    let (required, optional): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !p.optional);

    // Generate required params first, then optional params
    let mut params: Vec<String> = required
        .iter()
        .map(|p| {
            let param_type = if bridge_param_names.contains(p.name.as_str()) {
                "object".to_string()
            } else {
                substitute_capsule_type(&python_type(&p.ty), capsule_names)
            };
            format!("{}: {}", p.name, param_type)
        })
        .collect();

    params.extend(optional.iter().map(|p| {
        let type_str = if bridge_param_names.contains(p.name.as_str()) {
            "object".to_string()
        } else {
            substitute_capsule_type(&python_type(&p.ty), capsule_names)
        };
        let param_type = if !type_str.ends_with("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        format!("{}: {} = None", p.name, param_type)
    }));

    // If any param's type is the options-type of an OptionsField trait bridge, the PyO3
    // wrapper exposes an additional `{kwarg_name}: {type_alias} | None = None` kwarg.
    // Surface it here so api.py callers type-check (the visitor field is cfg-gated and so
    // does not appear directly on the IR struct, but the binding accepts it as a kwarg).
    let bridge_kwarg = func.params.iter().find_map(|p| {
        let type_name = match &p.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        }?;
        let (kwarg_name, type_alias) = options_field_bridges.get(type_name)?;
        Some((*kwarg_name, *type_alias))
    });
    if let Some((kwarg_name, type_alias)) = bridge_kwarg {
        let visitor_type = type_alias.unwrap_or("object");
        params.push(format!("{kwarg_name}: {visitor_type} | None = None"));
    }

    // Check whether this function has a streaming adapter (free-function form: owner_type == None).
    // When it does, override the return type with `AsyncIterator[ItemType]` so the stub matches
    // the real async iterator emitted by the Rust shim rather than the buffered placeholder type.
    let streaming_key = (None::<String>, func.name.clone());
    let return_type = if let Some(item_type) = streaming_return_types.get(&streaming_key) {
        format!("AsyncIterator[{item_type}]")
    } else {
        substitute_capsule_type(&python_type(&func.return_type), capsule_names)
    };
    let safe_name = python_safe_name(&func.name);
    // pyo3 async functions return a Python awaitable (via `pyo3_async_runtimes::*::future_into_py`),
    // not the bare value. The .pyi stub must reflect that with `async def` so callers using the
    // generated `api.py` wrapper (which `await`s the underlying pyo3 call) type-check correctly.
    let def_kw = if func.is_async { "async def" } else { "def" };

    let has_builtin_param = params
        .iter()
        .any(|p| is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));
    let single_line = format!(
        "{} {}({}) -> {}: ...",
        def_kw,
        safe_name,
        params.join(", "),
        return_type
    );
    if single_line.len() <= 100 && !has_builtin_param {
        single_line
    } else {
        let mut wrapped = format!("{} {}(\n", def_kw, safe_name);
        for param in &params {
            let name = param.split(':').next().unwrap_or("").trim();
            if is_python_builtin_name(name) {
                wrapped.push_str(&crate::template_env::render(
                    "stub_param_wrapped_noqa.jinja",
                    minijinja::context! { param => param, indent => "    " },
                ));
            } else {
                wrapped.push_str(&crate::template_env::render(
                    "stub_param_wrapped.jinja",
                    minijinja::context! { param => param, indent => "    " },
                ));
            }
        }
        wrapped.push_str(&crate::template_env::render(
            "stub_method_signature_end.jinja",
            minijinja::context! { return_type => &return_type },
        ));
        wrapped
    }
}
