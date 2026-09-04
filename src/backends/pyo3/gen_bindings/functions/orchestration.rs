use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use ahash::{AHashMap, AHashSet};

use super::async_wrappers::{
    adapter_param_python_type, adapter_return_converter, emit_adapter_wrapper, streaming_item_converter,
};
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
    config: &crate::core::config::ResolvedCrateConfig,
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

    // Types `options.py` emits as public `@dataclass` DTOs. An adapter param typed as one of
    // these crosses the Python/native boundary at a different shape than the engine call
    // actually accepts, so it needs the `_to_rust_*` converter — the same requirement plain
    // function wrappers already honor via `default_types`, scoped down to the subset that is
    // genuinely a public *input* dataclass (excludes return types; see `options_return_types`
    // and `options_publishable_return_types` below for those). ~keep
    let options_dataclass_types =
        crate::backends::pyo3::gen_bindings::types::options_dataclass_type_names(api, reexported_types);

    // `has_default` alone under-counts the types api.py must be able to convert: a type can lack
    // a core `Default` impl purely because one of its fields is required with no sensible default
    // (e.g. `CaptioningConfig { llm: LlmConfig, .. }`), while still being reachable as a field of
    // some function/adapter parameter (directly, or nested inside another dataclass such as
    // `ExtractionConfig.captioning: Option<CaptioningConfig>`). `options_dataclass_types` is the
    // exact closure that already accounts for this (see its doc); widening `default_types` with
    // it (rather than replacing the `has_default` half) means every type this map covered before
    // is still covered identically, and the closure types join without disturbing them. ~keep
    let default_types: AHashMap<String, &crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| (t.has_default || options_dataclass_types.contains(&t.name)) && !t.name.ends_with("Update"))
        .map(|t| (t.name.clone(), t))
        .collect();

    // Return types `options.py` publishes itself (as `@dataclass`, never `TypedDict` -- see
    // `types::gen_options_py`'s doc). A function returning one of these must name the public type
    // and convert into it, not name the native `#[pyclass]` behind the same word.
    let options_return_types =
        crate::backends::pyo3::gen_bindings::types::options_return_dataclass_names(api, dto, reexported_types);

    // The question a RETURN value (a plain function's, an adapter's, or a streaming adapter's
    // item) has to answer is "does `options.py` publish this name", which is
    // `options_dataclass_types` OR `options_return_types` -- not `options_dataclass_types` alone
    // and not `options_return_types` alone. `api.py`'s own import classification
    // (`options_type_names` further below) already consults the union, so a return type that is
    // a public *input* dataclass (not a return-only published type) still gets imported from
    // `.options` and named in the `-> ReturnType` annotation. `adapter_return_converter` /
    // `streaming_item_converter` (adapters) and `emit_function_wrappers` / `function_return_converters`
    // (plain functions) must all be handed this union, not either half alone, or the annotation
    // names the public type while the body hands back the untouched native pyclass. ~keep
    let options_publishable_return_types: std::collections::HashSet<String> =
        options_dataclass_types.union(&options_return_types).cloned().collect();

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
                if let Some((name, _)) = classify_param_type(&field.ty)
                    && default_types.contains_key(name)
                {
                    collect_needed(name, default_types, needed, visited);
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
    // An adapter wrapper is not exempt from the same param-conversion requirement as a plain
    // function wrapper: a param typed as a public dataclass needs the `_to_rust_*` converter
    // `emit_adapter_wrapper` applies below. Only walk params that are genuinely emitted as
    // dataclasses — an `is_return_type` param would have no converter to find. ~keep
    for adapter in adapters {
        for param in &adapter.params {
            if options_dataclass_types.contains(&param.ty) {
                collect_needed(&param.ty, &default_types, &mut needed_converters, &mut visited);
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
        typing_parts.push("TypedDict");
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
    let options_type_names: AHashSet<String> = {
        // Widened (OR), not replaced, with `options_dataclass_types` -- see `default_types`'
        // comment above for why a raw `has_default` filter under-counts, and why widening rather
        // than substituting preserves every case (including a reexported `has_default` type,
        // which `options_dataclass_types` deliberately excludes) that already worked here. ~keep
        let mut names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| {
                (t.has_default || options_dataclass_types.contains(&t.name))
                    && !t.name.ends_with("Update")
                    && !t.is_return_type
            })
            .map(|t| t.name.clone())
            .collect();
        names.extend(options_return_types.iter().cloned());
        names
    };
    let return_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_return_type && !capsule_types.contains_key(&t.name))
        .map(|t| t.name.clone())
        .collect();
    let all_ir_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    let options_enum_names: AHashSet<String> = {
        let mut set = AHashSet::new();
        // Widened (OR) with `options_dataclass_types`, same rationale as `options_type_names`
        // above: an enum field on a closure-added type (has_default == false) needs the same
        // "defined/imported as an options enum" treatment as one on a has_default type. ~keep
        for typ in api
            .types
            .iter()
            .filter(|t| (t.has_default || options_dataclass_types.contains(&t.name)) && !t.name.ends_with("Update"))
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
                if let Some(name) = inner_name
                    && enum_names.contains(name)
                    && !data_enum_names.contains(name)
                {
                    set.insert(name.to_string());
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

    let streaming_item_converters: std::collections::BTreeSet<String> = adapters
        .iter()
        .filter_map(|adapter| streaming_item_converter(adapter, &options_publishable_return_types))
        .collect();
    let adapter_return_converters: std::collections::BTreeSet<String> = adapters
        .iter()
        .filter_map(|adapter| adapter_return_converter(adapter, &options_publishable_return_types))
        .collect();

    // A wrapper returning a type `options.py` publishes calls that type's `_from_native_*`
    // converter, so `api.py` has to import it alongside the type itself. The publishable set is
    // the union (`options_publishable_return_types`), not `options_return_types` alone: a plain
    // function's return type is routinely a public *input* dataclass rather than a return-only
    // `TypedDict`, and the narrower set left that shape's converter uncalled and unimported --
    // the same asymmetry `adapter_return_converter`/`streaming_item_converter` already avoid. ~keep
    let function_return_converters: std::collections::BTreeSet<String> = api
        .functions
        .iter()
        .filter(|func| !exclude_functions.contains(&func.name))
        .filter_map(|func| match &func.return_type {
            crate::core::ir::TypeRef::Named(name) => Some(name),
            crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                crate::core::ir::TypeRef::Named(name) => Some(name),
                _ => None,
            },
            _ => None,
        })
        .filter(|name| options_publishable_return_types.contains(*name))
        .map(|name| crate::backends::pyo3::gen_bindings::types::from_native_converter_name(name))
        .collect();

    options_imports.extend(streaming_item_converters.iter().map(String::as_str));
    options_imports.extend(adapter_return_converters.iter().map(String::as_str));
    options_imports.extend(function_return_converters.iter().map(String::as_str));
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
        reexported_types,
        config,
        &crate::backends::pyo3::gen_bindings::types::OptionsFieldDefaults::new(api),
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
        &options_publishable_return_types,
    );

    for adapter in adapters {
        emit_adapter_wrapper(
            &mut out,
            adapter,
            &api.types,
            &options_dataclass_types,
            &options_publishable_return_types,
        );
    }

    out
}
