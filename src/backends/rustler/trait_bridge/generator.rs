use super::visitor_bridge::{VisitorBridgeCtx, gen_visitor_bridge};
use crate::codegen::generators::trait_bridge::{BridgeOutput, TraitBridgeSpec, gen_bridge_all};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::HashMap;

/// Rustler-specific trait bridge generator.
/// Implements code generation for bridging Elixir modules to Rust traits via NIFs.
pub struct RustlerBridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name -> fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"SampleCrateError"`).
    pub error_type: String,
    /// Callback-param type names that marshal to the Elixir host as the binding's NATIVE struct
    /// term — known serde structs per the shared
    /// [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`] rule. For such a
    /// param the bridge builds the binding's `NifStruct`/`NifMap` (via the same `From<core::T>`
    /// conversion used for return values) and encodes it through Rustler's `Encoder`, so the host
    /// receives a native struct/map rather than a JSON string. All other args are encoded as their
    /// natural native terms (strings, numbers, booleans, lists); enums/opaque/unknown `Named` params
    /// fall back to a debug string term.
    pub struct_param_types: std::collections::HashSet<String>,
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> anyhow::Result<BridgeOutput> {
    // Build type name → rust_path lookup: convert to owned HashMap<String, String>
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods referencing them (e.g. `&InternalDocument`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let mut out = String::with_capacity(8192);
        let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
        let trait_path = trait_type.rust_path.replace('-', "_");

        gen_visitor_bridge(
            &mut out,
            &VisitorBridgeCtx {
                trait_type,
                struct_name: &struct_name,
                trait_path: &trait_path,
                core_crate: core_import,
                type_paths: &type_paths,
                bridge_cfg,
                api,
            },
        )?;
        Ok(BridgeOutput {
            imports: vec![],
            code: out,
        })
    } else {
        // Plugin-style bridge: use the IR-driven TraitBridgeGenerator infrastructure.
        //
        // Classify which callback params marshal to the host as the binding's native struct term
        // using the SHARED rule (`native_marshalled_struct_params`), identical to every other
        // backend's allowlist. For such params the bridge builds the binding `NifStruct` via
        // `From<core::T>` and encodes it natively; other args are encoded as their natural native
        // terms.
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        let generator = super::generator::RustlerBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            struct_param_types,
        };
        let lifetime_type_names: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|typ| typ.has_lifetime_params)
            .map(|typ| typ.name.clone())
            .collect();
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Rustler",
            type_paths,
            lifetime_type_names,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        let output = gen_bridge_all(&spec, &generator);
        // Note: trait support NIFs (complete_trait_call/fail_trait_call) must be emitted
        // only once, not per-bridge. They are now emitted in gen_bindings/mod.rs after
        // trait bridge generation to avoid duplicate NIF definitions.
        Ok(output)
    }
}
