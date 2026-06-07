//! PyO3 (Python) backend: orchestration and `Backend` trait implementation.

pub mod capsule;
pub mod enums;
pub mod errors;
pub mod functions;
pub mod methods;
pub mod service_api;
pub mod types;

use crate::backends::pyo3::type_map::Pyo3Mapper;
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators::{self, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::binding_fields;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use std::path::PathBuf;

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
            opaque_type_names: &[],
            skip_impl_constructor: false,
            cast_uints_to_i32: false,
            cast_large_ints_to_f64: false,
            named_non_opaque_params_by_ref: false,
            lossy_skip_types: &[],
            serializable_opaque_type_names: &[],
            never_skip_cfg_field_names: &[],
            emit_delegating_default_impl: false,
            skip_methods_when_not_delegatable: false,
        }
    }

    /// Variant of `binding_config` that uses `unsendable` instead of `frozen` for types
    /// that contain `Rc<...>`-based handles (e.g. visitor handles).  PyO3 requires either
    /// `Send + Sync` (for `frozen`) or the `unsendable` marker (for single-threaded types).
    fn unsendable_binding_config(core_import: &str, has_serde: bool) -> RustBindingConfig<'_> {
        RustBindingConfig {
            struct_attrs: &["pyclass(unsendable, from_py_object)"],
            ..Self::binding_config(core_import, has_serde)
        }
    }
}

/// Whether a `#[cfg(...)]` predicate is satisfied for PyO3 bindings.
///
/// PyO3 bindings always compile for native CPython (never WASM), so we can include
/// cfg-gated fields in constructor signatures if either:
/// - The field is gated on `not(target_arch = "wasm32")` (always present on native)
/// - The field is gated on a feature (statically enabled when kompiling pyo3 module)
///
/// This is safe because the PyO3 compilation unit is directly controlled by the
/// binding's `Cargo.toml` (sample_core-py), which explicitly lists all features
/// like `pdf`, `html`, `tree-sitter`, etc. Unlike FFI-based bindings that link
/// against a separately-compiled core library, pyo3 builds the core with known
/// features, so feature gates are deterministic at binding-compilation time.
fn cfg_present_for_pyo3(cfg: &str) -> bool {
    let normalized: String = cfg.chars().filter(|c| !c.is_whitespace()).collect();
    // Accept `not(target_arch="wasm32")` — always true on native Python
    if normalized == "not(target_arch=\"wasm32\")" {
        return true;
    }
    // Accept feature gates — pyo3 features are statically enabled/disabled
    if normalized.starts_with("feature=") {
        return true;
    }
    // Accept `any(...)` containing only feature gates and native-target gates
    if normalized.starts_with("any(") && normalized.ends_with(")") {
        let inner = &normalized[4..normalized.len() - 1];
        return inner
            .split(',')
            .all(|part| part.starts_with("feature=") || part == "not(target_arch=\"wasm32\")");
    }
    false
}

/// Replace the constructor in an impl block with one that honors serde_rename.
/// For has_default types, the constructor parameters should use serde_rename names
/// (the JSON wire names) to match other language bindings' public APIs.
/// This function finds the existing constructor and replaces it with a custom one.
/// Also adds extra parameters for any options-field bridges (e.g., visitor).
#[allow(clippy::too_many_arguments)]
fn replace_constructor_with_serde_rename(
    impl_block: &str,
    typ: &crate::core::ir::TypeDef,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    config: &crate::codegen::generators::RustBindingConfig,
    config_renames: Option<&std::collections::HashMap<String, String>>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    never_skip_cfg_field_names: &[String],
    api: &crate::core::ir::ApiSurface,
) -> String {
    use crate::codegen::shared::binding_fields;
    use crate::core::keywords::{is_valid_rust_ident_chars, rust_raw_ident};

    // When the type already has an explicit static `new()` method in its IR, do not
    // emit a second field-based `#[new]` constructor — the static method will be emitted
    // as `#[staticmethod] pub fn new(...)` and PyO3 forbids two `new` registrations in
    // the same impl block (E0592 duplicate definitions).
    let has_explicit_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");
    if has_explicit_new {
        return impl_block.to_string();
    }

    /// Check if a field should be emitted as Option<T> to accept None for BLK-5 fix.
    /// This applies when:
    /// - The parent type has_default=true
    /// - The field is non-optional (!f.optional && not already Optional)
    /// - The field type is a Named type
    /// - The referenced type has has_default=true
    fn should_option_for_nested_default(
        typ: &crate::core::ir::TypeDef,
        field: &crate::core::ir::FieldDef,
        api: &crate::core::ir::ApiSurface,
    ) -> bool {
        if !typ.has_default || field.optional || matches!(&field.ty, crate::core::ir::TypeRef::Optional(_)) {
            return false;
        }
        if let crate::core::ir::TypeRef::Named(ref type_name) = field.ty {
            api.types
                .iter()
                .find(|t| t.name == *type_name)
                .map(|t| t.has_default)
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Resolve the constructor parameter identifier for a field.
    ///
    /// Prefers the serde rename (or config rename) over the bare Rust field name, but only
    /// when the resolved name is a syntactically valid Rust identifier (i.e. contains only
    /// `[A-Za-z0-9_]` and does not start with a digit).  Names like `"self-harm"` or
    /// `"self-harm/intent"` (containing hyphens or slashes) are not valid Rust identifiers
    /// even with `r#` escaping, so the function falls back to the Rust field name in those
    /// cases.  When the resolved name is valid but happens to be a Rust keyword (e.g. `"type"`),
    /// it is escaped as a raw identifier (`r#type`).
    fn resolve_param_ident<'a>(
        field_name: &'a str,
        serde_rename: Option<&'a String>,
        config_renames: Option<&std::collections::HashMap<String, String>>,
    ) -> String {
        let wire_name = serde_rename
            .map(|s| s.as_str())
            .or_else(|| config_renames.and_then(|r| r.get(field_name)).map(|s| s.as_str()))
            .unwrap_or(field_name);
        if is_valid_rust_ident_chars(wire_name) {
            rust_raw_ident(wire_name)
        } else {
            // Wire name contains characters that are not valid in a Rust identifier (e.g. hyphens,
            // slashes in serde renames like "self-harm" or "self-harm/intent").  Fall back to the
            // Rust field name, which is guaranteed to be a valid identifier.
            field_name.to_string()
        }
    }

    // Check if this type has an options-field bridge (e.g., ParseOptions.visitor).
    // The bridge field is appended later via `bridge_param`; filter it out of
    // `sorted_fields` so we do not emit it twice when the field is force-restored
    // through `never_skip_cfg_field_names`.
    let bridge_field_name = trait_bridges
        .iter()
        .find(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && b.options_type.as_deref() == Some(&typ.name)
        })
        .and_then(|b| b.resolved_options_field());

    // Build parameter list with serde_rename and config-based renames.
    // Include cfg-gated fields that the consumer has force-restored via
    // `never_skip_cfg_field_names` (e.g. sample_core-py builds with all features so
    // pdf_options / keywords / html_* / layout / tree_sitter need to be kwargs).
    let mut sorted_fields: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| !f.binding_excluded && (f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name)))
        .filter(|f| bridge_field_name.is_none() || f.name != bridge_field_name.unwrap())
        .collect();
    sorted_fields.sort_by_key(|f| f.optional as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            // Use serde_rename if available (and valid), otherwise the Rust field name.
            // Keywords are escaped as raw identifiers (e.g. "type" → "r#type").
            let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

            // Determine if this field should be optional in the constructor.
            // This matches the logic in gen_struct_with_per_field_attrs (structs.rs lines 128-131).
            let force_optional = config.option_duration_on_defaults
                && typ.has_default
                && !f.optional
                && matches!(f.ty, crate::core::ir::TypeRef::Duration);

            // BLK-5 fix: for non-optional nested-struct fields on has_default types,
            // if the nested struct also has_default, emit as Option<T> to accept None.
            let nested_default_optional = should_option_for_nested_default(typ, f, api);

            let ty = if (f.optional || force_optional || nested_default_optional)
                && !matches!(f.ty, crate::core::ir::TypeRef::Optional(_))
            {
                // All optional constructor parameters are emitted as Option<T>.
                // The IR unwraps TypeRef::Optional to mark fields as optional,
                // so we need to re-wrap the base type for the constructor signature.
                // Skip re-wrapping when the IR field type is *already* Optional —
                // that happens for Update structs where a source field of
                // `Option<Option<T>>` peels to `f.optional = true, f.ty = Optional(T)`.
                // Mirrors the same guard in `gen_struct_with_per_field_attrs`.
                format!("Option<{}>", mapper.map_type(&f.ty))
            } else {
                mapper.map_type(&f.ty)
            };
            format!("{}: {}", param_ident, ty)
        })
        .collect();

    let bridge_param = trait_bridges
        .iter()
        .find(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && b.options_type.as_deref() == Some(&typ.name)
        })
        .and_then(|b| {
            let param_name = b.param_name.as_deref()?;
            Some((param_name, b.type_alias.as_deref().unwrap_or("object")))
        });

    let defaults: Vec<String> = sorted_fields
        .iter()
        .filter(|f| bridge_field_name.is_none() || f.name != bridge_field_name.unwrap())
        .map(|f| {
            // PyO3 strips the `r#` prefix when deriving the Python-facing keyword argument
            // name, so `r#type` in the signature → Python `type`.
            let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

            // Same force_optional logic as above.
            let force_optional = config.option_duration_on_defaults
                && typ.has_default
                && !f.optional
                && matches!(f.ty, crate::core::ir::TypeRef::Duration);

            // BLK-5 fix: for non-optional nested-struct fields on has_default types,
            // if the nested struct also has_default, emit default as None.
            let nested_default_optional = should_option_for_nested_default(typ, f, api);

            if f.optional || force_optional || nested_default_optional {
                format!("{}=None", param_ident)
            } else if typ.has_default {
                // For has_default types, non-optional fields get a default value in the signature
                // so the generated `__new__` is callable with keyword args omitted.
                // The field's default is `Self::default().<field>`.
                format!("{}=Self::default().{}", param_ident, f.name)
            } else {
                // For non-has_default types, required fields have no default in the signature
                // (they are required keyword arguments).
                param_ident
            }
        })
        .collect();

    // Struct literal uses bare Rust field names (never renamed).
    // For non-cfg fields: use constructor parameters (with explicit form if renamed).
    // For cfg-gated fields: initialize with default (None for Option types, Default::default() otherwise).
    // Bridge fields are handled separately below.
    let assignments: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded && (bridge_field_name.is_none() || f.name != bridge_field_name.unwrap()))
        .map(|f| {
            if f.cfg.is_some() && !never_skip_cfg_field_names.contains(&f.name) {
                // Cfg-gated field that was NOT force-restored: not a constructor parameter, use default
                if f.optional {
                    format!("{}: None", f.name)
                } else {
                    format!("{}: Default::default()", f.name)
                }
            } else {
                // Non-cfg field: use constructor parameter.
                // Use the same resolve_param_ident logic so the struct literal references
                // exactly the same variable as the parameter declaration.
                let param_ident = resolve_param_ident(&f.name, f.serde_rename.as_ref(), config_renames);

                // BLK-5 fix: for nested-struct fields emitted as Option<T> due to has_default,
                // use unwrap_or_else to fall back to the nested type's default.
                let nested_default_optional = should_option_for_nested_default(typ, f, api);

                // The binding struct's Rust field name is python-keyword-escaped
                // (e.g. `from` -> `from_`), so the LEFT side of the struct literal must
                // match that escaped name, not the core IR field name.
                let binding_field = crate::core::keywords::python_ident(&f.name);
                if nested_default_optional {
                    // Use unwrap_or_else for nested default optional fields
                    format!(
                        "{}: {}.unwrap_or_else(|| Self::default().{})",
                        binding_field, param_ident, binding_field
                    )
                } else if param_ident != binding_field {
                    // Parameter name differs from binding struct field name:
                    // use explicit form to match the parameter variable
                    format!("{}: {}", binding_field, param_ident)
                } else {
                    // Names match: use shorthand
                    binding_field
                }
            }
        })
        .collect();

    // Add bridge parameter to defaults and params if present.
    //
    // The bridge parameter's *type* must match the struct field it ultimately
    // populates — the struct literal below emits a bare `visitor: visitor`,
    // which would fail to compile if the parameter type and field type differ
    // (e.g. user-facing `VisitorHandle` pyclass vs the binding's internal
    // `PyVisitorRef` wrapper). Look up the matching field's actual mapped
    // type and use it. Fall back to the bridge's `type_alias` only when the
    // bridge field can't be located in the struct (which would be a bug, but
    // preserves the prior behaviour).
    let mut all_defaults = defaults.clone();
    let mut all_params = params.clone();
    if let Some((param_name, type_alias)) = bridge_param {
        let field_type = bridge_field_name
            .and_then(|fname| typ.fields.iter().find(|f| f.name == fname))
            .map(|f| mapper.map_type(&f.ty))
            .unwrap_or_else(|| type_alias.to_string());
        // The field's mapped type may already include `Option<...>` (it
        // typically does, since bridge fields are optional). Avoid double-
        // wrapping by checking for the prefix.
        let param_type = if field_type.starts_with("Option<") {
            field_type
        } else {
            format!("Option<{}>", field_type)
        };
        all_params.push(format!("{}: {}", param_name, param_type));
        all_defaults.push(format!("{}=None", param_name));
    }

    let param_list = if all_params.join(", ").len() > 100 {
        format!("\n        {},\n    ", all_params.join(",\n        "))
    } else {
        all_params.join(", ")
    };

    // Build the assignment for the bridge field
    let mut all_assignments = assignments.clone();
    if let Some((param_name, _)) = bridge_param {
        if let Some(field_name) = bridge_field_name {
            all_assignments.push(format!("{}: {}", field_name, param_name));
        }
    }

    // Build the new constructor method (without impl wrapper — we'll inject it into existing impl)
    let new_constructor = format!(
        "    #[allow(clippy::too_many_arguments)]\n    \
         #[must_use]\n    \
         #[pyo3(signature = ({}))]#[new]\n    \
         pub fn new({}) -> Self {{\n        \
         Self {{ {} }}\n    \
         }}",
        all_defaults.join(", "),
        param_list,
        all_assignments.join(", ")
    );

    // Find and replace the old constructor in the impl block
    // Look for the pattern that includes the signature and fn new
    if let Some(start) = impl_block.find("#[pyo3(signature = (") {
        if let Some(new_start) = impl_block[..start].rfind("\n") {
            // Find the end of the constructor (closing brace of the function)
            if let Some(fn_new_pos) = impl_block.find("pub fn new(") {
                // Find the closing brace of this constructor
                let mut brace_count = 0;
                let mut in_fn = false;
                let mut end_pos = None;

                for (i, c) in impl_block[fn_new_pos..].chars().enumerate() {
                    if c == '{' {
                        in_fn = true;
                        brace_count += 1;
                    } else if c == '}' && in_fn {
                        brace_count -= 1;
                        if brace_count == 0 {
                            end_pos = Some(fn_new_pos + i + 1);
                            break;
                        }
                    }
                }

                if let Some(end) = end_pos {
                    let before = &impl_block[..new_start + 1];
                    let after = &impl_block[end..];
                    return format!("{}{}{}", before, new_constructor, after);
                }
            }
        }
    }

    // Fallback: if we can't find the constructor to replace, return the original
    impl_block.to_string()
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
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
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
        let mut cfg = Self::binding_config(&core_import, has_serde);
        let mut cfg_unsendable = Self::unsendable_binding_config(&core_import, has_serde);

        // Build adapter body map for method body substitution
        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Python)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        // Suppress documentation and cast lints in generated code — doc comments are provided
        // by Python stubs (.pyi), and the numeric casts are intentional FFI conversions.
        builder.add_inner_attribute("allow(missing_docs)");
        builder.add_inner_attribute("allow(deprecated, dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute(
            "allow(clippy::default_trait_access, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::let_unit_value, clippy::needless_borrow, clippy::too_many_arguments, clippy::map_identity, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions, clippy::useless_conversion, clippy::arc_with_non_send_sync, clippy::collapsible_if, clippy::clone_on_copy, clippy::should_implement_trait)",
        );
        // Pedantic/nursery lints that don't apply to autogenerated FFI bindings:
        // - `unsafe_derive_deserialize`: pyclasses derive Serialize/Deserialize for the
        //   pyo3 conversion path; the unsafe in `Drop`/raw-pointer methods is unrelated
        //   to construction invariants.
        // - `must_use_candidate` / `return_self_not_must_use`: every binding accessor and
        //   builder returns a value the caller will use; flagging each is noise.
        // - `use_self`: the wrapper functions intentionally name the surface type so the
        //   generated source reads naturally next to the rustdoc / .pyi stubs.
        // - `missing_const_for_fn`: pyclass methods aren't allowed to be `const fn`.
        // - `missing_errors_doc`: errors are documented in the .pyi stub, not the impl.
        // - `needless_pass_by_value`: `from_py_object` types are conventionally moved.
        // - `doc_markdown` / `derive_partial_eq_without_eq`: derive choice + docs come
        //   from the analyzed Rust source, not under the generator's control.
        // - `uninlined_format_args` / `redundant_clone` / `implicit_clone` /
        //   `redundant_closure_for_method_calls` / `wildcard_imports` / `option_if_let_else`:
        //   stylistic — improvements would require per-template rewrites with no
        //   functional impact on generated FFI code.
        builder.add_inner_attribute(
            "allow(clippy::unsafe_derive_deserialize, clippy::must_use_candidate, clippy::return_self_not_must_use, clippy::use_self, clippy::missing_const_for_fn, clippy::missing_errors_doc, clippy::needless_pass_by_value, clippy::doc_markdown, clippy::derive_partial_eq_without_eq, clippy::uninlined_format_args, clippy::redundant_clone, clippy::implicit_clone, clippy::redundant_closure_for_method_calls, clippy::wildcard_imports, clippy::option_if_let_else, clippy::too_many_lines)",
        );
        // Capsule-type functions use multiple separate `unsafe {}` blocks, each wrapping a
        // single CPython FFI call.  The `multiple_unsafe_ops_per_block` lint fires even when
        // each block contains exactly one operation, because capsule functions have two
        // consecutive unsafe blocks.  Suppress it — the SAFETY comments on each block are
        // the authoritative documentation.
        builder.add_inner_attribute("allow(clippy::multiple_unsafe_ops_per_block)");
        // Capsule-type functions use `unsafe { PyCapsule_New(...) }` and
        // `unsafe { Bound::from_owned_ptr(...) }` — these are intentional, well-documented
        // CPython FFI calls.  Downstreams that have `unsafe_code = "deny"` at the workspace
        // level (e.g. parser-language-pack) must not need to add per-crate overrides.
        builder.add_inner_attribute("allow(unsafe_code)");
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

        // Check if we have opaque types and add Arc import if needed
        let opaque_types: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
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
        let mut never_skip_cfg_field_names: Vec<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| {
                if b.bind_via == crate::core::config::BridgeBinding::OptionsField {
                    b.resolved_options_field().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
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
        for typ in api.types.iter().filter(|t| t.has_default && !t.is_trait) {
            for field in binding_fields(&typ.fields) {
                let Some(cfg) = field.cfg.as_deref() else {
                    continue;
                };
                let present = !typ.has_stripped_cfg_fields || cfg_present_for_pyo3(cfg);
                if present && !never_skip_cfg_field_names.contains(&field.name) {
                    never_skip_cfg_field_names.push(field.name.clone());
                }
            }
        }
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

        // PyVisitorRef: a thin wrapper around Py<PyAny> that implements Clone.
        // This newtype makes Py<PyAny> work with PyO3's #[pyclass] field derivations,
        // which require Clone. Uses std::sync::Arc to make the handle cheaply cloneable
        // without needing the GIL (Clone doesn't require GIL entry, only Arc::clone).
        let py_visitor_ref_def = r#"
/// Wrapper for trait visitor types (Py<PyAny>) that implements Clone.
///
/// Py<PyAny> is not Clone. This wrapper uses Arc<Py<PyAny>> internally for cheap cloning.
/// The .inner field is public for compatibility with generated code that needs to access
/// the underlying Py<PyAny> for trait dispatch.
#[derive(Debug)]
pub struct PyVisitorRef {
    pub inner: std::sync::Arc<pyo3::Py<pyo3::PyAny>>,
}

impl Clone for PyVisitorRef {
    fn clone(&self) -> Self {
        PyVisitorRef {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

impl From<pyo3::Py<pyo3::PyAny>> for PyVisitorRef {
    fn from(visitor: pyo3::Py<pyo3::PyAny>) -> Self {
        PyVisitorRef {
            inner: std::sync::Arc::new(visitor),
        }
    }
}

impl<'a, 'py> pyo3::FromPyObject<'a, 'py> for PyVisitorRef {
    type Error = pyo3::PyErr;

    fn extract(ob: pyo3::Borrowed<'a, 'py, pyo3::PyAny>) -> pyo3::PyResult<Self> {
        Ok(PyVisitorRef {
            inner: std::sync::Arc::new(ob.to_owned().unbind()),
        })
    }
}

impl<'py> pyo3::conversion::IntoPyObject<'py> for PyVisitorRef {
    type Target = pyo3::PyAny;
    type Output = pyo3::Bound<'py, pyo3::PyAny>;
    type Error = std::convert::Infallible;

    fn into_pyobject(self, py: pyo3::Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok((*self.inner).bind(py).clone())
    }
}
"#;
        builder.add_item(py_visitor_ref_def);

        // Serde helper for fields where the Python binding stores JSON as a `String` but
        // the input may be either a JSON string or a raw value. Used via
        // `#[serde(default, deserialize_with = "alef_json_str::deserialize")]` on Json
        // fields so `from_json` accepts both `"parameters": "{...}"` and the more
        // ergonomic `"parameters": {...}`.
        let alef_json_helper = r#"
mod alef_json_str {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;
    pub fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        Ok(match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }
}

mod alef_json_str_opt {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Option<Value> = Option::deserialize(deserializer)?;
        Ok(v.and_then(|val| match val {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(other.to_string()),
        }))
    }
}
"#;
        builder.add_item(alef_json_helper);

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
        // Declared opaque types are external host-runtime references — they cannot be
        // wrapped as #[pyclass] because their actual Rust path carries generic params
        // that the injected IR cannot model.
        py_exclude_types.extend(config.opaque_types.keys().cloned());
        // Types listed in capsule_types bypass #[pyclass] generation entirely — they are
        // passed through as raw PyCapsule handles or Python-side-constructed objects.
        let capsule_types = config
            .python
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();

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
        let mut emitted_pyclass_names: AHashSet<&str> = AHashSet::new();

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
                    struct_code = rewrite_to_tokio_mutex_struct(&struct_code);
                    impl_block = rewrite_to_tokio_mutex_impl(&impl_block);
                }
                // Rewrite methods whose return type is a capsule type so they produce
                // PyCapsule objects instead of the (non-existent) #[pyclass] wrapper structs.
                if !capsule_types.is_empty() {
                    impl_block = rewrite_capsule_methods(impl_block, typ, &capsule_types, &error_converters);
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
                    && let Some(ctor_body) = variant_wrapper_constructor_body(typ, &mapper)
                {
                    impl_block = inject_into_impl_block(&impl_block, &ctor_body);
                }
                builder.add_item(&struct_code);
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
                // Emit `impl Default for Type` when the type has a no-arg new() constructor
                // to satisfy clippy's `new_without_default` lint
                if should_emit_default_impl(typ, &impl_block) {
                    builder.add_item(&emit_default_impl(&typ.name));
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
                impl_block = replace_constructor_with_serde_rename(
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
        for e in &api.enums {
            if generators::enum_has_data_variants(e) {
                builder.add_item(&generators::gen_pyo3_data_enum(e, &core_import));
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
                    && returns_tokio_mutex_type(f, &tokio_mutex_types)
                {
                    fn_code = fn_code.replace("Arc::new(std::sync::Mutex::new(", "Arc::new(tokio::sync::Mutex::new(");
                }
                builder.add_item(&fn_code);
            }
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
        let mut py_field_renames = std::collections::HashMap::new();
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for field in binding_fields(&typ.fields) {
                if let Some(escaped) =
                    config.resolve_field_name(crate::core::config::Language::Python, &typ.name, &field.name)
                {
                    py_field_renames.insert(format!("{}.{}", typ.name, field.name), escaped);
                }
            }
        }
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
        let stubs_config = match config.python.as_ref().and_then(|c| c.stubs.as_ref()) {
            Some(s) => s,
            None => return Ok(vec![]),
        };

        let stubs_exclude_functions: AHashSet<String> = config
            .python
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let content =
            crate::backends::pyo3::gen_stubs::gen_stubs(api, &config.trait_bridges, config, &stubs_exclude_functions);

        let stubs_path = resolve_output_dir(
            Some(&stubs_config.output),
            &config.name,
            stubs_config.output.to_string_lossy().as_ref(),
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&stubs_path).join(format!("{}.pyi", config.python_module_name())),
            content,
            generated_header: true,
        }])
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_name = config.python_module_name();

        // Use stubs output path as the package directory (e.g., packages/python/sample_markdown/)
        // This ensures we write to the correct Python package, not the Rust crate name.
        let output_base = config
            .python
            .as_ref()
            .and_then(|p| p.stubs.as_ref())
            .map(|s| PathBuf::from(&s.output))
            .unwrap_or_else(|| {
                let package_name = config.name.replace('-', "_");
                PathBuf::from(format!("packages/python/{}", package_name))
            });
        let package_name = output_base
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| config.name.replace('-', "_"));

        let mut files = vec![];

        // 1. Generate options.py (enums and dataclasses)
        let options_content = types::gen_options_py(api, &module_name, &config.dto);
        files.push(GeneratedFile {
            path: output_base.join("options.py"),
            content: options_content,
            generated_header: true,
        });

        // 2. Generate api.py (wrapper functions)
        let exclude_functions: AHashSet<String> = config
            .python
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let capsule_types = config
            .python
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
        let reexported_types = config
            .python
            .as_ref()
            .map(|c| c.reexported_types.clone())
            .unwrap_or_default();
        let api_content = functions::gen_api_py(
            api,
            &module_name,
            &package_name,
            &config.trait_bridges,
            &config.dto,
            &capsule_types,
            &config.adapters,
            &reexported_types,
            &exclude_functions,
        );
        files.push(GeneratedFile {
            path: output_base.join("api.py"),
            content: api_content,
            generated_header: true,
        });

        // 3. Generate exceptions.py (exception hierarchy)
        let exceptions_content = errors::gen_exceptions_py(api);
        files.push(GeneratedFile {
            path: output_base.join("exceptions.py"),
            content: exceptions_content,
            generated_header: true,
        });

        // 4. Generate __init__.py (re-exports)
        let extra_init_imports = config
            .python
            .as_ref()
            .map(|c| c.extra_init_imports.clone())
            .unwrap_or_default();
        let init_content = errors::gen_init_py(
            api,
            &module_name,
            &api.version,
            &config.dto,
            &config.trait_bridges,
            &extra_init_imports,
            &capsule_types,
            &config.adapters,
            &config.opaque_types,
            &exclude_functions,
        );
        files.push(GeneratedFile {
            path: output_base.join("__init__.py"),
            content: init_content,
            generated_header: true,
        });

        Ok(files)
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

/// Rewrite opaque impl-block methods whose return type is a capsule type.
///
/// The generic method generator emits `Ok({CapsuleType} { inner: Arc::new(result) })` for
/// methods returning capsule-configured types.  Because capsule types have no `#[pyclass]`
/// struct, that code does not compile.  This function replaces each such method with a
/// capsule-aware body that either calls `into_raw()` + `PyCapsule_New` (Capsule variant) or
/// constructs the Python object via the dependency capsule (ConstructFrom variant), mirroring
/// what `capsule::gen_capsule_function` does for free functions.
fn rewrite_capsule_methods(
    impl_block: String,
    typ: &crate::core::ir::TypeDef,
    capsule_types: &std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    error_converters: &[String],
) -> String {
    use crate::codegen::type_mapper::TypeMapper as _;
    use crate::core::ir::TypeRef;
    use heck::ToSnakeCase;

    let mut result = impl_block;

    for method in &typ.methods {
        // Determine whether this method's return type is a capsule type.
        let capsule_ret_name: Option<&str> = match &method.return_type {
            TypeRef::Named(n) if capsule_types.contains_key(n.as_str()) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if capsule_types.contains_key(n.as_str()) {
                        Some(n.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };

        // Check if any parameter is a capsule type.
        let has_capsule_param = method
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Named(n) if capsule_types.contains_key(n.as_str())));

        // Skip methods that don't involve capsules in parameters or return type.
        if capsule_ret_name.is_none() && !has_capsule_param {
            continue;
        }

        // If we're only handling parameter extraction (no return capsule), emit a simpler body.
        let cfg = capsule_ret_name.map(|n| &capsule_types[n]);

        // Build the old signature fragment that the generic generator emitted.
        let old_sig_search = if let Some(ret_name) = capsule_ret_name {
            // Methods returning capsules: search for `-> PyResult<{CapsuleTypeName}>`
            format!("-> PyResult<{ret_name}>")
        } else {
            // Methods only with capsule params: search for method name + opening paren.
            // We'll match by method name pattern and update params + body.
            format!("pub fn {}(", method.name)
        };

        // For methods returning capsules, verify the signature exists.
        if capsule_ret_name.is_some() && !result.contains(&old_sig_search) {
            continue;
        }

        // Detect capsule-type parameters and prepare extraction code.
        let mut capsule_param_extract = String::new();
        let mut call_args_parts: Vec<String> = Vec::new();

        for p in &method.params {
            let param_is_capsule = matches!(&p.ty, TypeRef::Named(n) if capsule_types.contains_key(n.as_str()));

            if param_is_capsule {
                if let TypeRef::Named(capsule_name) = &p.ty {
                    // Generate extraction code for this capsule parameter
                    capsule_param_extract.push_str(&crate::backends::pyo3::template_env::render(
                        "pyo3_capsule_param_extract.jinja",
                        minijinja::context! {
                            param_name => p.name.as_str(),
                            capsule_name => capsule_name,
                        },
                    ));
                    call_args_parts.push(p.name.clone());
                } else {
                    // Fallback for non-Named types
                    call_args_parts.push(p.name.clone());
                }
            } else {
                let needs_borrow = p.is_ref && matches!(p.ty, TypeRef::String | TypeRef::Char);
                if needs_borrow {
                    call_args_parts.push(format!("&{}", p.name));
                } else {
                    call_args_parts.push(p.name.clone());
                }
            }
        }
        let call_args_str = call_args_parts.join(", ");

        // Build param list for the new signature.
        // Always prepend `py: pyo3::Python<'_>` since we need it for PyCapsule_New / Python calls.
        let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
        let mut sig_params = vec!["&self".to_string(), "py: pyo3::Python<'_>".to_string()];
        for p in &method.params {
            // Capsule-type parameters are accepted as Py<PyAny>, not as the Rust type
            let param_type = if matches!(&p.ty, TypeRef::Named(n) if capsule_types.contains_key(n.as_str())) {
                "pyo3::Py<pyo3::PyAny>".to_string()
            } else {
                mapper.map_type(&p.ty)
            };
            sig_params.push(format!("{}: {}", p.name, param_type));
        }

        // Build the #[pyo3(signature = (...))] attribute (skipped when there are no params).
        let sig_attr = if method.params.is_empty() {
            String::new()
        } else {
            let names = method
                .params
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("    #[pyo3(signature = ({names}))]\n")
        };

        // Build the inner core call (self.inner.method(args)).
        let core_call = format!("self.inner.{}({})", method.name, call_args_str);

        // Build the `.map_err(…)?` suffix when the method is fallible.
        let err_map_suffix = if method.error_type.is_some() {
            let converter = method
                .error_type
                .as_ref()
                .and_then(|et| {
                    let short = et.split("::").last().unwrap_or(et.as_str());
                    let candidate = format!("{}_to_py_err", short.to_snake_case());
                    if error_converters.iter().any(|c| c == &candidate) {
                        Some(candidate)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string())".to_string());
            format!(".map_err({converter})?")
        } else {
            String::new()
        };

        let params_str = sig_params.join(", ");
        let method_name = &method.name;

        // Generate the new method body based on capsule variant.
        // For methods with only capsule params (no return capsule), emit a simple wrapper.
        let new_body = if cfg.is_none() {
            // Method only has capsule params, no capsule return.
            // Just rewrite to extract capsule params and call the inner method.
            let return_annotation = if matches!(method.return_type, TypeRef::Unit) {
                "".to_string()
            } else {
                format!(" -> PyResult<{}>", mapper.map_type(&method.return_type))
            };
            format!(
                r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}){return_annotation} {{
{capsule_param_extract}        {core_call}{err_map_suffix}
    }}"#,
            )
        } else if let Some(cfg) = cfg {
            // Method returns a capsule (and may also have capsule params).
            match cfg {
                crate::core::config::CapsuleTypeConfig::Capsule(capsule_name_str) => {
                    let capsule_cstr = capsule_name_str.replace('.', "_").to_ascii_uppercase();
                    // If capsule_name_str is dotted (e.g. "tree_sitter.Language"), also construct the
                    // target Python type from the capsule so callers receive a real tree_sitter.Language,
                    // not the bare PyCapsule.
                    let construct = match capsule_name_str.rsplit_once('.') {
                    Some((module_path, class_name)) => format!(
                        r#"        // SAFETY: capsule_ptr is a valid, non-null Python object pointer we just created above.
        let _capsule_obj = unsafe {{ pyo3::Bound::from_owned_ptr(py, capsule_ptr) }};
        let _ts_mod = py.import("{module_path}")?;
        let _cls = _ts_mod.getattr("{class_name}")?;
        Ok(_cls.call1((_capsule_obj,))?.unbind())"#,
                    ),
                    None => {
                        "        // SAFETY: capsule_ptr is a valid, non-null Python object pointer we just created above.\n        Ok(unsafe { pyo3::Bound::from_owned_ptr(py, capsule_ptr) }.unbind())".to_string()
                    }
                };
                    format!(
                        r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {{
        const {capsule_cstr}_NAME: &::std::ffi::CStr = c"{capsule_name_str}";
{capsule_param_extract}        let result = {core_call}{err_map_suffix};
        let raw_ptr = result.into_raw();
        // SAFETY: raw_ptr is a valid pointer derived from into_raw() on a value with program lifetime.
        let capsule_ptr = unsafe {{ pyo3::ffi::PyCapsule_New(raw_ptr as *mut _, {capsule_cstr}_NAME.as_ptr(), None) }};
        if capsule_ptr.is_null() {{
            return Err(pyo3::exceptions::PyRuntimeError::new_err("Failed to create PyCapsule"));
        }}
{construct}
    }}"#,
                    )
                }
                crate::core::config::CapsuleTypeConfig::ConstructFrom {
                    python_type,
                    construct_from,
                } => {
                    // For ConstructFrom: produce the dependency capsule by calling the matching
                    // free function, then call the Python factory to construct the target type.
                    let dep_snake = construct_from.to_snake_case();
                    let first_str_param = method.params.iter().find(|p| matches!(p.ty, TypeRef::String));
                    let dep_expr = if let Some(sp) = first_str_param {
                        format!("get_{dep_snake}(py, {}.clone())?.bind(py).clone()", sp.name)
                    } else {
                        format!("/* Unsupported: obtain {construct_from} capsule */ unreachable!()")
                    };

                    if let Some((module_path, class_name)) = python_type.rsplit_once('.') {
                        format!(
                            r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {{
        // Construct {python_type} via Python-side factory.
        let _dep = {dep_expr};
        let _ts_mod = py.import("{module_path}")?;
        let _cls = _ts_mod.getattr("{class_name}")?;
        Ok(_cls.call1((_dep,))?.unbind())
    }}"#,
                        )
                    } else {
                        format!(
                            r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {{
        // Construct {python_type} via Python-side factory.
        let _dep = {dep_expr};
        let _cls = py.eval(c"{python_type}", None, None)?;
        Ok(_cls.call1((_dep,))?.unbind())
    }}"#,
                        )
                    }
                }
            }
        } else {
            unreachable!("Method capsule config should be present when cfg.is_none() is false.");
        };

        // Find and replace the old method in the impl block.
        // The method generator emits `pub fn {name}(` at the start of a line with no
        // guaranteed leading indentation (the impl_block template wraps the content but
        // doesn't add per-line indentation).  Search for the bare `pub fn {name}(`.
        let method_start_marker = format!("pub fn {method_name}(");
        if let Some(start_idx) = result.find(&method_start_marker) {
            let attr_start = find_method_attrs_start(&result, start_idx);
            if let Some(end_idx) = find_method_end(&result, start_idx) {
                result = format!("{}{}{}", &result[..attr_start], new_body, &result[end_idx..]);
            }
        }
    }

    result
}

/// Returns true when `line` (trimmed) consists entirely of `#[…]` attribute patterns and
/// intervening whitespace — i.e. it contains no non-attribute tokens such as `impl Foo {`.
///
/// This correctly handles:
/// - A single attribute: `#[pyo3(signature = (name))]`  → true
/// - Multiple attributes on one line: `#[allow(dead_code)]  #[pyo3(get)]`  → true
/// - A block-attr + impl opener on one line: `#[pymethods]impl Foo {`  → false
fn is_method_attr_line(line: &str) -> bool {
    let mut rest = line.trim();
    if rest.is_empty() {
        return false; // handled by the blank-line branch; don't treat blank as attr
    }
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if !rest.starts_with("#[") {
            return false;
        }
        // Consume the `#[…]` span, respecting nested brackets.
        let mut depth = 0usize;
        let mut consumed = 0usize;
        let mut found_close = false;
        for (i, ch) in rest.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        consumed = i + 1;
                        found_close = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !found_close {
            return false;
        }
        rest = &rest[consumed..];
    }
}

/// Find the byte index of the start of the attribute block that precedes the `pub fn` at
/// `fn_idx`.  Walks backward line-by-line past `#[…]` attribute lines and blank lines.
/// Stops as soon as it encounters a line that is not purely made of `#[…]` attributes
/// (e.g. `#[pymethods]impl Foo {`).  Returns the byte index of the first character of the
/// first method-attribute line (or `fn_idx` when there are none).
fn find_method_attrs_start(code: &str, fn_idx: usize) -> usize {
    let before = &code[..fn_idx];
    // Collect line-start byte offsets so we can walk backward.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(before.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    let mut attr_start_byte = fn_idx;
    // Walk the line-start offsets in reverse (skip the last one — that is the `pub fn` line).
    for &line_byte_start in line_starts.iter().rev() {
        let line = &before[line_byte_start..before.len().min(attr_start_byte)];
        let trimmed = line.trim_end_matches('\n').trim();
        if trimmed.is_empty() || is_method_attr_line(trimmed) {
            attr_start_byte = line_byte_start;
        } else {
            break;
        }
    }
    attr_start_byte
}

/// Find the byte index just after the closing `}` of a Rust method block whose `pub fn`
/// starts at byte `fn_idx` in `code`.
fn find_method_end(code: &str, fn_idx: usize) -> Option<usize> {
    let slice = &code[fn_idx..];
    let mut depth = 0usize;
    let mut found_open = false;
    let mut byte_offset = 0usize;
    for ch in slice.chars() {
        match ch {
            '{' => {
                depth += 1;
                found_open = true;
            }
            '}' if found_open => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    byte_offset += ch.len_utf8();
                    return Some(fn_idx + byte_offset);
                }
            }
            _ => {}
        }
        byte_offset += ch.len_utf8();
    }
    None
}

/// Whether a free function's return type involves an opaque type that requires the
/// tokio variant of `Mutex` (because every `&mut self` method on that type is async).
fn returns_tokio_mutex_type(func: &crate::core::ir::FunctionDef, tokio_mutex_types: &AHashSet<String>) -> bool {
    use crate::core::ir::TypeRef;
    fn check(ty: &TypeRef, set: &AHashSet<String>) -> bool {
        match ty {
            TypeRef::Named(n) => set.contains(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => check(inner, set),
            _ => false,
        }
    }
    check(&func.return_type, tokio_mutex_types)
}

fn rewrite_to_tokio_mutex_struct(struct_code: &str) -> String {
    struct_code.replace("Arc<std::sync::Mutex<", "Arc<tokio::sync::Mutex<")
}

fn rewrite_to_tokio_mutex_impl(impl_code: &str) -> String {
    impl_code
        .replace("Arc<std::sync::Mutex<", "Arc<tokio::sync::Mutex<")
        .replace("Arc::new(std::sync::Mutex::new(", "Arc::new(tokio::sync::Mutex::new(")
        .replace(".lock().unwrap()", ".lock().await")
}

/// For a wrapper type referenced by registration variants (i.e. one whose
/// `is_variant_wrapper` flag is set by the extractor), produce a `#[new]
/// pub fn py_new(...) -> Self { Self::new(...) }` method body suitable for
/// in-place insertion into the type's existing `#[pymethods] impl T { ... }`
/// block via [`inject_into_impl_block`].
///
/// Returns `None` when the wrapper has no `new` method (or the constructor's
/// receiver is not static) — the variant body would not compile in that
/// case either, but we silently skip rather than panic so the rest of the
/// surface can still be generated for diagnosis.
fn variant_wrapper_constructor_body(typ: &crate::core::ir::TypeDef, mapper: &Pyo3Mapper) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);
    let call_args = ctor
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    // The binding wrapper's static `new` already does the
    // `binding-side type → core-side type` conversion for each argument and
    // produces a `Self`; we just delegate.
    let body = if call_args.is_empty() {
        "Self::new()".to_string()
    } else {
        format!("Self::new({call_args})")
    };
    Some(format!(
        "    #[new]\n    pub fn py_new({sig_params}) -> Self {{\n        {body}\n    }}\n"
    ))
}

/// Check if a type has a no-arg `pub fn new() -> Self` method (either static or constructor).
/// This is used to determine whether we should emit an `impl Default for Type` block.
///
/// Returns true only when:
/// - The type has `has_default = true` (indicating it has impl Default in core Rust)
/// - The type has at least one method named "new"
/// - That method takes no parameters and is static (receiver.is_none())
/// - No existing `impl Default` is already present in the impl_block
fn should_emit_default_impl(typ: &crate::core::ir::TypeDef, impl_block: &str) -> bool {
    // Only emit if the core Rust type has impl Default
    if !typ.has_default {
        return false;
    }

    // Check if Default impl already exists
    if impl_block.contains("impl Default") {
        return false;
    }

    // Check if there's a no-arg static new() method
    typ.methods.iter().any(|m| {
        m.name == "new" && m.params.is_empty() && m.receiver.is_none() // static method (not &self or &mut self)
    })
}

/// Generate an `impl Default for Type { fn default() -> Self { Self::new() } }` block
/// for a no-arg constructor. This satisfies clippy's `new_without_default` lint.
fn emit_default_impl(type_name: &str) -> String {
    format!("impl Default for {type_name} {{\n    fn default() -> Self {{\n        Self::new()\n    }}\n}}\n")
}

/// Inject a method body into the existing `#[pymethods] impl T { ... }`
/// block produced by `gen_opaque_impl_block`. The block ends with a closing
/// `}`; the body is inserted right before it.
fn inject_into_impl_block(impl_block: &str, body: &str) -> String {
    let trimmed = impl_block.trim_end();
    let Some(close_idx) = trimmed.rfind('}') else {
        return impl_block.to_string();
    };
    let (head, tail) = trimmed.split_at(close_idx);
    let head_trimmed = head.trim_end();
    format!("{head_trimmed}\n\n{body}{tail}\n")
}

#[cfg(test)]
mod tests {
    use super::{Pyo3Backend, cfg_present_for_pyo3, rewrite_to_tokio_mutex_impl, rewrite_to_tokio_mutex_struct};
    use crate::core::backend::Backend;
    use crate::core::config::Language;

    /// Pyo3Backend::name returns "pyo3".
    #[test]
    fn pyo3_backend_name_is_pyo3() {
        let b = Pyo3Backend;
        assert_eq!(b.name(), "pyo3");
    }

    /// Pyo3Backend::language returns Language::Python.
    #[test]
    fn pyo3_backend_language_is_python() {
        let b = Pyo3Backend;
        assert_eq!(b.language(), Language::Python);
    }

    /// rewrite_to_tokio_mutex_struct replaces std::sync::Mutex with tokio::sync::Mutex in struct.
    #[test]
    fn rewrite_tokio_mutex_struct_replaces_std_mutex() {
        let input = "pub inner: Arc<std::sync::Mutex<MyType>>";
        let result = rewrite_to_tokio_mutex_struct(input);
        assert_eq!(result, "pub inner: Arc<tokio::sync::Mutex<MyType>>");
    }

    /// rewrite_to_tokio_mutex_struct is a no-op when no std::sync::Mutex is present.
    #[test]
    fn rewrite_tokio_mutex_struct_noop_when_no_std_mutex() {
        let input = "pub inner: Arc<tokio::sync::Mutex<MyType>>";
        let result = rewrite_to_tokio_mutex_struct(input);
        assert_eq!(result, input);
    }

    /// rewrite_to_tokio_mutex_impl replaces all three patterns in impl block.
    #[test]
    fn rewrite_tokio_mutex_impl_replaces_all_patterns() {
        let input = concat!(
            "pub inner: Arc<std::sync::Mutex<MyType>>,\n",
            "Self { inner: Arc::new(std::sync::Mutex::new(val)) }\n",
            "let guard = self.inner.lock().unwrap();\n",
        );
        let result = rewrite_to_tokio_mutex_impl(input);
        assert!(result.contains("Arc<tokio::sync::Mutex<MyType>>"));
        assert!(result.contains("Arc::new(tokio::sync::Mutex::new(val))"));
        assert!(result.contains("self.inner.lock().await"));
    }

    /// rewrite_to_tokio_mutex_impl is a no-op when no std patterns are present.
    #[test]
    fn rewrite_tokio_mutex_impl_noop_when_already_tokio() {
        let input = concat!(
            "pub inner: Arc<tokio::sync::Mutex<MyType>>,\n",
            "Self { inner: Arc::new(tokio::sync::Mutex::new(val)) }\n",
            "let guard = self.inner.lock().await;\n",
        );
        let result = rewrite_to_tokio_mutex_impl(input);
        assert_eq!(result, input);
    }

    /// `cfg_present_for_pyo3` accepts `not(target_arch = "wasm32")` gates.
    #[test]
    fn cfg_present_for_pyo3_accepts_non_wasm_gate() {
        assert!(cfg_present_for_pyo3("not(target_arch = \"wasm32\")"));
        assert!(cfg_present_for_pyo3("not (target_arch = \"wasm32\")"));
    }

    /// `cfg_present_for_pyo3` accepts feature gates since pyo3 compiles with known features.
    #[test]
    fn cfg_present_for_pyo3_accepts_feature_gates() {
        assert!(cfg_present_for_pyo3("feature = \"pdf\""));
        assert!(cfg_present_for_pyo3("feature = \"html\""));
        assert!(cfg_present_for_pyo3("feature=\"tree-sitter\""));
        assert!(cfg_present_for_pyo3(
            "any(feature=\"keywords-yake\", feature=\"keywords-rake\")"
        ));
    }

    /// `cfg_present_for_pyo3` rejects unsupported gates.
    #[test]
    fn cfg_present_for_pyo3_rejects_unsupported_gates() {
        assert!(!cfg_present_for_pyo3("target_arch = \"wasm32\""));
        assert!(!cfg_present_for_pyo3("any(unix, windows)"));
        assert!(!cfg_present_for_pyo3("any(unix, feature=\"pdf\")"));
    }
}
