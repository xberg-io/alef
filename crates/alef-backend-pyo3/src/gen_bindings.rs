use crate::type_map::Pyo3Mapper;
use ahash::AHashSet;
use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators::{self, AsyncPattern, RustBindingConfig};
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{
    AdapterPattern, AlefConfig, DtoConfig, Language, PythonDtoStyle, detect_serde_available, resolve_output_dir,
};
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

/// Convert an identifier to be safe for use as a Python attribute name by appending `_`
/// to reserved keywords and builtins that cannot be used as identifiers.
fn python_safe_name(name: &str) -> String {
    const PYTHON_KEYWORDS: &[&str] = &[
        "from", "import", "class", "def", "return", "yield", "pass", "break", "continue", "and", "or", "not", "is",
        "in", "if", "else", "elif", "for", "while", "with", "as", "try", "except", "finally", "raise", "del", "global",
        "nonlocal", "lambda", "assert", "type", // Python builtins that cannot be used as identifiers
        "None", "True", "False",
    ];
    if PYTHON_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

pub struct Pyo3Backend;

impl Pyo3Backend {
    fn binding_config(core_import: &str, has_serde: bool) -> RustBindingConfig<'_> {
        RustBindingConfig {
            struct_attrs: &["pyclass(frozen, from_py_object)"],
            field_attrs: &["pyo3(get)"],
            struct_derives: &["Clone"],
            method_block_attr: Some("pymethods"),
            constructor_attr: "#[new]",
            static_attr: Some("staticmethod"),
            function_attr: "#[pyfunction]",
            enum_attrs: &["pyclass(eq, eq_int, from_py_object)"],
            enum_derives: &["Clone", "PartialEq"],
            needs_signature: true,
            signature_prefix: "    #[pyo3(signature = (",
            signature_suffix: "))]",
            core_import,
            async_pattern: AsyncPattern::Pyo3FutureIntoPy,
            has_serde,
            type_name_prefix: "",
            // Duration fields on has_default types become Option<u64> so that unset fields
            // fall back to the core type's Default rather than Duration::ZERO.
            option_duration_on_defaults: true,
        }
    }
}

impl Backend for Pyo3Backend {
    fn name(&self) -> &str {
        "pyo3"
    }

    fn language(&self) -> Language {
        Language::Python
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let mapper = Pyo3Mapper;
        let core_import = config.core_import();

        // Detect serde availability from the output crate's Cargo.toml
        let output_dir = resolve_output_dir(
            config.output.python.as_ref(),
            &config.crate_config.name,
            "crates/{name}-py/src/",
        );
        let has_serde = detect_serde_available(&output_dir);
        let cfg = Self::binding_config(&core_import, has_serde);

        // Build adapter body map for method body substitution
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Python)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        // Suppress documentation and cast lints in generated code — doc comments are provided
        // by Python stubs (.pyi), and the numeric casts are intentional FFI conversions.
        builder.add_inner_attribute("allow(missing_docs)");
        // PyO3 0.22+ deprecates auto-derived FromPyObject; silence until upstream stabilises.
        builder.add_inner_attribute("allow(deprecated, dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute(
            "allow(clippy::default_trait_access, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::let_unit_value, clippy::needless_borrow, clippy::too_many_arguments)",
        );
        builder.add_import("pyo3::prelude::*");
        // Note: core_import and path_mapping crates are referenced via fully-qualified paths
        // in generated code (e.g. `core_import::TypeName`), so no bare `use crate_name;`
        // import is needed — that would trigger clippy::single_component_path_imports.

        // Import serde_json when available (needed for serde-based param conversion)
        if has_serde {
            builder.add_import("serde_json");
        }

        // Import traits needed for trait method dispatch
        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }
        // Core crate types are referenced via fully-qualified paths (e.g.
        // `html_to_markdown_rs::ConversionOptions`) in generated code, so no
        // named or glob imports from the core crate are needed.  Importing
        // core type names would shadow the local PyO3 wrapper structs that
        // share the same names, causing compilation errors.
        // Node and WASM backends already follow this fully-qualified pattern.

        // Check if we have non-sanitized async functions (sanitized async methods produce stubs, not async code)
        let has_async = api.functions.iter().any(|f| f.is_async && !f.sanitized)
            || api
                .types
                .iter()
                .any(|t| t.methods.iter().any(|m| m.is_async && !m.sanitized));
        if has_async {
            builder.add_import("pyo3_async_runtimes");
            // PyRuntimeError is needed for async error mapping via PyErr::new::<PyRuntimeError, _>
            let has_async_error = api
                .functions
                .iter()
                .any(|f| f.is_async && !f.sanitized && f.error_type.is_some())
                || api.types.iter().any(|t| {
                    t.methods
                        .iter()
                        .any(|m| m.is_async && !m.sanitized && m.error_type.is_some())
                });
            if has_async_error {
                builder.add_import("pyo3::exceptions::PyRuntimeError");
            }
        }

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
        }

        // Check if we have Map types and add HashMap import if needed
        let has_maps = api.types.iter().any(|t| {
            t.fields
                .iter()
                .any(|f| matches!(&f.ty, alef_core::ir::TypeRef::Map(_, _)))
        }) || api.functions.iter().any(|f| {
            f.params
                .iter()
                .any(|p| matches!(&p.ty, alef_core::ir::TypeRef::Map(_, _)))
                || matches!(&f.return_type, alef_core::ir::TypeRef::Map(_, _))
        });
        if has_maps {
            builder.add_import("std::collections::HashMap");
        }

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::Python);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        // Add adapter-generated standalone items (streaming iterators, callback bridges)
        for adapter in &config.adapters {
            match adapter.pattern {
                AdapterPattern::Streaming => {
                    let key = format!("{}.__stream_struct__", adapter.item_type.as_deref().unwrap_or(""));
                    if let Some(struct_code) = adapter_bodies.get(&key) {
                        builder.add_item(struct_code);
                    }
                }
                AdapterPattern::CallbackBridge => {
                    let struct_key = format!("{}.__bridge_struct__", adapter.name);
                    let impl_key = format!("{}.__bridge_impl__", adapter.name);
                    if let Some(struct_code) = adapter_bodies.get(&struct_key) {
                        builder.add_item(struct_code);
                    }
                    if let Some(impl_code) = adapter_bodies.get(&impl_key) {
                        builder.add_item(impl_code);
                    }
                }
                _ => {}
            }
        }

        // Collect error type names — these are handled by create_exception! below and must not
        // also be generated as #[pyclass] structs (doing both causes E0428/E0119/E0592).
        let error_type_names: AHashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            // Error types are emitted as pyo3::create_exception! macros, not as pyclass structs.
            if error_type_names.contains(typ.name.as_str()) {
                continue;
            }
            if typ.is_opaque {
                builder.add_item(&generators::gen_opaque_struct(typ, &cfg));
                let impl_block = generators::gen_opaque_impl_block(typ, &mapper, &cfg, &opaque_types, &adapter_bodies);
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
            } else {
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // so no separate Default impl is needed.
                builder.add_item(&generators::gen_struct(typ, &mapper, &cfg));
                let impl_block = generators::gen_impl_block(typ, &mapper, &cfg, &adapter_bodies, &opaque_types);
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
            }
        }
        for e in &api.enums {
            if generators::enum_has_data_variants(e) {
                builder.add_item(&generators::gen_pyo3_data_enum(e, &core_import));
            } else {
                builder.add_item(&generators::gen_enum(e, &cfg));
            }
        }
        for f in &api.functions {
            builder.add_item(&generators::gen_function(
                f,
                &mapper,
                &cfg,
                &adapter_bodies,
                &opaque_types,
            ));
        }

        // Trait bridge wrappers — generate PyO3 bridge structs that delegate to Python objects
        if !config.trait_bridges.is_empty() {
            // Add imports needed by trait bridge generated code
            builder.add_import("async_trait::async_trait");
            // std::sync::Arc is already conditionally imported above for opaque types;
            // ensure it's present for trait bridges too.
            if opaque_types.is_empty() {
                builder.add_import("std::sync::Arc");
            }
            for bridge_cfg in &config.trait_bridges {
                if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                    let bridge_code = crate::trait_bridge::gen_trait_bridge(trait_type, bridge_cfg, &core_import, api);
                    builder.add_item(&bridge_code);
                }
            }
        }

        // Error types (create_exception! macros + converter functions)
        let module_name = config.python_module_name();
        for error in &api.errors {
            builder.add_item(&alef_codegen::error_gen::gen_pyo3_error_types(error, &module_name));
            builder.add_item(&alef_codegen::error_gen::gen_pyo3_error_converter(error, &core_import));
        }

        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        let pyo3_conversion_cfg = alef_codegen::conversions::ConversionConfig {
            option_duration_on_defaults: true,
            ..Default::default()
        };
        // From/Into conversions — separate sets for each direction
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            // binding→core: strict (no sanitized fields)
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &pyo3_conversion_cfg,
                ));
            }
            // core→binding: permissive (sanitized fields use format!("{:?}"))
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &pyo3_conversion_cfg,
                ));
            }
        }
        for e in &api.enums {
            // Data enums generate their own From impls inside gen_pyo3_data_enum; skip here.
            if generators::enum_has_data_variants(e) {
                continue;
            }
            // Binding→core: only for enums with simple fields (Default::default() must work)
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core(
                    e,
                    &core_import,
                ));
            }
            // Core→binding: always possible (data variants discarded with `..`)
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding(
                    e,
                    &core_import,
                ));
            }
        }

        // Async runtime initialization (if needed)
        if has_async {
            builder.add_item(&gen_async_runtime_init());
        }

        // Module init
        builder.add_item(&gen_module_init(&config.python_module_name(), api, config));

        let content = builder.build();

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_type_stubs(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let stubs_config = match config.python.as_ref().and_then(|c| c.stubs.as_ref()) {
            Some(s) => s,
            None => return Ok(vec![]),
        };

        let content = crate::gen_stubs::gen_stubs(api);

        let stubs_path = resolve_output_dir(
            Some(&stubs_config.output),
            &config.crate_config.name,
            stubs_config.output.to_string_lossy().as_ref(),
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&stubs_path).join(format!("{}.pyi", config.python_module_name())),
            content,
            generated_header: true,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_name = config.python_module_name();

        // Use stubs output path as the package directory (e.g., packages/python/html_to_markdown/)
        // This ensures we write to the correct Python package, not the Rust crate name.
        let output_base = config
            .python
            .as_ref()
            .and_then(|p| p.stubs.as_ref())
            .map(|s| PathBuf::from(&s.output))
            .unwrap_or_else(|| {
                let package_name = config.crate_config.name.replace('-', "_");
                PathBuf::from(format!("packages/python/{}", package_name))
            });
        let package_name = output_base
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| config.crate_config.name.replace('-', "_"));

        let mut files = vec![];

        // 1. Generate options.py (enums and dataclasses)
        let options_content = gen_options_py(api, &package_name, &config.dto);
        files.push(GeneratedFile {
            path: output_base.join("options.py"),
            content: options_content,
            generated_header: true,
        });

        // 2. Generate api.py (wrapper functions)
        let api_content = gen_api_py(api, &module_name, &package_name);
        files.push(GeneratedFile {
            path: output_base.join("api.py"),
            content: api_content,
            generated_header: true,
        });

        // 3. Generate exceptions.py (exception hierarchy)
        let exceptions_content = gen_exceptions_py(api);
        files.push(GeneratedFile {
            path: output_base.join("exceptions.py"),
            content: exceptions_content,
            generated_header: true,
        });

        // 4. Generate __init__.py (re-exports)
        let init_content = gen_init_py(api, &module_name, &api.version, &config.dto);
        files.push(GeneratedFile {
            path: output_base.join("__init__.py"),
            content: init_content,
            generated_header: true,
        });

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "maturin",
            crate_suffix: "-py",
            depends_on_ffi: false,
            post_build: vec![],
        })
    }
}

/// Generate options.py — Python-side enums (StrEnum) and @dataclass / TypedDict config types.
///
/// Enum fields in dataclasses use `str` type (not enum class) so users can pass
/// plain strings like `"atx"` instead of `HeadingStyle.Atx`.
/// Default values come from `typed_default` if available, otherwise type-appropriate zeros.
///
/// When `dto.python_output_style() == TypedDict` and a type has `is_return_type = true`,
/// it is emitted as a `TypedDict` (with `total=False`) instead of a `@dataclass`.
fn gen_options_py(api: &ApiSurface, _package_name: &str, dto: &DtoConfig) -> String {
    use alef_core::ir::TypeRef;
    use heck::{ToShoutySnakeCase, ToSnakeCase};

    // Determine whether any type will be emitted as TypedDict so we know which imports to add.
    let output_style = dto.python_output_style();
    let any_typeddict = output_style == PythonDtoStyle::TypedDict
        && api
            .types
            .iter()
            .any(|t| t.has_default && t.is_return_type && !t.fields.is_empty() && !t.name.ends_with("Update"));

    let mut out = String::with_capacity(4096);
    out.push_str("# This file is auto-generated by alef. DO NOT EDIT.\n");
    out.push_str("\"\"\"Configuration options for the conversion API.\"\"\"\n\n");
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from dataclasses import dataclass, field\n");
    out.push_str("from enum import Enum\n");
    if any_typeddict {
        out.push_str("from typing import Any, TypedDict\n\n\n");
    } else {
        out.push_str("from typing import Any\n\n\n");
    }

    // Collect enum names for type detection (plain unit enums vs data enums)
    let enum_names: std::collections::HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    // Data enums (tagged unions) are exposed as dict-accepting structs, not str enums.
    let data_enum_names: std::collections::HashSet<String> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.clone())
        .collect();

    // Collect all Named types referenced by has_default types
    let mut referenced_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default {
            for field in &typ.fields {
                if let TypeRef::Named(name) = &field.ty {
                    referenced_types.insert(name.clone());
                } else if let TypeRef::Optional(inner) = &field.ty {
                    if let TypeRef::Named(name) = inner.as_ref() {
                        referenced_types.insert(name.clone());
                    }
                }
            }
        }
    }

    // Generate only "public" enums — skip internal types like TextDirection, LinkType etc.
    // that aren't part of the user-facing config API.
    // Only generate enums referenced by has_default type fields.
    let mut needed_enums: std::collections::HashSet<String> = std::collections::HashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default {
            for field in &typ.fields {
                if let TypeRef::Named(name) = &field.ty {
                    if enum_names.contains(name) {
                        needed_enums.insert(name.clone());
                    }
                } else if let TypeRef::Optional(inner) = &field.ty {
                    if let TypeRef::Named(name) = inner.as_ref() {
                        if enum_names.contains(name) {
                            needed_enums.insert(name.clone());
                        }
                    }
                }
            }
        }
    }

    // Build map of enum name → default variant string value.
    // Uses the variant with is_default=true (#[default] attr), falls back to first variant.
    let enum_defaults: std::collections::HashMap<String, String> = api
        .enums
        .iter()
        .filter_map(|e| {
            let default_v = e.variants.iter().find(|v| v.is_default).or(e.variants.first());
            default_v.map(|v| (e.name.clone(), v.name.to_snake_case()))
        })
        .collect();

    for enum_def in &api.enums {
        if !needed_enums.contains(&enum_def.name) {
            continue;
        }
        // Data enums are dict-accepting structs on the Rust side; skip str,Enum generation.
        if data_enum_names.contains(&enum_def.name) {
            continue;
        }
        out.push_str(&format!("class {}(str, Enum):\n", enum_def.name));
        let enum_doc = if !enum_def.doc.is_empty() {
            enum_def.doc.lines().next().unwrap_or("").to_string()
        } else {
            class_name_to_docstring(&enum_def.name)
        };
        out.push_str(&format!("    \"\"\"{enum_doc}\"\"\"\n\n"));
        for variant in &enum_def.variants {
            let value = variant.name.to_snake_case();
            out.push_str(&format!(
                "    {} = \"{}\"\n",
                variant.name.to_shouty_snake_case(),
                value
            ));
        }
        out.push_str("\n\n");
    }

    // Generate stub classes for non-enum Named types that are referenced
    for type_name in &referenced_types {
        if !enum_names.contains(type_name) && !api.types.iter().any(|t| &t.name == type_name && t.has_default) {
            out.push_str(&format!("class {}:\n", type_name));
            out.push_str(&format!("    \"\"\"Placeholder for {} type.\"\"\"\n", type_name));
            out.push_str("\n\n");
        }
    }

    // Generate @dataclass or TypedDict for types with has_default (user-facing config types)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if !typ.has_default || typ.fields.is_empty() {
            continue;
        }
        // Skip "Update" types — they're internal
        if typ.name.ends_with("Update") {
            continue;
        }

        // Use TypedDict for return types when the output style is configured as TypedDict.
        let use_typeddict = output_style == PythonDtoStyle::TypedDict && typ.is_return_type;

        // Return types are defined authoritatively by the Rust native module as #[pyclass]
        // structs. Emitting a @dataclass with the same name creates a shadow class that breaks
        // static analysis — Pylance reports a type mismatch because the @dataclass and the
        // native PyO3 class are unrelated types even though they share a name.
        // Only emit a TypedDict when explicitly configured; otherwise skip entirely.
        if typ.is_return_type && !use_typeddict {
            continue;
        }

        if use_typeddict {
            out.push_str(&gen_typeddict(typ, &enum_names, &data_enum_names));
        } else {
            out.push_str("@dataclass\n");
            out.push_str(&format!("class {}:\n", typ.name));
            let class_doc = if !typ.doc.is_empty() {
                typ.doc.lines().next().unwrap_or("").to_string()
            } else {
                class_name_to_docstring(&typ.name)
            };
            out.push_str(&format!("    \"\"\"{class_doc}\"\"\"\n\n"));

            for field in &typ.fields {
                // Determine Python type hint
                let type_hint = python_field_type(&field.ty, field.optional, &enum_names, &data_enum_names);

                // Determine default value and check if we need | None
                let (type_hint_with_none, default) = if let Some(td) = &field.typed_default {
                    // For optional fields with Empty default, use None — not a zero value.
                    // This ensures Option<usize> defaults to None (not 0), preventing
                    // "max_concurrent must be > 0" validation errors.
                    let default = if field.optional && matches!(td, alef_core::ir::DefaultValue::Empty) {
                        "None".to_string()
                    } else {
                        typed_default_to_python(td, &field.ty, &enum_defaults, &data_enum_names)
                    };
                    // When the effective default is None (e.g. Duration with Empty typed_default),
                    // add | None to the type hint so the annotation matches the default value.
                    let hint = if default == "None" && !type_hint.contains('|') {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (hint, default)
                } else if field.optional {
                    // If default is None but type is Named (not already Optional), add | None
                    let final_hint = if !type_hint.contains('|') && matches!(&field.ty, TypeRef::Named(_)) {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (final_hint, "None".to_string())
                } else {
                    let default = python_zero_value(&field.ty, &enum_names, &data_enum_names);
                    // When the zero value is None (e.g. data enum fields), add | None so the
                    // annotation matches — `dict[str, Any] = None` is a mypy type error.
                    let hint = if default == "None" && !type_hint.contains('|') {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (hint, default)
                };

                let safe_name = python_safe_name(&field.name);
                if !field.doc.is_empty() {
                    out.push_str(&format!("    {}: {} = {}\n", safe_name, type_hint_with_none, default));
                    out.push_str(&format!(
                        "    \"\"\"{}\"\"\"\n\n",
                        field.doc.lines().next().unwrap_or("")
                    ));
                } else {
                    out.push_str(&format!("    {}: {} = {}\n", safe_name, type_hint_with_none, default));
                }
            }
            out.push('\n');
        }
    }

    out
}

/// Generate a `TypedDict` class for a return type.
///
/// TypedDict is emitted with `total=False` because all fields are optional at the
/// call site — the caller may receive only a subset of keys.  Default values are
/// not supported by TypedDict, so we only emit field name + type hint.
///
/// ```python
/// class ConversionResult(TypedDict, total=False):
///     """One-line doc."""
///
///     content: str | None
///     tables: list[ExtractedTable]
/// ```
fn gen_typeddict(
    typ: &alef_core::ir::TypeDef,
    enum_names: &std::collections::HashSet<String>,
    data_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("class {}(TypedDict, total=False):\n", typ.name));
    let typeddict_doc = if !typ.doc.is_empty() {
        typ.doc.lines().next().unwrap_or("").to_string()
    } else {
        class_name_to_docstring(&typ.name)
    };
    out.push_str(&format!("    \"\"\"{typeddict_doc}\"\"\"\n\n"));
    for field in &typ.fields {
        let type_hint = python_field_type(&field.ty, field.optional, enum_names, data_enum_names);
        // Ensure Optional-like fields always include `| None`
        let type_hint_with_none = if field.optional && !type_hint.contains('|') {
            if matches!(&field.ty, alef_core::ir::TypeRef::Named(_)) {
                format!("{} | None", type_hint)
            } else {
                type_hint
            }
        } else {
            type_hint
        };
        let safe_name = python_safe_name(&field.name);
        if !field.doc.is_empty() {
            out.push_str(&format!("    {}: {}\n", safe_name, type_hint_with_none));
            out.push_str(&format!(
                "    \"\"\"{}\"\"\"\n\n",
                field.doc.lines().next().unwrap_or("")
            ));
        } else {
            out.push_str(&format!("    {}: {}\n", safe_name, type_hint_with_none));
        }
    }
    out.push('\n');
    out
}

/// Map IR TypeRef to Python type hint string for dataclass fields.
/// Enum-typed fields become `str` (users pass string literals).
/// Data enum-typed fields become `dict` (users pass dicts with type + fields).
/// Non-enum Named types that aren't defined become `Any` to avoid F821 errors.
fn python_field_type(
    ty: &alef_core::ir::TypeRef,
    optional: bool,
    enum_names: &std::collections::HashSet<String>,
    data_enum_names: &std::collections::HashSet<String>,
) -> String {
    use alef_core::ir::TypeRef;
    let base = match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "bool".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "float".to_string(),
            _ => "int".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "str".to_string(),
        TypeRef::Bytes => "bytes".to_string(),
        TypeRef::Vec(inner) => format!("list[{}]", python_field_type(inner, false, enum_names, data_enum_names)),
        TypeRef::Map(k, v) => format!(
            "dict[{}, {}]",
            python_field_type(k, false, enum_names, data_enum_names),
            python_field_type(v, false, enum_names, data_enum_names)
        ),
        // Data enums: users pass a dict with a "type" discriminator and fields.
        TypeRef::Named(name) if data_enum_names.contains(name) => "dict[str, Any]".to_string(),
        TypeRef::Named(name) if enum_names.contains(name) => "str".to_string(),
        TypeRef::Named(_name) => "Any".to_string(), // Use Any for undefined types to avoid F821
        TypeRef::Optional(inner) => {
            return format!(
                "{} | None",
                python_field_type(inner, false, enum_names, data_enum_names)
            );
        }
        TypeRef::Unit => "None".to_string(),
        TypeRef::Duration => "int".to_string(),
    };
    if optional { format!("{} | None", base) } else { base }
}

/// Convert a typed default value to Python literal.
/// For `Empty` on enum-typed fields, resolves to the enum's default (first) variant.
/// For `Empty` on data enum-typed fields, resolves to None (no sensible default dict).
fn typed_default_to_python(
    td: &alef_core::ir::DefaultValue,
    ty: &alef_core::ir::TypeRef,
    enum_defaults: &std::collections::HashMap<String, String>,
    data_enum_names: &std::collections::HashSet<String>,
) -> String {
    use alef_core::ir::{DefaultValue, TypeRef};
    match td {
        DefaultValue::BoolLiteral(true) => "True".to_string(),
        DefaultValue::BoolLiteral(false) => "False".to_string(),
        DefaultValue::StringLiteral(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('\"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r");
            format!("\"{}\"", escaped)
        }
        DefaultValue::IntLiteral(i) => i.to_string(),
        DefaultValue::FloatLiteral(f) => format!("{}", f),
        DefaultValue::EnumVariant(v) => {
            use heck::ToSnakeCase;
            format!("\"{}\"", v.to_snake_case())
        }
        DefaultValue::Empty => {
            // For data enum-typed fields, use None (no sensible default dict).
            if let TypeRef::Named(name) = ty {
                if data_enum_names.contains(name) {
                    return "None".to_string();
                }
            }
            // For plain enum-typed fields, resolve to the default variant's string value.
            // For other Named types, use None (Rust binding applies its own default).
            if let TypeRef::Named(name) = ty {
                if let Some(default_variant) = enum_defaults.get(name) {
                    return format!("\"{}\"", default_variant);
                }
            }
            // Type-appropriate zero values for Python
            match ty {
                TypeRef::Primitive(p) => match p {
                    alef_core::ir::PrimitiveType::Bool => "False".to_string(),
                    alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
                    _ => "0".to_string(),
                },
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"".to_string(),
                TypeRef::Bytes => "b\"\"".to_string(),
                // Duration fields with Empty default are Option<u64> in the binding;
                // use None so the core type's Default provides the real default value.
                TypeRef::Duration => "None".to_string(),
                TypeRef::Vec(_) => "field(default_factory=list)".to_string(),
                TypeRef::Map(_, _) => "field(default_factory=dict)".to_string(),
                _ => "None".to_string(),
            }
        }
        DefaultValue::None => "None".to_string(),
    }
}

/// Generate a Python zero value for a type (when no typed_default is available).
fn python_zero_value(
    ty: &alef_core::ir::TypeRef,
    enum_names: &std::collections::HashSet<String>,
    data_enum_names: &std::collections::HashSet<String>,
) -> String {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "False".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"".to_string(),
        TypeRef::Bytes => "b\"\"".to_string(),
        TypeRef::Vec(_) => "field(default_factory=list)".to_string(),
        TypeRef::Map(_, _) => "field(default_factory=dict)".to_string(),
        // Data enums have no simple zero value; default to None (they're typically Optional).
        TypeRef::Named(name) if data_enum_names.contains(name) => "None".to_string(),
        TypeRef::Named(name) if enum_names.contains(name) => "\"\"".to_string(),
        TypeRef::Named(_) => "None".to_string(),
        TypeRef::Optional(_) => "None".to_string(),
        TypeRef::Unit => "None".to_string(),
        // Duration fields are stored as Option<u64> in has_default binding structs,
        // so None is the correct zero value (falls back to core Default).
        TypeRef::Duration => "None".to_string(),
    }
}

/// Recursively collect all Named type references from a TypeRef.
fn collect_named_types(ty: &alef_core::ir::TypeRef, out: &mut std::collections::BTreeSet<String>) {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) => {
            out.insert(n.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Generate api.py — wrapper functions that convert Python types to Rust binding types.
///
/// For each function parameter whose type is a `has_default` struct (e.g. `ConversionOptions`),
/// we generate a `_to_rust_{snake_name}` converter that maps the Python `@dataclass` instance
/// to the Rust binding's pyclass by passing every field as a keyword argument.
fn gen_api_py(api: &ApiSurface, module_name: &str, package_name: &str) -> String {
    use alef_core::ir::TypeRef;
    use heck::ToSnakeCase;

    // Build lookup: type_name → TypeDef for has_default types
    let default_types: std::collections::HashMap<String, &alef_core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update"))
        .map(|t| (t.name.clone(), t))
        .collect();

    // Collect enum names for conversion detection
    let enum_names: std::collections::HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Separate data enums (tagged unions exposed as dict-accepting structs) from simple int enums.
    // Data enums are passed through as dicts; simple enums need string→variant lookup.
    let data_enum_names: std::collections::HashSet<String> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.clone())
        .collect();

    // Build lookup: simple enum name → EnumDef (for generating value→Rust-variant maps).
    let simple_enum_defs: std::collections::HashMap<String, &alef_core::ir::EnumDef> = api
        .enums
        .iter()
        .filter(|e| !generators::enum_has_data_variants(e))
        .map(|e| (e.name.clone(), e))
        .collect();

    // Determine which has_default types are referenced by function parameters (directly or nested)
    let mut needed_converters: Vec<String> = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn collect_needed(
        type_name: &str,
        default_types: &std::collections::HashMap<String, &alef_core::ir::TypeDef>,
        needed: &mut Vec<String>,
        visited: &mut std::collections::HashSet<String>,
    ) {
        if !visited.insert(type_name.to_string()) {
            return;
        }
        if let Some(typ) = default_types.get(type_name) {
            // First collect nested types so they appear before the parent converter
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
                    if default_types.contains_key(name) {
                        collect_needed(name, default_types, needed, visited);
                    }
                }
            }
            needed.push(type_name.to_string());
        }
    }

    for func in &api.functions {
        for param in &func.params {
            let type_name = match &param.ty {
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
            if let Some(name) = type_name {
                collect_needed(name, &default_types, &mut needed_converters, &mut visited);
            }
        }
    }

    // Collect all type names referenced in function signatures (params + returns)
    // that aren't converters — these need to be imported too.
    let mut all_type_imports: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for type_name in &needed_converters {
        all_type_imports.insert(type_name.clone());
    }
    for func in &api.functions {
        for param in &func.params {
            collect_named_types(&param.ty, &mut all_type_imports);
        }
        // Collect return type references so they are imported and can be used as bare
        // names in annotations. This avoids `_rust.`-prefixed return types which cause
        // type checkers to see a different type than the public re-export.
        collect_named_types(&func.return_type, &mut all_type_imports);
    }

    let mut out = String::with_capacity(4096);
    out.push_str("# This file is auto-generated by alef. DO NOT EDIT.\n");
    out.push_str("\"\"\"Public API for conversion.\"\"\"\n\n");
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from typing import TYPE_CHECKING\n\n");
    out.push_str(&format!("import {package_name}.{module_name} as _rust\n"));

    // Split type imports: opaque/error types and non-options types come from the native module,
    // has_default dataclass types come from .options.
    let opaque_names: std::collections::BTreeSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let error_names: std::collections::BTreeSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    // Types that exist in options.py: has_default structs (excluding Update types and return
    // types — return types are defined in the native module, not options.py).
    let options_type_names: std::collections::BTreeSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update") && !t.is_return_type)
        .map(|t| t.name.clone())
        .collect();
    // All non-enum IR type names (used to distinguish structs from enums in classification).
    let all_ir_type_names: std::collections::BTreeSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    let enum_type_names: std::collections::BTreeSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let mut options_imports: Vec<&str> = Vec::new();
    let mut native_imports: Vec<&str> = Vec::new();
    for name in &all_type_imports {
        let is_options = options_type_names.contains(name) || enum_type_names.contains(name);
        let is_native = opaque_names.contains(name)
            || error_names.contains(name)
            || (all_ir_type_names.contains(name) && !is_options);
        if is_native {
            native_imports.push(name.as_str());
        } else {
            options_imports.push(name.as_str());
        }
    }

    if !options_imports.is_empty() || !native_imports.is_empty() {
        out.push_str("\nif TYPE_CHECKING:\n");
        // Emit native module imports before .options imports so isort (ruff I001) is satisfied.
        // `._module` sorts before `.options` alphabetically when treating `.` as a separator.
        if !native_imports.is_empty() {
            out.push_str(&format!(
                "    from .{module_name} import {}\n",
                native_imports.join(", ")
            ));
        }
        if !options_imports.is_empty() {
            out.push_str(&format!("    from .options import {}\n", options_imports.join(", ")));
        }
    }
    out.push_str("\n\n");

    // Collect simple enums referenced by converter types (for lookup map generation).
    // Walk all fields of needed_converters types to find enum references.
    let mut needed_simple_enums: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for type_name in &needed_converters {
        let typ = default_types[type_name];
        for field in &typ.fields {
            let enum_name_opt: Option<&str> = match &field.ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                TypeRef::Vec(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(name) = enum_name_opt {
                if simple_enum_defs.contains_key(name) {
                    needed_simple_enums.insert(name.to_string());
                }
            }
        }
    }

    // Emit module-level lookup dicts for simple int enums.
    // Keys are the snake_case string values that options.py assigns to Python enum members.
    // Values are the corresponding Rust binding enum variants (accessed via attribute).
    for enum_name in &needed_simple_enums {
        let enum_def = simple_enum_defs[enum_name];
        let map_name = format!("_TO_RUST_{}_MAP", enum_name.to_uppercase());
        out.push_str(&format!("{map_name} = {{\n"));
        for variant in &enum_def.variants {
            let str_value = variant.name.to_snake_case();
            out.push_str(&format!(
                "    \"{str_value}\": _rust.{enum_name}.{},\n",
                python_safe_name(&variant.name)
            ));
        }
        out.push_str("}\n\n\n");
    }

    // Generate converter functions for each needed has_default type
    for type_name in &needed_converters {
        let typ = default_types[type_name];
        let snake = type_name.to_snake_case();

        // Single-line: "def _to_rust_{snake}(value: {type_name} | None) -> _rust.{type_name} | None:"
        // Prefix "def _to_rust_" (13) + snake + "(value: " (8) + type_name + " | None) -> _rust." (18)
        // + type_name + " | None:" (8) = 47 + snake.len + 2 * type_name.len
        let sig_len = 47 + snake.len() + 2 * type_name.len();
        if sig_len > 100 {
            out.push_str(&format!(
                "def _to_rust_{snake}(\n    value: {type_name} | None,\n) -> _rust.{type_name} | None:\n"
            ));
        } else {
            out.push_str(&format!(
                "def _to_rust_{snake}(value: {type_name} | None) -> _rust.{type_name} | None:\n"
            ));
        }
        out.push_str(&format!(
            "    \"\"\"Convert Python {type_name} to Rust binding type.\"\"\"\n"
        ));
        out.push_str("    if value is None:\n");
        out.push_str("        return None\n");
        out.push_str(&format!("    return _rust.{type_name}(\n"));

        for field in &typ.fields {
            // Check if the field's type is itself a has_default Named type (needs nested conversion)
            let inner_named = match &field.ty {
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

            if let Some(nested_name) = inner_named {
                if default_types.contains_key(nested_name) {
                    let nested_snake = nested_name.to_snake_case();
                    // Non-optional fields: converter returns T | None but field expects T.
                    // Add type: ignore since the value is guaranteed non-None at runtime.
                    let ignore = if !field.optional && !matches!(&field.ty, TypeRef::Optional(_)) {
                        "  # type: ignore[arg-type]"
                    } else {
                        ""
                    };
                    out.push_str(&format!(
                        "        {}=_to_rust_{nested_snake}(value.{}),{ignore}\n",
                        field.name, field.name
                    ));
                    continue;
                }
                // Single enum field: convert str -> Rust enum
                if enum_names.contains(nested_name) {
                    if data_enum_names.contains(nested_name) {
                        // Data enum (tagged union): PyO3 constructor accepts a dict directly.
                        if matches!(&field.ty, TypeRef::Optional(_)) || field.optional {
                            out.push_str(&format!(
                                "        {name}=_rust.{enum_name}(value.{name}) if value.{name} is not None else None,\n",
                                name = field.name,
                                enum_name = nested_name,
                            ));
                        } else {
                            out.push_str(&format!(
                                "        {name}=_rust.{enum_name}(value.{name}),\n",
                                name = field.name,
                                enum_name = nested_name,
                            ));
                        }
                    } else {
                        // Simple int enum: PyO3 eq_int enums can't be constructed from a string.
                        // Look up the Rust variant by the snake_case string value that options.py produces.
                        let map_name = format!("_TO_RUST_{}_MAP", nested_name.to_uppercase());
                        if matches!(&field.ty, TypeRef::Optional(_)) || field.optional {
                            out.push_str(&format!(
                                "        {name}={map_name}[value.{name}] if value.{name} is not None else None,\n",
                                name = field.name,
                            ));
                        } else {
                            out.push_str(&format!(
                                "        {name}={map_name}[value.{name}],\n",
                                name = field.name,
                            ));
                        }
                    }
                    continue;
                }
            }

            // Vec<Enum> field: convert list[str] -> list[RustEnum]
            if let TypeRef::Vec(inner) = &field.ty {
                if let TypeRef::Named(enum_name) = inner.as_ref() {
                    if enum_names.contains(enum_name) {
                        if data_enum_names.contains(enum_name) {
                            // Data enum list: each element is a dict passed to the PyO3 constructor.
                            out.push_str(&format!(
                                "        {name}=[_rust.{enum_name}(v) for v in value.{name}],\n",
                                name = field.name,
                                enum_name = enum_name,
                            ));
                        } else {
                            // Simple int enum list: look up each element by snake_case string value.
                            let map_name = format!("_TO_RUST_{}_MAP", enum_name.to_uppercase());
                            out.push_str(&format!(
                                "        {name}=[{map_name}[str(v)] for v in value.{name}],\n",
                                name = field.name,
                            ));
                        }
                        continue;
                    }
                }
            }

            out.push_str(&format!("        {name}=value.{name},\n", name = field.name));
        }

        out.push_str("    )\n\n\n");
    }

    // Generate wrapper for each function
    for func in &api.functions {
        // Build Python-side params — required first, then optional (Python syntax rule)
        let mut sig_parts = Vec::new();
        let (required, optional): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !p.optional);
        for param in required.iter().chain(optional.iter()) {
            let py_type = if param.optional {
                format!("{} | None = None", crate::type_map::python_type(&param.ty))
            } else {
                crate::type_map::python_type(&param.ty)
            };
            sig_parts.push(format!("{}: {}", param.name, py_type));
        }

        let return_type_str = crate::type_map::python_type(&func.return_type);
        out.push_str(&format!(
            "def {}({}) -> {}:\n",
            func.name,
            sig_parts.join(", "),
            return_type_str
        ));
        {
            let doc_with_period = if !func.doc.is_empty() {
                let doc_first_line = func.doc.lines().next().unwrap_or("");
                let doc_trimmed = doc_first_line.trim();
                // `    """..."""` is 10 chars of overhead; period may add 1 more char.
                // Limit content to 89 chars so that with a trailing period the full line stays ≤100.
                let doc_content = if doc_trimmed.len() > 89 {
                    &doc_trimmed[..89]
                } else {
                    doc_trimmed
                };
                if doc_content.ends_with('.') {
                    doc_content.to_string()
                } else {
                    format!("{}.", doc_content)
                }
            } else {
                use heck::ToSnakeCase;
                let snake = func.name.to_snake_case();
                let sentence = snake.replace('_', " ");
                let mut chars = sentence.chars();
                let capitalized = match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                };
                format!("{}.", capitalized)
            };
            out.push_str(&format!("    \"\"\"{doc_with_period}\"\"\"\n"));
        }

        // For each param that has a converter, emit a local conversion variable
        let mut call_args = Vec::new();
        for param in &func.params {
            let type_name = match &param.ty {
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

            if let Some(name) = type_name {
                if default_types.contains_key(name) {
                    let snake = name.to_snake_case();
                    let var = format!("_rust_{}", param.name);
                    out.push_str(&format!("    {var} = _to_rust_{snake}({})\n", param.name));
                    call_args.push(var);
                    continue;
                }
            }
            call_args.push(param.name.clone());
        }

        out.push_str(&format!(
            "    return _rust.{}({})\n\n\n",
            func.name,
            call_args.join(", ")
        ));
    }

    out
}

/// Convert a CamelCase class name to a human-readable docstring sentence.
///
/// Examples: `AuthenticationError` → `"Authentication error."`,
/// `LiterLlmError` → `"Liter llm error."`
fn class_name_to_docstring(name: &str) -> String {
    use heck::ToSnakeCase;
    let snake = name.to_snake_case();
    let sentence = snake.replace('_', " ");
    let mut chars = sentence.chars();
    let capitalized = match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    };
    format!("{}.", capitalized)
}

/// Generate exceptions.py — exception hierarchy from IR error definitions.
/// Appends "Error" suffix to variant names that don't already have it (N818 compliance).
/// Prefixes names that would shadow Python builtins (A004 compliance).
fn gen_exceptions_py(api: &ApiSurface) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("# This file is auto-generated by alef. DO NOT EDIT.\n");
    out.push_str("\"\"\"Exception hierarchy.\"\"\"\n\n");
    out.push_str("from __future__ import annotations\n\n\n");

    for error in &api.errors {
        // Base exception class
        out.push_str(&format!("class {}(Exception):\n", error.name));
        let doc = if !error.doc.is_empty() {
            let first_line = error.doc.lines().next().unwrap_or("").trim();
            if first_line.ends_with('.') {
                first_line.to_string()
            } else {
                format!("{}.", first_line)
            }
        } else {
            class_name_to_docstring(&error.name)
        };
        out.push_str(&format!("    \"\"\"{}\"\"\"\n", doc));
        out.push_str("\n\n");

        // Per-variant exception subclasses
        for variant in &error.variants {
            let variant_name = alef_codegen::error_gen::python_exception_name(&variant.name, &error.name);
            out.push_str(&format!("class {}({}):\n", variant_name, error.name));
            let doc = if !variant.doc.is_empty() {
                let first_line = variant.doc.lines().next().unwrap_or("").trim();
                if first_line.ends_with('.') {
                    first_line.to_string()
                } else {
                    format!("{}.", first_line)
                }
            } else {
                class_name_to_docstring(&variant_name)
            };
            out.push_str(&format!("    \"\"\"{}\"\"\"\n", doc));
            out.push_str("\n\n");
        }
    }

    out
}

/// Generate __init__.py — re-exports and version.
/// Only exports user-facing types (not internal Update types or all enums).
fn gen_init_py(api: &ApiSurface, module_name: &str, version: &str, dto: &DtoConfig) -> String {
    use alef_core::ir::TypeRef;

    let mut out = String::with_capacity(1024);
    out.push_str("# This file is auto-generated by alef. DO NOT EDIT.\n");
    out.push_str(&format!(
        "\"\"\"Public API for the conversion library.\n\nVersion: {version}\n\"\"\"\n\n"
    ));

    // Collect enum names referenced by config types (user-facing enums only)
    let enum_names: std::collections::HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let data_enum_names: std::collections::HashSet<String> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.clone())
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
                    if data_enum_names.contains(name) {
                        if !needed_data_enums.iter().any(|n| n == name) {
                            needed_data_enums.push(name.to_string());
                        }
                    } else if enum_names.contains(name) && !needed_enums.contains(&name.to_string()) {
                        needed_enums.push(name.to_string());
                    }
                }
            }
        }
    }

    // Collect all imports and sort them
    let mut imports_from_api = Vec::new();
    let mut imports_from_options = Vec::new();
    let mut imports_from_native = Vec::new();
    let mut imports_from_exceptions = Vec::new();

    // Import functions from api
    if !api.functions.is_empty() {
        let mut names: Vec<_> = api.functions.iter().map(|f| f.name.clone()).collect();
        names.sort();
        imports_from_api.extend(names);
    }

    // Data enums and return types are backed by native Rust structs — import from the native module.
    needed_data_enums.sort();
    imports_from_native.extend(needed_data_enums.iter().cloned());
    native_return_types.sort();
    imports_from_native.extend(native_return_types.iter().cloned());
    imports_from_native.sort();

    // Import plain enums and config types from options
    let mut opt_imports = needed_enums.clone();
    opt_imports.extend(config_types.iter().cloned());
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
                out.push_str(&format!("    {},\n", name));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&format!("{}\n", import_line));
        }
    }
    if !imports_from_exceptions.is_empty() {
        let import_line = format!("from .exceptions import {}", imports_from_exceptions.join(", "));
        if import_line.len() > 88 {
            out.push_str("from .exceptions import (\n");
            for name in &imports_from_exceptions {
                out.push_str(&format!("    {},\n", name));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&format!("{}\n", import_line));
        }
    }
    // Data enums are Rust-backed structs; re-export from the native module.
    if !imports_from_native.is_empty() {
        let import_line = format!("from .{module_name} import {}", imports_from_native.join(", "));
        if import_line.len() > 88 {
            out.push_str(&format!("from .{module_name} import (\n"));
            for name in &imports_from_native {
                out.push_str(&format!("    {},\n", name));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&format!("{}\n", import_line));
        }
    }
    if !imports_from_options.is_empty() {
        let import_line = format!("from .options import {}", imports_from_options.join(", "));
        if import_line.len() > 88 {
            out.push_str("from .options import (\n");
            for name in &imports_from_options {
                out.push_str(&format!("    {},\n", name));
            }
            out.push_str(")\n");
        } else {
            out.push_str(&format!("{}\n", import_line));
        }
    }

    // __all__
    let mut all_items = Vec::new();
    for f in &api.functions {
        all_items.push(f.name.clone());
    }
    all_items.extend(needed_enums);
    all_items.extend(needed_data_enums);
    all_items.extend(native_return_types);
    all_items.extend(config_types);
    all_items.extend(exc_names);
    all_items.sort();

    out.push_str("\n__all__ = [\n");
    for name in &all_items {
        out.push_str(&format!("    \"{name}\",\n"));
    }
    out.push_str("]\n\n");
    out.push_str(&format!("__version__ = \"{version}\"\n"));

    out
}

/// Generate the async runtime initialization function.
fn gen_async_runtime_init() -> String {
    r#"#[pyfunction]
pub fn init_async_runtime() -> PyResult<()> {
    // Tokio runtime auto-initializes on first future_into_py call
    Ok(())
}"#
    .to_string()
}

/// Generate the module initialization function.
fn gen_module_init(module_name: &str, api: &ApiSurface, config: &AlefConfig) -> String {
    let mut lines = vec![
        "#[pymodule]".to_string(),
        format!("pub fn {module_name}(m: &Bound<'_, PyModule>) -> PyResult<()> {{"),
    ];

    // Check if we have async functions
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));

    if has_async {
        lines.push("    m.add_function(wrap_pyfunction!(init_async_runtime, m)?)?;".to_string());
    }

    // Custom registrations (before generated ones so hand-written classes are registered first)
    if let Some(reg) = config.custom_registrations.for_language(Language::Python) {
        for class in &reg.classes {
            lines.push(format!("    m.add_class::<{class}>()?;"));
        }
        for func in &reg.functions {
            lines.push(format!("    m.add_function(wrap_pyfunction!({func}, m)?)?;"));
        }
        for call in &reg.init_calls {
            lines.push(format!("    {call}"));
        }
    }

    // Error types are registered via m.add(...) with the exception types, not m.add_class.
    let error_type_names: AHashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

    // Deduplicate registered types and enums
    let mut registered: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        // Error types are handled by gen_pyo3_error_registration below.
        if error_type_names.contains(typ.name.as_str()) {
            continue;
        }
        if registered.insert(typ.name.clone()) {
            lines.push(format!("    m.add_class::<{}>()?;", typ.name));
        }
    }
    for enum_def in &api.enums {
        if registered.insert(enum_def.name.clone()) {
            lines.push(format!("    m.add_class::<{}>()?;", enum_def.name));
        }
    }
    for func in &api.functions {
        lines.push(format!("    m.add_function(wrap_pyfunction!({}, m)?)?;", func.name));
    }

    // Register trait bridge registration functions
    for register_fn in crate::trait_bridge::collect_bridge_register_fns(&config.trait_bridges) {
        lines.push(format!("    m.add_function(wrap_pyfunction!({register_fn}, m)?)?;"));
    }

    // Register error exception types
    for error in &api.errors {
        for reg_line in alef_codegen::error_gen::gen_pyo3_error_registration(error) {
            lines.push(reg_line);
        }
    }

    lines.push("    Ok(())".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}
