use super::{
    OptionsFieldBridges, constructor_param_type, constructor_rust_type_to_python, is_python_builtin_name,
    pyi_docstring, python_safe_name, substitute_capsule_type,
};
use crate::backends::pyo3::type_map::python_type;
use crate::codegen::shared::binding_fields;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

/// Builtin type names that a struct field can shadow. When a field is named after one of these
/// (e.g. a field `bytes`), that name refers to the field variable inside the class body, so any
/// annotation using the builtin resolves to the field instead of the type — and mypy `--strict`
/// rejects it (`Variable "X.bytes" is not valid as a type [valid-type]`). Qualifying such
/// annotations as `builtins.<name>` breaks the shadowing; `gen_stubs.rs` emits `import builtins`
/// whenever the body references it.
const SHADOWABLE_BUILTINS: &[&str] = &[
    "bytes",
    "str",
    "int",
    "float",
    "bool",
    "type",
    "list",
    "dict",
    "set",
    "tuple",
    "frozenset",
];

/// The builtins shadowed by a field name in `typ`, using the resolved Python stub field names.
fn shadowed_builtins(typ: &TypeDef, config: &ResolvedCrateConfig) -> Vec<&'static str> {
    SHADOWABLE_BUILTINS
        .iter()
        .copied()
        .filter(|builtin| {
            binding_fields(&typ.fields).any(|f| {
                let name = config
                    .resolve_field_name(Language::Python, &typ.name, &f.name)
                    .unwrap_or_else(|| f.name.clone());
                name == *builtin
            })
        })
        .collect()
}

/// Qualify whole-identifier occurrences of each shadowed builtin in a type annotation as
/// `builtins.<name>` (e.g. `bytes | None` -> `builtins.bytes | None`).
fn qualify_shadowed_builtins(annotation: &str, shadowed: &[&str]) -> String {
    let mut out = annotation.to_string();
    for builtin in shadowed {
        out = replace_bare_ident(&out, builtin, &format!("builtins.{builtin}"));
    }
    out
}

/// Replace whole-identifier occurrences of `ident` — not preceded by `.`, an ASCII alphanumeric,
/// or `_`, and not followed by an ASCII alphanumeric or `_` — with `replacement`. Annotations are
/// ASCII, so byte-wise scanning is safe.
fn replace_bare_ident(haystack: &str, ident: &str, replacement: &str) -> String {
    let bytes = haystack.as_bytes();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < bytes.len() {
        if haystack[i..].starts_with(ident) {
            let before_ok = i == 0 || {
                let b = bytes[i - 1];
                !(b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
            };
            let end = i + ident.len();
            let after_ok = end >= bytes.len() || {
                let b = bytes[end];
                !(b.is_ascii_alphanumeric() || b == b'_')
            };
            if before_ok && after_ok {
                out.push_str(replacement);
                i = end;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

pub(super) fn gen_opaque_type_stub(
    typ: &TypeDef,
    capsule_names: &std::collections::HashSet<&str>,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
    ctor: Option<&ClientConstructorConfig>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("class {}:", typ.name));

    // Emit __init__ stub when the type has a client constructor so mypy
    // recognises `TypeName(params...)` construction call sites.
    if let Some(ctor) = ctor {
        let mut params: Vec<String> = ctor
            .params
            .iter()
            .map(|p| {
                let py_type = constructor_rust_type_to_python(&p.ty);
                format!("{}: {}", p.name, py_type)
            })
            .collect();
        let single = format!("    def __init__(self, {}) -> None: ...", params.join(", "));
        if single.len() <= 100 {
            lines.push(single);
        } else {
            let mut wrapped = String::from("    def __init__(\n        self,\n");
            for param in &mut params {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_wrapped_param_line.jinja",
                    minijinja::context! { param => param },
                ));
            }
            wrapped.push_str("    ) -> None: ...");
            lines.push(wrapped);
        }
    }

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

    // If no methods and no constructor, emit as a one-liner.
    if typ.methods.is_empty() && ctor.is_none() {
        return format!("class {}: ...", typ.name);
    }

    lines.join("\n")
}

/// Generate a Python type stub for a struct.
pub(super) fn gen_type_stub(
    typ: &TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    capsule_names: &std::collections::HashSet<&str>,
    options_field_bridges: &OptionsFieldBridges<'_>,
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
    // When a field name shadows a builtin type (e.g. a field `bytes`), annotations that use that
    // builtin must be qualified as `builtins.<name>` (see `shadowed_builtins`).
    let shadowed = shadowed_builtins(typ, config);
    // Field names that are Python reserved keywords are shown with their escaped name
    // (e.g. `class_`) because that is the attribute name callers must use in Python.
    // The underlying `#[pyo3(get, name = "class")]` attribute on the Rust struct exposes
    // it as `obj.class_` (the escaped name), NOT as `obj.class`, because `class` is a
    // syntax error in a Python attribute access expression.  The stub must match.
    for field in binding_fields(&typ.fields) {
        // Check if this field is a trait bridge marker (e.g., visitor field on ConversionOptions).
        // When it is, prefer the trait Protocol class name (e.g., HtmlVisitor) over the
        // binding-internal opaque handle (e.g., VisitorHandle), matching the __init__ signature logic.
        let type_str = if let Some((_, type_alias, trait_name)) = options_field_bridges.get(typ.name.as_str()) {
            if let Some(alias) = type_alias {
                if field.name == *alias {
                    // This field is the bridge marker; use the trait Protocol name if available
                    trait_name.or(*type_alias).unwrap_or("object").to_string()
                } else {
                    python_type(&field.ty)
                }
            } else {
                python_type(&field.ty)
            }
        } else {
            python_type(&field.ty)
        };
        // Duration fields on has_default types are Option<u64> in PyO3, so annotate as int | None
        let is_optional_duration = typ.has_default && matches!(field.ty, TypeRef::Duration) && !field.optional;
        let field_type = if (is_optional_duration || field.optional) && !type_str.contains("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        let field_type = qualify_shadowed_builtins(&field_type, &shadowed);
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
    options_field_bridges: &OptionsFieldBridges<'_>,
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
    // Exclude the OptionsField trait-bridge field: it is emitted below as the dedicated
    // bridge kwarg (mirroring the `#[new]` constructor, which also filters it out), so
    // emitting it here too would duplicate the parameter in the `__init__` stub.
    let bridge_field_name = options_field_bridges.get(typ.name.as_str()).map(|(kwarg, _, _)| *kwarg);
    let (required, optional): (Vec<_>, Vec<_>) = binding_fields(&typ.fields)
        .filter(|f| f.cfg.as_deref().is_none_or(cfg_present_for_pyo3_stub))
        .filter(|f| bridge_field_name != Some(f.name.as_str()))
        .partition(|f| {
            if typ.has_default {
                // All fields are optional in the Rust signature — nothing is required.
                return false;
            }
            let is_optional_duration = matches!(f.ty, TypeRef::Duration) && !f.optional;
            !f.optional && !is_optional_duration
        });

    // Per-language `rename_fields` map, keyed by Rust field name. Combined with each field's
    // `serde_rename` by the shared `resolve_param_ident` — the SAME resolver the `#[new]`
    // constructor uses — so the stub param names cannot drift from the runtime constructor (the
    // constructor deliberately prefers serde-rename wire names for cross-binding API parity).
    let py_field_renames: std::collections::HashMap<String, String> = typ
        .fields
        .iter()
        .filter_map(|f| {
            config
                .resolve_field_name(Language::Python, &typ.name, &f.name)
                .map(|renamed| (f.name.clone(), renamed))
        })
        .collect();
    let renames_ref = if py_field_renames.is_empty() {
        None
    } else {
        Some(&py_field_renames)
    };

    // Annotations that use a builtin shadowed by a sibling field name must be qualified as
    // `builtins.<name>` — the same rule the field annotations above apply.
    let shadowed = shadowed_builtins(typ, config);

    // Generate required params first, then optional params.
    // For constructor params, use str instead of enum types (PyO3 accepts any string).
    // Field names that are Python reserved keywords are emitted with their escaped name
    // (e.g. `class_`) so the generated `__init__` signature is valid Python syntax.
    let mut params: Vec<String> = required
        .iter()
        .map(|f| {
            let param_type = qualify_shadowed_builtins(&constructor_param_type(&f.ty, api), &shadowed);
            let param_name = crate::backends::pyo3::gen_bindings::constructors::resolve_param_ident(
                &f.name,
                f.serde_rename.as_ref(),
                renames_ref,
            );
            // PyO3 exposes a raw-ident param (`r#type`) to Python as `type`; the stub must
            // match the runtime kwarg, so strip the Rust raw-identifier prefix here.
            let param_name = param_name.strip_prefix("r#").map(str::to_owned).unwrap_or(param_name);
            format!("{param_name}: {param_type}")
        })
        .collect();

    params.extend(optional.iter().map(|f| {
        let type_str = qualify_shadowed_builtins(&constructor_param_type(&f.ty, api), &shadowed);
        let param_type = if !type_str.ends_with("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        let param_name = crate::backends::pyo3::gen_bindings::constructors::resolve_param_ident(
            &f.name,
            f.serde_rename.as_ref(),
            renames_ref,
        );
        // PyO3 exposes a raw-ident param (`r#type`) to Python as `type`; the stub must
        // match the runtime kwarg, so strip the Rust raw-identifier prefix here.
        let param_name = param_name.strip_prefix("r#").map(str::to_owned).unwrap_or(param_name);
        format!("{param_name}: {param_type} = None")
    }));

    // When this struct is the options-type of a trait bridge with `bind_via=OptionsField`,
    // the PyO3 `#[new]` constructor accepts an additional `{kwarg_name}: {trait_name} = None`
    // kwarg (e.g. `visitor: HtmlVisitor | object | None = None`). The bridge field is cfg-gated in
    // the IR, so the partition above strips it, but the PyO3 macro keeps it via
    // `never_skip_cfg_field_names`. Surface it here so api.py callers type-check.
    //
    // Prefer the trait's Protocol class name (e.g. `HtmlVisitor`) over the binding-internal
    // `type_alias` (e.g. `VisitorHandle`) because the runtime bridge wraps any object that
    // implements the protocol methods — callers should pass an `HtmlVisitor`, not a handle.
    if let Some((kwarg_name, type_alias, trait_name)) = options_field_bridges.get(typ.name.as_str()) {
        // Widen the constructor kwarg to accept any duck-typed object — see the matching
        // comment in `functions.rs` for the runtime dispatch behavior.
        let visitor_type = trait_name.or(*type_alias).unwrap_or("object");
        params.push(format!("{kwarg_name}: {visitor_type} | object | None = None"));
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
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_wrapped_noqa.jinja",
                    minijinja::context! { param => param, indent => "        " },
                ));
            } else {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_wrapped.jinja",
                    minijinja::context! { param => param, indent => "        " },
                ));
            }
        }
        wrapped.push_str("    ) -> None: ...");
        wrapped
    }
}

fn cfg_present_for_pyo3_stub(cfg: &str) -> bool {
    let normalized: String = cfg.chars().filter(|c| !c.is_whitespace()).collect();
    if normalized == "not(target_arch=\"wasm32\")" {
        return true;
    }
    if normalized.starts_with("feature=") {
        return true;
    }
    if normalized.starts_with("any(") && normalized.ends_with(')') {
        let inner = &normalized[4..normalized.len() - 1];
        return inner
            .split(',')
            .all(|part| part.starts_with("feature=") || part == "not(target_arch=\"wasm32\")");
    }
    false
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
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_method_wrapped_noqa.jinja",
                    minijinja::context! { indent => indent, param => param },
                ));
            } else {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::FieldDef;
    use crate::core::ir::PrimitiveType;

    #[test]
    fn type_init_stub_keeps_pyo3_present_cfg_fields() {
        let typ = TypeDef {
            name: "UrlExtractionConfig".to_string(),
            fields: vec![
                FieldDef {
                    name: "mode".to_string(),
                    ty: TypeRef::Named("UrlExtractionMode".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "crawl".to_string(),
                    ty: TypeRef::Named("CrawlConfig".to_string()),
                    cfg: Some("any(feature = \"url-ingestion\", feature = \"url-config-types\")".to_string()),
                    ..Default::default()
                },
            ],
            has_default: true,
            ..Default::default()
        };
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "CrawlConfig".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let stub = gen_type_init_stub(
            &typ,
            &api,
            &ResolvedCrateConfig::default(),
            &OptionsFieldBridges::default(),
        );

        assert!(stub.contains("mode: UrlExtractionMode | None = None"), "{stub}");
        assert!(stub.contains("crawl: CrawlConfig | None = None"), "{stub}");
    }

    #[test]
    fn type_init_stub_still_omits_non_pyo3_cfg_fields() {
        let typ = TypeDef {
            name: "PlatformConfig".to_string(),
            fields: vec![FieldDef {
                name: "windows_only".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                cfg: Some("target_os = \"windows\"".to_string()),
                ..Default::default()
            }],
            has_default: true,
            ..Default::default()
        };

        let stub = gen_type_init_stub(
            &typ,
            &ApiSurface::default(),
            &ResolvedCrateConfig::default(),
            &OptionsFieldBridges::default(),
        );

        assert!(!stub.contains("windows_only"), "{stub}");
    }
}
