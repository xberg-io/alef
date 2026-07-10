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
    opaque_types: &std::collections::HashMap<String, String>,
    adapters: &[crate::core::config::AdapterConfig],
    reexported_types: &[String],
    exclude_functions: &AHashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;

    let bridge_param_names: ahash::AHashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    let options_field_bridges: AHashMap<&str, (&str, &str, Option<&str>)> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let param_name = b.param_name.as_deref()?;
            let field_name = b.resolved_options_field()?;
            let trait_present = api.types.iter().any(|t| t.name == b.trait_name);
            let handle_type = if trait_present {
                Some(b.trait_name.as_str())
            } else {
                b.type_alias.as_deref()
            };
            Some((options_type, (param_name, field_name, handle_type)))
        })
        .collect();

    let default_types: AHashMap<String, &crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update"))
        .map(|t| (t.name.clone(), t))
        .collect();

    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    // A sanitized data enum has an unresolvable variant field, so no serde-based `#[new]` is
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e) && !generators::enum_has_sanitized_fields(e))
        .map(|e| e.name.as_str())
        .collect();

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
            if let Some((name, _)) = classify_param_type(&param.ty) {
                collect_needed(name, &default_types, &mut needed_converters, &mut visited);
            }
        }
    }

    let mut all_type_imports: AHashSet<String> = AHashSet::new();
    for type_name in &needed_converters {
        all_type_imports.insert(type_name.clone());
    }
    for func in &api.functions {
        for param in &func.params {
            collect_named_types(&param.ty, &mut all_type_imports);
        }
        collect_named_types(&func.return_type, &mut all_type_imports);
    }
    for adapter in adapters {
        if let Some(owner) = adapter.owner_type.as_deref() {
            all_type_imports.insert(owner.to_string());
        }
        if let Some(item) = adapter.item_type.as_deref() {
            all_type_imports.insert(item.to_string());
        }
        for param in &adapter.params {
            let mapped = adapter_param_python_type(&param.ty);
            if matches!(mapped, "str" | "bytes" | "None" | "int" | "float" | "bool") {
                continue;
            }
            all_type_imports.insert(param.ty.clone());
        }
        if let Some(returns) = adapter.returns.as_deref() {
            let mapped = adapter_param_python_type(returns);
            if !matches!(mapped, "str" | "bytes" | "None" | "int" | "float" | "bool") {
                all_type_imports.insert(returns.to_string());
            }
        }
    }
    for bridge in trait_bridges {
        let trait_present = api.types.iter().any(|t| t.name == bridge.trait_name);
        if trait_present {
            all_type_imports.insert(bridge.trait_name.clone());
        } else if let Some(alias) = &bridge.type_alias {
            all_type_imports.insert(alias.clone());
        }
    }

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
    let mut typing_parts: Vec<&str> = vec!["Any", "TypeVar"];
    if needs_cast || !needed_converters.is_empty() {
        typing_parts.push("cast");
    }
    if !needed_converters.is_empty() {
        typing_parts.push("overload");
    }
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
    out.push_str(&crate::backends::pyo3::template_env::render(
        "import_as_module.jinja",
        minijinja::context! {
            package_name => package_name,
            module_name => module_name,
        },
    ));

    let opaque_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let error_names: AHashSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    let reexported_names: AHashSet<&str> = reexported_types.iter().map(|s| s.as_str()).collect();
    let output_style = dto.python_output_style();
    let options_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| {
            t.has_default
                && !t.name.ends_with("Update")
                && !(t.is_return_type
                    && (output_style != crate::core::config::PythonDtoStyle::TypedDict
                        || reexported_names.contains(t.name.as_str())))
        })
        .map(|t| t.name.clone())
        .collect();
    let return_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_return_type && !capsule_types.contains_key(&t.name))
        .map(|t| t.name.clone())
        .collect();
    let all_ir_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
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
        if capsule_types.contains_key(name) {
            continue;
        }
        let is_options = options_type_names.contains(name) || options_enum_names.contains(name);
        // binding-side #[pyclass] wrapper struct emitted in mod.rs and are exported from
        let is_opaque_wrapper = opaque_types.contains_key(name) && !capsule_types.contains_key(name);
        let is_native = !is_options
            && (opaque_names.contains(name)
                || error_names.contains(name)
                || all_ir_type_names.contains(name)
                || is_opaque_wrapper
                || (all_enum_names.contains(name) && !options_enum_names.contains(name)));
        if is_native {
            native_imports.push(name.as_str());
        } else {
            options_imports.push(name.as_str());
        }
    }

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
        reexported_types,
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

    for adapter in adapters {
        emit_adapter_wrapper(&mut out, adapter, &api.types);
    }

    out
}
