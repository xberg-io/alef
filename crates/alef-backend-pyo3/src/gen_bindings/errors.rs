//! Python exception hierarchy and `__init__.py` generation.

use ahash::AHashSet;
use alef_codegen::doc_emission::doc_first_paragraph_joined;
use alef_codegen::generators;
use alef_core::config::{DtoConfig, PythonDtoStyle};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;

use super::enums::{class_name_to_docstring, sanitize_python_doc};

/// Generate exceptions.py — exception hierarchy from IR error definitions.
/// Appends "Error" suffix to variant names that don't already have it (N818 compliance).
/// Prefixes names that would shadow Python builtins (A004 compliance).
pub(super) fn gen_exceptions_py(api: &ApiSurface) -> String {
    let mut out = String::with_capacity(1024);
    let mut seen_classes: AHashSet<String> = AHashSet::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str("\"\"\"Exception hierarchy.\"\"\"\n\n\n");

    for error in &api.errors {
        // Base exception class
        if !seen_classes.insert(error.name.clone()) {
            continue; // skip duplicate base class
        }
        let doc = if !error.doc.is_empty() {
            let first_line = sanitize_python_doc(&doc_first_paragraph_joined(&error.doc));
            if first_line.ends_with('.') {
                first_line
            } else {
                format!("{}.", first_line)
            }
        } else {
            class_name_to_docstring(&error.name)
        };
        out.push_str(&crate::template_env::render(
            "exception_base_class.jinja",
            minijinja::context! { name => &error.name, doc => doc },
        ));

        // Per-variant exception subclasses
        for variant in &error.variants {
            let variant_name = alef_codegen::error_gen::python_exception_name(&variant.name, &error.name);
            if !seen_classes.insert(variant_name.clone()) {
                continue; // skip duplicate variant class
            }
            let doc = if !variant.doc.is_empty() {
                let first_line = sanitize_python_doc(&doc_first_paragraph_joined(&variant.doc));
                if first_line.ends_with('.') {
                    first_line
                } else {
                    format!("{}.", first_line)
                }
            } else {
                class_name_to_docstring(&variant_name)
            };
            out.push_str(&crate::template_env::render(
                "exception_variant_class.jinja",
                minijinja::context! { name => &variant_name, base => &error.name, doc => doc },
            ));
        }
    }

    out
}

/// Generate __init__.py — re-exports and version.
/// Only exports user-facing types (not internal Update types or all enums).
pub(super) fn gen_init_py(
    api: &ApiSurface,
    module_name: &str,
    version: &str,
    dto: &DtoConfig,
    trait_bridges: &[alef_core::config::TraitBridgeConfig],
) -> String {
    use alef_core::ir::TypeRef;

    let mut out = String::with_capacity(1024);
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str(&crate::template_env::render(
        "init_header.jinja",
        minijinja::context! { version => version },
    ));

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
    // module. When not using TypedDict style (which emits a structural type in options.py),
    // they must be re-exported from the native module — not from .options — so that the
    // type seen by static analysis tools matches the actual runtime object returned by functions.
    let mut native_return_types: Vec<String> = Vec::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default && !typ.name.ends_with("Update") && !typ.fields.is_empty() {
            let is_native_return = typ.is_return_type && output_style != PythonDtoStyle::TypedDict;
            if is_native_return {
                native_return_types.push(typ.name.clone());
            } else {
                config_types.push(typ.name.clone());
            }
            // Collect enum references regardless of whether the type is a return type or config
            // type — some enums are shared across both categories.
            for field in &typ.fields {
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
    // Update types are internal; skip them.
    for typ in api.types.iter().filter(|t| !t.is_trait) {
        if typ.name.ends_with("Update") {
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

    // Import functions from api (regular functions + trait-bridge registration helpers).
    // Trait-bridge register_* functions are emitted as pass-through wrappers in api.py but
    // do not appear in api.functions — add them here so __init__.py re-exports them and they
    // appear in __all__.
    {
        let mut names: Vec<_> = api.functions.iter().map(|f| f.name.clone()).collect();
        names.extend(crate::trait_bridge::collect_bridge_register_fns(trait_bridges));
        names.sort();
        names.dedup();
        imports_from_api.extend(names);
    }

    // Data enums and return types are backed by native Rust structs — import from the native module.
    needed_data_enums.sort();
    imports_from_native.extend(needed_data_enums.iter().cloned());
    native_return_types.sort();
    imports_from_native.extend(native_return_types.iter().cloned());
    imports_from_native.sort();
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
            let variant_name = alef_codegen::error_gen::python_exception_name(&variant.name, &error.name);
            exc_names.push(variant_name);
        }
    }
    exc_names.sort();
    imports_from_exceptions.extend(exc_names.clone());

    // Output imports in sorted order (by module name: api, exceptions, native, options)
    // Use multi-line format if the import line would be too long (>88 chars for ruff)
    if !imports_from_api.is_empty() {
        let import_line = format!("from .api import {}", imports_from_api.join(", "));
        if import_line.len() > 88 {
            out.push_str("from .api import (\n");
            for name in &imports_from_api {
                out.push_str(&crate::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
                out.push('\n');
            }
            out.push_str(")\n");
        } else {
            out.push_str(&crate::template_env::render(
                "trait_bridge/single_line.jinja",
                minijinja::context! { text => format!("{}\n", import_line) },
            ));
        }
    }
    if !imports_from_exceptions.is_empty() {
        let import_line = format!("from .exceptions import {}", imports_from_exceptions.join(", "));
        if import_line.len() > 88 {
            out.push_str("from .exceptions import (\n");
            for name in &imports_from_exceptions {
                out.push_str(&crate::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
                out.push('\n');
            }
            out.push_str(")\n");
        } else {
            out.push_str(&crate::template_env::render(
                "trait_bridge/single_line.jinja",
                minijinja::context! { text => format!("{}\n", import_line) },
            ));
        }
    }
    // Data enums are Rust-backed structs; re-export from the native module.
    if !imports_from_native.is_empty() {
        let import_line = format!("from .{module_name} import {}", imports_from_native.join(", "));
        if import_line.len() > 88 {
            out.push_str(&crate::template_env::render(
                "trait_bridge/single_line.jinja",
                minijinja::context! { text => format!("from .{module_name} import (\n") },
            ));
            for name in &imports_from_native {
                out.push_str(&crate::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
                out.push('\n');
            }
            out.push_str(")\n");
        } else {
            out.push_str(&crate::template_env::render(
                "trait_bridge/single_line.jinja",
                minijinja::context! { text => format!("{}\n", import_line) },
            ));
        }
    }
    if !imports_from_options.is_empty() {
        let import_line = format!("from .options import {}", imports_from_options.join(", "));
        if import_line.len() > 88 {
            out.push_str("from .options import (\n");
            for name in &imports_from_options {
                out.push_str(&crate::template_env::render(
                    "trait_bridge/indented_import_item.jinja",
                    minijinja::context! { name => name },
                ));
                out.push('\n');
            }
            out.push_str(")\n");
        } else {
            out.push_str(&crate::template_env::render(
                "trait_bridge/single_line.jinja",
                minijinja::context! { text => format!("{}\n", import_line) },
            ));
        }
    }

    // __all__
    let mut all_items = Vec::new();
    for f in &api.functions {
        all_items.push(f.name.clone());
    }
    // Include trait-bridge registration helpers in __all__ — they are exported via api.py
    // and must be discoverable from the package root.
    all_items.extend(crate::trait_bridge::collect_bridge_register_fns(trait_bridges));
    all_items.extend(needed_enums);
    all_items.extend(imports_from_native.iter().cloned());
    all_items.extend(config_types);
    all_items.extend(exc_names);
    all_items.sort();
    all_items.dedup();

    out.push_str("\n__all__ = [\n");
    for name in &all_items {
        out.push_str(&crate::template_env::render(
            "init_all_entry.jinja",
            minijinja::context! { name => name },
        ));
    }
    out.push_str("]\n\n");
    out.push_str(&crate::template_env::render(
        "version_declaration.jinja",
        minijinja::context! { version => version },
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::{gen_exceptions_py, gen_init_py};
    use alef_core::config::DtoConfig;
    use alef_core::ir::ApiSurface;

    fn empty_api() -> ApiSurface {
        ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        }
    }

    /// gen_exceptions_py with no errors produces a file with only the header.
    #[test]
    fn gen_exceptions_py_empty_api_produces_header_only() {
        let api = empty_api();
        let result = gen_exceptions_py(&api);
        assert!(result.contains("Exception hierarchy"));
        assert!(!result.contains("class "));
    }

    /// gen_init_py with no types produces a file with version and empty __all__.
    #[test]
    fn gen_init_py_empty_api_has_version() {
        let api = empty_api();
        let dto = DtoConfig::default();
        let result = gen_init_py(&api, "_mod", "1.2.3", &dto, &[]);
        assert!(result.contains("__version__ = \"1.2.3\""));
        assert!(result.contains("__all__"));
    }
}
