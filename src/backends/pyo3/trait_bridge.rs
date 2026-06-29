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

pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> anyhow::Result<BridgeOutput> {
    // Build type name → rust_path lookup for qualifying Named types in signatures
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods referencing them (for example, `&HiddenDoc`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Determine bridge pattern: visitor-style (all methods have defaults, no registry) vs
    // plugin-style (cached fields, registry, super-trait).
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
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
        )?;
        Ok(BridgeOutput { imports: vec![], code })
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure.
        //
        // Classify which callback params get native-object marshalling using the SHARED rule
        // (`native_marshalled_struct_params`) so the allowlist is identical to what other
        // backends will consult. For such params the bridge hands the host the binding's native
        // Python object (the `#[pyclass]`, built via the same `From<core::T>` conversion used for
        // return values) instead of a JSON string.
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        // Return-side counterpart: a host may return the binding's native result object, which the
        // bridge extracts and converts via `From<Binding>` (falling back to the mapping/JSON path).
        let struct_return_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_returns(trait_type, api);
        let generator = Pyo3BridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            struct_param_types,
            struct_return_types,
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

// Generate a visitor-style bridge: thin wrapper over `Py<PyAny>` where every trait method
// tries to call the corresponding Python method, falling back to the default if absent.
//
// This pattern is used for traits where:
// - All methods have default implementations
// - No registration function is needed (per-call construction via `type_alias`)

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
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::new(),
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
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::new(),
        };

        // A trait method returning a struct `Doc` exercises the json.dumps -> from_str path.
        let make_method = |is_async: bool| MethodDef {
            name: "build".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("Doc".to_owned()),
            is_async,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
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
        // `Doc` is on the native-marshalled return allowlist.
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            struct_param_types: HashSet::new(),
            struct_return_types: HashSet::from(["Doc".to_owned()]),
        };

        let make_method = |is_async: bool| MethodDef {
            name: "build".to_owned(),
            params: vec![],
            return_type: TypeRef::Named("Doc".to_owned()),
            is_async,
            error_type: Some("SampleError".to_owned()),
            receiver: Some(ReceiverKind::Ref),
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
            // Native object converts via `From<Doc>` for the core type; mapping path is still present.
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
    /// `xberg::T` has no `IntoPyObject` and fails to compile (E0277). Borrowed native-struct
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
        // `Input` is on the native-marshalled struct-param allowlist.
        let generator = super::Pyo3BridgeGenerator {
            core_import: "sample_core".to_owned(),
            type_paths: HashMap::new(),
            error_type: "SampleError".to_owned(),
            struct_param_types: HashSet::from(["Input".to_owned()]),
            struct_return_types: HashSet::new(),
        };

        let make_method = |is_async: bool| MethodDef {
            name: "handle".to_owned(),
            // Owned (by-value) native-struct param — no `&`.
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

    #[test]
    fn visitor_bridge_uses_configured_context_and_result_metadata() {
        let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
        let output = super::gen_trait_bridge(
            &trait_type,
            &bridge,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
        )
        .expect("visitor bridge should generate");

        crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
        assert!(output.code.contains("\"display_name\""));
    }
}
