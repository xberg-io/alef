//! Selects the context surface a generated Python e2e visitor probes, per callback.
//!
//! A crate may declare several visitor bridges, each with its own `context_type`. A callback is
//! resolved through the bridge whose trait actually declares a method of that name, so a fixture
//! never probes another bridge's context type.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ErrorDef, MethodDef, TypeDef};
use crate::e2e::fixture::{CallbackAction, Fixture};

/// One entry per fixture callback: the callback, its action, and the context probe of the bridge
/// that declares it (if any).
pub(super) type CallbackProbe<'a> = (&'a str, &'a CallbackAction, Option<VisitorContextProbe>);

/// Pair every callback in `fixture` with the probe for its own bridge's context type.
///
/// `convertible_types` is the caller's already-computed
/// [`crate::codegen::conversions::core_to_binding_convertible_types`] result (see
/// `RenderTestFunctionContext::convertible_types` in `test_function.rs`, itself computed once per
/// file in `render_test_file`) -- the same set `effective_options_via_for_type` reuses for its own
/// pyo3 gate, rather than a third recomputation of the same fixpoint here. ~keep
pub(super) fn visitor_callback_probes<'a>(
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    errors: &[ErrorDef],
    convertible_types: &ahash::AHashSet<String>,
    fixture: &'a Fixture,
) -> Vec<CallbackProbe<'a>> {
    fixture
        .visitor
        .iter()
        .flat_map(|visitor_spec| visitor_spec.callbacks.iter())
        .map(|(method_name, action)| {
            (
                method_name.as_str(),
                action,
                visitor_context_probe(config, type_defs, errors, convertible_types, method_name),
            )
        })
        .collect()
}

/// The probe helpers to emit on `_TestVisitor`: one per distinct context type, in first-use order.
pub(super) fn distinct_context_probes<'a>(callbacks: &'a [CallbackProbe<'a>]) -> Vec<&'a VisitorContextProbe> {
    let mut distinct: Vec<&VisitorContextProbe> = Vec::new();
    for probe in callbacks.iter().filter_map(|(_, _, probe)| probe.as_ref()) {
        if !distinct.iter().any(|seen| seen.probe_method == probe.probe_method) {
            distinct.push(probe);
        }
    }
    distinct
}

/// The context surface one visitor callback's `ctx` argument must expose.
#[derive(serde::Serialize)]
pub(super) struct VisitorContextProbe {
    /// Name of the `_TestVisitor` helper that reads this context type. One helper per distinct
    /// context type, so two callbacks on the same bridge share it and two bridges do not.
    pub probe_method: String,
    /// Names read with `getattr`.
    pub attributes: Vec<String>,
    /// Zero-argument instance methods. Read *and called*: `getattr` alone proves only that a name
    /// resolves, which a map-shaped context can satisfy by accident (`dict.items`, `dict.keys`)
    /// while the declared method is absent. Calling is what distinguishes them. ~keep
    pub methods: Vec<String>,
}

impl VisitorContextProbe {
    fn is_empty(&self) -> bool {
        self.attributes.is_empty() && self.methods.is_empty()
    }
}

/// Resolve the probe for `callback_name` through the bridge that declares it, or `None` when no
/// bridge declares that callback, the bridge declares no `context_type`, the context type is
/// absent from the IR, or nothing on it is safe to probe.
pub(super) fn visitor_context_probe(
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    errors: &[ErrorDef],
    convertible_types: &ahash::AHashSet<String>,
    callback_name: &str,
) -> Option<VisitorContextProbe> {
    let context_def = callback_context_type(config, type_defs, errors, convertible_types, callback_name)?;
    let probe = VisitorContextProbe {
        probe_method: format!("_probe_{}", crate::codegen::naming::to_python_name(&context_def.name)),
        attributes: probed_attribute_names(config, context_def),
        methods: probed_method_names(config, context_def),
    };
    (!probe.is_empty()).then_some(probe)
}

/// The context type of the bridge whose trait declares `callback_name`.
///
/// The generated bridge dispatches a callback by its trait method name (`obj.hasattr("<name>")`),
/// so the trait method list is the join key between a fixture callback and its bridge.
///
/// A context type the Python module emits no `#[pyclass]` for is skipped: the bridge hands the
/// callback a `PyDict` for it, and the probe's `getattr` calls describe the class surface, not a
/// dict's. `[crates.python] exclude_types` and `capsule_types` are the removals that leave no IR
/// flag behind, so asking the same
/// [`crate::backends::pyo3::gen_bindings::binding_exclusions::pyclass_absent_type_names`] the
/// bridge and the `.pyi` stub ask is what keeps this generator from probing a surface the binding
/// never publishes.
///
/// A `#[pyclass]` being emitted is necessary but not sufficient: `context_binding_class` in
/// `src/backends/pyo3/trait_bridge/visitor_bridge.rs` gates the bridge's own choice of class vs.
/// dict on two further, independent conditions --
/// [`eligible_context_def`] mirrors them so this probe can never assert `getattr` on a context the
/// bridge is actually handing over as a `PyDict`. ~keep
fn callback_context_type<'a>(
    config: &ResolvedCrateConfig,
    type_defs: &'a [TypeDef],
    errors: &[ErrorDef],
    convertible_types: &ahash::AHashSet<String>,
    callback_name: &str,
) -> Option<&'a TypeDef> {
    let pyclass_absent =
        crate::backends::pyo3::gen_bindings::binding_exclusions::pyclass_absent_type_names(config, type_defs, errors);
    config
        .trait_bridges_for(crate::core::config::Language::Python)
        .find_map(|bridge| {
            let trait_def = type_defs
                .iter()
                .find(|type_def| type_def.is_trait && type_def.name == bridge.trait_name)?;
            trait_def.methods.iter().find(|method| method.name == callback_name)?;
            let context_type = bridge.context_type.as_deref()?;
            if pyclass_absent.contains(context_type) {
                return None;
            }
            let context_def = type_defs.iter().find(|type_def| type_def.name == context_type)?;
            eligible_context_def(context_def, convertible_types)
        })
}

/// The bridge's remaining two conditions from `context_binding_class`, quoted from
/// `src/backends/pyo3/trait_bridge/visitor_bridge.rs`:
///
/// ```text
/// if !context_def.is_clone {
///     return None;
/// }
/// let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
/// crate::codegen::conversions::core_to_binding_from_impl_emitted(context_def, &core_to_binding).then_some(context_def)
/// ```
///
/// `is_clone` is required because the generated `From` impl takes the core value **by value**
/// while the bridge only holds `&core::T`; `core_to_binding_from_impl_emitted` is required because
/// an emitted `#[pyclass]` does not imply an emitted `From` impl for it (e.g. a field that fails
/// the core→binding convertibility fixpoint). Mirroring only the pyclass-presence half of the
/// bridge's condition previously let this probe assert class-shaped `getattr` calls on a context
/// the bridge was actually handing over as a bare dict. ~keep
fn eligible_context_def<'a>(
    context_def: &'a TypeDef,
    convertible_types: &ahash::AHashSet<String>,
) -> Option<&'a TypeDef> {
    if !context_def.is_clone {
        return None;
    }
    crate::codegen::conversions::core_to_binding_from_impl_emitted(context_def, convertible_types)
        .then_some(context_def)
}

/// Field-backed attributes the generated `#[pyclass]` publishes, under the name it publishes them.
///
/// `binding_excluded` fields are dropped by `codegen::shared::binding_fields` before the struct is
/// emitted at all. A `cfg`-gated field's presence depends on which core features the binding was
/// compiled with, which a generated test cannot know, so it is left out rather than asserted on.
/// The surviving names come from the pyo3 backend's own
/// [`crate::backends::pyo3::gen_bindings::python_visible_field_name`], not from a second copy of
/// the rename rule here. ~keep
fn probed_attribute_names(config: &ResolvedCrateConfig, context_def: &TypeDef) -> Vec<String> {
    crate::codegen::shared::binding_fields(&context_def.fields)
        .filter(|field| field.cfg.is_none())
        .map(|field| crate::backends::pyo3::gen_bindings::python_visible_field_name(config, &context_def.name, field))
        .collect()
}

/// Zero-argument instance methods the generated `#[pymethods]` block publishes *and* that a test
/// can call without observing anything but the call itself.
///
/// Every exclusion below drops a method that is either not emitted or not safely callable, so the
/// probe stays a subset of the real surface and can never fail on a method the binding never
/// promised:
/// - `binding_excluded` and `sanitized`: dropped from the emitted `#[pymethods]` block (a
///   sanitized method survives only when an adapter body supplies it, which is not visible here).
/// - static / receiverless: not an attribute of an instance.
/// - `cfg`-gated: presence depends on the compiled feature set, as for fields.
/// - takes parameters: there is no argument the probe could invent.
/// - `async`: calling returns an un-awaited coroutine, which is a warning, not a probe.
/// - fallible (`error_type`): a raised binding error is not the `AttributeError` this is
///   measuring, and inside a visitor callback it would be swallowed by the bridge.
/// - a Python-keyword name: the escaped `#[pymethods]` spelling is not established the way the
///   field rename rule is, so the name is not asserted on.
/// - colliding with a published field name: pyo3 binds one name once, and the emitter drops the
///   method wrapper so the field getter survives. Checked against both the published name and the
///   configured rename so a method dropped under either reading is left alone. ~keep
fn probed_method_names(config: &ResolvedCrateConfig, context_def: &TypeDef) -> Vec<String> {
    let field_names: std::collections::HashSet<String> = context_def
        .fields
        .iter()
        .flat_map(|field| {
            [
                crate::backends::pyo3::gen_bindings::python_visible_field_name(config, &context_def.name, field),
                field.name.clone(),
            ]
        })
        .collect();
    context_def
        .methods
        .iter()
        .filter(|method| is_probeable_method(method))
        .filter(|method| !field_names.contains(&method.name))
        .map(|method| method.name.clone())
        .collect()
}

fn is_probeable_method(method: &MethodDef) -> bool {
    !method.binding_excluded
        && !method.sanitized
        && !method.is_static
        && method.receiver.is_some()
        && method.cfg.is_none()
        && method.params.is_empty()
        && !method.is_async
        && method.error_type.is_none()
        && crate::core::keywords::python_safe_name(&method.name).is_none()
}

#[cfg(test)]
mod tests;
