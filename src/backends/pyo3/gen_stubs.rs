mod classes;
mod enums;
mod functions;
mod protocol;

use crate::backends::pyo3::gen_bindings::enums::sanitize_python_doc;
use crate::backends::pyo3::type_map::python_type;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, MethodDef, ParamDef, TypeDef, TypeRef};

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

/// Return `typ` unchanged, or a clone carrying a synthetic `from_json` static method entry when
/// `gen_bindings::mod` would inject that staticmethod into the compiled `#[pymethods]` block
/// (see [`crate::backends::pyo3::gen_bindings::type_has_from_json`]). Routing through a real
/// `MethodDef` lets `gen_method_stub` render the declaration the same way it renders every other
/// staticmethod, instead of a hand-written stub line that could drift from that format. ~keep
fn with_from_json_stub<'a>(typ: &'a TypeDef, api: &ApiSurface, has_serde: bool) -> std::borrow::Cow<'a, TypeDef> {
    if !crate::backends::pyo3::gen_bindings::type_has_from_json(typ, api, has_serde) {
        return std::borrow::Cow::Borrowed(typ);
    }
    let mut augmented = typ.clone();
    augmented.methods.push(MethodDef {
        name: "from_json".to_string(),
        params: vec![ParamDef {
            name: "json_str".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        return_type: TypeRef::Named(typ.name.clone()),
        is_static: true,
        ..Default::default()
    });
    std::borrow::Cow::Owned(augmented)
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

    let bridge_param_names: std::collections::HashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // Mirrors the opaque-type set `gen_bindings/mod.rs` computes for the same purpose: the write-
    // back rewrite (`mut_writeback`) must never fire for an opaque type, whose host handle is
    // already a live reference. ~keep
    let opaque_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

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

    let streaming_return_types: std::collections::HashMap<(Option<String>, String), String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .map(|a| {
            let item = a.item_type.as_deref().unwrap_or("Any").to_string();
            ((a.owner_type.clone(), a.name.clone()), item)
        })
        .collect();

    let capsule_names: std::collections::HashSet<&str> = config
        .python
        .as_ref()
        .map(|p| p.capsule_types.keys().map(String::as_str).collect())
        .unwrap_or_default();

    let emit_docstrings = config
        .python
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .is_some_and(|s| s.emit_docstrings);

    let pyclass_absent_types = crate::backends::pyo3::gen_bindings::binding_exclusions::pyclass_absent_type_names(
        config,
        &api.types,
        &api.errors,
    );
    let (opaque, non_opaque): (Vec<_>, Vec<_>) = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !pyclass_absent_types.contains(&typ.name))
        .partition(|typ| typ.is_opaque);

    let has_serde = crate::backends::pyo3::gen_bindings::crate_has_serde(config);

    let mut body_lines: Vec<String> = Vec::new();

    let mut protocol_trait_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let stub_reexported_types = config
        .python
        .as_ref()
        .map(|c| c.reexported_types.clone())
        .unwrap_or_default();
    let options_types = crate::backends::pyo3::gen_bindings::options_dataclass_type_names(api, &stub_reexported_types);
    // Precomputed once for this stub-generation pass rather than inside `context_binding_class`
    // per bridge -- the fixpoint is transitive over every type and field in `api`, so a per-bridge
    // recompute scales as bridges x fixpoint instead of once. ~keep
    let core_to_binding_convertible_types = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
    for bridge in trait_bridges {
        if crate::backends::pyo3::trait_bridge::active_bridge_trait(bridge, api).is_none() {
            continue;
        }
        let is_protocol_bridge =
            bridge.bind_via == crate::core::config::BridgeBinding::OptionsField || bridge.register_fn.is_some();
        if !is_protocol_bridge {
            continue;
        }
        if let Some(stub) = gen_visitor_protocol_stub(
            bridge,
            api,
            &capsule_names,
            emit_docstrings,
            &options_types,
            &pyclass_absent_types,
            &core_to_binding_convertible_types,
        ) {
            body_lines.push(stub);
            body_lines.push("".to_string());
            protocol_trait_names.insert(bridge.trait_name.clone());
        }
    }

    for typ in &non_opaque {
        let typ_for_stub = with_from_json_stub(typ, api, has_serde);
        body_lines.push(gen_type_stub(
            &typ_for_stub,
            api,
            config,
            &capsule_names,
            &options_field_bridges,
            emit_docstrings,
            &streaming_return_types,
        ));
        body_lines.push("".to_string());
    }

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

    // `options.py` `@dataclass`, not the compiled `#[pyclass]` this stub declares. A data-enum
    let coercible_dtos = crate::backends::pyo3::gen_bindings::wire_schema::coercible_dto_names(api, config);

    let core_import = config.core_import_name();
    for enum_def in &api.enums {
        let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(&core_import, &enum_def.rust_path);
        body_lines.push(gen_enum_stub(enum_def, emit_docstrings, &coercible_dtos, is_host_enum));
        body_lines.push("".to_string());
    }

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

    // Companion `{Error}Info` class + `{error}_info` free function for each error with
    // whitelisted introspection methods -- see `gen_init_py`'s matching block in
    // `gen_bindings/errors.rs` for why these must be declared here too (they never flow
    // through `api.functions`/`api.types`, so nothing else in this file would emit them).
    {
        let mut info_lines: Vec<String> = Vec::new();
        for error in &api.errors {
            if !crate::codegen::error_gen::pyo3_error_has_methods(error) {
                continue;
            }
            let struct_name = crate::codegen::error_gen::pyo3_error_info_struct_name(error);
            let fn_name = crate::codegen::error_gen::pyo3_error_info_fn_name(error);
            info_lines.push(format!("class {struct_name}:"));
            for (field, py_type) in crate::codegen::error_gen::pyo3_error_info_field_specs(error) {
                info_lines.push(format!("    {field}: {py_type}"));
            }
            info_lines.push("".to_string());
            info_lines.push(format!("def {fn_name}(err: Any) -> {struct_name}: ..."));
        }
        if !info_lines.is_empty() {
            body_lines.extend(info_lines);
            body_lines.push("".to_string());
        }
    }

    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        body_lines.push(gen_function_stub(
            func,
            &bridge_param_names,
            &capsule_names,
            &options_field_bridges,
            &streaming_return_types,
            &opaque_types,
        ));
    }

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
    let declared_function_names: std::collections::HashSet<&str> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .map(|f| f.name.as_str())
        .collect();
    for bridge in trait_bridges {
        if crate::backends::pyo3::trait_bridge::active_bridge_trait(bridge, api).is_none() {
            continue;
        }
        if let Some(register_fn) = crate::codegen::generators::trait_bridge::bridge_register_symbol(bridge)
            && !declared_function_names.contains(register_fn)
        {
            let backend_type = if protocol_trait_names.contains(&bridge.trait_name) {
                bridge.trait_name.as_str()
            } else {
                "object"
            };
            body_lines.push(format!("def {register_fn}(backend: {backend_type}) -> None: ..."));
        }
        if let Some(unregister_fn) = bridge.unregister_fn.as_deref()
            && !declared_function_names.contains(unregister_fn)
        {
            body_lines.push(format!("def {unregister_fn}(name: str) -> None: ..."));
        }
        if let Some(clear_fn) = bridge.clear_fn.as_deref()
            && !declared_function_names.contains(clear_fn)
        {
            body_lines.push(format!("def {clear_fn}() -> None: ..."));
        }
    }

    let body_joined = body_lines.join("\n");
    let used_typing: Vec<&str> = [
        "Any",
        "AsyncIterator",
        "Iterable",
        "Literal",
        "Protocol",
        "TypeAlias",
        "TypedDict",
    ]
    .iter()
    .copied()
    .filter(|name| contains_word(&body_joined, name))
    .collect();
    let mut lines = header_lines;
    if contains_word(&body_joined, "builtins") {
        lines.push("import builtins".to_string());
        lines.push("".to_string());
    }
    if body_joined.contains("options.") {
        lines.push("from . import options".to_string());
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
        let list_pattern = format!("list[{name}]");
        if result.contains(&list_pattern) {
            result = result.replace(&list_pattern, "list[Any]");
            continue;
        }
        let optional_pattern = format!("{name} | None");
        if result.contains(&optional_pattern) {
            result = result.replace(&optional_pattern, "Any");
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::{gen_stubs, gen_type_stub, with_from_json_stub};
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

    fn python_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "test_lib"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// A class-level attribute annotation and a same-named `def` in one `.pyi` class body is a
    /// redefinition (`mypy: Name "providers" already defined`). The attribute wins, matching the
    /// binding, which drops the same-named `#[pymethods]` wrapper.
    #[test]
    fn class_stub_emits_a_field_and_same_named_method_once() {
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "LlmConfig".to_string(),
                rust_path: "test_lib::LlmConfig".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "providers".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..Default::default()
                }],
                methods: vec![MethodDef {
                    name: "providers".to_string(),
                    return_type: TypeRef::String,
                    receiver: Some(ReceiverKind::Ref),
                    cfg: None,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let stub = gen_stubs(&api, &[], &python_config(), &ahash::AHashSet::default());

        assert!(
            stub.contains("providers: str | None"),
            "the attribute must still be declared:\n{stub}"
        );
        let methods = stub.matches("def providers(").count();
        assert_eq!(
            methods, 0,
            "the same-named method stub must be dropped, found {methods}:\n{stub}"
        );
    }

    fn widget_request() -> TypeDef {
        TypeDef {
            name: "WidgetRequest".to_string(),
            rust_path: "my_lib::WidgetRequest".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "label".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn render_type_stub(typ: &TypeDef, api: &ApiSurface) -> String {
        gen_type_stub(
            typ,
            api,
            &python_config(),
            &std::collections::HashSet::new(),
            &super::OptionsFieldBridges::default(),
            false,
            &std::collections::HashMap::new(),
        )
    }

    #[test]
    fn type_stub_declares_from_json_for_a_convertible_serde_type() {
        let typ = widget_request();
        let api = ApiSurface {
            types: vec![typ.clone()],
            ..Default::default()
        };

        let typ_for_stub = with_from_json_stub(&typ, &api, true);
        assert!(
            typ_for_stub
                .methods
                .iter()
                .any(|m| m.name == "from_json" && m.is_static),
            "expected a synthetic from_json method entry"
        );

        let stub = render_type_stub(&typ_for_stub, &api);
        assert!(
            stub.contains("    @staticmethod\n    def from_json(json_str: str) -> WidgetRequest: ..."),
            "expected a from_json staticmethod declaration:\n{stub}"
        );
    }

    #[test]
    fn type_stub_omits_from_json_when_crate_has_no_serde() {
        let typ = widget_request();
        let api = ApiSurface {
            types: vec![typ.clone()],
            ..Default::default()
        };

        let typ_for_stub = with_from_json_stub(&typ, &api, false);
        assert!(!typ_for_stub.methods.iter().any(|m| m.name == "from_json"));

        let stub = render_type_stub(&typ_for_stub, &api);
        assert!(
            !stub.contains("def from_json"),
            "unexpected from_json declaration:\n{stub}"
        );
    }

    #[test]
    fn type_stub_omits_from_json_for_an_opaque_type() {
        let typ = TypeDef {
            is_opaque: true,
            ..widget_request()
        };
        let api = ApiSurface {
            types: vec![typ.clone()],
            ..Default::default()
        };

        let typ_for_stub = with_from_json_stub(&typ, &api, true);
        assert!(!typ_for_stub.methods.iter().any(|m| m.name == "from_json"));

        let stub = render_type_stub(&typ_for_stub, &api);
        assert!(
            !stub.contains("def from_json"),
            "unexpected from_json declaration:\n{stub}"
        );
    }

    /// Regression (liter-llm E2E failures): the `.pyi` stub must declare the companion
    /// `{Error}Info` class and `{error}_info` free function alongside every other symbol the
    /// native module exports -- see the matching regression test in `gen_bindings/errors.rs`
    /// for why these are otherwise invisible (they never flow through `api.functions`/
    /// `api.types`).
    #[test]
    fn gen_stubs_declares_error_info_class_and_function() {
        use crate::core::ir::{ErrorDef, MethodDef};

        let api = ApiSurface {
            errors: vec![ErrorDef {
                name: "LiterLlmError".to_string(),
                rust_path: "liter_llm::LiterLlmError".to_string(),
                original_rust_path: String::new(),
                variants: vec![],
                doc: String::new(),
                methods: vec![MethodDef {
                    name: "status_code".to_string(),
                    ..Default::default()
                }],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            ..Default::default()
        };

        let stub = gen_stubs(&api, &[], &python_config(), &ahash::AHashSet::default());

        assert!(
            stub.contains("class LiterLlmErrorInfo:"),
            "missing LiterLlmErrorInfo class declaration:\n{stub}"
        );
        assert!(stub.contains("    code: int"), "missing code: int field:\n{stub}");
        assert!(
            stub.contains("    status_code: int"),
            "missing status_code: int field:\n{stub}"
        );
        assert!(
            stub.contains("def liter_llm_error_info(err: Any) -> LiterLlmErrorInfo: ..."),
            "missing liter_llm_error_info function declaration:\n{stub}"
        );
    }

    /// An error with no whitelisted introspection methods gets no companion info class or
    /// function -- `gen_stubs` must not invent one either.
    #[test]
    fn gen_stubs_omits_error_info_when_error_has_no_methods() {
        use crate::core::ir::ErrorDef;

        let api = ApiSurface {
            errors: vec![ErrorDef {
                name: "PlainError".to_string(),
                rust_path: "lib::PlainError".to_string(),
                original_rust_path: String::new(),
                variants: vec![],
                doc: String::new(),
                methods: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            ..Default::default()
        };

        let stub = gen_stubs(&api, &[], &python_config(), &ahash::AHashSet::default());

        assert!(
            !stub.contains("PlainErrorInfo"),
            "no info class should exist for a method-less error:\n{stub}"
        );
        assert!(
            !stub.contains("plain_error_info"),
            "no info function should exist for a method-less error:\n{stub}"
        );
    }
}
