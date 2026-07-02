use crate::codegen::generators::{AsyncPattern, RustBindingConfig};

pub(super) fn binding_config(core_import: &str, has_serde: bool) -> RustBindingConfig<'_> {
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
        emit_delegating_default_impl: true,
        skip_methods_when_not_delegatable: false,
        source_crate_remaps: &[],
        // Populated in gen_bindings before the type loop so that the delegating Default
        // is only emitted when the matching From<core::T> impl will also be emitted.
        emit_delegating_default_for_types: None,
    }
}

/// Variant of `binding_config` that uses `unsendable` instead of `frozen` for types
/// that contain `Rc<...>`-based handles (e.g. visitor handles).  PyO3 requires either
/// `Send + Sync` (for `frozen`) or the `unsendable` marker (for single-threaded types).
pub(super) fn unsendable_binding_config(core_import: &str, has_serde: bool) -> RustBindingConfig<'_> {
    RustBindingConfig {
        struct_attrs: &["pyclass(unsendable, from_py_object)"],
        ..binding_config(core_import, has_serde)
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
pub(super) fn cfg_present_for_pyo3(cfg: &str) -> bool {
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
