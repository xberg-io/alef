//! Python exception hierarchy and `__init__.py` generation.

use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::core::config::{DtoConfig, PythonDtoStyle};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::AHashSet;

/// True when `typ` is rendered as a pure-Python dataclass / TypedDict in `options.py` rather than
/// being re-exported from the native module. For such types the public name resolves to the
/// wrapper (a `@dataclass` or a plain `dict`), NOT the compiled `#[pyclass]`. This is the single
/// source of truth for the config-vs-native-return split: `gen_init_py` uses it to route imports,
/// and the data-enum variant-constructor generator uses it to decide which `Named` payload fields
/// must be coerced (a native-return type stays compiled, so no coercion is needed or wanted).
pub(super) fn is_dataclass_backed_config(
    typ: &TypeDef,
    output_style: PythonDtoStyle,
    reexported_names: &AHashSet<&str>,
) -> bool {
    if typ.is_trait || typ.binding_excluded {
        return false;
    }
    if typ.name.ends_with("Builder") || typ.name.ends_with("Update") {
        return false;
    }
    if !typ.has_default || typ.fields.is_empty() {
        return false;
    }
    // A `has_default` type that is also a return type is handed back to callers as an
    // attribute-access pyclass and re-exported from the native module (unless the structural
    // TypedDict style applies and it is not in `reexported_types`).
    let is_native_return = typ.is_return_type
        && (output_style != PythonDtoStyle::TypedDict || reexported_names.contains(typ.name.as_str()));
    !is_native_return
}

fn render_relative_import(module_name: &str, imports: &[String]) -> String {
    crate::backends::pyo3::template_env::render(
        "import_from_module.jinja",
        minijinja::context! {
            module_name => module_name,
            imports => imports.join(", "),
        },
    )
}

fn render_absolute_import(module_name: &str, imports: &[String]) -> String {
    crate::backends::pyo3::template_env::render(
        "import_from_absolute_module.jinja",
        minijinja::context! {
            module_name => module_name,
            imports => imports.join(", "),
        },
    )
}

fn is_long_import(import_statement: &str) -> bool {
    import_statement.trim_end_matches('\n').len() > 88
}

/// Generate exceptions.py — re-export the exception hierarchy from the native module.
///
/// The native module (`lib.rs`) defines every exception via `pyo3::create_exception!`
/// (the base error under `PyException`, each variant under the base error) and registers
/// them on the native extension module. Re-exporting those classes here — instead of
/// defining duplicate Python classes — guarantees that the class a user catches
/// (`from <pkg> import DownloadError`) is the exact class the native code raises, so
/// `except DownloadError:` works (GitHub issue #147).
///
/// ## Cross-Language Pattern (Alef Consistency)
///
/// This pattern applies to all language bindings, not just Python:
/// - **Node.js (NAPI-RS)**: Export exception classes from the native module, not new TypeScript classes
/// - **Ruby (Magnus)**: Expose exception classes through the native module, not thin wrappers
/// - **PHP (ext-php-rs)**: Map Rust exceptions to PHP exception classes in the native extension
/// - **Go (cgo)**: Return error types with preserved error codes; no type identity issues
/// - **Java (JNI/Panama FFM)**: Map Rust errors to Java exception classes; preserve numeric error codes
/// - **C# (P/Invoke)**: Map Rust errors to C# exception types; re-export from native wrapper
/// - **Elixir (Rustler)**: Return error tuples with consistent error codes; preserve error context
/// - **WebAssembly (wasm-bindgen)**: Map Rust errors to JavaScript Error types with numeric codes
///
/// The core principle: the **class/type identity of exceptions raised by native code must match
/// the class/type exposed by the public API**. This ensures users can catch exceptions using
/// public imports, avoiding the issue where `except SomeError:` fails because the native module
/// raises a different class object.
///
/// Re-exporting the variants under the base exception also preserves the ability to catch
/// all variants with `except Error:` (or language-equivalent).
pub(super) fn gen_exceptions_py(api: &ApiSurface, module_name: &str) -> String {
    let mut out = String::with_capacity(1024);
    let mut seen_classes: AHashSet<String> = AHashSet::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str("\"\"\"Exception hierarchy (re-exported from the native module).\"\"\"\n\n");

    // Collect base + variant exception names, de-duplicated, in a stable sorted order
    // (matches isort / ruff RUF022 so the generated file is format-stable).
    let mut exc_names: Vec<String> = Vec::new();
    for error in &api.errors {
        if seen_classes.insert(error.name.clone()) {
            exc_names.push(error.name.clone());
        }
        for variant in &error.variants {
            let variant_name = crate::codegen::error_gen::python_exception_name(&variant.name, &error.name);
            if seen_classes.insert(variant_name.clone()) {
                exc_names.push(variant_name);
            }
        }
    }

    if exc_names.is_empty() {
        return out;
    }
    exc_names.sort();

    // Re-export `from .<native module> import (...)`, wrapping to multi-line when the
    // single-line form would exceed ruff's line length (the established pattern in gen_init_py).
    let import_statement = render_relative_import(module_name, &exc_names);
    if is_long_import(&import_statement) {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "import_from_relative_module_header.jinja",
            minijinja::context! { module_name => module_name },
        ));
        for name in &exc_names {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "trait_bridge/indented_import_item.jinja",
                minijinja::context! { name => name },
            ));
        }
        out.push_str(")\n");
    } else {
        out.push_str(&import_statement);
    }

    out.push('\n');
    out.push_str("__all__ = [\n");
    for name in &exc_names {
        out.push_str(&format!("    \"{name}\",\n"));
    }
    out.push_str("]\n");

    // Re-point each re-exported exception's `__module__` at the public package. The objects are
    // the native ones (`create_exception!` sets their module to the compiled extension), so
    // tracebacks and `repr()` otherwise read `_native.DownloadError` instead of the public name.
    // This is cosmetic and also makes them picklable under their public path (GitHub issue #147).
    out.push_str(concat!(
        "\n",
        "# Re-point each exception's __module__ at the public package so tracebacks show the\n",
        "# public name (e.g. \"DownloadError\"), not the native module name (GitHub issue #147).\n",
        "_public_module = __name__.rsplit(\".\", 1)[0]\n",
        "for _name in __all__:\n",
        "    globals()[_name].__module__ = _public_module\n",
        "del _name, _public_module\n",
    ));

    out
}

/// Generate __init__.py — re-exports and version.
/// Only exports user-facing types (not internal Update types or all enums).
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_init_py(
    api: &ApiSurface,
    module_name: &str,
    version: &str,
    dto: &DtoConfig,
    reexported_types: &[String],
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    extra_init_imports: &std::collections::BTreeMap<String, Vec<String>>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    adapters: &[crate::core::config::AdapterConfig],
    opaque_types: &std::collections::HashMap<String, String>,
    exclude_functions: &AHashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;

    let mut out = String::with_capacity(1024);
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str(&crate::backends::pyo3::template_env::render(
        "init_header.jinja",
        minijinja::context! { module_name => module_name, version => version },
    ));
    // ruff format: blank line required after module docstring before first import.
    out.push('\n');

    // Collect enum names referenced by config types (user-facing enums only)
    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();
    let output_style = dto.python_output_style();
    let mut needed_enums: Vec<String> = Vec::new();
    let mut needed_data_enums: Vec<String> = Vec::new();
    let mut config_types: Vec<String> = Vec::new();
    // Return types with is_return_type=true are defined authoritatively in the native Rust
    // module and are returned to callers as attribute-access pyclasses. They must be re-exported
    // from the native module — not from .options — so the type seen by static analysis matches
    // the runtime object. Under the structural (TypedDict) style every other type is emitted in
    // options.py, so only the curated `reexported_types` results stay native; a config type that
    // is also `is_return_type` (e.g. `ExtractionConfig`, returned by a resolver yet built by the
    // caller) is not listed there and remains a structural type.
    let reexported_names: AHashSet<&str> = reexported_types.iter().map(String::as_str).collect();
    let mut native_return_types: Vec<String> = Vec::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait && !typ.binding_excluded) {
        if typ.name.ends_with("Builder") {
            continue;
        }
        if typ.has_default && !typ.name.ends_with("Update") && !typ.fields.is_empty() {
            if is_dataclass_backed_config(typ, output_style, &reexported_names) {
                config_types.push(typ.name.clone());
            } else {
                native_return_types.push(typ.name.clone());
            }
            // Collect enum references regardless of whether the type is a return type or config
            // type — some enums are shared across both categories.
            for field in binding_fields(&typ.fields) {
                let inner_name = match &field.ty {
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
                if let Some(name) = inner_name {
                    if data_enum_names.contains(&name) {
                        if !needed_data_enums.iter().any(|n| n == name) {
                            needed_data_enums.push(name.to_string());
                        }
                    } else if enum_names.contains(&name) && !needed_enums.contains(&name.to_string()) {
                        needed_enums.push(name.to_string());
                    }
                }
            }
        }
    }

    // Collect ALL non-trait types from the native module that aren't config types.
    // Every type in api.types (except traits) is registered as a #[pyclass] in the native
    // module — ensure they are all re-exported from __init__.py so users can access them.
    let mut imports_from_native: Vec<String> = Vec::new();
    let options_type_set: AHashSet<&str> = config_types.iter().map(|s| s.as_str()).collect();
    let error_type_set: AHashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();
    // Update and Builder types are internal; skip them.
    for typ in api.types.iter().filter(|t| !t.is_trait && !t.binding_excluded) {
        if typ.name.ends_with("Update") || typ.name.ends_with("Builder") {
            continue;
        }
        if error_type_set.contains(typ.name.as_str()) {
            continue;
        }
        // Config types (has_default, non-return) go via options.py; already in config_types.
        if options_type_set.contains(typ.name.as_str()) {
            continue;
        }
        // Return types already collected in native_return_types; skip to avoid duplicates.
        if native_return_types.iter().any(|n| n == &typ.name) {
            continue;
        }
        // Everything else (opaque types, non-default structs, etc.) lives in the native module.
        if !needed_data_enums.iter().any(|n| n == &typ.name) {
            imports_from_native.push(typ.name.clone());
        }
    }
    // Collect ALL enums from the native module.
    // Unit enums (needed_enums) are registered as #[pyclass] and exported from the native module.
    // Data enums are also in the native module. All other enums live in native too.
    for enum_def in &api.enums {
        if needed_data_enums.iter().any(|n| n == &enum_def.name) {
            // Data enums already in native list.
            continue;
        }
        if !imports_from_native.iter().any(|n| n == &enum_def.name) {
            imports_from_native.push(enum_def.name.clone());
        }
    }

    // Collect remaining imports and sort them
    let mut imports_from_api = Vec::new();
    let mut imports_from_options = Vec::new();
    let mut imports_from_exceptions = Vec::new();

    // Import functions from api (regular functions + trait-bridge registration helpers + adapters).
    // Trait-bridge register_* functions are emitted as pass-through wrappers in api.py but
    // do not appear in api.functions — add them here so __init__.py re-exports them and they
    // appear in __all__.
    // Similarly, adapter-based streaming methods are emitted as module-level wrappers in api.py
    // but are not in api.functions — add them too.
    {
        let mut names: Vec<_> = api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(&f.name))
            .map(|f| f.name.clone())
            .collect();
        names.extend(crate::backends::pyo3::trait_bridge::collect_bridge_register_fns(
            trait_bridges,
        ));
        names.extend(crate::backends::pyo3::trait_bridge::collect_bridge_unregister_fns(
            trait_bridges,
        ));
        names.extend(crate::backends::pyo3::trait_bridge::collect_bridge_clear_fns(
            trait_bridges,
        ));
        names.extend(adapters.iter().map(|a| a.name.clone()));
        names.sort();
        names.dedup();
        imports_from_api.extend(names);
    }

    // Data enums and return types are backed by native Rust structs — import from the native module.
    needed_data_enums.sort();
    imports_from_native.extend(needed_data_enums.iter().cloned());
    native_return_types.sort();
    imports_from_native.extend(native_return_types.iter().cloned());
    // Capsule types are not registered in the native module; users get the target Python type
    // (e.g. tree_sitter.Language) instead, so the symbol does not exist in `._native` and must
    // be excluded from the import list.
    imports_from_native.retain(|n| !capsule_types.contains_key(n));
    // Declared opaque types from `[workspace.opaque_types]` that have a capsule_types override
    // are external references not registered as #[pyclass] in the native module, so they must
    // be excluded from __init__.py public imports to avoid ImportError.
    // Opaque types WITHOUT a capsule override DO get a binding-side #[pyclass] wrapper struct
    // and must remain in imports_from_native so they are re-exported from the public package.
    let python_capsule_type_names: ahash::AHashSet<&str> = capsule_types.keys().map(|k| k.as_str()).collect();
    imports_from_native.retain(|n| {
        if opaque_types.contains_key(n) {
            // Keep if no capsule override (has a wrapper struct), filter if capsule override exists
            !python_capsule_type_names.contains(n.as_str())
        } else {
            true
        }
    });
    // Case-insensitive sort matches isort's ordering (e.g. VisitorHandle < WalkDecision).
    imports_from_native.sort_by_key(|a| a.to_lowercase());
    imports_from_native.dedup();

    // Import config types from options.
    // Unit enums (needed_enums) are now imported from the native module (see above) — they must
    // NOT appear in imports_from_options, otherwise __init__.py would import the str,Enum shadow
    // class from options.py instead of the authoritative native pyclass.
    let mut opt_imports: Vec<String> = config_types.to_vec();
    opt_imports.sort();
    imports_from_options.extend(opt_imports);

    // Import exceptions (append "Error" suffix to variant names if not present,
    // prefix if shadowing Python builtins — A004 compliance)
    let mut exc_names = Vec::new();
    for error in &api.errors {
        exc_names.push(error.name.clone());
        for variant in &error.variants {
            let variant_name = crate::codegen::error_gen::python_exception_name(&variant.name, &error.name);
            exc_names.push(variant_name);
        }
    }
    exc_names.sort();
    // De-duplicate: distinct error enums can contribute variants that map to the same Python
    // exception name (e.g. two enums each with a `Validation` variant → `ValidationError`).
    // Emitting the symbol twice in `from .exceptions import (...)` trips ruff F811/I001, which
    // rewrites the file and breaks `alef verify`.
    exc_names.dedup();
    imports_from_exceptions.extend(exc_names.clone());

    // Output imports in isort order: underscore-prefixed modules before regular names.
    // Correct order: ._native (underscore) → .api → .exceptions → .options
    // Use multi-line format if the import line would be too long (>88 chars for ruff)

    // Data enums are Rust-backed structs; re-export from the native module first (isort: _ < a).
    if !imports_from_native.is_empty() {
        let import_statement = render_relative_import(module_name, &imports_from_native);
        if is_long_import(&import_statement) {
            // Use push_str directly to avoid the double-newline produced by routing through
            // single_line.jinja when the text already ends with `\n` (the template itself
            // appends another newline, yielding `(\n\n` which ruff then flags as E303).
            out.push_str(&crate::backends::pyo3::template_env::render(
                "import_from_relative_module_header.jinja",
                minijinja::context! { module_name => module_name },
            ));
            for name in &imports_from_native {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&import_statement);
        }
    }
    if !imports_from_api.is_empty() {
        let import_statement = render_relative_import("api", &imports_from_api);
        if is_long_import(&import_statement) {
            out.push_str("from .api import (\n");
            for name in &imports_from_api {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&import_statement);
        }
    }
    if !imports_from_exceptions.is_empty() {
        let import_statement = render_relative_import("exceptions", &imports_from_exceptions);
        if is_long_import(&import_statement) {
            out.push_str("from .exceptions import (\n");
            for name in &imports_from_exceptions {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&import_statement);
        }
    }
    if !imports_from_options.is_empty() {
        let import_statement = render_relative_import("options", &imports_from_options);
        if is_long_import(&import_statement) {
            out.push_str("from .options import (\n");
            for name in &imports_from_options {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&import_statement);
        }
    }

    // Service owners (e.g. `App`) are generated by `generate_service_api` into the `service`
    // module, not the native extension, so re-export them from `.service`.
    let mut service_owners: Vec<String> = api.services.iter().map(|s| s.name.clone()).collect();
    service_owners.sort();
    service_owners.dedup();
    if !service_owners.is_empty() {
        let import_statement = render_relative_import("service", &service_owners);
        if is_long_import(&import_statement) {
            out.push_str("from .service import (\n");
            for name in &service_owners {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&import_statement);
        }
    }

    // User-configured extra imports (e.g. hand-written sibling modules generated by the project's
    // own build script). Emitted last so they cannot shadow alef-generated symbols.
    let mut extra_all_items: Vec<String> = Vec::new();
    for (module, symbols) in extra_init_imports {
        if symbols.is_empty() {
            continue;
        }
        let import_statement = render_absolute_import(module, symbols);
        if is_long_import(&import_statement) {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "import_from_module_header.jinja",
                minijinja::context! { module_name => module },
            ));
            for name in symbols {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&import_statement);
        }
        extra_all_items.extend(symbols.iter().cloned());
    }

    // __all__
    let mut all_items = Vec::new();
    for f in &api.functions {
        all_items.push(f.name.clone());
    }
    // Include trait-bridge registration helpers in __all__ — they are exported via api.py
    // and must be discoverable from the package root.
    all_items.extend(crate::backends::pyo3::trait_bridge::collect_bridge_register_fns(
        trait_bridges,
    ));
    all_items.extend(crate::backends::pyo3::trait_bridge::collect_bridge_unregister_fns(
        trait_bridges,
    ));
    all_items.extend(crate::backends::pyo3::trait_bridge::collect_bridge_clear_fns(
        trait_bridges,
    ));
    // Include adapter-based streaming methods in __all__ — they are exported via api.py
    // and must be discoverable from the package root.
    all_items.extend(adapters.iter().map(|a| a.name.clone()));
    all_items.extend(needed_enums);
    all_items.extend(imports_from_native.iter().cloned());
    all_items.extend(config_types);
    all_items.extend(exc_names);
    all_items.extend(service_owners);
    all_items.extend(extra_all_items);
    all_items.sort();
    all_items.dedup();
    // Filter out declared opaque types that have a capsule override from __all__ — they are
    // not registered as #[pyclass] in the native module.
    // Opaque types WITHOUT a capsule override DO have a wrapper struct and SHOULD appear in
    // __all__ so they are importable from the public package root.
    all_items.retain(|n| {
        if opaque_types.contains_key(n) {
            !python_capsule_type_names.contains(n.as_str())
        } else {
            true
        }
    });

    out.push_str("\n__all__ = [\n");
    for name in &all_items {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "init_all_entry.jinja",
            minijinja::context! { name => name },
        ));
    }
    out.push_str("]\n\n");
    out.push_str(&crate::backends::pyo3::template_env::render(
        "version_declaration.jinja",
        minijinja::context! { version => version },
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::{gen_exceptions_py, gen_init_py};
    use crate::core::config::DtoConfig;
    use crate::core::ir::ApiSurface;

    fn empty_api() -> ApiSurface {
        ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// exceptions.py must RE-EXPORT the native exception classes (so the class a user
    /// catches is the one the native code raises — tslp issue #147), not redefine them.
    #[test]
    fn gen_exceptions_py_reexports_native_classes() {
        use crate::core::ir::{ErrorDef, ErrorVariant};

        let error = ErrorDef {
            name: "LibError".to_string(),
            rust_path: "lib::LibError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Io".to_string(),
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: "I/O error.".to_string(),
            }],
            doc: "Library errors.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut api = empty_api();
        api.errors.push(error);
        let result = gen_exceptions_py(&api, "_native");

        // Re-export from the native module, not class definitions.
        assert!(
            result.contains("from ._native import"),
            "exceptions.py must re-export from the native module, got:\n{result}",
        );
        assert!(
            !result.contains("class "),
            "exceptions.py must not define duplicate classes, got:\n{result}",
        );
        // Both base and the Error-suffixed variant must be re-exported.
        assert!(result.contains("LibError"), "missing base LibError in:\n{result}");
        assert!(result.contains("IoError"), "missing variant IoError in:\n{result}");
        // __all__ lists the re-exported names.
        assert!(result.contains("__all__"), "missing __all__ in:\n{result}");
        // Each re-exported exception's __module__ is re-pointed at the public package so
        // tracebacks read the public name, not `_native.*` (GitHub issue #147).
        assert!(
            result.contains("__module__ = _public_module"),
            "exceptions.py must re-point __module__ at the public package, got:\n{result}",
        );
        assert!(
            result.contains("__name__.rsplit"),
            "exceptions.py must derive the public package from __name__, got:\n{result}",
        );
    }

    /// gen_exceptions_py with no errors produces a file with only the header.
    #[test]
    fn gen_exceptions_py_empty_api_produces_header_only() {
        let api = empty_api();
        let result = gen_exceptions_py(&api, "_native");
        assert!(result.contains("Exception hierarchy"));
        assert!(!result.contains("class "));
        assert!(!result.contains("from ._native import"));
    }

    /// gen_init_py with no types produces a file with version and empty __all__.
    #[test]
    fn gen_init_py_empty_api_has_version() {
        let api = empty_api();
        let dto = DtoConfig::default();
        let extra = std::collections::BTreeMap::new();
        let caps = std::collections::HashMap::new();
        let adapters = vec![];
        let opaque = std::collections::HashMap::new();
        let result = gen_init_py(
            &api,
            "_mod",
            "1.2.3",
            &dto,
            &[],
            &[],
            &extra,
            &caps,
            &adapters,
            &opaque,
            &ahash::AHashSet::new(),
        );
        assert!(result.contains("__version__ = \"1.2.3\""));
        assert!(result.contains("__all__"));
    }

    /// When multiple error enums contribute variants that map to the same Python exception
    /// name (e.g. two enums each with a `Validation` variant → `ValidationError`), the
    /// `from .exceptions import (...)` block and `__all__` must each list every symbol once.
    /// Duplicate imports trip ruff F811/I001 and break `alef verify`.
    #[test]
    fn gen_init_py_dedups_overlapping_exception_names() {
        use crate::core::ir::{ErrorDef, ErrorVariant};

        let make_variant = |name: &str| ErrorVariant {
            name: name.to_string(),
            message_template: None,
            fields: vec![],
            has_source: false,
            has_from: false,
            is_unit: true,
            is_tuple: false,
            doc: String::new(),
        };
        let make_error = |name: &str, variants: Vec<&str>| ErrorDef {
            name: name.to_string(),
            rust_path: format!("lib::{name}"),
            original_rust_path: String::new(),
            variants: variants.into_iter().map(make_variant).collect(),
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut api = empty_api();
        // Two distinct error enums whose variants collide on Python exception names.
        api.errors.push(make_error(
            "ParseError",
            vec!["Validation", "ComplexityLimitExceeded", "DepthLimitExceeded"],
        ));
        api.errors.push(make_error(
            "QueryError",
            vec!["Validation", "ComplexityLimitExceeded", "DepthLimitExceeded"],
        ));

        let dto = DtoConfig::default();
        let extra = std::collections::BTreeMap::new();
        let caps = std::collections::HashMap::new();
        let adapters = vec![];
        let opaque = std::collections::HashMap::new();
        let result = gen_init_py(
            &api,
            "_mod",
            "1.2.3",
            &dto,
            &[],
            &[],
            &extra,
            &caps,
            &adapters,
            &opaque,
            &ahash::AHashSet::new(),
        );

        for symbol in [
            "ValidationError",
            "ComplexityLimitExceededError",
            "DepthLimitExceededError",
        ] {
            // Inside the exceptions import block, each symbol must appear exactly once.
            let import_occurrences = result.matches(&format!("    {symbol},\n")).count();
            assert_eq!(
                import_occurrences, 1,
                "{symbol} must be imported once, got {import_occurrences} in:\n{result}",
            );
            // In __all__ each symbol must appear exactly once.
            let all_occurrences = result.matches(&format!("\"{symbol}\"")).count();
            assert_eq!(
                all_occurrences, 1,
                "{symbol} must appear once in __all__, got {all_occurrences} in:\n{result}",
            );
        }
    }

    /// `extra_init_imports` adds the import lines and the symbols into `__all__`.
    #[test]
    fn gen_init_py_extra_imports_are_emitted_and_in_all() {
        let api = empty_api();
        let dto = DtoConfig::default();
        let mut extra = std::collections::BTreeMap::new();
        extra.insert(
            "._supported_languages".to_string(),
            vec!["SupportedLanguage".to_string()],
        );
        let caps = std::collections::HashMap::new();
        let adapters = vec![];
        let opaque = std::collections::HashMap::new();
        let result = gen_init_py(
            &api,
            "_mod",
            "1.2.3",
            &dto,
            &[],
            &[],
            &extra,
            &caps,
            &adapters,
            &opaque,
            &ahash::AHashSet::new(),
        );
        assert!(
            result.contains("from ._supported_languages import SupportedLanguage"),
            "missing import line in:\n{result}",
        );
        assert!(
            result.contains("\"SupportedLanguage\""),
            "SupportedLanguage missing from __all__ in:\n{result}",
        );
    }
}
