use crate::type_map::python_type;
use alef_core::config::{AlefConfig, BridgeBinding, Language, TraitBridgeConfig};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, EnumDef, FunctionDef, MethodDef, TypeDef, TypeRef};
use heck::ToShoutySnakeCase;

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

pub fn gen_stubs(api: &ApiSurface, trait_bridges: &[TraitBridgeConfig], config: &AlefConfig) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut lines: Vec<String> = header.lines().map(str::to_string).collect();
    lines.push("".to_string());
    lines.push("from typing import Any, Literal, TypeAlias, TypedDict".to_string());
    lines.push("".to_string());

    // Collect bridge param names so function stubs can emit `object | None` instead of
    // `str | None` for params that are sanitized trait bridge parameters.
    let bridge_param_names: std::collections::HashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // For options-field bridges, collect a map from options_type name → bridge field name so
    // gen_type_stub can override the field type to `object | None` instead of `str | None`.
    let bridge_field_overrides: std::collections::HashMap<&str, &str> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let field_name = b.resolved_options_field()?;
            Some((options_type, field_name))
        })
        .collect();

    // Generate type stubs — collect opaque types separately so consecutive
    // one-liner class stubs are emitted without blank lines between them
    // (ruff strips those in .pyi files).
    let (opaque, non_opaque): (Vec<_>, Vec<_>) = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait)
        .partition(|typ| typ.is_opaque);

    for typ in &non_opaque {
        lines.push(gen_type_stub(typ, api, config, &bridge_field_overrides));
        lines.push("".to_string());
    }

    if !opaque.is_empty() {
        for typ in &opaque {
            lines.push(gen_opaque_type_stub(typ));
        }
        lines.push("".to_string());
    }

    // Generate enum stubs
    for enum_def in &api.enums {
        lines.push(gen_enum_stub(enum_def));
        lines.push("".to_string());
    }

    // Generate function stubs — no blank lines between consecutive stubs (ruff strips them)
    for func in &api.functions {
        lines.push(gen_function_stub(func, &bridge_param_names));
    }

    lines.join("\n")
}

/// Generate a Python type stub for an opaque type (no fields, only methods).
fn gen_opaque_type_stub(typ: &TypeDef) -> String {
    let mut lines = vec![];

    lines.push(format!("class {}:", typ.name));

    // Instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(method, false));
        }
    }

    // Static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(method, true));
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
    config: &AlefConfig,
    bridge_field_overrides: &std::collections::HashMap<&str, &str>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("class {}:", typ.name));

    // Add field type annotations.
    // Field names that are Python reserved keywords are shown with their escaped name
    // (e.g. `class_`) because that is the attribute name callers must use in Python.
    // The underlying `#[pyo3(get, name = "class")]` attribute on the Rust struct exposes
    // it as `obj.class_` (the escaped name), NOT as `obj.class`, because `class` is a
    // syntax error in a Python attribute access expression.  The stub must match.
    for field in &typ.fields {
        // When this type is the options_type for an options-field bridge and this is the
        // bridge field, override the type hint to `object | None` so Python callers know
        // they can pass any visitor object.  The IR sanitizes the type to `String`, which
        // would otherwise show up as `str | None` in the stub — incorrect and confusing.
        let is_bridge_field = bridge_field_overrides
            .get(typ.name.as_str())
            .is_some_and(|&bridge_field_name| field.name == bridge_field_name);
        let field_type = if is_bridge_field {
            "object | None".to_string()
        } else {
            let type_str = python_type(&field.ty);
            // Duration fields on has_default types are Option<u64> in PyO3, so annotate as int | None
            let is_optional_duration = typ.has_default && matches!(field.ty, TypeRef::Duration) && !field.optional;
            if (is_optional_duration || field.optional) && !type_str.contains("| None") {
                format!("{} | None", type_str)
            } else {
                type_str
            }
        };
        // Resolve the field name: use config-driven rename if available, otherwise apply
        // automatic keyword escaping via python_safe_name.
        let stub_field_name = config
            .resolve_field_name(Language::Python, &typ.name, &field.name)
            .unwrap_or_else(|| field.name.clone());
        lines.push(format!("    {stub_field_name}: {field_type}"));
    }

    // Add __init__ signature
    lines.push(gen_type_init_stub(typ, api, config, bridge_field_overrides));

    // Add instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(method, false));
        }
    }

    // Add static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(method, true));
        }
    }

    lines.join("\n")
}

/// Generate __init__ signature stub for a struct.
fn gen_type_init_stub(
    typ: &TypeDef,
    api: &ApiSurface,
    config: &AlefConfig,
    bridge_field_overrides: &std::collections::HashMap<&str, &str>,
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
    let (required, optional): (Vec<_>, Vec<_>) = typ.fields.iter().filter(|f| f.cfg.is_none()).partition(|f| {
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
    //
    // Bridge fields (e.g. `visitor`) accept any Python object — use `object | None`
    // to match the field annotation and avoid mypy incompatible-argument errors.
    let is_bridge_field = |f: &alef_core::ir::FieldDef| {
        bridge_field_overrides
            .get(typ.name.as_str())
            .is_some_and(|&bridge_field_name| f.name == bridge_field_name)
    };

    let mut params: Vec<String> = required
        .iter()
        .map(|f| {
            let param_type = if is_bridge_field(f) {
                "object".to_string()
            } else {
                constructor_param_type(&f.ty, api)
            };
            let param_name = config
                .resolve_field_name(Language::Python, &typ.name, &f.name)
                .unwrap_or_else(|| f.name.clone());
            format!("{param_name}: {param_type}")
        })
        .collect();

    params.extend(optional.iter().map(|f| {
        let type_str = if is_bridge_field(f) {
            "object".to_string()
        } else {
            constructor_param_type(&f.ty, api)
        };
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
                wrapped.push_str(&format!("        {},  # noqa: A002\n", param));
            } else {
                wrapped.push_str(&format!("        {},\n", param));
            }
        }
        wrapped.push_str("    ) -> None: ...");
        wrapped
    }
}

/// Generate a method stub.
fn gen_method_stub(method: &MethodDef, is_static: bool) -> String {
    // Partition params into required (non-optional) and optional
    let (required, optional): (Vec<_>, Vec<_>) = method.params.iter().partition(|p| !p.optional);

    // Generate required params first, then optional params
    let mut params: Vec<String> = required
        .iter()
        .map(|p| {
            let param_type = python_type(&p.ty);
            format!("{}: {}", p.name, param_type)
        })
        .collect();

    params.extend(optional.iter().map(|p| {
        let type_str = python_type(&p.ty);
        let param_type = if !type_str.ends_with("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        format!("{}: {} = None", p.name, param_type)
    }));

    let return_type = python_type(&method.return_type);
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
                wrapped.push_str(&format!("{}    {},  # noqa: A002\n", indent, param));
            } else {
                wrapped.push_str(&format!("{}    {},\n", indent, param));
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

/// Generate a Python enum stub.
///
/// For unit enums, emits both the PascalCase variant names (as PyO3 exposes them) and the
/// SCREAMING_SNAKE_CASE aliases that `options.py` adds at runtime via attribute assignment.
/// This allows mypy to resolve both forms without errors.
fn gen_enum_stub(enum_def: &EnumDef) -> String {
    use alef_codegen::generators::enum_has_data_variants;
    let mut lines = vec![];

    if enum_has_data_variants(enum_def) {
        // Data enums: emit a TypedDict per variant and a Union type alias.
        gen_data_enum_typeddicts(&mut lines, enum_def);
    } else {
        lines.push(format!("class {}:", enum_def.name));
        // PascalCase variants — as exposed by the PyO3 #[pyclass] enum.
        for variant in &enum_def.variants {
            lines.push(format!(
                "    {}: {} = ...",
                python_safe_name(&variant.name),
                enum_def.name
            ));
        }
        // SCREAMING_SNAKE_CASE aliases — added at runtime in options.py so callers can use
        // e.g. `CodeBlockStyle.BACKTICKS` in addition to `CodeBlockStyle.Backticks`.
        // Declaring them here lets mypy resolve both forms without attr-defined errors.
        for variant in &enum_def.variants {
            let screaming = variant.name.to_shouty_snake_case();
            let pascal = python_safe_name(&variant.name);
            // Only emit an alias if it differs from the PascalCase form (e.g. `Backticks` →
            // `BACKTICKS`). Skip identical duplicates (e.g. single-word all-caps names).
            if screaming != pascal {
                lines.push(format!("    {}: {} = ...", screaming, enum_def.name));
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

    // Emit the Union type alias
    if variant_class_names.len() <= 3 {
        lines.push(format!("{} = {}", enum_def.name, variant_class_names.join(" | ")));
    } else {
        // Multi-line for readability
        lines.push(format!("{} = (", enum_def.name));
        for (i, name) in variant_class_names.iter().enumerate() {
            if i < variant_class_names.len() - 1 {
                lines.push(format!("    {} |", name));
            } else {
                lines.push(format!("    {}", name));
            }
        }
        lines.push(")".to_string());
    }
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
fn gen_function_stub(func: &FunctionDef, bridge_param_names: &std::collections::HashSet<&str>) -> String {
    // Partition params into required (non-optional) and optional
    let (required, optional): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !p.optional);

    // Generate required params first, then optional params
    let mut params: Vec<String> = required
        .iter()
        .map(|p| {
            let param_type = if bridge_param_names.contains(p.name.as_str()) {
                "object".to_string()
            } else {
                python_type(&p.ty)
            };
            format!("{}: {}", p.name, param_type)
        })
        .collect();

    params.extend(optional.iter().map(|p| {
        let type_str = if bridge_param_names.contains(p.name.as_str()) {
            "object".to_string()
        } else {
            python_type(&p.ty)
        };
        let param_type = if !type_str.ends_with("| None") {
            format!("{} | None", type_str)
        } else {
            type_str
        };
        format!("{}: {} = None", p.name, param_type)
    }));

    let return_type = python_type(&func.return_type);
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
                wrapped.push_str(&format!("    {},  # noqa: A002\n", param));
            } else {
                wrapped.push_str(&format!("    {},\n", param));
            }
        }
        wrapped.push_str(&format!(") -> {}: ...", return_type));
        wrapped
    }
}
