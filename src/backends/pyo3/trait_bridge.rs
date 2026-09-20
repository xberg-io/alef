//! PyO3-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Python objects via PyO3.

mod bridge_methods;
mod generator;
mod options_field;
mod registry;
mod visitor_bridge;

pub use crate::codegen::generators::trait_bridge::find_bridge_param;
pub use bridge_methods::gen_bridge_function;
pub use generator::Pyo3BridgeGenerator;
pub use options_field::gen_bridge_field_function;
pub use registry::{
    collect_bridge_clear_fns, collect_bridge_register_fns, collect_bridge_unregister_fns, trait_bridge_imports,
};

use crate::codegen::generators::trait_bridge::{BridgeOutput, TraitBridgeSpec, gen_bridge_all};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::{HashMap, HashSet};
use visitor_bridge::gen_visitor_bridge;

/// The `exclude_languages` spellings that name this target: the language (`"python"`) and the
/// backend (`"pyo3"`). Both are honoured so a consumer who names either one gets the same answer
/// from every site that asks. ~keep
pub const TARGET_SPELLINGS: [&str; 2] = ["python", "pyo3"];

/// The trait a bridge wraps, when PyO3 emits that bridge at all.
///
/// Every PyO3 site that decides whether a bridge exists — the `#[pyclass]` marker, the emitted
/// bridge struct, the module-init registration, and `Pyo3Backend::trait_bridge_registration_surface`
/// — asks this, so they cannot disagree and leave one pass referring to a symbol another pass
/// never wrote. ~keep
pub fn active_bridge_trait<'a>(bridge: &TraitBridgeConfig, api: &'a ApiSurface) -> Option<&'a TypeDef> {
    crate::codegen::generators::trait_bridge::active_bridge_trait_def(bridge, api, &TARGET_SPELLINGS)
}

/// Whether PyO3 emits this bridge as a *visitor* bridge -- the shape whose callbacks receive a
/// context object and whose Rust side substitutes a default result rather than propagating a host
/// error.
///
/// Shared because the `.pyi` protocol stub has to annotate the context parameter with whatever the
/// visitor bridge actually hands the host, and only this shape has a context parameter at all: a
/// `register_fn` (plugin) bridge marshals its parameters normally, so applying the visitor's
/// fallback annotation there would describe a shape the bridge never produces. ~keep
pub fn is_visitor_bridge(trait_type: &TypeDef, bridge_cfg: &TraitBridgeConfig) -> bool {
    bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl)
}

pub(crate) use visitor_bridge::context_binding_class;

/// Generate the PyO3 trait bridge for `trait_type`.
///
/// This is public API with a semver-locked 7-argument signature as of 0.79.x. It assumes no
/// context type has had its `#[pyclass]` removed by `[crates.python] exclude_types`/
/// `capsule_types` -- the same assumption every call made before that distinction existed.
/// alef's own generation pipeline knows the real answer (via
/// `binding_exclusions::pyclass_absent_type_names`) and calls
/// [`gen_trait_bridge_with_absent_types`] directly instead of this wrapper. ~keep
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
    reexported_types: &[String],
) -> anyhow::Result<BridgeOutput> {
    let core_to_binding_convertible_types = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
    gen_trait_bridge_with_absent_types(
        trait_type,
        bridge_cfg,
        core_import,
        error_type,
        error_constructor,
        api,
        reexported_types,
        &ahash::AHashSet::new(),
        &core_to_binding_convertible_types,
    )
}

/// The internal seam carrying `pyclass_absent_types` and the precomputed
/// `core_to_binding_convertible_types` fixpoint -- both come from the caller's generation scope
/// rather than being rebuilt here, so a caller driving many bridges pays for the fixpoint once,
/// not once per bridge. [`gen_trait_bridge`] is the public, semver-locked 7-argument entry point;
/// this is what alef's own generation pipeline calls instead, since it has the real
/// `pyclass_absent_types` answer and already owns the precomputed fixpoint. ~keep
#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_trait_bridge_with_absent_types(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
    reexported_types: &[String],
    pyclass_absent_types: &ahash::AHashSet<String>,
    core_to_binding_convertible_types: &ahash::AHashSet<String>,
) -> anyhow::Result<BridgeOutput> {
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    if is_visitor_bridge(trait_type, bridge_cfg) {
        let trait_path = trait_type.rust_path.replace('-', "_");
        let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
        let code = gen_visitor_bridge(
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
            api,
            pyclass_absent_types,
            core_to_binding_convertible_types,
        )?;
        Ok(BridgeOutput { imports: vec![], code })
    } else {
        // Python object (the `#[pyclass]`, built via the same `From<core::T>` conversion used for
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        let struct_return_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_returns(trait_type, api);
        let forwardable_defaulted =
            crate::codegen::generators::trait_bridge::forwardable_defaulted_method_names(trait_type, api);
        let options_dataclass_types =
            crate::backends::pyo3::gen_bindings::options_dataclass_type_names(api, reexported_types);
        let unit_enum_return_types: HashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.variants.iter().all(|v| v.fields.is_empty()))
            .map(|e| e.name.clone())
            .collect();
        let generator = Pyo3BridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            struct_param_types,
            opaque_param_types: crate::codegen::generators::trait_bridge::native_marshalled_opaque_params(api)
                .into_iter()
                .filter(|name| !pyclass_absent_types.contains(name))
                .collect(),
            struct_return_types,
            forwardable_defaulted,
            options_dataclass_types,
            unit_enum_return_types,
        };
        let lifetime_type_names: HashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_lifetime_params)
            .map(|t| t.name.clone())
            .collect();
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Py",
            type_paths,
            lifetime_type_names,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        Ok(gen_bridge_all(&spec, &generator))
    }
}

mod tests {
    /// Trait callbacks must run inside the caller's contextvars Context so any ContextVar
    /// set by the caller is visible inside the callback. The generated bridge body must capture
    /// `contextvars.copy_context()` and invoke the host method via `ctx.run(bound_method, ...)`
    /// (rendered as `call_method1("run", ...)`) rather than calling the method directly.
    /// Regression test for issue #137.
    #[test]
    fn trait_callback_runs_in_caller_contextvars_context() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SampleService".to_owned(),
            rust_path: "sample_core::SampleService".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SampleService".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::new(),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::new(),
        };

        let make_method = |is_async: bool| MethodDef {
            name: "process".to_owned(),
            params: vec![ParamDef {
                name: "text".to_owned(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::String,
            is_async,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        let async_body = generator.gen_async_method_body(&make_method(true), &spec);
        assert!(
            async_body.contains("copy_context"),
            "async bridge must capture the caller's contextvars context:\n{async_body}"
        );
        assert!(
            async_body.contains("call_method1(\"run\""),
            "async bridge must invoke the host method via ctx.run:\n{async_body}"
        );

        let sync_body = generator.gen_sync_method_body(&make_method(false), &spec);
        assert!(
            sync_body.contains("copy_context"),
            "sync bridge must capture the caller's contextvars context:\n{sync_body}"
        );
        assert!(
            sync_body.contains("call_method1(\"run\""),
            "sync bridge must invoke the host method via ctx.run:\n{sync_body}"
        );
    }

    /// When a host returns a value that does not match a struct return type, the bridge
    /// deserializes it via `serde_json::from_str::<ReturnType>(...)`. The failure message must
    /// name the expected return TYPE and hint that the value must be a mapping matching the
    /// type's fields, so the host can fix their return value. The serde error (`{e}`) already
    /// carries the offending field/path. Regression test for issue #138.
    #[test]
    fn trait_callback_deserialize_error_names_return_type() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SampleService".to_owned(),
            rust_path: "sample_core::SampleService".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SampleService".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::new(),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::new(),
        };

        let make_method = |is_async: bool| MethodDef {
            name: "build".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("Doc".to_owned()),
            is_async,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        for is_async in [true, false] {
            let body = if is_async {
                generator.gen_async_method_body(&make_method(true), &spec)
            } else {
                generator.gen_sync_method_body(&make_method(false), &spec)
            };
            assert!(
                body.contains("expected return type `Doc`"),
                "deserialize error must name the expected return type `Doc` (is_async={is_async}):\n{body}"
            );
            assert!(
                body.contains("must be a mapping"),
                "deserialize error must hint the value must be a mapping matching the type's fields (is_async={is_async}):\n{body}"
            );
        }
    }

    /// When the return type is a native-marshalled struct, the bridge tries to extract the host's
    /// native binding object first (and convert via `From<Binding>`), falling back to the JSON
    /// mapping path. Return-side counterpart to the native-arg marshalling. See issue #153.
    #[test]
    fn trait_callback_native_struct_return_extracts_native_object_first() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SampleService".to_owned(),
            rust_path: "sample_core::SampleService".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SampleService".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::from(["Doc".to_owned()]),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::new(),
        };

        let make_method = |is_async: bool| MethodDef {
            name: "build".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("Doc".to_owned()),
            is_async,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        for is_async in [true, false] {
            let body = if is_async {
                generator.gen_async_method_body(&make_method(true), &spec)
            } else {
                generator.gen_sync_method_body(&make_method(false), &spec)
            };
            assert!(
                body.contains("extract::<Doc>()"),
                "native return must try extracting the binding object `Doc` first (is_async={is_async}):\n{body}"
            );
            assert!(
                body.contains("::from(native)"),
                "native return must convert the extracted object via From<Binding> (is_async={is_async}):\n{body}"
            );
            assert!(
                body.contains("serde_json::from_str"),
                "the JSON/mapping fallback must remain (is_async={is_async}):\n{body}"
            );
        }
    }

    /// Regression: an owned (by-value) native-struct callback param — e.g. the URI-based
    /// `ExtractInput` envelope, which the core API now passes by value — must be marshalled to
    /// the binding's native Python object via `From<core::T>`, not handed to the host raw. A raw
    /// core value has no `IntoPyObject` and fails to compile (E0277). Borrowed native-struct
    /// params were already marshalled; owned ones regressed when the param lost its `&`.
    #[test]
    fn trait_callback_owned_native_struct_param_is_marshalled() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SampleExtractor".to_owned(),
            rust_path: "sample_core::SampleExtractor".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SampleExtractor".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::from(["Input".to_owned()]),
            struct_return_types: HashSet::new(),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::new(),
        };

        let make_method = |is_async: bool| MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "input".to_owned(),
                ty: TypeRef::Named("Input".to_owned()),
                is_ref: false,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            is_async,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        for is_async in [true, false] {
            let body = if is_async {
                generator.gen_async_method_body(&make_method(true), &spec)
            } else {
                generator.gen_sync_method_body(&make_method(false), &spec)
            };
            assert!(
                body.contains("Input::from("),
                "owned native-struct param must be marshalled via From<core::T> (is_async={is_async}):\n{body}"
            );
            assert!(
                !body.contains("(bound_method, input)") && !body.contains("(bound_method, input,"),
                "owned native-struct param must not be handed to the host raw (is_async={is_async}):\n{body}"
            );
        }
    }

    /// Regression: a sync (infallible) callback returning a unit-only enum (e.g.
    /// `ProcessingStage`) must accept the bare variant name (`"Early"`) that a host naturally
    /// returns from a plain Python string, in addition to the JSON-quoted form the generic
    /// mapping path required (`"\"Early\""`). Previously the bridge only tried
    /// `serde_json::from_str`, which rejects a bare/unquoted variant name as invalid JSON and
    /// silently substitutes the default via `unwrap_or_else` — turning a real host return value
    /// into a wrong-but-plausible default. The error message for this shape must also stop
    /// claiming the value "must be a mapping", since unit enums have no fields to map.
    #[test]
    fn sync_unit_enum_return_accepts_bare_variant_name() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SamplePostProcessor".to_owned(),
            rust_path: "sample_core::SamplePostProcessor".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SamplePostProcessor".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::new(),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::from(["ProcessingStage".to_owned()]),
        };

        let method = MethodDef {
            name: "processing_stage".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("ProcessingStage".to_owned()),
            is_async: false,
            error_type: None,
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        let body = generator.gen_sync_method_body(&method, &spec);
        assert!(
            body.contains("serde_json::from_value(serde_json::Value::String"),
            "unit-enum return must fall back to treating the string as a bare variant name:\n{body}"
        );
        assert!(
            !body.contains("must be a mapping"),
            "unit-enum deserialize error must not claim the value must be a mapping:\n{body}"
        );
        assert!(
            body.contains("variant names"),
            "unit-enum deserialize error should mention variant names:\n{body}"
        );
    }

    /// Regression: a struct return (as opposed to a unit-only enum) must keep the strict
    /// mapping-only deserialization and error wording — a bare string can't represent a
    /// struct's fields, so the fallback added for unit enums must not apply here.
    #[test]
    fn sync_struct_return_keeps_strict_mapping_error() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SampleService".to_owned(),
            rust_path: "sample_core::SampleService".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SampleService".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::new(),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::new(),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::new(),
        };

        let method = MethodDef {
            name: "build".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("Doc".to_owned()),
            is_async: false,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        let body = generator.gen_sync_method_body(&method, &spec);
        assert!(
            body.contains("must be a mapping"),
            "struct deserialize error must keep the mapping-fields wording:\n{body}"
        );
        assert!(
            !body.contains("serde_json::from_value(serde_json::Value::String"),
            "struct return must not gain the unit-enum bare-string fallback:\n{body}"
        );
    }

    /// Regression: `PostProcessor::process(&self, result: &mut ExtractedDocument, ..)` cannot be
    /// implemented from Python at all under the naive bridge — it cloned `result`, handed the
    /// clone to the host, and discarded whatever the host returned via `.map(|_| ())`, so the
    /// `&mut` parameter was unfulfillable no matter what the Python plugin did. The bridge must
    /// instead treat the callback's return value as the (optionally) updated document and write
    /// it back into `*result` after the call, so Python `PostProcessor` plugins can actually
    /// modify the extraction result.
    #[test]
    fn async_mut_param_writes_back_host_return_value() {
        use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let trait_def = TypeDef {
            name: "SamplePostProcessor".to_owned(),
            rust_path: "sample_core::SamplePostProcessor".to_owned(),
            is_trait: true,
            is_opaque: true,
            ..TypeDef::default()
        };
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "SamplePostProcessor".to_owned(),
            register_fn: Some("register_sample".to_owned()),
            registry_getter: Some("sample_core::registry::get".to_owned()),
            ..TraitBridgeConfig::default()
        };
        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "sample_core",
            wrapper_prefix: "Py",
            type_paths: HashMap::from([("Doc".to_owned(), "sample_core::Doc".to_owned())]),
            lifetime_type_names: HashSet::new(),
            error_type: "SampleError".to_owned(),
            error_constructor: "SampleError::Message { message: {msg} }".to_owned(),
        };
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::from([("Doc".to_owned(), "sample_core::Doc".to_owned())]),
            error_type: "SampleError".to_owned(),
            opaque_param_types: HashSet::new(),
            struct_param_types: HashSet::from(["Doc".to_owned()]),
            struct_return_types: HashSet::new(),
            forwardable_defaulted: HashSet::new(),
            options_dataclass_types: HashSet::new(),
            unit_enum_return_types: HashSet::new(),
        };

        let method = MethodDef {
            name: "process".to_owned(),
            params: vec![
                ParamDef {
                    name: "result".to_owned(),
                    ty: TypeRef::Named("Doc".to_owned()),
                    is_ref: true,
                    is_mut: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "config".to_owned(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        };

        let body = generator.gen_async_method_body(&method, &spec);
        assert!(
            body.contains("*result = "),
            "the host's return value must be written back into the &mut param:\n{body}"
        );
        assert!(
            !body.contains(".map(|_| ())"),
            "the old discard-everything bridge shape must be gone for this method:\n{body}"
        );
        assert!(
            body.contains("py_result.is_none()"),
            "the host must be allowed to return None to mean \"unchanged\":\n{body}"
        );
    }

    #[test]
    fn visitor_bridge_uses_configured_context_and_result_metadata() {
        let (mut api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
        let result_enum = api
            .enums
            .iter_mut()
            .find(|enum_def| enum_def.name == "WalkOutcome")
            .expect("neutral visitor fixture should include its result enum");
        result_enum.serde_rename_all = Some("snake_case".to_string());
        result_enum.variants.push(crate::core::ir::EnumVariant {
            name: "ReplaceOutput".to_string(),
            fields: vec![crate::core::ir::FieldDef {
                name: "output".to_string(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        });
        let output = super::gen_trait_bridge(
            &trait_type,
            &bridge,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
            &[],
        )
        .expect("visitor bridge should generate");

        crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
        assert!(output.code.contains("\"display_name\""));
        assert!(output.code.contains("d.get_item(\"type\")"), "{}", output.code);
        assert!(
            output
                .code
                .contains("\"keep_going\" => return sample_core::walk::WalkOutcome::KeepGoing"),
            "{}",
            output.code
        );
        assert!(
            output.code.contains("d.get_item(\"output\")") && output.code.contains("WalkOutcome::ReplaceOutput"),
            "{}",
            output.code
        );
        assert!(
            output.code.contains("d.get_item(\"replace_output\")"),
            "{}",
            output.code
        );
    }
}
