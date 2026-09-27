use super::*;

use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{FieldDef, MethodDef, ReceiverKind, TypeRef};

fn field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..FieldDef::default()
    }
}

fn zero_arg_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type: TypeRef::String,
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

fn visitor_trait(name: &str, callbacks: &[&str]) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        is_trait: true,
        methods: callbacks
            .iter()
            .map(|callback| MethodDef {
                name: (*callback).to_string(),
                has_default_impl: true,
                ..MethodDef::default()
            })
            .collect(),
        ..TypeDef::default()
    }
}

fn bridge(trait_name: &str, context_type: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        type_alias: Some(format!("{trait_name}Handle")),
        context_type: Some(context_type.to_string()),
        ..TraitBridgeConfig::default()
    }
}

/// A context type on the bridge's class-path side: `Clone` and otherwise plain, so it also
/// clears `core_to_binding_from_impl_emitted` (a fieldless-or-string-fielded, non-opaque,
/// non-trait type is always in the convertible set). Every fixture in this file that expects a
/// probe uses this, rather than bare `TypeDef::default()` -- which is `is_clone: false` and would
/// now resolve to the dict path. ~keep
fn clone_context(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        is_clone: true,
        fields,
        ..TypeDef::default()
    }
}

/// The core→binding convertible set for a synthetic type slice, mirroring what
/// `render_test_file` computes once per file via `helpers::core_to_binding_convertible_types` and
/// threads down as `RenderTestFunctionContext::convertible_types`. ~keep
fn convertible(type_defs: &[TypeDef]) -> ahash::AHashSet<String> {
    crate::e2e::codegen::python::helpers::core_to_binding_convertible_types(type_defs, &[])
}

/// Two bridges, two context types. Resolving the context from "the first bridge that declares
/// one" applied one bridge's context type to every fixture in the crate, so half the probes
/// asserted attributes the callback's own context never had. ~keep
#[test]
fn each_callback_resolves_the_context_type_of_its_own_bridge() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        visitor_trait("FrameWalker", &["visit_frame"]),
        clone_context("TraversalState", vec![field("node_kind")]),
        clone_context("FrameState", vec![field("frame_id")]),
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![
            bridge("DocumentWalker", "TraversalState"),
            bridge("FrameWalker", "FrameState"),
        ],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);

    let text = visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_text")
        .expect("visit_text has a bridge");
    let frame = visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_frame")
        .expect("visit_frame has a bridge");

    assert_eq!(text.probe_method, "_probe_traversal_state");
    assert_eq!(text.attributes, vec!["node_kind".to_string()]);
    assert_eq!(frame.probe_method, "_probe_frame_state");
    assert_eq!(frame.attributes, vec!["frame_id".to_string()]);
}

#[test]
fn a_callback_no_bridge_declares_gets_no_probe() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        clone_context("TraversalState", vec![field("node_kind")]),
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);

    assert!(visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_image").is_none());
}

/// A `cfg`-gated field is present only when the core crate was compiled with that feature, and a
/// `binding_excluded` one is dropped before the `#[pyclass]` is emitted. Probing either would
/// fail a generated test against a binding that is behaving exactly as configured. ~keep
#[test]
fn cfg_gated_and_excluded_fields_are_not_probed() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        clone_context(
            "TraversalState",
            vec![
                field("node_kind"),
                FieldDef {
                    cfg: Some("feature = \"extras\"".to_string()),
                    ..field("experimental_depth")
                },
                FieldDef {
                    binding_excluded: true,
                    ..field("internal_cursor")
                },
            ],
        ),
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);

    let probe = visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_text")
        .expect("bridge declares a context");
    assert_eq!(probe.attributes, vec!["node_kind".to_string()]);
}

/// Only methods the `#[pymethods]` block actually publishes *and* that a probe can invoke with no
/// arguments and no side effect on the assertion are selected. Each rejected shape below is a
/// method the previous `getattr`-only probe would have happily reported as present. ~keep
#[test]
fn only_callable_zero_arg_instance_methods_are_probed() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        TypeDef {
            name: "TraversalState".to_string(),
            is_clone: true,
            fields: vec![field("node_kind")],
            methods: vec![
                zero_arg_method("attributes"),
                MethodDef {
                    is_static: true,
                    receiver: None,
                    ..zero_arg_method("build_default")
                },
                MethodDef {
                    params: vec![crate::core::ir::ParamDef {
                        name: "index".to_string(),
                        ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize),
                        ..crate::core::ir::ParamDef::default()
                    }],
                    ..zero_arg_method("child_at")
                },
                MethodDef {
                    is_async: true,
                    ..zero_arg_method("resolve_later")
                },
                MethodDef {
                    error_type: Some("SampleError".to_string()),
                    ..zero_arg_method("reparse")
                },
                MethodDef {
                    sanitized: true,
                    ..zero_arg_method("raw_secret")
                },
                MethodDef {
                    binding_excluded: true,
                    ..zero_arg_method("hidden")
                },
                MethodDef {
                    cfg: Some("feature = \"extras\"".to_string()),
                    ..zero_arg_method("experimental")
                },
                zero_arg_method("node_kind"),
            ],
            ..TypeDef::default()
        },
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);

    let probe = visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_text")
        .expect("bridge declares a context");
    assert_eq!(
        probe.methods,
        vec!["attributes".to_string()],
        "only the plain zero-arg instance method survives"
    );
}

/// The fixture-level join: one fixture whose two callbacks belong to two different bridges gets
/// two probe helpers, each callback wired to its own. The previous implementation picked the
/// first `context_type` in the crate config and handed it to every callback in every fixture. ~keep
#[test]
fn a_fixture_spanning_two_bridges_gets_one_probe_helper_per_bridge() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        visitor_trait("FrameWalker", &["visit_frame"]),
        clone_context("TraversalState", vec![field("node_kind")]),
        clone_context("FrameState", vec![field("frame_id")]),
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![
            bridge("DocumentWalker", "TraversalState"),
            bridge("FrameWalker", "FrameState"),
        ],
        ..ResolvedCrateConfig::default()
    };
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "spans_two_bridges",
        "description": "Visits text and frames",
        "visitor": {"callbacks": {
            "visit_text": {"action": "skip"},
            "visit_frame": {"action": "continue"}
        }}
    }))
    .expect("fixture must parse");
    let convertible_types = convertible(&type_defs);

    let callbacks = visitor_callback_probes(&config, &type_defs, &[], &convertible_types, &fixture);
    let wiring: Vec<(&str, Option<&str>)> = callbacks
        .iter()
        .map(|(name, _, probe)| (*name, probe.as_ref().map(|probe| probe.probe_method.as_str())))
        .collect();
    assert_eq!(
        wiring,
        vec![
            ("visit_frame", Some("_probe_frame_state")),
            ("visit_text", Some("_probe_traversal_state")),
        ]
    );

    let distinct = distinct_context_probes(&callbacks);
    assert_eq!(distinct.len(), 2, "each bridge contributes its own probe helper");
}

/// A context type with nothing safely probeable yields no probe at all, rather than an empty
/// helper whose `context_reads > 0` assertion would fire without having read anything.
#[test]
fn a_context_with_no_probeable_surface_yields_no_probe() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        clone_context(
            "TraversalState",
            vec![FieldDef {
                cfg: Some("feature = \"extras\"".to_string()),
                ..field("experimental_depth")
            }],
        ),
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);

    assert!(visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_text").is_none());
}

/// A context type removed by `[crates.python] exclude_types` gets no `#[pyclass]`, so the bridge
/// hands the callback a `PyDict` and the probe's `getattr` calls would describe a surface the
/// binding never publishes. The removal sets no IR flag, which is exactly why this generator, the
/// trait bridge, and the `.pyi` stub all have to read it from the resolved config.
///
/// Both directions are asserted: an identical fixture with the exclusion removed must still yield
/// a probe, or the assertion would also hold for a change that suppressed every probe. ~keep
#[test]
fn a_config_excluded_context_is_not_probed_while_an_unexcluded_one_still_is() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        clone_context("TraversalState", vec![field("node_kind")]),
    ];
    let base = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);

    assert!(
        visitor_context_probe(&base, &type_defs, &[], &convertible_types, "visit_text").is_some(),
        "control: an unexcluded context must still be probed, or the exclusion assertion below \
         would hold for a change that suppressed every probe"
    );

    let mut excluded = base.clone();
    excluded.python = Some(python_config_excluding(&["TraversalState"]));
    assert!(
        visitor_context_probe(&excluded, &type_defs, &[], &convertible_types, "visit_text").is_none(),
        "a context with no generated #[pyclass] must not be probed for class attributes"
    );
}

/// The bridge's own condition (`context_binding_class` in
/// `src/backends/pyo3/trait_bridge/visitor_bridge.rs`) requires `is_clone` in addition to a
/// generated `#[pyclass]`: the generated `From` impl takes the core value by value, and the
/// bridge only holds `&core::T`. A non-`Clone` context is handed to the callback as a `PyDict`,
/// so a probe built from the class surface would fail a completely correct binding -- this was
/// the release-blocking gap: the probe checked pyclass-presence only.
///
/// Both directions are asserted: an identical context with `is_clone: true` must still resolve to
/// the class path, or the assertion below would hold for a change that suppressed every probe. ~keep
#[test]
fn a_non_clone_context_is_not_probed_while_a_clone_one_still_is() {
    let type_defs_for = |is_clone: bool| {
        vec![
            visitor_trait("DocumentWalker", &["visit_text"]),
            TypeDef {
                name: "TraversalState".to_string(),
                is_clone,
                fields: vec![field("node_kind")],
                ..TypeDef::default()
            },
        ]
    };
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };

    let clone_defs = type_defs_for(true);
    assert!(
        visitor_context_probe(&config, &clone_defs, &[], &convertible(&clone_defs), "visit_text").is_some(),
        "control: a Clone context with an emitted From impl must still be probed"
    );

    let non_clone_defs = type_defs_for(false);
    assert!(
        visitor_context_probe(
            &config,
            &non_clone_defs,
            &[],
            &convertible(&non_clone_defs),
            "visit_text"
        )
        .is_none(),
        "a non-Clone context is handed to the callback as a dict, not the generated pyclass"
    );
}

/// `is_clone` alone is not the bridge's whole condition: it also requires the generated
/// `impl From<core::T> for T` to actually exist (`core_to_binding_from_impl_emitted`). An opaque
/// type is never in the core→binding convertible set (`core_to_binding_convertible_types` seeds
/// its candidate set with `!t.is_opaque`), so even a `Clone` opaque context still gets the dict
/// path -- proving the two conditions are independently necessary, not one gate wearing two names. ~keep
#[test]
fn a_clone_but_unconvertible_context_is_not_probed() {
    let type_defs = vec![
        visitor_trait("DocumentWalker", &["visit_text"]),
        TypeDef {
            name: "TraversalState".to_string(),
            is_clone: true,
            is_opaque: true,
            fields: vec![field("node_kind")],
            ..TypeDef::default()
        },
    ];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge("DocumentWalker", "TraversalState")],
        ..ResolvedCrateConfig::default()
    };
    let convertible_types = convertible(&type_defs);
    assert!(
        !convertible_types.contains("TraversalState"),
        "test setup: an opaque type must not be core-to-binding convertible"
    );

    assert!(visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_text").is_none());
}

/// The `Clone` + convertible context fixtures exercised as the eligible ("gets a pyclass") side of
/// `context_binding_class`'s condition, across this file. If this table ever emptied out, every
/// fixture in the file would have silently drifted onto the dict-path assertions and the class
/// path -- the one this whole probe exists to protect -- would have zero coverage: exactly the
/// failure mode of gating `is_clone` naively over bare `TypeDef::default()` fixtures. ~keep
fn eligible_context_fixtures() -> Vec<(&'static str, &'static str, TypeDef)> {
    vec![
        (
            "DocumentWalker",
            "TraversalState",
            clone_context("TraversalState", vec![field("node_kind")]),
        ),
        (
            "FrameWalker",
            "FrameState",
            clone_context("FrameState", vec![field("frame_id")]),
        ),
    ]
}

#[test]
fn eligible_context_fixture_set_is_non_empty_and_every_entry_resolves_to_the_class_path() {
    let fixtures = eligible_context_fixtures();
    assert!(
        !fixtures.is_empty(),
        "the eligible (class-path) fixture set emptied out -- the is_clone/convertibility gate \
         would then only ever be exercised on the dict-path side and a class-path regression \
         could land unnoticed"
    );
    for (trait_name, context_name, context_def) in fixtures {
        let type_defs = vec![visitor_trait(trait_name, &["visit_probe"]), context_def];
        let config = ResolvedCrateConfig {
            trait_bridges: vec![bridge(trait_name, context_name)],
            ..ResolvedCrateConfig::default()
        };
        let convertible_types = convertible(&type_defs);
        assert!(
            visitor_context_probe(&config, &type_defs, &[], &convertible_types, "visit_probe").is_some(),
            "{context_name} must still resolve to the class path"
        );
    }
}

/// A `PythonConfig` with only `exclude_types` populated.
fn python_config_excluding(exclude_types: &[&str]) -> crate::core::config::PythonConfig {
    crate::core::config::PythonConfig {
        dependency_versions: Default::default(),
        send_sync_types: Vec::new(),
        module_name: None,
        async_runtime: None,
        stubs: None,
        pip_name: None,
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
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
