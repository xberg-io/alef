use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use ahash::{AHashMap, AHashSet};

use super::async_wrappers::{adapter_param_python_type, emit_adapter_wrapper};
use super::converters::emit_converters;
use super::function_wrappers::emit_function_wrappers;
use super::helper_type_mapping::classify_param_type;
use crate::backends::pyo3::gen_bindings::types::collect_named_types;

/// Generate api.py — wrapper functions that convert Python types to Rust binding types.
///
/// For each function parameter whose type is a `has_default` struct (e.g. `ParseOptions`),
/// we generate a `_to_rust_{snake_name}` converter that maps the Python `@dataclass` instance
/// to the Rust binding's pyclass by passing every field as a keyword argument.
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::pyo3::gen_bindings) fn gen_api_py(
    api: &ApiSurface,
    module_name: &str,
    package_name: &str,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    dto: &crate::core::config::DtoConfig,
    capsule_types: &std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    adapters: &[crate::core::config::AdapterConfig],
    reexported_types: &[String],
    exclude_functions: &AHashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;

    // Collect bridge param names so they can be typed as `object | None` instead of
    // `str | None`. The IR sanitizes trait handle types to String, but callers pass
    // arbitrary Python objects implementing the visitor protocol.
    let bridge_param_names: ahash::AHashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // Build lookup for options-field bridges: options_type_name → (visitor_kwarg_name, field_name, type_alias).
    // When a function parameter's type matches an options-field bridge's `options_type`, we add
    // a `visitor: {type_alias} | None = None` convenience kwarg to the Python wrapper.
    // The type_alias (e.g. "VisitorHandle") is the handle type exported from the native module.
    let options_field_bridges: AHashMap<&str, (&str, &str, Option<&str>)> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let param_name = b.param_name.as_deref()?;
            let field_name = b.resolved_options_field()?;
            let type_alias = b.type_alias.as_deref();
            Some((options_type, (param_name, field_name, type_alias)))
        })
        .collect();

    // Build lookup: type_name → TypeDef for has_default types
    let default_types: AHashMap<String, &crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update"))
        .map(|t| (t.name.clone(), t))
        .collect();

    // Collect enum names for conversion detection
    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    // Separate data enums (tagged unions exposed as dict-accepting structs) from simple int enums.
    // Data enums are passed through as dicts; simple enums need string→variant lookup.
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();

    // Determine which has_default types are referenced by function parameters (directly or nested)
    let mut needed_converters: Vec<String> = Vec::new();
    let mut visited: AHashSet<String> = AHashSet::new();

    fn collect_needed(
        type_name: &str,
        default_types: &AHashMap<String, &crate::core::ir::TypeDef>,
        needed: &mut Vec<String>,
        visited: &mut AHashSet<String>,
    ) {
        if !visited.insert(type_name.to_string()) {
            return;
        }
        if let Some(typ) = default_types.get(type_name) {
            // First collect nested types so they appear before the parent converter.
            // `classify_param_type` recursively unwraps Optional/Vec layers so a
            // `Vec<HasDefault>` field still discovers the leaf converter.
            for field in binding_fields(&typ.fields) {
                if let Some((name, _)) = classify_param_type(&field.ty) {
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
            // `classify_param_type` unwraps Optional/Vec/Optional<Vec> layers
            // so a `Vec<HasDefault>` parameter still triggers converter emission
            // for the leaf type.
            if let Some((name, _)) = classify_param_type(&param.ty) {
                collect_needed(name, &default_types, &mut needed_converters, &mut visited);
            }
        }
    }

    // Collect all type names referenced in function signatures (params + returns)
    // that aren't converters — these need to be imported too.
    let mut all_type_imports: AHashSet<String> = AHashSet::new();
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
    // Adapter wrappers (emitted later in this file) reference the adapter's owner_type,
    // item_type, and param types as bare names in their `async def` signatures
    // (`AsyncIterator[ItemType]`, owner-type parameter, request types). Without these
    // entries the generated `api.py` raises F821 / NameError at import time.
    for adapter in adapters {
        if let Some(owner) = adapter.owner_type.as_deref() {
            all_type_imports.insert(owner.to_string());
        }
        if let Some(item) = adapter.item_type.as_deref() {
            all_type_imports.insert(item.to_string());
        }
        for param in &adapter.params {
            // Skip Rust primitive types — they're emitted as their Python
            // equivalents in the wrapper signature (str/bytes/int/float/bool/
            // None) and have no corresponding name to import. Without this
            // filter, an adapter declared with a `String` param injects a
            // stray `from .options import ..., String` line that explodes
            // with ImportError at module load.
            let mapped = adapter_param_python_type(&param.ty);
            if matches!(mapped, "str" | "bytes" | "None" | "int" | "float" | "bool") {
                continue;
            }
            all_type_imports.insert(param.ty.clone());
        }
        // AsyncMethod adapters reference the return type as a bare name in their
        // `async def foo(...) -> ReturnType` signature; without this entry the
        // generated api.py raises F821 / NameError at import time. Skip names
        // that map to Python builtins (str, bytes, None) — those don't need
        // imports.
        if let Some(returns) = adapter.returns.as_deref() {
            let mapped = adapter_param_python_type(returns);
            if !matches!(mapped, "str" | "bytes" | "None" | "int" | "float" | "bool") {
                all_type_imports.insert(returns.to_string());
            }
        }
    }
    // Also collect type_alias names from options-field bridges so they can be used in
    // function signature annotations for visitor parameters.
    for bridge in trait_bridges {
        if let Some(alias) = &bridge.type_alias {
            all_type_imports.insert(alias.clone());
        }
    }

    // Detect whether any function or method returns a capsule type — drives whether the
    // api.py needs `cast` in typing imports (used to bridge `Any` from the native stub to
    // the public third-party return annotation, e.g. `tree_sitter.Language`).
    let needs_cast = api.functions.iter().any(|f| {
        let leaf = match &f.return_type {
            crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
            crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        };
        leaf.is_some_and(|n| capsule_types.contains_key(n))
    });

    let mut out = String::with_capacity(4096);
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str("\"\"\"Public API for conversion.\"\"\"\n\n");
    // stdlib first (isort section 1)
    // Adapter wrappers reference AsyncIterator in their return annotation, so include it
    // whenever the surface defines any adapters (they emit `async def ... -> AsyncIterator[T]:`).
    let mut typing_parts: Vec<&str> = vec!["Any", "TypeVar"];
    if needs_cast || !needed_converters.is_empty() {
        typing_parts.push("cast");
    }
    if !needed_converters.is_empty() {
        typing_parts.push("overload");
    }
    // AsyncIterator is only needed when at least one adapter uses the streaming pattern.
    // async_method adapters emit `return await engine.foo(...)` and never yield.
    let needs_async_iterator = adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming));
    if needs_async_iterator {
        typing_parts.push("AsyncIterator");
    }
    typing_parts.sort_unstable();
    if !needed_converters.is_empty() {
        out.push_str("import json\n");
    }
    out.push_str(&crate::backends::pyo3::template_env::render(
        "typing_import.jinja",
        minijinja::context! { names => typing_parts },
    ));
    out.push('\n');
    // third-party / package self-import (isort section 3)
    out.push_str(&crate::backends::pyo3::template_env::render(
        "import_as_module.jinja",
        minijinja::context! {
            package_name => package_name,
            module_name => module_name,
        },
    ));

    // Split type imports: opaque/error types and non-options types come from the native module,
    // has_default dataclass types come from .options.
    let opaque_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let error_names: AHashSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    // Types that exist in options.py: has_default structs that are not direct return types
    // of free functions. Update types are output-only. `t.is_return_type` is set during IR
    // extraction for types returned directly by a public free function — that's the only
    // case where a has_default type must live in the native module rather than .options.
    // Don't try to widen this with method returns or transitive field walks: a builder
    // method like `PackConfig::from_toml_file -> PackConfig` is a constructor, not evidence
    // that the type leaves through the native return surface, and falsely excluding it from
    // .options is what produced alef#72 (PackConfig/ProcessConfig imported from ._native
    // despite being dataclasses re-exported from .options).
    let options_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update") && !t.is_return_type)
        .map(|t| t.name.clone())
        .collect();
    // Types returned directly by free functions — these live in the native module,
    // not .options. Function return type annotations must qualify them with _rust,
    // UNLESS they are in reexported_types (re-exported in public __init__.py).
    let return_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_return_type)
        .map(|t| t.name.clone())
        .collect();
    // Types re-exported in the public package: skip _rust. qualification for these
    let reexported_names: AHashSet<&str> = reexported_types.iter().map(|s| s.as_str()).collect();
    // All non-enum IR type names (used to distinguish structs from enums in classification).
    let all_ir_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    // Enums that options.py actually exports: plain (non-data) unit enums referenced by
    // has_default struct fields. Data enums and enums not referenced by config structs live
    // in the native module, not options.py — so they must be imported from the native module.
    let options_enum_names: AHashSet<String> = {
        let mut set = AHashSet::new();
        for typ in api
            .types
            .iter()
            .filter(|t| t.has_default && !t.name.ends_with("Update"))
        {
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
                    if enum_names.contains(name) && !data_enum_names.contains(name) {
                        set.insert(name.to_string());
                    }
                }
            }
        }
        set
    };

    let all_enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let mut options_imports: Vec<&str> = Vec::new();
    let mut native_imports: Vec<&str> = Vec::new();
    for name in &all_type_imports {
        // Capsule types are not registered as #[pyclass] in the native module; skip them
        // here so api.py doesn't try `from ._native import <CapsuleType>` and crash at import.
        if capsule_types.contains_key(name) {
            continue;
        }
        let is_options = options_type_names.contains(name) || options_enum_names.contains(name);
        let is_native = !is_options
            && (opaque_names.contains(name)
                || error_names.contains(name)
                || all_ir_type_names.contains(name)
                // Enums not in options_enum_names live in the native module.
                || (all_enum_names.contains(name) && !options_enum_names.contains(name)));
        if is_native {
            native_imports.push(name.as_str());
        } else {
            options_imports.push(name.as_str());
        }
    }

    // Import types used in function signatures at runtime (not under TYPE_CHECKING)
    // since they appear as parameter/return type annotations in generated wrapper functions.
    // Sort for deterministic codegen — `all_type_imports` is an AHashSet, so iteration
    // order changes between runs; without sorting, hash-based caching always misses.
    native_imports.sort_unstable();
    options_imports.sort_unstable();
    if !native_imports.is_empty() {
        // isort: blank line between `import X as _rust` (absolute) and `from .Y import` (relative).
        out.push('\n');
        out.push_str(&crate::backends::pyo3::template_env::render(
            "import_from_module.jinja",
            minijinja::context! {
                module_name => module_name,
                imports => native_imports.join(", "),
            },
        ));
    }
    if !options_imports.is_empty() {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "import_from_options.jinja",
            minijinja::context! {
                imports => options_imports.join(", "),
            },
        ));
    }
    // Capsule type imports: group by module path, emit one `from {module} import {names}` per group.
    // Capsule types (e.g. tree_sitter.Language) are not in ._native or .options; they need their
    // own first-party import so bare names in function signatures resolve (ruff F821).
    {
        use std::collections::BTreeMap;
        let mut capsule_imports: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (rust_name, cfg) in capsule_types {
            let python_type = cfg.python_type();
            if let Some((module_path, _class_name)) = python_type.rsplit_once('.') {
                capsule_imports
                    .entry(module_path.to_string())
                    .or_default()
                    .push(rust_name.clone());
            }
        }
        if !capsule_imports.is_empty() {
            for (module_path, mut names) in capsule_imports {
                names.sort_unstable();
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "import_from_absolute_module.jinja",
                    minijinja::context! {
                        module_name => module_path,
                        imports => names.join(", "),
                    },
                ));
            }
        }
    }
    out.push('\n');

    emit_converters(
        &mut out,
        &needed_converters,
        &default_types,
        &options_field_bridges,
        &enum_names,
        &data_enum_names,
        dto,
    );

    emit_function_wrappers(
        &mut out,
        api,
        trait_bridges,
        capsule_types,
        exclude_functions,
        &bridge_param_names,
        &options_field_bridges,
        &default_types,
        &data_enum_names,
        &return_type_names,
        &reexported_names,
    );

    // Emit wrapper functions for adapter-based streaming methods.
    // Adapters define methods on the owner type (e.g. CrawlEngineHandle) in the binding layer.
    // We emit module-level wrapper functions here so the public API exposes them alongside
    // regular functions (e.g. `scrape`, `crawl`) rather than forcing users to call methods
    // on the engine handle. The wrapper accepts an engine handle as the first parameter.
    for adapter in adapters {
        emit_adapter_wrapper(&mut out, adapter, &api.types);
    }

    out
}
