//! PyO3 (Python) backend: orchestration and `Backend` trait implementation.

pub mod capsule;
mod capsule_methods;
mod cfg_fields;
mod config;
mod config_opaque;
mod constructors;
pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
mod mutex;
mod opaque_helpers;
mod public_files;
pub mod service_api;
mod support_items;
#[cfg(test)]
mod tests;
pub mod types;

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
        // Collapse same-named cfg-variant functions (e.g. a `not(windows)` real impl plus a
        // `windows` variant of the same fn) into one canonical entry. The pyo3 wrapper delegates
        // to the core crate — which resolves the cfg itself — and emits no `#[cfg]` gate on the
        // wrapper, so two same-named entries would otherwise produce duplicate `#[pyfunction]`
        // definitions (E0428) plus duplicate `m.add_function` registrations. Matches the FFI
        // backend's dedup; see codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

        // Build trait type names set so the mapper can emit Py<PyAny> for trait parameters
        // instead of bare trait names (which cause E0782 "bare trait used as type").
        //
        // Also include type_alias names from options-field bridges (e.g. `VisitorHandle`).
        // These are opaque types (is_opaque=true, is_trait=false) but they represent visitor
        // handles embedded as fields in has_default structs.  When they appear as struct field
        // types (e.g. ParseOptions.visitor: Option<VisitorHandle>), the binding struct
        // should store them as `Option<Py<PyAny>>` with `#[serde(skip)]` so the visitor can
        // be extracted before the serde round-trip in the bridge function.  Without this, the
        // mapper emits `Option<VisitorHandle>` which cannot implement `serde::Serialize`.
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

        // Detect serde availability from the output crate's Cargo.toml
        let output_dir = resolve_output_dir(config.output_paths.get("python"), &config.name, "crates/{name}-py/src/");
        let has_serde = detect_serde_available(&output_dir);
        let mut cfg = config::binding_config(&core_import, has_serde);
        let mut cfg_unsendable = config::unsendable_binding_config(&core_import, has_serde);

        // Build adapter body map for method body substitution
        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Python)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        support_items::add_generated_module_attributes(&mut builder);
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
        // `sample_crate::ParseOptions`) in generated code, so no
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

        // Check if we have opaque types and add Arc import if needed.
        // Also include config.opaque_types entries without a Python capsule override — they get
        // binding-side #[pyclass] wrapper structs and must be treated as opaque in return wrapping
        // so functions that return them emit `Language { inner: Arc::new(val) }` not `val.into()`.
        let mut opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        // Capsule types bypass #[pyclass] generation entirely; opaque types that
        // are also capsule types must NOT be added to opaque_types here.
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
        // Data enums (enums with data variants) are also generated as opaque wrappers —
        // include them so structs containing these types skip Default/Serialize/Deserialize.
        let data_enum_names: Vec<String> = api
            .enums
            .iter()
            .filter(|e| generators::enum_has_data_variants(e))
            .map(|e| e.name.clone())
            .collect();
        // Trait bridge type aliases are opaque — they map to Arc<Py<PyAny>> in the binding
        // layer and must not attempt From/Into conversion. Include them so struct fields
        // referencing these types use Default::default() and skip serialization.
        let bridge_type_aliases: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        // Build a separate set for From impl generation: only true opaque types and bridge type
        // aliases. Data enums (like StructureKind) have their own From impls via gen_pyo3_data_enum
        // and their fields can be converted with val.field.into() — do not Default::default() them.
        let conversion_opaque_set: AHashSet<String> =
            opaque_types.iter().chain(bridge_type_aliases.iter()).cloned().collect();
        let mut opaque_names_vec: Vec<String> = opaque_types.iter().cloned().collect();
        let serializable_opaque_names_vec: Vec<String> = data_enum_names.clone();
        opaque_names_vec.extend(data_enum_names);
        opaque_names_vec.extend(bridge_type_aliases);
        // Mirror the Vec in a HashSet so the transitive-closure loop's
        // membership check is O(1) instead of O(n) per type per iteration.
        // `field_references_opaque_type` still takes a slice (its public
        // signature is fixed by other callers), but that is bounded by a
        // single type's field count and not the per-iteration hot path.
        let mut opaque_names_set: AHashSet<String> = opaque_names_vec.iter().cloned().collect();
        // Transitively close: any non-opaque type whose fields reference an opaque/data-enum
        // type also can't derive Default/Serialize/Deserialize.
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
        // Force-restore cfg-gated config fields into pyo3 constructor signatures so the
        // generated api.py can pass them as kwargs without TypeError. Without this the
        // emitted `#[new]` filters out fields with `f.cfg.is_some()`, but the python
        // `_to_rust_extraction_config` helper always passes pdf_options/keywords/html_*/
        // layout etc. as kwargs and crashes at runtime.
        let never_skip_cfg_field_names = cfg_fields::never_skip_cfg_field_names(api, config);
        // Force-restore cfg-gated fields into the constructor when they are present in this
        // binding's compilation unit, so the generated api.py can pass them as kwargs without
        // a runtime TypeError. A field is restored when either:
        //   * the type has no stripped cfg fields at all (every field is unconditionally
        //     compiled in), or
        //   * the field's own cfg predicate holds on a native target — pyo3 always targets a
        //     native CPython host, so `not(target_arch = "wasm32")` fields are always present.
        // Feature gates and other predicates we cannot prove are left out (conservative): the
        // crate may be built without that feature, so the field stays defaulted in the struct
        // literal instead of becoming a parameter that fails to compile.
        cfg.never_skip_cfg_field_names = &never_skip_cfg_field_names;
        cfg_unsendable.never_skip_cfg_field_names = &never_skip_cfg_field_names;
        let mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        // Subset of mutex_types where every &mut self method is async — the binding wrapper
        // must use `tokio::sync::Mutex` (Send-across-await guard) to satisfy PyO3's `Send`
        // future bound. See `type_needs_tokio_mutex` for rationale.
        let tokio_mutex_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_tokio_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
            // Only import std::sync::Mutex when at least one mutex type does NOT use the
            // tokio variant; the rewriter substitutes `std::sync::Mutex` → `tokio::sync::Mutex`
            // inline (so no separate import is needed for tokio types).
            if mutex_types.iter().any(|n| !tokio_mutex_types.contains(n)) {
                builder.add_import("std::sync::Mutex");
            }
        }

        // Check if we have Map types and add HashMap import if needed.
        // Maps can appear in: struct fields, function parameters/returns, and opaque type methods.
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
            builder.add_import("std::collections::HashMap"); // Used in Map field conversions and method returns
        }

        support_items::add_py_visitor_ref(&mut builder);

        support_items::add_json_helpers(&mut builder);

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::Python);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        // Service-API glue lives in the generated `service.rs`; declare it so its
        // `#[pyfunction]` entrypoints are compiled and can be registered in the module init.
        if !api.services.is_empty() {
            builder.add_item("pub mod service;");
        }

        // Add adapter-generated standalone items (streaming iterators, callback bridges)
        for adapter in &config.adapters {
            match adapter.pattern {
                AdapterPattern::Streaming => {
                    let key = crate::adapters::stream_struct_key(adapter);
                    if let Some(struct_code) = adapter_bodies.get(&key) {
                        // Don't import item_type — the binding crate defines its own
                        // wrapper struct with the same name. The streaming struct should
                        // use the local wrapper type, not the core type.
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
        let mut py_exclude_types: ahash::AHashSet<String> = config
            .python
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();
        // Service owner types and handler-contract traits are marked binding_excluded
        // by the service extraction pass: they are emitted by generate_service_api,
        // not the generic struct/trait codegen, so skip them in the generic loop too.
        py_exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
        // Types listed in capsule_types bypass #[pyclass] generation entirely — they are
        // passed through as raw PyCapsule handles or Python-side-constructed objects.
        let capsule_types = config
            .python
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
        config_opaque::exclude_capsule_opaque_types(&mut py_exclude_types, config, &capsule_types);

        // Collect all names that will be emitted as pyo3::create_exception! macros.
        // This includes both the base error enum name AND all variant exception names
        // (which may differ from the variant name, e.g. "Validation" variant → "ValidationError"
        // exception name via python_exception_name). Any struct type sharing one of these names
        // must be skipped to avoid E0428 duplicate definition errors.
        let mut error_type_names: AHashSet<String> = AHashSet::new();
        for error in &api.errors {
            error_type_names.insert(error.name.clone());
            for variant in &error.variants {
                let exc_name = crate::codegen::error_gen::python_exception_name(&variant.name, &error.name);
                error_type_names.insert(exc_name);
            }
        }

        // Build the list of error converter function names available in the generated module.
        // These follow the pattern `{snake_error}_to_py_err` for each error in api.errors.
        // Used by bridge function generators and capsule method rewriters to dispatch typed
        // exceptions instead of PyRuntimeError when the IR records a generic error type.
        let error_converters: Vec<String> = api
            .errors
            .iter()
            .map(|e| {
                use heck::ToSnakeCase;
                format!("{}_to_py_err", e.name.to_snake_case())
            })
            .collect();

        // Track emitted #[pyclass] struct names to prevent duplicate definitions (E0255/E0428).
        // Duplicates can slip through when path-mapping collapses two distinct raw paths onto
        // the same name after dedup has already run on the pre-mapping IR.
        // Also used to skip wrapper emission for opaque_types already covered by the IR loop.
        let mut emitted_pyclass_names: AHashSet<&str> = AHashSet::new();
        // Opaque types that MUST implement `Default` because a `has_default` struct holds them
        // as a non-optional, directly-named field. That parent struct derives `Default`, which
        // only compiles if every non-optional field type is itself `Default`. Optional/collection
        // fields supply their own `Default` (None / empty) and so do not force it. This catches
        // core types whose `impl Default` is `#[alef(skip)]`'d (so `has_default` is false on the
        // type itself) yet are still required to be `Default` by a Default-deriving parent.
        let default_required_types = cfg_fields::default_required_types(api);
        for typ in api
            .types
            .iter()
            .filter(|typ| !typ.is_trait && !py_exclude_types.contains(&typ.name))
        {
            // Error types are emitted as pyo3::create_exception! macros, not as pyclass structs.
            if error_type_names.contains(typ.name.as_str()) {
                continue;
            }
            // Capsule types bypass #[pyclass] entirely — they travel as raw PyCapsule handles
            // or are constructed on the Python side. Emitting a wrapper struct for them would
            // produce an unused #[pyclass] that conflicts with the capsule-based call sites.
            if capsule_types.contains_key(typ.name.as_str()) {
                continue;
            }
            // Skip duplicate struct definitions — only emit the first occurrence.
            if !emitted_pyclass_names.insert(typ.name.as_str()) {
                continue;
            }
            // Only truly opaque types (those with raw FFI pointer handles or non-Send
            // internals such as Rc) must use `unsendable`. Plain data structs that merely
            // reference opaque types in their fields ARE Send + Sync and must use `frozen`
            // so that async Python code can move them between threads without a
            // "<TypeName> is unsendable" panic.
            //
            // We intentionally do NOT use the wider `opaque_names_set` here because that
            // transitive closure includes plain data structs that are themselves Send.
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
                // Rewrite methods whose return type is a capsule type so they produce
                // PyCapsule objects instead of the (non-existent) #[pyclass] wrapper structs.
                if !capsule_types.is_empty() {
                    impl_block =
                        capsule_methods::rewrite_capsule_methods(impl_block, typ, &capsule_types, &error_converters);
                }
                // Variant-wrapper constructor — when the type is referenced as the
                // wrapper of one or more registration variants (and therefore variant
                // bodies emit `WrapperType(args...)` constructor-syntax calls), opt
                // the type into a Python-level constructor by appending a `#[new]
                // pub fn py_new(...) -> Self { Self::new(...) }` to the SAME impl
                // block. pyo3 forbids multiple `#[pymethods] impl T` blocks (without
                // the `multiple-pymethods` feature flag), so the constructor lives
                // alongside the existing `#[staticmethod] pub fn new`. The two
                // coexist by giving the constructor a distinct Rust fn name
                // (`py_new`); pyo3 registers it as Python `__new__` via the
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
                // Emit `impl Default for Type` when the type has a no-arg new() constructor
                // to satisfy clippy's `new_without_default` lint
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
                // so no separate Default impl is needed.
                //
                // Use gen_struct_with_rename so that fields whose names are Python reserved
                // keywords (e.g. `class`) are emitted with an escaped name in the Rust struct
                // (e.g. `class_`) while the original name is preserved in the PyO3 property
                // attribute (e.g. `#[pyo3(get, name = "class")]`) and the serde rename
                // attribute (`#[serde(rename = "class")]`) so the user-facing API is unchanged.
                let type_name = typ.name.clone();
                let config_ref = config;
                builder.add_item(&generators::gen_struct_with_rename(
                    typ,
                    &mapper,
                    type_cfg,
                    |field| {
                        // For Json-typed fields whose Python binding stores `String`,
                        // route deserialisation through the `alef_json_str{,_opt}` helpers
                        // so callers may pass either a JSON-string-encoded value or a
                        // raw object/array (which the helper re-encodes to a string).
                        // The Json-field is detected via TypeRef directly (not via the
                        // mapped type name) so `Option<Json>` cases also match.
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

                        // When the field needs a keyword-escape rename, replace the default
                        // `pyo3(get)` with `pyo3(get, name = "original")` and add a serde
                        // rename attr so JSON serialization still uses the original name.
                        // Returning a non-empty vec here suppresses cfg.field_attrs for this
                        // field (gen_struct_with_rename skips cfg.field_attrs when the name is
                        // overridden AND extra_field_attrs is non-empty).
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
                // Build per-type field renames for the constructor.
                // Only includes config-based renames (keyword escaping like class → class_).
                // serde_rename is handled separately via custom constructor generation.
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

                // Generate impl block with config-based renames (not serde_rename — that's handled below)
                let mut impl_block = generators::gen_impl_block_with_renames(
                    typ,
                    &mapper,
                    type_cfg,
                    &adapter_bodies,
                    &opaque_types,
                    renames_ref,
                );

                // For all types, replace the constructor with one that honors serde_rename
                // For has_default types, fields get default values in the signature.
                // For non-has_default types, required fields stay required.
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
                // is available and a core→binding conversion exists. Injecting into the same block
                // avoids requiring the `multiple-pymethods` pyo3 feature.
                if has_serde && crate::codegen::conversions::core_to_binding_convertible_types(api).contains(&typ.name)
                {
                    let from_json_method = "    #[staticmethod]\n    \
                         fn from_json(json_str: String) -> pyo3::PyResult<Self> {\n        \
                         serde_json::from_str::<Self>(&json_str)\n            \
                         .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))\n    \
                         }"
                    .to_string();
                    if impl_block.is_empty() {
                        // No existing impl block — create one just for from_json.
                        let type_name = &typ.name;
                        impl_block = format!("#[pymethods]\nimpl {type_name} {{\n{from_json_method}\n}}");
                    } else {
                        // Inject before the closing `}` of the existing impl block.
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

        for e in &api.enums {
            if generators::enum_has_data_variants(e) {
                // Emit a `#[staticmethod]` constructor for each data-carrying struct variant
                // (`EmbeddingModelType.preset("balanced")`). These are the type-safe, discoverable
                // idiomatic path — the discriminator is carried by the variant name rather than a
                // magic `type="..."` string. The `#[new]` dict/kwargs/string constructor stays as-is
                // for flexibility; the variant constructors are additive.
                let data_enum_code = generators::gen_pyo3_data_enum_with_mapper(e, &core_import, Some(&mapper));
                // A data enum is rendered as an opaque `{ inner: CoreEnum }` wrapper. The renderer
                // already emits `impl Default` when the core enum's `has_default` is set. When a
                // `Default`-deriving parent struct holds the enum as a non-optional field (tracked
                // in `default_required_types`) but the core `impl Default` is `#[alef(skip)]`'d
                // (so `has_default` is false and no impl was emitted), forward the core `Default`
                // through `inner` ourselves — otherwise the parent's derive fails to compile. Guard
                // against a duplicate impl for enums the renderer already covered (e.g.
                // `CacheBackend`, whose `impl Default` is not skipped).
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
            // Check whether any parameter's type matches a trait bridge type_alias (function-param binding).
            let bridge_param = crate::backends::pyo3::trait_bridge::find_bridge_param(f, &config.trait_bridges);
            // Check whether any parameter's type carries a bridge field (options-field binding).
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
                // Function returns or accepts a capsule type — emit a PyCapsule-aware body
                // instead of the default Arc<> wrapping path.
                builder.add_item(&capsule::gen_capsule_function(
                    f,
                    &capsule_types,
                    &core_import,
                    &error_converters,
                ));
            } else {
                let mut fn_code =
                    generators::gen_function_with_mutex(f, &mapper, &cfg, &adapter_bodies, &opaque_types, &mutex_types);
                // Rewrite std::sync::Mutex → tokio::sync::Mutex when the returned opaque
                // type is in `tokio_mutex_types`. The struct/impl rewriter only touches
                // impl blocks, so apply targeted replacement here for free functions.
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
        // imported and subclassed in Python.  The Rust struct is named `Py<TraitName>Marker` to
        // avoid shadowing the trait import (e.g. `use core_crate::Validator;` would otherwise
        // collide with a `pub struct Validator;`); the PyO3-exposed name still matches the trait
        // so native-module imports resolve correctly on the Python side.
        for bridge_cfg in &config.trait_bridges {
            let trait_name = &bridge_cfg.trait_name;
            // Skip if the trait name was already emitted as a regular type or type alias
            if !emitted_pyclass_names.insert(trait_name) {
                continue;
            }
            let marker_class = format!("#[pyclass(name = \"{trait_name}\")]\npub struct Py{trait_name}Marker;\n");
            builder.add_item(&marker_class);
        }

        // Trait bridge wrappers — generate PyO3 bridge structs that delegate to Python objects
        if !config.trait_bridges.is_empty() {
            // async_trait is only needed for plugin-style bridges (those with async methods).
            // Visitor bridges are fully synchronous, so only add the import when needed.
            let needs_async_trait = config.trait_bridges.iter().any(|bridge_cfg| {
                api.types
                    .iter()
                    .find(|t| t.is_trait && t.name == bridge_cfg.trait_name)
                    .is_some_and(|trait_type| trait_type.methods.iter().any(|m| m.is_async))
            });
            if needs_async_trait {
                builder.add_import("async_trait::async_trait");
            }
            // std::sync::Arc is already conditionally imported above for opaque types;
            // ensure it's present for trait bridges too.
            if opaque_types.is_empty() {
                builder.add_import("std::sync::Arc");
            }
            for bridge_cfg in &config.trait_bridges {
                if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                    let bridge = crate::backends::pyo3::trait_bridge::gen_trait_bridge(
                        trait_type,
                        bridge_cfg,
                        &core_import,
                        &config.error_type_name(),
                        &config.error_constructor_expr(),
                        api,
                    )?;
                    for imp in &bridge.imports {
                        builder.add_import(imp);
                    }
                    builder.add_item(&bridge.code);
                }
            }
        }

        // Error types (create_exception! macros + converter functions)
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
            // populated by the converter above.
            let methods_impl = crate::codegen::error_gen::gen_pyo3_error_methods_impl(error);
            if !methods_impl.is_empty() {
                builder.add_item(&methods_impl);
            }
        }

        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = crate::codegen::conversions::input_type_names(api);
        // Build a rename map for all fields that needed keyword escaping so that From impls
        // use the correct binding struct field names (e.g. `class_` not `class`).
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
        // From/Into conversions — separate sets for each direction
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            // binding→core: strict (no sanitized fields)
            if input_types.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &pyo3_conversion_cfg,
                ));
            }
            // core→binding: permissive (sanitized fields use format!("{:?}"))
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
            // Data enums generate their own From impls inside gen_pyo3_data_enum; skip here.
            if generators::enum_has_data_variants(e) {
                continue;
            }
            // Binding→core: only for enums with simple fields (Default::default() must work)
            if input_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_binding_to_core(
                    e,
                    &core_import,
                ));
            }
            // Core→binding: always possible (data variants discarded with `..`)
            if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&crate::codegen::conversions::gen_enum_from_core_to_binding(
                    e,
                    &core_import,
                ));
            }
        }

        // Async runtime initialization (if needed)
        if has_async {
            builder.add_item(&methods::gen_async_runtime_init());
        }

        // Module init
        builder.add_item(&methods::gen_module_init(&config.python_module_name(), api, config));

        let mut content = builder.build();

        // Post-process generated code to fix bridge type builder methods.
        // Builder methods on has_default types with opaque bridge parameters
        // (e.g., visitor: PyVisitorRef) should not attempt to access .inner,
        // as there is no From impl from Arc<Py<PyAny>> to the core visitor type.
        // Replace patterns like .visitor(visitor.as_ref().map(|v| &v.inner))
        // with .visitor(None) to skip setting the visitor on the core builder.
        for bridge in &config.trait_bridges {
            if let Some(field_name) = bridge.resolved_options_field() {
                let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
                // Simple string replacement for the pattern:
                // .visitor(visitor.as_ref().map(|v| &v.inner))  →  .visitor(None)
                let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
                let replacement = format!(".{}(None)", field_name);
                content = content.replace(&pattern, &replacement);
            }
        }

        // Post-process to add visitor fallback in functions with options-field bridges.
        // When a function parameter is an options type with a visitor field, and the function
        // also has a separate visitor kwarg, the generated code needs to fallback to
        // options.visitor when the separate visitor kwarg is None.
        //
        // This handles the case where Python calls the function with visitor embedded in
        // options, but the Rust function expects visitor as a separate parameter.
        // Replace patterns like:
        //   let visitor_handle: Option<...> = visitor.map(|v| { ... })
        // with fallback logic that also checks options.visitor when visitor is None.
        for bridge in &config.trait_bridges {
            if bridge.bind_via != crate::core::config::BridgeBinding::OptionsField {
                continue;
            }
            if let Some(field_name) = bridge.resolved_options_field() {
                // The fallback below references `o.{field_name}` on the binding's options
                // struct. If the binding does not actually expose that field (e.g. the core
                // field is `#[cfg(feature = "...")]`-gated and the struct generator strips
                // cfg-gated fields), referencing it would fail to compile with `E0609 no
                // field`. Gate the rewrite on the field being present in the binding.
                let Some(options_type) = bridge.options_type.as_deref() else {
                    continue;
                };
                let field_in_binding = api
                    .types
                    .iter()
                    .filter(|t| t.name == options_type)
                    .flat_map(|t| t.fields.iter())
                    .any(|f| f.cfg.is_none() && f.name == field_name);
                if !field_in_binding {
                    continue;
                }
                // Replace the closing pattern of the visitor.map block with a chained .or_else()
                // that pulls from options.visitor when the kwarg is None.
                // Pattern: visitor.map(...) ending with:
                //   std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {resolved_handle_path}
                // });
                //
                // We need to insert .or_else(|| { ... }) before the });
                let handle_path =
                    crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge, &core_import);
                let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge);
                let closing_pattern =
                    format!("        std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    }});");
                if let Some(pos) = content.find(&closing_pattern) {
                    let before = &content[..pos];
                    let after = &content[pos + closing_pattern.len()..];

                    // Build the fallback that tries the configured options field when the kwarg is None.
                    let fallback = format!(
                        "        std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    }}).or_else(|| {{\n        options.as_ref().and_then(|o| o.{field_name}.as_ref()).map(|v| {{\n            let py_obj: pyo3::Py<pyo3::PyAny> = Python::attach(|py| (*v.inner).clone_ref(py));\n            let bridge = {struct_name}::new(py_obj);\n            std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n        }})\n    }});"
                    );

                    content = format!("{}{}{}", before, fallback, after);
                }
            }
        }

        // Fix wrapper functions that pass Option<T> params to core functions expecting Option<T>.
        // When a binding param is Optional<T> and serde deserializes to T, wrap in Some() at call site.
        // The core function expects Option<ParseOptions>, but serde deserialization produces
        // ParseOptions (not Optional). Wrap in Some() when passing to core.
        // Look for patterns like: sample_crate::parse(&source, options_core)
        // and replace with: sample_crate::parse(&source, Some(options_core))
        //
        // CRITICAL: only wrap when the SOURCE param is `Option<T>` — i.e. `param.optional == true`.
        // When the source is non-Option `T`, the core function expects `T` directly and wrapping
        // in `Some()` produces a type error. (Discovered via sample_core `embed_texts` taking
        // `config: EmbeddingConfig` rather than `Option<EmbeddingConfig>`.)
        for func in &api.functions {
            // Check if any parameter is a has_default type
            for param in &func.params {
                if !param.optional {
                    continue;
                }
                if let crate::core::ir::TypeRef::Named(name) = &param.ty {
                    // Check if this is a has_default type
                    if let Some(_typ) = api.types.iter().find(|t| &t.name == name && t.has_default) {
                        // Generate the variable name (param_name + "_core")
                        let core_var = format!("{}_core", param.name);
                        // Pattern: ..., {core_var}) where it appears in a function call
                        // Look for pattern: core_import::function_name(..., param_name_core)
                        let call_pattern = format!(", {core_var})");
                        let call_replacement = format!(", Some({core_var}))");
                        content = content.replace(&call_pattern, &call_replacement);
                    }
                }
            }
        }

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
        // Collapse cfg-variant duplicates (e.g. `ensure_crypto_provider`) the same way the
        // Rust binding does — Python has no `#[cfg]`, so two same-named defs in the `.pyi`
        // stub are a redefinition error.
        let deduped_api = api.with_deduped_functions();
        public_files::generate_type_stubs(&deduped_api, config)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Same cfg-variant collapse for the `api.py` wrapper functions.
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
