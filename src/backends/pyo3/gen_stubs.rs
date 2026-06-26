mod classes;
mod enums;
mod functions;
mod protocol;

use crate::backends::pyo3::gen_bindings::enums::sanitize_python_doc;
use crate::backends::pyo3::type_map::python_type;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};

type OptionsFieldBridges<'a> = std::collections::HashMap<&'a str, (&'a str, Option<&'a str>, Option<&'a str>)>;

use classes::{gen_opaque_type_stub, gen_type_stub};
use enums::gen_enum_stub;
use functions::gen_function_stub;
use protocol::gen_visitor_protocol_stub;

/// indented for inclusion inside a class body. Returns `None` when `doc` is
/// empty so callers can skip emission.
pub(super) fn pyi_docstring(doc: &str, indent: &str) -> Option<String> {
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
/// Delegates to the shared keyword list in `crate::core::keywords` so there is a single
/// source of truth for Python reserved words.  Use `resolve_field_name` on the config
/// when a per-field explicit rename is possible; this function handles the automatic
/// keyword-escape fallback for method names, enum variant names, etc.
pub(super) fn python_safe_name(name: &str) -> String {
    crate::core::keywords::python_ident(name)
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

/// Map a raw Rust type string from [`ClientConstructorConfig`] to its Python equivalent.
///
/// Constructor params come from `alef.toml` as raw Rust type strings (e.g. `"&str"`).
/// Falls back to `Any` for types that cannot be translated statically so the stub
/// remains valid Python even when exotic Rust types appear.
pub(super) fn constructor_rust_type_to_python(rust_type: &str) -> &str {
    match rust_type {
        "String" | "&str" | "&'static str" | "std::string::String" => "str",
        "bytes::Bytes" | "Vec<u8>" | "&[u8]" => "bytes",
        "bool" => "bool",
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => "int",
        "f32" | "f64" => "float",
        "()" => "None",
        _ => "Any",
    }
}

/// For constructor parameters, use the enum type name for enum fields.
/// The enum stub has `__init__(self, value: int | str)` so callers can pass
/// either a raw string/int or an enum instance.
/// Data enum fields accept a `dict`.
pub(super) fn constructor_param_type(ty: &TypeRef, api: &ApiSurface) -> String {
    use crate::codegen::generators::enum_has_data_variants;
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

pub fn gen_stubs(
    api: &ApiSurface,
    trait_bridges: &[TraitBridgeConfig],
    config: &ResolvedCrateConfig,
    exclude_functions: &ahash::AHashSet<String>,
) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut header_lines: Vec<String> = header.lines().map(str::to_string).collect();
    header_lines.push("".to_string());

    // Collect bridge param names so function stubs can emit `object | None` instead of
    // `str | None` for params that are sanitized trait bridge parameters.
    let bridge_param_names: std::collections::HashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // Build options-field-bridge lookup keyed by the options type name.
    // For each function whose params contain a value of one of these types, the PyO3
    // binding accepts an additional `{kwarg_name}: {trait_name} | None = None` kwarg
    // (e.g. `ConversionOptions` -> `visitor: HtmlVisitor | None = None`).
    //
    // The third tuple element is the trait name (e.g. `HtmlVisitor`). We emit a
    // generated `class HtmlVisitor(Protocol)` block below so user-facing call sites
    // type-check against the protocol the PyO3 bridge actually accepts at runtime
    // (any object implementing the visitor methods), not the binding-internal opaque
    // wrapper named by `type_alias` (e.g. `VisitorHandle`).
    //
    // `type_alias` is retained as a legacy fallback for bridges that have no trait
    // backing in the API surface — those continue to emit `VisitorHandle`-style names.
    let options_field_bridges: OptionsFieldBridges<'_> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let param_name = b.param_name.as_deref()?;
            let type_alias = b.type_alias.as_deref();
            let trait_name = if api.types.iter().any(|t| t.name == b.trait_name) {
                Some(b.trait_name.as_str())
            } else {
                None
            };
            Some((options_type, (param_name, type_alias, trait_name)))
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
        .filter(|typ| !typ.is_trait && !typ.binding_excluded)
        .partition(|typ| typ.is_opaque);

    let mut body_lines: Vec<String> = Vec::new();

    // Emit `class TraitName(Protocol):` for each trait bridge whose trait is resolvable in the
    // API surface. This surfaces the user-facing, host-implementable protocol the PyO3 bridge
    // expects, rather than exposing a binding-internal opaque wrapper to callers:
    //   - OptionsField/visitor bridges → the visitor protocol on the options struct.
    //   - Plugin bridges (those with a `register_fn`) → the protocol a host backend must
    //     implement to be registered via `register_*`. Method params that are known serde
    //     structs are typed as their native TypedDict/pyclass type and returns as the result
    //     type, matching the native objects the runtime bridge now passes/expects.
    //
    // Track the trait names that received a Protocol so the `register_*` signature below can type
    // its `backend` parameter against the Protocol instead of bare `object`.
    let mut protocol_trait_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for bridge in trait_bridges {
        let is_protocol_bridge =
            bridge.bind_via == crate::core::config::BridgeBinding::OptionsField || bridge.register_fn.is_some();
        if !is_protocol_bridge {
            continue;
        }
        if let Some(stub) = gen_visitor_protocol_stub(bridge, api, &capsule_names, emit_docstrings) {
            body_lines.push(stub);
            body_lines.push("".to_string());
            protocol_trait_names.insert(bridge.trait_name.clone());
        }
    }

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
            let ctor = config.client_constructors.get(&typ.name);
            body_lines.push(gen_opaque_type_stub(typ, &capsule_names, &streaming_return_types, ctor));
        }
        body_lines.push("".to_string());
    }

    // Generate enum stubs
    for enum_def in &api.enums {
        body_lines.push(gen_enum_stub(enum_def, emit_docstrings));
        body_lines.push("".to_string());
    }

    // Generate exception class stubs. The native module defines these via
    // `pyo3::create_exception!` (base error under `Exception`, each variant under the base) and
    // the generated `exceptions.py` re-exports them from this module. The stub must therefore
    // declare them, or mypy reports `_native` "has no attribute <Variant>Error" on the re-export
    // (tslp issue #147). Base is emitted before its variants so the variant base class resolves.
    {
        let mut seen_exc: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut exc_lines: Vec<String> = Vec::new();
        for error in &api.errors {
            if seen_exc.insert(error.name.clone()) {
                exc_lines.push(format!("class {}(Exception): ...", error.name));
            }
            for variant in &error.variants {
                let variant_name = crate::codegen::error_gen::python_exception_name(&variant.name, &error.name);
                if seen_exc.insert(variant_name.clone()) {
                    exc_lines.push(format!("class {variant_name}({}): ...", error.name));
                }
            }
        }
        if !exc_lines.is_empty() {
            body_lines.extend(exc_lines);
            body_lines.push("".to_string());
        }
    }

    // Generate function stubs — no blank lines between consecutive stubs (ruff strips them).
    // Skip functions excluded for this language backend: they are absent from the native
    // Rust module and must not appear in the .pyi type-stub either.
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        body_lines.push(gen_function_stub(
            func,
            &bridge_param_names,
            &capsule_names,
            &options_field_bridges,
            &streaming_return_types,
        ));
    }

    // Service entrypoints are registered as `service::{service_snake}_{method}`
    // pyfunctions by methods.rs at module-init time. Their stubs are not in
    // `api.functions` because they're emitted from `api.services`, so declare
    // them here so mypy can resolve the `_native.{name}` call sites that the
    // generated service.py wrapper produces.
    {
        use heck::ToSnakeCase as _;
        for service in &api.services {
            let service_snake = service.name.to_snake_case();
            for ep in &service.entrypoints {
                let func_name = format!("{service_snake}_{}", ep.method);
                let return_annot = match &ep.return_type {
                    TypeRef::Unit => "None".to_string(),
                    _ => "Any".to_string(),
                };
                body_lines.push(format!(
                    "def {func_name}(registrations: list[Any]) -> {return_annot}: ..."
                ));
            }
        }
    }
    for bridge in trait_bridges {
        if let Some(register_fn) = bridge.register_fn.as_deref() {
            // Type the `backend` param against the host-implementable Protocol when one was
            // emitted for this bridge's trait; otherwise fall back to `object`.
            let backend_type = if protocol_trait_names.contains(&bridge.trait_name) {
                bridge.trait_name.as_str()
            } else {
                "object"
            };
            body_lines.push(format!("def {register_fn}(backend: {backend_type}) -> None: ..."));
        }
        if let Some(unregister_fn) = bridge.unregister_fn.as_deref() {
            body_lines.push(format!("def {unregister_fn}(name: str) -> None: ..."));
        }
        if let Some(clear_fn) = bridge.clear_fn.as_deref() {
            body_lines.push(format!("def {clear_fn}() -> None: ..."));
        }
    }

    // Build the `from typing import …` line based on names actually referenced in the body,
    // so unused-import lint (F401) stays clean even when a particular API surface doesn't
    // need every helper.
    let body_joined = body_lines.join("\n");
    let used_typing: Vec<&str> = ["Any", "AsyncIterator", "Literal", "Protocol", "TypeAlias", "TypedDict"]
        .iter()
        .copied()
        .filter(|name| contains_word(&body_joined, name))
        .collect();
    let mut lines = header_lines;
    // A data-enum factory whose name shadows a builtin container forces its annotations to be
    // written as `builtins.<name>[...]` (see `gen_data_enum_variant_constructor_stubs`), which needs
    // an explicit `import builtins`. Emit it only when actually referenced so F401 stays clean.
    if contains_word(&body_joined, "builtins") {
        lines.push("import builtins".to_string());
        lines.push("".to_string());
    }
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
/// Handles bare names (`Language` -> `Any`), optional forms (`Language | None` -> `Any`),
/// and list forms (`list[Language]` -> `list[Any]`). Matching is whole-word to avoid
/// touching unrelated identifiers that share a prefix.
pub(super) fn substitute_capsule_type(type_str: &str, capsule_names: &std::collections::HashSet<&str>) -> String {
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
