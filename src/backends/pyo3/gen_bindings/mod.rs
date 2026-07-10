//! PyO3 (Python) backend: orchestration and `Backend` trait implementation.

pub mod capsule;
mod capsule_methods;
mod cfg_fields;
mod config;
mod config_opaque;
pub(in crate::backends::pyo3) mod constructors;
pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
mod mutex;
mod opaque_helpers;
mod postprocess;
mod public_files;
pub mod service_api;
mod support_items;
#[cfg(test)]
mod tests;
pub mod types;
pub(in crate::backends::pyo3) use types::options_dataclass_type_names;
pub(in crate::backends::pyo3) mod wire_schema;

use crate::backends::pyo3::type_map::Pyo3Mapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use std::path::PathBuf;

pub struct Pyo3Backend;

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
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // to the core crate — which resolves the cfg itself — and emits no `#[cfg]` gate on the
        // wrapper, so two same-named entries would otherwise produce duplicate `#[pyfunction]`
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

        // should store them as `Option<Py<PyAny>>` with `#[serde(skip)]` so the visitor can
        let mut trait_type_names: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_trait)
            .map(|t| t.name.clone())
            .collect();
        for bridge in &config.trait_bridges {
            if bridge.bind_via == crate::core::config::BridgeBinding::OptionsField {
                if let Some(alias) = &bridge.type_alias {
                    trait_type_names.insert(alias.clone());
                }
            }
        }
        let mapper = Pyo3Mapper { trait_type_names };
        let core_import = config.core_import_name();

        let output_dir = resolve_output_dir(config.output_paths.get("python"), &config.name, "crates/{name}-py/src/");
        let has_serde = detect_serde_available(&output_dir);
        let mut cfg = config::binding_config(&core_import, has_serde);
        let mut cfg_unsendable = config::unsendable_binding_config(&core_import, has_serde);

        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Python)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        support_items::add_generated_module_attributes(&mut builder, &config.extra_clippy_allows);
        builder.add_import("pyo3::prelude::*");
        // import is needed — that would trigger clippy::single_component_path_imports.

        if has_serde {
            builder.add_import("serde_json");
        }

        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        let has_async = api.functions.iter().any(|f| f.is_async && !f.sanitized)
            || api
                .types
                .iter()
                .any(|t| t.methods.iter().any(|m| m.is_async && !m.sanitized));
        if has_async {
            builder.add_import("pyo3_async_runtimes");
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

        // binding-side #[pyclass] wrapper structs and must be treated as opaque in return wrapping
        let mut opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        // Capsule types bypass #[pyclass] generation entirely; opaque types that
        let early_capsule_types = config
            .python
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
        for name in config.opaque_types.keys() {
            if !early_capsule_types.contains_key(name) {
                opaque_types.insert(name.clone());
            }
        }
        let data_enum_names: Vec<String> = api
            .enums
            .iter()
            .filter(|e| generators::enum_has_data_variants(e))
            .map(|e| e.name.clone())
            .collect();
        let bridge_type_aliases: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let conversion_opaque_set: AHashSet<String> =
            opaque_types.iter().chain(bridge_type_aliases.iter()).cloned().collect();
        let mut opaque_names_vec: Vec<String> = opaque_types.iter().cloned().collect();
        let serializable_opaque_names_vec: Vec<String> = data_enum_names.clone();
        opaque_names_vec.extend(data_enum_names);
        opaque_names_vec.extend(bridge_type_aliases);
        let mut opaque_names_set: AHashSet<String> = opaque_names_vec.iter().cloned().collect();
        let mut changed = true;
        while changed {
            changed = false;
            for typ in api.types.iter().filter(|t| !t.is_opaque) {
                if opaque_names_set.contains(&typ.name) {
                    continue;
                }
                let has_opaque = typ
                    .fields
                    .iter()
                    .any(|f| generators::structs::field_references_opaque_type(&f.ty, &opaque_names_vec));
                if has_opaque {
                    opaque_names_vec.push(typ.name.clone());
                    opaque_names_set.insert(typ.name.clone());
                    changed = true;
                }
            }
        }
        cfg.opaque_type_names = &opaque_names_vec;
        cfg_unsendable.opaque_type_names = &opaque_names_vec;
        cfg.serializable_opaque_type_names = &serializable_opaque_names_vec;
        cfg_unsendable.serializable_opaque_type_names = &serializable_opaque_names_vec;
        // emitted `#[new]` filters out fields with `f.cfg.is_some()`, but the python
        let never_skip_cfg_field_names = cfg_fields::never_skip_cfg_field_names(api, config);
        cfg.never_skip_cfg_field_names = &never_skip_cfg_field_names;
        cfg_unsendable.never_skip_cfg_field_names = &never_skip_cfg_field_names;
        let mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        let tokio_mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_tokio_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
            if mutex_types.iter().any(|n| !tokio_mutex_types.contains(n)) {
                builder.add_import("std::sync::Mutex");
            }
        }

        let type_ref_is_map = |ty: &crate::core::ir::TypeRef| matches!(ty, crate::core::ir::TypeRef::Map(_, _));
        let has_maps = api.types.iter().any(|t| {
            t.fields.iter().any(|f| type_ref_is_map(&f.ty))
                || t.methods
                    .iter()
                    .any(|m| m.params.iter().any(|p| type_ref_is_map(&p.ty)) || type_ref_is_map(&m.return_type))
        }) || api
            .functions
            .iter()
            .any(|f| f.params.iter().any(|p| type_ref_is_map(&p.ty)) || type_ref_is_map(&f.return_type));
        if has_maps {
            builder.add_import("std::collections::HashMap");
        }

        support_items::add_py_visitor_ref(&mut builder);

        support_items::add_json_helpers(&mut builder);

        let custom_mods = config.custom_modules.for_language(Language::Python);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        // `#[pyfunction]` entrypoints are compiled and can be registered in the module init.
        if !api.services.is_empty() {
            builder.add_item("pub mod service;");
        }

        for adapter in &config.adapters {
            match adapter.pattern {
                AdapterPattern::Streaming => {
                    let key = crate::adapters::stream_struct_key(adapter);
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

        let py_exclude_functions: ahash::AHashSet<String> = config
            .python
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        // otherwise emit as a SECOND `#[pyfunction]` of the same Rust name — a redefinition
        let bridge_duck_register_fns: AHashSet<&str> = config
            .trait_bridges
            .iter()
            .filter(|b| b.registry_getter.is_some() && api.types.iter().any(|t| t.is_trait && t.name == b.trait_name))
            .filter_map(|b| b.register_fn.as_deref())
            .collect();
        let mut py_exclude_types: ahash::AHashSet<String> = config
            .python
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();
        py_exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
        // Types listed in capsule_types bypass #[pyclass] generation entirely — they are
        let capsule_types = config
            .python
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
        config_opaque::exclude_capsule_opaque_types(&mut py_exclude_types, config, &capsule_types);

        let mut error_type_names: AHashSet<String> = AHashSet::new();
        for error in &api.errors {
            error_type_names.insert(error.name.clone());
            for variant in &error.variants {
                let exc_name = crate::codegen::error_gen::python_exception_name(&variant.name, &error.name);
                error_type_names.insert(exc_name);
            }
        }

        let error_converters: Vec<String> = api
            .errors
            .iter()
            .map(|e| {
                use heck::ToSnakeCase;
                format!("{}_to_py_err", e.name.to_snake_case())
            })
            .collect();

        // Track emitted #[pyclass] struct names to prevent duplicate definitions (E0255/E0428).
        let mut emitted_pyclass_names: AHashSet<&str> = AHashSet::new();
        // core types whose `impl Default` is `#[alef(skip)]`'d (so `has_default` is false on the
        let default_required_types = cfg_fields::default_required_types(api);
        // whose type is not in the convertible set) must keep #[derive(Default)] instead of
        let core_to_binding_for_default = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
        cfg.emit_delegating_default_for_types = Some(&core_to_binding_for_default);
        cfg_unsendable.emit_delegating_default_for_types = Some(&core_to_binding_for_default);
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !py_exclude_types.contains(&typ.name))
        {
            if error_type_names.contains(typ.name.as_str()) {
                continue;
            }
            // Capsule types bypass #[pyclass] entirely — they travel as raw PyCapsule handles
            // produce an unused #[pyclass] that conflicts with the capsule-based call sites.
            if capsule_types.contains_key(typ.name.as_str()) {
                continue;
            }
            if !emitted_pyclass_names.insert(typ.name.as_str()) {
                continue;
            }
            let type_cfg = if opaque_types.contains(typ.name.as_str()) {
                &cfg_unsendable
            } else {
                &cfg
            };
            if typ.is_opaque {
                let mut struct_code = generators::gen_opaque_struct(typ, type_cfg);
                let mut impl_block = generators::gen_opaque_impl_block(
                    typ,
                    &mapper,
                    type_cfg,
                    &opaque_types,
                    &mutex_types,
                    &adapter_bodies,
                );
                if tokio_mutex_types.contains(&typ.name) {
                    struct_code = mutex::rewrite_to_tokio_mutex_struct(&struct_code);
                    impl_block = mutex::rewrite_to_tokio_mutex_impl(&impl_block);
                }
                // PyCapsule objects instead of the (non-existent) #[pyclass] wrapper structs.
                if !capsule_types.is_empty() {
                    impl_block =
                        capsule_methods::rewrite_capsule_methods(impl_block, typ, &capsule_types, &error_converters);
                }
                // the type into a Python-level constructor by appending a `#[new]
                // block. pyo3 forbids multiple `#[pymethods] impl T` blocks (without
                // alongside the existing `#[staticmethod] pub fn new`. The two
                // `#[new]` attribute regardless of the Rust name.
                if typ.is_variant_wrapper
                    && !impl_block.is_empty()
                    && let Some(ctor_body) = opaque_helpers::variant_wrapper_constructor_body(typ, &mapper)
                {
                    impl_block = opaque_helpers::inject_into_impl_block(&impl_block, &ctor_body);
                }
                builder.add_item(&struct_code);
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
                if opaque_helpers::should_emit_default_impl(typ, &impl_block, &default_required_types) {
                    builder.add_item(&opaque_helpers::emit_default_impl(typ));
                }
                // Client constructor — emit a separate #[pymethods] impl with #[new]
                if let Some(ctor) = config.client_constructors.get(&typ.name) {
                    let ctor_body = generators::gen_opaque_constructor(ctor, &typ.name, &core_import, "#[new]");
                    let ctor_impl = format!("#[pymethods]\nimpl {} {{\n{}}}", typ.name, ctor_body);
                    builder.add_item(&ctor_impl);
                }
            } else {
                // gen_struct adds #[derive(Default)] when typ.has_default is true,
                // attribute (e.g. `#[pyo3(get, name = "class")]`) and the serde rename
                // attribute (`#[serde(rename = "class")]`) so the user-facing API is unchanged.
                let type_name = typ.name.clone();
                let config_ref = config;
                builder.add_item(&generators::gen_struct_with_rename(
                    typ,
                    &mapper,
                    type_cfg,
                    |field| {
                        let is_json_field = matches!(field.ty, crate::core::ir::TypeRef::Json);
                        let is_opt_json_field = field.optional && is_json_field
                            || matches!(&field.ty, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Json));
                        let json_attr = if is_opt_json_field {
                            Some("serde(default, deserialize_with = \"alef_json_str_opt::deserialize\")".to_string())
                        } else if is_json_field {
                            Some("serde(default, deserialize_with = \"alef_json_str::deserialize\")".to_string())
                        } else {
                            None
                        };

                        if config_ref
                            .resolve_field_name(crate::core::config::Language::Python, &type_name, &field.name)
                            .is_some()
                        {
                            let mut attrs = vec![
                                format!("pyo3(get, name = \"{}\")", field.name),
                                format!("serde(rename = \"{}\")", field.name),
                            ];
                            if let Some(a) = json_attr {
                                attrs.push(a);
                            }
                            attrs
                        } else if let Some(a) = json_attr {
                            vec![a]
                        } else {
                            vec![]
                        }
                    },
                    |field| config_ref.resolve_field_name(crate::core::config::Language::Python, &type_name, &field.name),
                ));
                let py_field_renames: std::collections::HashMap<String, String> = typ
                    .fields
                    .iter()
                    .filter_map(|field| {
                        config_ref
                            .resolve_field_name(crate::core::config::Language::Python, &type_name, &field.name)
                            .map(|renamed| (field.name.clone(), renamed))
                    })
                    .collect();
                let renames_ref = if py_field_renames.is_empty() {
                    None
                } else {
                    Some(&py_field_renames)
                };

                let mut impl_block = generators::gen_impl_block_with_renames(
                    typ,
                    &mapper,
                    type_cfg,
                    &adapter_bodies,
                    &opaque_types,
                    renames_ref,
                );

                impl_block = constructors::replace_constructor_with_serde_rename(
                    &impl_block,
                    typ,
                    &mapper,
                    type_cfg,
                    renames_ref,
                    &config.trait_bridges,
                    type_cfg.never_skip_cfg_field_names,
                    api,
                );
                // Inject from_json staticmethod into the existing #[pymethods] block when serde
                if has_serde
                    && crate::codegen::conversions::core_to_binding_convertible_types(api, &[]).contains(&typ.name)
                {
                    let from_json_method = "    #[staticmethod]\n    \
                         fn from_json(json_str: String) -> pyo3::PyResult<Self> {\n        \
                         serde_json::from_str::<Self>(&json_str)\n            \
                         .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))\n    \
                         }"
                    .to_string();
                    if impl_block.is_empty() {
                        let type_name = &typ.name;
                        impl_block = format!("#[pymethods]\nimpl {type_name} {{\n{from_json_method}\n}}");
                    } else {
                        if let Some(close_pos) = impl_block.rfind('}') {
                            impl_block.insert_str(close_pos, &format!("\n{from_json_method}\n"));
                        }
                    }
                }
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
            }
        }
        config_opaque::emit_wrappers(
            &mut builder,
            config,
            &capsule_types,
            &emitted_pyclass_names,
            &error_type_names,
            opaque_types.is_empty(),
        );

        // `@dataclass`/`dict`, not the compiled `#[pyclass]`) so enum-variant payloads of those
        let coercible_dto_names = wire_schema::coercible_dto_names(api, config);
        let coercion_section = wire_schema::emit_dto_coercion_section(api, has_serde, &coercible_dto_names);
        if !coercion_section.is_empty() {
            builder.add_item(&coercion_section);
        }

        for e in &api.enums {
            if generators::enum_has_data_variants(e) {
                // Emit a `#[staticmethod]` constructor for each data-carrying struct variant
                // magic `type="..."` string. The `#[new]` dict/kwargs/string constructor stays as-is
                let data_enum_code =
                    generators::gen_pyo3_data_enum_with_coercion(e, &core_import, Some(&mapper), &coercible_dto_names);
                // in `default_required_types`) but the core `impl Default` is `#[alef(skip)]`'d
                let needs_default = default_required_types.contains(e.name.as_str())
                    && !data_enum_code.contains(&format!("impl Default for {}", e.name));
                builder.add_item(&data_enum_code);
                if needs_default {
                    builder.add_item(&opaque_helpers::emit_inner_default_impl(&e.name));
                }
            } else {
                builder.add_item(&generators::gen_enum(e, &cfg));
            }
        }
        for f in &api.functions {
            if py_exclude_functions.contains(&f.name) {
                continue;
            }
            if bridge_duck_register_fns.contains(f.name.as_str()) {
                continue;
            }
            let bridge_param = crate::backends::pyo3::trait_bridge::find_bridge_param(f, &config.trait_bridges);
            let bridge_field =
                crate::codegen::generators::trait_bridge::find_bridge_field(f, &api.types, &config.trait_bridges);
            if let Some((param_idx, bridge_cfg)) = bridge_param {
                builder.add_item(&crate::backends::pyo3::trait_bridge::gen_bridge_function(
                    api,
                    f,
                    param_idx,
                    bridge_cfg,
                    &mapper,
                    &cfg,
                    &adapter_bodies,
                    &opaque_types,
                    &core_import,
                    &error_converters,
                ));
            } else if let Some(ref bm) = bridge_field {
                builder.add_item(&crate::backends::pyo3::trait_bridge::gen_bridge_field_function(
                    api,
                    f,
                    bm,
                    bm.bridge,
                    &mapper,
                    &cfg,
                    &opaque_types,
                    &core_import,
                    &error_converters,
                ));
            } else if !capsule_types.is_empty() && capsule::function_involves_capsule(f, &capsule_types) {
                builder.add_item(&capsule::gen_capsule_function(
                    f,
                    &capsule_types,
                    &core_import,
                    &error_converters,
                ));
            } else {
                let mut fn_code =
                    generators::gen_function_with_mutex(f, &mapper, &cfg, &adapter_bodies, &opaque_types, &mutex_types);
                if !tokio_mutex_types.is_empty()
                    && fn_code.contains("Arc::new(std::sync::Mutex::new(")
                    && mutex::returns_tokio_mutex_type(f, &tokio_mutex_types)
                {
                    fn_code = fn_code.replace("Arc::new(std::sync::Mutex::new(", "Arc::new(tokio::sync::Mutex::new(");
                }
                builder.add_item(&fn_code);
            }
        }

        // Trait marker classes — emit empty #[pyclass] structs for plugin traits so they can be
        for bridge_cfg in &config.trait_bridges {
            let trait_name = &bridge_cfg.trait_name;
            if !emitted_pyclass_names.insert(trait_name) {
                continue;
            }
            let marker_class = format!("#[pyclass(name = \"{trait_name}\")]\npub struct Py{trait_name}Marker;\n");
            builder.add_item(&marker_class);
        }

        if !config.trait_bridges.is_empty() {
            let needs_async_trait = config.trait_bridges.iter().any(|bridge_cfg| {
                api.types
                    .iter()
                    .find(|t| t.is_trait && t.name == bridge_cfg.trait_name)
                    .is_some_and(|trait_type| trait_type.methods.iter().any(|m| m.is_async))
            });
            if needs_async_trait {
                builder.add_import("async_trait::async_trait");
            }
            if opaque_types.is_empty() {
                builder.add_import("std::sync::Arc");
            }
            let reexported_types = config
                .python
                .as_ref()
                .map(|c| c.reexported_types.clone())
                .unwrap_or_default();
            builder.add_item(&crate::backends::pyo3::template_env::render(
                "trait_bridge/options_from_native_helper.jinja",
                minijinja::context! {
                    options_module => format!("{}.options", config.python_module_name()),
                },
            ));
            for bridge_cfg in &config.trait_bridges {
                if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                    let bridge = crate::backends::pyo3::trait_bridge::gen_trait_bridge(
                        trait_type,
                        bridge_cfg,
                        &core_import,
                        &config.error_type_name(),
                        &config.error_constructor_expr(),
                        api,
                        &reexported_types,
                    )?;
                    for imp in &bridge.imports {
                        builder.add_import(imp);
                    }
                    builder.add_item(&bridge.code);
                }
            }
        }

        let module_name = config.python_module_name();
        let mut seen_exceptions = AHashSet::new();
        for error in &api.errors {
            builder.add_item(&crate::codegen::error_gen::gen_pyo3_error_types(
                error,
                &module_name,
                &mut seen_exceptions,
            ));
            builder.add_item(&crate::codegen::error_gen::gen_pyo3_error_converter(
                error,
                &core_import,
            ));
            // Emit #[pymethods] impl block when the error exposes introspection methods.
            // The impl adds #[getter] properties that read from the exception args tuple
            let methods_impl = crate::codegen::error_gen::gen_pyo3_error_methods_impl(error);
            if !methods_impl.is_empty() {
                builder.add_item(&methods_impl);
            }
        }

        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
        let input_types = crate::codegen::conversions::input_type_names(api);
        let py_field_renames = cfg_fields::py_field_renames(api, config);
        let pyo3_conversion_cfg = crate::codegen::conversions::ConversionConfig {
            option_duration_on_defaults: true,
            binding_field_renames: if py_field_renames.is_empty() {
                None
            } else {
                Some(&py_field_renames)
            },
            opaque_types: Some(&conversion_opaque_set),
            never_skip_cfg_field_names: &never_skip_cfg_field_names,
            ..Default::default()
        };
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if input_types.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &pyo3_conversion_cfg,
                ));
            }
            if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &conversion_opaque_set,
                    &pyo3_conversion_cfg,
                ));
            }
        }
        for e in &api.enums {
            if generators::enum_has_data_variants(e) {
                continue;
            }
            if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core(
                    e,
                    &core_import,
                ));
            }
            if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding(
                    e,
                    &core_import,
                ));
            }
        }

        if has_async {
            builder.add_item(&methods::gen_async_runtime_init());
        }

        builder.add_item(&methods::gen_module_init(&config.python_module_name(), api, config));

        let mut content = builder.build();
        postprocess::clear_bridge_builder_opaque_params(&mut content, config);
        postprocess::add_options_field_visitor_fallback(&mut content, api, config, &core_import);
        postprocess::wrap_optional_default_args(&mut content, api);

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("lib.rs"),
            content,
            generated_header: false,
        }])
    }

    fn generate_type_stubs(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Rust binding does — Python has no `#[cfg]`, so two same-named defs in the `.pyi`
        let deduped_api = api.with_deduped_functions();
        public_files::generate_type_stubs(&deduped_api, config)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let deduped_api = api.with_deduped_functions();
        public_files::generate_public_api(&deduped_api, config)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "maturin",
            crate_suffix: "-py",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}
