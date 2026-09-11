use super::{
    SHADOWABLE_BUILTIN_TYPES, pyi_docstring, python_safe_name, qualify_parameter_type, qualify_shadowed_builtin_types,
    substitute_capsule_type,
};
use crate::backends::pyo3::type_map::{python_callback_return_type, python_type};
use crate::codegen::shared::substitute_excluded_types;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// What `nodecontext_to_py_object`'s fallback arm actually builds: a `PyDict` keyed by the
/// context's public field names, whose values span every field shape the helper can lower --
/// `str`, `str | None`, `bool`, an integer, the `{:?}` rendering of an enum, `dict[str, str]`, or
/// `list[str]` (see `codegen::visitor_context::pyo3_field_line`).
///
/// The value type is `object`, not `Any`. `Any` switches the checker off at the parameter a
/// consumer's visitor callback is written against, so `ctx["depth"] + 1` and `ctx.depth` both
/// type-check against a shape that has neither. `object` still admits every value the fallback
/// can insert while forcing the consumer to narrow before use, which is the whole point of
/// annotating the degraded arm at all. It is also a builtin, so unlike `Any` it pulls no
/// `from typing import ...` line that `contains_word` would then have to keep in sync -- the
/// same hazard `gen_bindings::types.rs` records for Json-typed fields. ~keep
const VISITOR_CONTEXT_DICT_ANNOTATION: &str = "dict[str, object]";

/// Generate a `class TraitName(Protocol):` stub for an `OptionsField` trait bridge.
///
/// Returns `None` when the bridge's trait is absent from the API surface (e.g. excluded
/// from the public surface) — callers fall back to the legacy `type_alias` name in that
/// case so the stub still compiles.
///
/// The method signatures come from `TraitBridgeConfig::resolve_methods(api)`, which
/// looks up `bridge.trait_name` in `api.types` — the same source the trait-bridge code
/// generators use to emit the runtime vtable. This guarantees the Protocol surface in
/// the `.pyi` matches the methods the bridge actually forwards through PyO3.
pub(super) fn gen_visitor_protocol_stub(
    bridge: &TraitBridgeConfig,
    api: &ApiSurface,
    capsule_names: &std::collections::HashSet<&str>,
    emit_docstrings: bool,
    options_types: &std::collections::HashSet<String>,
    pyclass_absent_types: &ahash::AHashSet<String>,
    core_to_binding_convertible_types: &ahash::AHashSet<String>,
) -> Option<String> {
    let methods = bridge.resolve_methods(api);
    if methods.is_empty() {
        return None;
    }
    let trait_def = api.types.iter().find(|t| t.name == bridge.trait_name)?;

    let is_plugin_bridge = bridge.register_fn.is_some();
    let (required, optional): (Vec<&crate::core::ir::MethodDef>, Vec<&crate::core::ir::MethodDef>) =
        methods.iter().partition(|m| !(is_plugin_bridge && m.has_default_impl));
    let shadowed: Vec<&str> = SHADOWABLE_BUILTIN_TYPES
        .iter()
        .copied()
        .filter(|builtin| {
            required
                .iter()
                .any(|method| !method.binding_excluded && python_safe_name(&method.name) == *builtin)
        })
        .collect();

    let excluded: std::collections::HashSet<&str> = api
        .excluded_type_paths
        .keys()
        .map(String::as_str)
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect();

    // A visitor bridge whose context type has no generated `#[pyclass]` hands the callback a
    // `PyDict` instead. Ask the bridge's own predicate rather than re-deriving the condition, so
    // the annotation here cannot promise a class the bridge does not construct -- and so a
    // config-only removal (`[crates.python] exclude_types`, `capsule_types`), which no IR flag
    // records, moves both sides together. ~keep
    let dict_fallback_context: Option<&str> = crate::backends::pyo3::trait_bridge::is_visitor_bridge(trait_def, bridge)
        .then_some(bridge.context_type.as_deref())
        .flatten()
        .filter(|_| {
            crate::backends::pyo3::trait_bridge::context_binding_class(
                api,
                bridge,
                pyclass_absent_types,
                core_to_binding_convertible_types,
            )
            .is_none()
        });

    let mut lines = vec![format!("class {}(Protocol):", bridge.trait_name)];

    let mut doc = if emit_docstrings {
        trait_def.doc.clone()
    } else {
        String::new()
    };
    if emit_docstrings && !optional.is_empty() {
        let optional_list = optional
            .iter()
            .map(|m| format!("`{}`", python_safe_name(&m.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let lifecycle_note = if bridge.super_trait.is_some() {
            " The lifecycle hooks `initialize()` and `shutdown()` (and `name()` / `version()`) are likewise optional."
        } else {
            ""
        };
        if !doc.is_empty() {
            doc.push_str("\n\n");
        }
        doc.push_str(&format!(
            "Optional methods a backend may additionally implement — the bridge calls them when the object defines them, otherwise the trait's Rust default behavior applies: {optional_list}.{lifecycle_note}"
        ));
    }
    if let Some(docstring) = pyi_docstring(&doc, "    ") {
        lines.push(docstring);
    }

    let mut body_emitted = false;
    for method in required {
        if method.binding_excluded {
            continue;
        }
        body_emitted = true;
        let mut params: Vec<String> = vec!["self".to_string()];
        for p in &method.params {
            let param_type = match &p.ty {
                crate::core::ir::TypeRef::Named(n) if Some(n.as_str()) == dict_fallback_context => {
                    VISITOR_CONTEXT_DICT_ANNOTATION.to_string()
                }
                crate::core::ir::TypeRef::Named(n) if is_plugin_bridge && options_types.contains(n) => {
                    format!("options.{n}")
                }
                _ => substitute_capsule_type(
                    &python_type(&substitute_excluded_types(&p.ty, &excluded)),
                    capsule_names,
                ),
            };
            let param_type = qualify_parameter_type(&p.name, &param_type);
            let param_type = qualify_shadowed_builtin_types(&param_type, &shadowed);
            params.push(format!("{}: {}", p.name, param_type));
        }
        // Return position: the host produces this value and the bridge extracts it, so it takes
        // the widest annotation the extraction accepts. Parameters above stay on `python_type`.
        //
        // A `&mut Named` parameter with a `Unit` return (e.g. `PostProcessor.process`) is the
        // in-place-mutation pattern: Python can't mutate the frozen native object, so the bridge
        // treats the callback's return value as the (optionally) updated value and writes it
        // back — `None` means "left unchanged". Document that contract in the return type
        // instead of the misleading `None` a `Unit` return would otherwise suggest.
        let mut_param_type = method
            .params
            .iter()
            .find(|p| p.is_mut)
            .filter(|_| matches!(method.return_type, crate::core::ir::TypeRef::Unit))
            .map(|p| {
                substitute_capsule_type(
                    &python_type(&substitute_excluded_types(&p.ty, &excluded)),
                    capsule_names,
                )
            });
        let return_type = if let Some(mut_ty) = mut_param_type {
            format!("{mut_ty} | None")
        } else {
            substitute_capsule_type(
                &python_callback_return_type(&substitute_excluded_types(&method.return_type, &excluded)),
                capsule_names,
            )
        };
        let return_type = qualify_shadowed_builtin_types(&return_type, &shadowed);
        let safe_name = python_safe_name(&method.name);
        let signature = format!("    def {}({}) -> {}: ...", safe_name, params.join(", "), return_type);
        lines.push(signature);
    }

    if !body_emitted {
        lines.push("    ...".to_string());
    }

    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use crate::codegen::visitor_context::test_support::neutral_visitor_fixture;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{ApiSurface, MethodDef, ParamDef, TypeRef};
    use ahash::AHashSet;

    /// The neutral fixture's context type is `TypeDef::default()`-shaped, so `is_clone` is
    /// false and `context_binding_class` already resolves it to the dict fallback -- the very
    /// shape the *excluded* half of the test below is about. A fixture that is on the dict path
    /// before anything is excluded cannot serve as the control for excluding it.
    ///
    /// `is_clone` is the only condition it was missing: the fixture is already convertible,
    /// because every field is a primitive, a `String`, an `Optional<String>`, or the
    /// `TraversalKind` enum, and `can_generate_enum_conversion_from_core` admits any non-empty
    /// enum. The assertion in the test pins both halves so this cannot rot back. ~keep
    fn class_path_fixture() -> (ApiSurface, TraitBridgeConfig) {
        let (mut api, _, bridge) = neutral_visitor_fixture();
        api.types
            .iter_mut()
            .find(|type_def| type_def.name == "TraversalState")
            .expect("neutral visitor fixture should include its context type")
            .is_clone = true;
        (api, bridge)
    }

    /// Fails when the fixture is not on the `#[pyclass]` side of `context_binding_class`.
    fn assert_on_the_class_path(api: &ApiSurface) {
        let context = api
            .types
            .iter()
            .find(|type_def| type_def.name == "TraversalState")
            .expect("context type stays in the surface");
        let convertible = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
        assert!(
            context.is_clone,
            "fixture must be Clone: the generated From takes the core value by value while the \
             bridge holds only &core::T"
        );
        assert!(
            crate::codegen::conversions::core_to_binding_from_impl_emitted(context, &convertible),
            "fixture must have an emitted core-to-binding From impl, or the control below asserts \
             the dict fallback against a context that was already on the dict path"
        );
    }

    fn render(api: &ApiSurface, bridge: &TraitBridgeConfig, pyclass_absent_types: &AHashSet<String>) -> String {
        let convertible = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
        super::gen_visitor_protocol_stub(
            bridge,
            api,
            &std::collections::HashSet::new(),
            false,
            &std::collections::HashSet::new(),
            pyclass_absent_types,
            &convertible,
        )
        .expect("visitor protocol stub should generate")
    }

    /// The stub and the bridge must remove the same context types. Both were blind to
    /// `[crates.python] exclude_types` and `capsule_types`, so they agreed with each other while
    /// the stub named a class the compiled module never exported.
    ///
    /// Asserted in both directions on purpose: a change that annotated every context as a dict
    /// would satisfy the exclusion half alone, so the unexcluded run must still name the class. ~keep
    #[test]
    fn an_excluded_visitor_context_is_annotated_as_a_dict_and_an_unexcluded_one_keeps_its_class() {
        let (api, bridge) = class_path_fixture();
        assert_on_the_class_path(&api);

        let included = render(&api, &bridge, &AHashSet::new());
        assert!(
            included.contains("state: TraversalState"),
            "control: an unexcluded context must keep its generated class, or the exclusion \
             assertion below proves nothing:\n{included}"
        );
        assert!(
            !included.contains("dict[str, object]"),
            "control: an unexcluded context must not be annotated as a dict:\n{included}"
        );

        let excluded_names: AHashSet<String> = ["TraversalState".to_string()].into_iter().collect();
        let excluded = render(&api, &bridge, &excluded_names);
        assert!(
            excluded.contains("state: dict[str, object]"),
            "a context whose #[pyclass] the config removed must be annotated as the dict the \
             bridge actually passes:\n{excluded}"
        );
        assert!(
            !excluded.contains("state: TraversalState"),
            "the stub must not name a class the emitter skipped:\n{excluded}"
        );
    }

    /// The dict fallback belongs to the visitor shape only. A `register_fn` (plugin) bridge
    /// marshals its parameters normally and has no context fallback at all, so the annotation must
    /// not follow the exclusion there -- otherwise the fix would describe a shape that bridge never
    /// produces. ~keep
    #[test]
    fn a_plugin_bridge_parameter_is_never_annotated_with_the_visitor_dict_fallback() {
        let (mut api, _, mut bridge) = neutral_visitor_fixture();
        bridge.register_fn = Some("register_walker".to_string());
        api.types
            .iter_mut()
            .find(|type_def| type_def.name == "DocumentWalker")
            .expect("neutral visitor fixture should include its trait")
            .methods
            .iter_mut()
            .for_each(|method| method.has_default_impl = false);

        let excluded_names: AHashSet<String> = ["TraversalState".to_string()].into_iter().collect();
        let stub = render(&api, &bridge, &excluded_names);

        assert!(
            !stub.contains("dict[str, object]"),
            "a plugin bridge has no visitor context fallback to describe:\n{stub}"
        );
    }

    #[test]
    fn protocol_method_named_bytes_qualifies_sibling_annotations() {
        let (mut api, _, bridge) = neutral_visitor_fixture();
        let trait_def = api
            .types
            .iter_mut()
            .find(|type_def| type_def.name == bridge.trait_name)
            .expect("neutral visitor fixture should include its trait");
        trait_def.methods = vec![
            MethodDef {
                name: "bytes".to_string(),
                return_type: TypeRef::Unit,
                ..Default::default()
            },
            MethodDef {
                name: "consume".to_string(),
                params: vec![ParamDef {
                    name: "payload".to_string(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                return_type: TypeRef::Bytes,
                ..Default::default()
            },
        ];

        let stub = render(&api, &bridge, &AHashSet::new());

        assert!(
            stub.contains("def bytes("),
            "fixture must emit the shadowing method:\n{stub}"
        );
        assert!(
            stub.contains("def consume(self, payload: builtins.bytes) -> builtins.bytes"),
            "protocol methods share their class annotation scope:\n{stub}"
        );
    }

    #[test]
    fn protocol_method_named_object_qualifies_sibling_annotations() {
        let (mut api, bridge) = class_path_fixture();
        let trait_def = api
            .types
            .iter_mut()
            .find(|type_def| type_def.name == bridge.trait_name)
            .expect("neutral visitor fixture should include its trait");
        trait_def.methods.insert(
            0,
            MethodDef {
                name: "object".to_string(),
                return_type: TypeRef::Unit,
                has_default_impl: true,
                ..Default::default()
            },
        );

        let absent: AHashSet<String> = ["TraversalState".to_string()].into_iter().collect();
        let stub = render(&api, &bridge, &absent);

        assert!(
            stub.contains("def object("),
            "fixture must emit the shadowing method:\n{stub}"
        );
        assert!(
            stub.contains("dict[str, builtins.object]"),
            "Protocol method names shadow object annotations throughout the class:\n{stub}"
        );

        let Ok(pyrefly) = which::which("pyrefly") else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("protocol.pyi");
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.pyrefly]\npreset = \"strict\"\nproject-includes = [\"*.pyi\"]\n",
        )
        .expect("write strict pyrefly config");
        let source = format!("import builtins\nfrom typing import Protocol\nclass WalkOutcome: ...\n\n{stub}\n");
        std::fs::write(&path, &source).expect("write generated Protocol stub");
        let clean = std::process::Command::new(&pyrefly)
            .arg("check")
            .arg(dir.path())
            .output()
            .expect("run pyrefly on generated Protocol stub");
        assert!(
            clean.status.success(),
            "qualified generated Protocol must pass pyrefly:\n{}{}",
            String::from_utf8_lossy(&clean.stdout),
            String::from_utf8_lossy(&clean.stderr)
        );

        std::fs::write(&path, source.replace("builtins.object", "object")).expect("write adversarial Protocol stub");
        let broken = std::process::Command::new(pyrefly)
            .arg("check")
            .arg(dir.path())
            .output()
            .expect("run pyrefly negative control");
        let diagnostics = format!(
            "{}{}",
            String::from_utf8_lossy(&broken.stdout),
            String::from_utf8_lossy(&broken.stderr)
        );
        assert!(!broken.status.success(), "negative control must fail pyrefly");
        assert!(
            diagnostics.contains("[not-a-type]"),
            "negative control must reproduce the object shadowing diagnostic:\n{diagnostics}"
        );
    }
}
