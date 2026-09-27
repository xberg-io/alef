use crate::backends::pyo3::gen_bindings::binding_exclusions::pyclass_absent_type_names;
use crate::backends::pyo3::trait_bridge::{gen_trait_bridge, gen_trait_bridge_with_absent_types};
use crate::codegen::visitor_context::test_support::neutral_visitor_fixture;
use crate::core::config::{CapsuleTypeConfig, PythonConfig, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::AHashSet;

/// A `PythonConfig` with only the two exclusion surfaces under test populated.
fn python_config(exclude_types: &[&str], capsule_types: &[&str]) -> PythonConfig {
    PythonConfig {
        dependency_versions: Default::default(),
        send_sync_types: Vec::new(),
        module_name: None,
        async_runtime: None,
        stubs: None,
        pip_name: None,
        features: None,
        serde_rename_all: None,
        capsule_types: capsule_types
            .iter()
            .map(|name| {
                (
                    (*name).to_string(),
                    CapsuleTypeConfig::Capsule(format!("sample_package.{name}")),
                )
            })
            .collect(),
        release_gil: false,
        exclude_functions: vec![],
        exclude_types: exclude_types.iter().map(|name| (*name).to_string()).collect(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        sdist_include: Vec::new(),
        scaffold_output: None,
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: Default::default(),
        reexported_types: Vec::new(),
        target_dep_overrides: Vec::new(),
    }
}

fn clone_able_fixture() -> (ApiSurface, TypeDef, TraitBridgeConfig) {
    let (mut api, trait_type, bridge) = neutral_visitor_fixture();
    api.types
        .iter_mut()
        .find(|type_def| type_def.name == "TraversalState")
        .expect("neutral visitor fixture should include its context type")
        .is_clone = true;
    (api, trait_type, bridge)
}

fn render_bridge(
    api: &ApiSurface,
    trait_type: &TypeDef,
    bridge: &TraitBridgeConfig,
    pyclass_absent_types: &AHashSet<String>,
) -> String {
    // Exercises the internal seam directly: this helper's whole point is varying
    // `pyclass_absent_types`, which the public 7-arg `gen_trait_bridge` no longer accepts. ~keep
    let convertible = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
    gen_trait_bridge_with_absent_types(
        trait_type,
        bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        api,
        &[],
        pyclass_absent_types,
        &convertible,
    )
    .expect("visitor bridge should generate")
    .code
}

/// `[crates.python] exclude_types` and `capsule_types` remove a type's `#[pyclass]` without setting
/// any IR flag, so a bridge that derived eligibility from the IR alone emitted
/// `let value: TraversalState = ctx.clone().into();` against a struct the emitter had skipped, and
/// the generated crate did not compile.
///
/// Both directions are asserted deliberately. A change that excluded every context type would
/// satisfy the exclusion half on its own, so the unexcluded run must still take the `#[pyclass]`
/// path in the same fixture for the exclusion to mean anything. ~keep
#[test]
fn a_config_excluded_context_falls_back_to_dict_and_an_unexcluded_one_does_not() {
    let (api, trait_type, bridge) = clone_able_fixture();

    let included = render_bridge(&api, &trait_type, &bridge, &AHashSet::new());
    assert!(
        included.contains("let value: TraversalState = ctx.clone().into();"),
        "control: an unexcluded context must still take the #[pyclass] path, or the exclusion \
         assertions below prove nothing:\n{included}"
    );
    assert!(
        !included.contains("pyo3::types::PyDict::new(py)"),
        "control: an unexcluded context must not fall back to a dict:\n{included}"
    );

    let excluded_names: AHashSet<String> = ["TraversalState".to_string()].into_iter().collect();
    let excluded = render_bridge(&api, &trait_type, &bridge, &excluded_names);
    assert!(
        excluded.contains("pyo3::types::PyDict::new(py)"),
        "a context whose #[pyclass] the config removed must fall back to a dict:\n{excluded}"
    );
    assert!(
        !excluded.contains("ctx.clone().into()"),
        "the bridge must not construct a binding class the emitter skipped:\n{excluded}"
    );
}

/// The set the bridge consults is built from the resolved Python config, not from the IR. This
/// pins the config-to-set link the previous version was missing entirely: neither
/// `exclude_types` nor `capsule_types` leaves a trace on `TypeDef`. ~keep
#[test]
fn python_exclude_types_and_capsule_types_both_reach_the_pyclass_absent_set() {
    let (api, _, _) = neutral_visitor_fixture();

    let mut config = ResolvedCrateConfig::default();
    let empty = pyclass_absent_type_names(&config, &api.types, &api.errors);
    assert!(
        !empty.contains("TraversalState"),
        "control: with no config exclusions the context must be present, or the assertions below \
         would pass for a function that excluded everything"
    );

    config.python = Some(python_config(&["TraversalState"], &[]));
    let via_exclude_types = pyclass_absent_type_names(&config, &api.types, &api.errors);
    assert!(
        via_exclude_types.contains("TraversalState"),
        "`exclude_types` must remove the type from the pyclass surface: {via_exclude_types:?}"
    );

    config.python = Some(python_config(&[], &["TraversalState"]));
    let via_capsule_types = pyclass_absent_type_names(&config, &api.types, &api.errors);
    assert!(
        via_capsule_types.contains("TraversalState"),
        "`capsule_types` must remove the type from the pyclass surface: {via_capsule_types:?}"
    );
}
/// The dict-fallback arm inserted every field with `.unwrap_or(())`, so a value PyO3 could not
/// convert disappeared from the dict silently; the callback then raised a `KeyError` that the
/// bridge's default-result arm swallowed. Each insert now logs its own failure with the field name
/// and the remaining fields are still populated -- the branch is already the degraded shape, so one
/// field's failure must not cost the callback the whole context. ~keep
#[test]
fn a_failed_dict_insert_is_logged_with_its_field_rather_than_discarded() {
    let (api, trait_type, bridge) = clone_able_fixture();
    let excluded_names: AHashSet<String> = ["TraversalState".to_string()].into_iter().collect();
    let code = render_bridge(&api, &trait_type, &bridge, &excluded_names);

    assert!(
        code.contains(r#"if let Err(error) = d.set_item("display_name", &ctx.display_name) {"#),
        "the fallback insert must bind its error rather than dropping it:\n{code}"
    );
    assert!(
        code.contains(r#"field = "display_name""#) && code.contains("visitor context field omitted from fallback dict"),
        "the failure must be logged with the field it lost:\n{code}"
    );
    assert!(
        !code.contains("unwrap_or(())"),
        "no context field may swallow its conversion failure:\n{code}"
    );

    let optional_field_insert = r#"if let Err(error) = d.set_item("parent_label", ctx.parent_label.as_deref()) {"#;
    assert!(
        code.contains(optional_field_insert),
        "every field shape must go through the logging form, not just the plain-string one:\n{code}"
    );
}

/// The `.pyi` protocol stub types the visitor callback's context parameter with the context
/// type's own generated class (`python_type` on `TypeRef::Named` keeps the name), and that
/// class is emitted as a `#[pyclass]` with `#[pyo3(get)]` fields plus its own methods. The
/// bridge used to hand the host a `PyDict` instead, so every attribute access the stub
/// promised raised `AttributeError` at runtime -- and the callback's exception was swallowed
/// by the bridge's default-result arm, making the break silent. ~keep
#[test]
fn visitor_bridge_passes_context_as_binding_class_not_dict() {
    let (mut api, trait_type, bridge) = neutral_visitor_fixture();
    api.types
        .iter_mut()
        .find(|type_def| type_def.name == "TraversalState")
        .expect("neutral visitor fixture should include its context type")
        .is_clone = true;

    let output = gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
        &[],
    )
    .expect("visitor bridge should generate");

    assert!(
        output.code.contains("fn nodecontext_to_py_object"),
        "context helper must build a Python object, not a dict:\n{}",
        output.code
    );
    assert!(
        output.code.contains("let value: TraversalState = ctx.clone().into();"),
        "context helper must construct the generated binding class:\n{}",
        output.code
    );
    assert!(
        output.code.contains("nodecontext_to_py_object(py, state)"),
        "the callback must receive the binding class instance:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("pyo3::types::PyDict::new(py)"),
        "the context must not be flattened into a dict:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("d.set_item(\"display_name\""),
        "the context must not be flattened into a dict:\n{}",
        output.code
    );
}

/// The construction can fail (`Bound::new` returns `PyResult`), and the trait method it sits
/// in is infallible on the Rust side. The error must still reach a log with its message and
/// end the call in the configured default action -- the first version of this fix mapped
/// `Err(_)` to `py.None()`, which handed the callback a `None` typed as the context class and
/// discarded the `PyErr` entirely, trading one silent failure for another. ~keep
#[test]
fn visitor_bridge_propagates_a_context_construction_error() {
    let (mut api, trait_type, bridge) = neutral_visitor_fixture();
    api.types
        .iter_mut()
        .find(|type_def| type_def.name == "TraversalState")
        .expect("neutral visitor fixture should include its context type")
        .is_clone = true;

    let output = gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
        &[],
    )
    .expect("visitor bridge should generate");

    assert!(
        output.code.contains("-> pyo3::PyResult<pyo3::Bound<'py, pyo3::PyAny>>"),
        "the helper must return the construction error, not absorb it:\n{}",
        output.code
    );
    assert!(
        output.code.contains("Ok(pyo3::Bound::new(py, value)?.into_any())"),
        "the helper must propagate `Bound::new`'s error:\n{}",
        output.code
    );
    assert!(
        output
            .code
            .contains("\"failed to build visitor context object; using default action\""),
        "the call site must log the error before falling back:\n{}",
        output.code
    );
    // WARN, not ERROR: the bridge recovers by falling back to the configured default action and
    // continues -- the repo's tracing contract reserves ERROR for unrecoverable failure or data
    // loss, neither of which applies here. ~keep
    assert!(
        output.code.contains("tracing::warn!(wrapper ="),
        "a recovered construction failure must log at WARN, not a higher level:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("tracing::error!(wrapper ="),
        "this call site continues after falling back, so it must not log at ERROR:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("py.None()"),
        "a None stand-in must never reach the callback:\n{}",
        output.code
    );
}

/// A context type that cannot be built from a borrowed core value (no `Clone`, so the
/// generated `From<core::T<'_>>` is out of reach) keeps the map-shaped fallback rather than
/// emitting a reference to a conversion that was never generated. ~keep
#[test]
fn visitor_bridge_keeps_dict_fallback_for_non_clone_context() {
    let (api, trait_type, bridge) = neutral_visitor_fixture();

    let output = gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
        &[],
    )
    .expect("visitor bridge should generate");

    assert!(
        output.code.contains("pyo3::types::PyDict::new(py)"),
        "non-Clone context must keep the dict fallback:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("ctx.clone().into()"),
        "non-Clone context must not emit a clone-based conversion:\n{}",
        output.code
    );
}

/// `Clone` is necessary but not sufficient: the `From<core::T>` the bridge calls is emitted
/// only for types in `core_to_binding_convertible_types`, and this context type is knocked out
/// of it by a field whose `type_rust_path` points at a different type than the enum of the
/// same name (`field_has_path_mismatch`). Asking that shared set -- rather than a local
/// `is_clone && !is_opaque && !binding_excluded` guess -- is what stops the bridge emitting
/// `.into()` against a conversion the type emitter never wrote. ~keep
#[test]
fn visitor_bridge_keeps_dict_fallback_when_no_core_to_binding_impl_is_emitted() {
    let (mut api, trait_type, bridge) = neutral_visitor_fixture();
    let context = api
        .types
        .iter_mut()
        .find(|type_def| type_def.name == "TraversalState")
        .expect("neutral visitor fixture should include its context type");
    context.is_clone = true;
    context
        .fields
        .iter_mut()
        .find(|field| field.name == "kind")
        .expect("context fixture should include its enum field")
        .type_rust_path = Some("unrelated_crate::TraversalKind".to_string());

    let convertible = crate::codegen::conversions::core_to_binding_convertible_types(&api, &[]);
    let context_def = api
        .types
        .iter()
        .find(|type_def| type_def.name == "TraversalState")
        .expect("context type stays in the surface");
    assert!(
        !crate::codegen::conversions::core_to_binding_from_impl_emitted(context_def, &convertible),
        "fixture must actually be outside the emitted-conversion set, or this test proves nothing"
    );

    let output = gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
        &[],
    )
    .expect("visitor bridge should generate");

    assert!(
        output.code.contains("pyo3::types::PyDict::new(py)"),
        "a context with no emitted From impl must keep the dict fallback:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("ctx.clone().into()"),
        "the bridge must not call a conversion the type emitter never wrote:\n{}",
        output.code
    );
}

#[test]
fn visitor_context_local_avoids_parameters_and_preserves_tuple_order() {
    let (mut api, mut trait_type, bridge) = clone_able_fixture();
    let method = trait_type
        .methods
        .iter_mut()
        .find(|method| method.name == "inspect_node")
        .expect("neutral fixture should include inspect_node");
    let context_index = method
        .params
        .iter()
        .position(|param| matches!(&param.ty, crate::core::ir::TypeRef::Named(name) if name == "TraversalState"))
        .expect("inspect_node should receive the visitor context");
    let context_name = method.params[context_index].name.clone();
    method.params.insert(
        context_index + 1,
        crate::core::ir::ParamDef {
            name: format!("{context_name}_py"),
            ty: crate::core::ir::TypeRef::String,
            is_ref: true,
            ..Default::default()
        },
    );
    api.types
        .iter_mut()
        .find(|type_def| type_def.name == trait_type.name)
        .expect("API should include the visitor trait")
        .methods = trait_type.methods.clone();

    let code = render_bridge(&api, &trait_type, &bridge, &AHashSet::new());
    let collision = format!("{context_name}_py");
    let unique = format!("{context_name}_py_2");
    assert!(
        code.contains(&format!("let {unique} = match nodecontext_to_py_object")),
        "generated local must not shadow the existing `{collision}` parameter:\n{code}"
    );
    assert!(
        code.contains(&format!("({unique}, {collision},")),
        "the collision-safe local must replace only the context argument and retain exact parameter order:\n{code}"
    );
}
