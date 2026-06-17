//! Emits the Rust-side swift-bridge crate for Phase 2C.
//!
//! Three files are generated into `packages/swift/rust/`:
//! - `Cargo.toml` — declares the `cdylib`/`staticlib` crate with swift-bridge dependencies
//! - `src/lib.rs` — `#[swift_bridge::bridge] mod ffi { … }` plus wrapper newtypes and shims
//! - `build.rs` — calls `swift_bridge_build::parse_bridges` to run codegen at build time

pub(crate) mod cargo;
pub(crate) mod default_construction;
pub(crate) mod enums;
pub(crate) mod extern_block;
mod feature_gate;
mod json_bridge;
pub(crate) mod plugin_inbound;
pub(crate) mod service_app_wrappers;
pub(crate) mod shims;
pub mod trait_bridge;
pub(crate) mod type_bridge;
pub(crate) mod wrappers;

use crate::codegen::generators::type_paths::build_type_path_lookup;
use crate::core::backend::GeneratedFile;
use crate::core::config::extras::Language;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, TypeDef};
use crate::core::template_versions;
use heck::AsSnakeCase;
use std::collections::HashSet;
use std::path::PathBuf;

/// Top-level entry point: emit all three files for the swift-bridge crate.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let base = PathBuf::from("packages/swift/rust");
    let crate_name = &api.crate_name;
    let version = &api.version;

    let swift_bridge_ver = crate::backends::swift::naming::swift_bridge_version(config);
    let swift_bridge_build_ver = template_versions::cargo::SWIFT_BRIDGE_BUILD;
    let core_crate_dir = config.core_crate_for_language(Language::Swift);
    let swift_override = config.swift.as_ref().and_then(|c| c.core_crate_override.as_deref());
    let same_as_workspace =
        swift_override.is_none() && core_crate_dir == *crate_name && config.workspace_root.is_none();
    let core_path = if same_as_workspace {
        "../../..".to_string()
    } else {
        format!("../../../crates/{core_crate_dir}")
    };
    // Cargo dep KEY: when override is set, use it as-is; otherwise preserve
    // the historical behaviour (Rust-ident form of the umbrella crate name).
    let core_dep_key: String = match swift_override {
        Some(name) => name.to_string(),
        None => crate_name.replace('-', "_"),
    };

    let base_features = config.features_for_language(Language::Swift);
    // The IR may record a broad feature condition for a re-exported type whose concrete
    // module requires a narrower companion feature. Include the companion feature only
    // when the source crate actually exposes it so we do not inject unknown features.
    let mut features_owned: Vec<String>;
    let ocr_active = base_features.iter().any(|f| f == "ocr" || f == "full");
    let ocr_wasm_present = base_features.iter().any(|f| f == "ocr-wasm");
    let source_has_ocr_wasm = feature_gate::source_crate_has_feature(config, &core_crate_dir, "ocr-wasm");
    let features: &[String] = if ocr_active && !ocr_wasm_present && source_has_ocr_wasm {
        features_owned = base_features.to_vec();
        features_owned.push("ocr-wasm".to_string());
        &features_owned
    } else {
        base_features
    };
    let mut exclude_functions: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut exclude_types: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        exclude_types.extend(ffi.exclude_types.iter().cloned());
    }
    // Augment exclude_types with IR `binding_excluded` types. Service-owner types
    // (e.g. `App`) are marked binding_excluded by the service extractor so the
    // service_api codegen has exclusive ownership of their declaration. Without
    // this filter the regular extern-block emission below would emit a stale
    // `extern "Rust" { type App; }` + `app_route(..., handler: H)` pre-pivot
    // signature, which swift-bridge-build rejects with "Type must be declared
    // with `type H`."
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(api.enums.iter().filter(|e| e.binding_excluded).map(|e| e.name.clone()));
    // Response-adapter fns referenced from `HandlerContractDef.response_adapter` are
    // library-internal plumbing called from the generated handler bridges; surfacing
    // them as bridge-callable free functions would require representing their
    // `Result<Wire, BoxErr>` parameter and HTTP-typed return — neither of which the
    // host languages can model. Skip them in every binding that consults this set.
    for contract in &api.handler_contracts {
        if let Some(adapter) = contract.response_adapter.as_deref() {
            if let Some(short) = adapter.rsplit("::").next() {
                exclude_functions.insert(short.to_string());
            }
        }
    }
    let exclude_fields: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_fields.iter().cloned().collect())
        .unwrap_or_default();
    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.as_deref())
        .unwrap_or("MIT");
    let has_streaming_adapters = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming));
    let extra_deps = crate::scaffold::render_extra_deps(config, Language::Swift);
    let target_overrides = config
        .swift
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let excluded_default_features = config
        .swift
        .as_ref()
        .map(|c| c.excluded_default_features.as_slice())
        .unwrap_or(&[]);
    let cargo_toml = cargo::emit_cargo_toml(
        crate_name,
        &core_dep_key,
        &core_crate_dir,
        version,
        &swift_bridge_ver,
        swift_bridge_build_ver,
        &core_path,
        features,
        &extra_deps,
        license,
        has_streaming_adapters,
        target_overrides,
        api,
        excluded_default_features,
    );
    let configured_features: HashSet<&str> = features.iter().map(String::as_str).collect();
    let lib_rs = emit_lib_rs(
        api,
        config,
        crate_name,
        &exclude_functions,
        &exclude_types,
        &exclude_fields,
        &configured_features,
    );
    let build_rs = cargo::emit_build_rs();

    // Service-API callback registration functions live in a sibling Rust source
    // file so swift-bridge 0.1.59 (which parses src/lib.rs through syn 1.x and
    // chokes on Rust 2024 `#[unsafe(no_mangle)]`) never sees them.
    let extern_callbacks_rs =
        super::gen_bindings::service_api::generate_rust_callback_c_functions(api).unwrap_or_default();
    let mut files = vec![
        GeneratedFile {
            path: base.join("Cargo.toml"),
            content: cargo_toml,
            generated_header: false,
        },
        GeneratedFile {
            path: base.join("src/lib.rs"),
            content: lib_rs,
            generated_header: false,
        },
        GeneratedFile {
            path: base.join("build.rs"),
            content: build_rs,
            generated_header: false,
        },
    ];
    if !extern_callbacks_rs.is_empty() {
        let mut body = String::new();
        body.push_str("// Generated by alef. Do not edit by hand.\n");
        body.push_str("//\n");
        body.push_str("// Callback registration functions for service-API entrypoints.\n");
        body.push_str("// Kept out of src/lib.rs because swift-bridge 0.1.59 uses syn 1.x,\n");
        body.push_str("// which rejects the Rust 2024 `#[unsafe(no_mangle)]` attribute form.\n");
        body.push_str("// swift-bridge only parses src/lib.rs, so this file is invisible to it.\n\n");
        body.push_str("#![allow(unused_variables, unreachable_code, unreachable_patterns)]\n\n");
        body.push_str("use super::*;\n\n");
        for func_block in &extern_callbacks_rs {
            body.push_str(func_block);
            body.push('\n');
        }
        files.push(GeneratedFile {
            path: base.join("src/extern_callbacks.rs"),
            content: body,
            generated_header: false,
        });
    }
    Ok(files)
}

fn emit_lib_rs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    crate_name: &str,
    exclude_functions: &HashSet<String>,
    exclude_types: &HashSet<String>,
    exclude_fields: &HashSet<String>,
    configured_features: &HashSet<&str>,
) -> String {
    let source_crate = crate_name.replace('-', "_");

    // Build lookup from short type name → fully-qualified rust_path.
    let type_paths = build_type_path_lookup(api);

    let mut out = String::new();
    out.push_str("// Generated by alef. Do not edit by hand.\n\n");
    out.push_str("#![allow(unused_variables, unreachable_code, unreachable_patterns, missing_docs)]\n");
    out.push_str("#![allow(\n");
    out.push_str("    clippy::collapsible_if,\n");
    out.push_str("    clippy::collapsible_match,\n");
    out.push_str("    clippy::needless_borrows_for_generic_args,\n");
    out.push_str("    clippy::field_reassign_with_default,\n");
    out.push_str("    clippy::too_many_arguments,\n");
    out.push_str("    clippy::clone_on_copy,\n");
    out.push_str("    clippy::unnecessary_cast,\n");
    out.push_str("    clippy::manual_flatten,\n");
    out.push_str("    clippy::match_single_binding,\n");
    out.push_str("    clippy::redundant_closure,\n");
    // `useless_conversion`: bytes-field defaults emit `__target.x = x.into();`
    // (see templates/default_field_bytes_assign.jinja). For consumers where the
    // source and target field types are identical (e.g. `Option<Vec<u8>>`), the
    // `.into()` is a no-op and clippy flags it; for consumers with different
    // types it is necessary. The template can't know which case applies, so
    // suppress at the crate root.
    out.push_str("    clippy::useless_conversion,\n");
    out.push_str("    clippy::inherent_to_string,\n");
    // `new_without_default`: the App wrapper exposes `pub fn new() -> Self` so
    // swift-bridge can map it to a Swift initializer; deriving Default doesn't
    // help downstream because Swift can't call it through the bridge.
    out.push_str("    clippy::new_without_default,\n");
    out.push_str(")]\n\n");

    // Emit the process-wide tokio runtime accessor. Async swift-bridge wrappers
    // share this single runtime instead of building one per call — the per-call
    // pattern orphans reqwest's connection pool when its host runtime drops,
    // surfacing as `error sending request for url (...)` on every subsequent
    // call. The accessor is only used by async-shim emission; for crates with
    // no async source functions it's harmless dead code (the `unused` lint
    // already suppressed at the top of the file).
    out.push_str(shims::ALEF_TOKIO_RUNTIME_DEFINITION);
    out.push('\n');

    let visible_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait)
        .filter(|t| feature_gate::cfg_satisfied(t.cfg.as_deref(), configured_features))
        .collect();
    let visible_enums: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .filter(|e| feature_gate::cfg_satisfied(e.cfg.as_deref(), configured_features))
        .collect();

    // Set of enum names (not struct names) so wrappers can use the correct
    // conversion idiom: `T::from(val)` for enums, `T(val)` for struct newtypes.
    let enum_names: HashSet<&str> = visible_enums.iter().map(|e| e.name.as_str()).collect();

    // Set of UNIT enum names (variants with no fields). Only these implement From<String>;
    // tagged enums (variants with fields) must use serde_json::from_str instead.
    let unit_enum_names: HashSet<&str> = visible_enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    // Set of TAGGED enum names (variants with fields). These must use JSON deserialization.
    let tagged_enum_names: HashSet<&str> = visible_enums
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    // Union of all visible type names (structs + enums) that have swift-bridge wrapper newtypes
    // in the generated lib.rs. Used by trait bridge trampolines to decide whether a Named
    // return type should be wrapped (it has a newtype) or JSON-serialised (excluded/foreign type).
    let visible_type_names: HashSet<&str> = visible_types
        .iter()
        .map(|t| t.name.as_str())
        .chain(enum_names.iter().copied())
        .collect();

    // Set of type names that do NOT implement serde (Serialize + Deserialize).
    // These cannot be JSON-bridged when they appear as inner Named types in
    // Optional/Vec fields or return types.
    let no_serde_names: HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.has_serde)
        .map(|t| t.name.as_str())
        .collect();
    let no_serde_enum_names: HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| !e.has_serde)
        .map(|e| e.name.as_str())
        .collect();

    // api types that are returned by value from public functions/methods.  These appear as
    // `*mut T` opaque handles in the FFI; swift-bridge must declare them as the bare
    // Named type so the Swift side wraps them in the opaque class instead of receiving
    // a `RustString` (the JSON fallback).  Mirrors `compute_handle_returned_types` in
    // alef-backend-csharp's errors.rs.
    let handle_returned_types: HashSet<String> = type_bridge::compute_handle_returned_types(api);

    // Filter to only functions that can be fully bridged.
    // Unbridgeable functions (enum params, Vec<u8> tuple params, non-serde return types)
    // are silently excluded from both the extern block and the shim impl — callers in Swift
    // simply won't see these functions rather than panicking at runtime.
    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .filter(|f| feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features))
        .filter(|f| {
            !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
        })
        .filter(|f| {
            shims::is_bridgeable_fn(
                f,
                &unit_enum_names,
                &type_paths,
                &no_serde_names,
                &no_serde_enum_names,
                &handle_returned_types,
            )
        })
        .collect();

    // Collect trait bridge definitions for configured traits.
    let active_bridges: Vec<(&TraitBridgeConfig, &TypeDef)> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.iter().any(|l| l == "swift"))
        .filter_map(|b| {
            api.types
                .iter()
                .find(|t| t.is_trait && t.name == b.trait_name)
                .map(|t| (b, t))
        })
        .collect();

    // Collect result enum names from trait bridges — result-type enums must have
    // a private swift_name to avoid collision with the first-class Swift enums
    // emitted in gen_bindings.rs.
    let result_type_enums: std::collections::HashSet<String> = active_bridges
        .iter()
        .filter_map(|(bridge_cfg, _)| bridge_cfg.result_type.as_deref().map(|s| s.to_string()))
        .collect();

    // Collect extern "Rust" blocks for the ffi module
    // Build a HashSet<String> from enum_names (&str) for the enum-aware bridge type helper.
    let enum_names_owned: std::collections::HashSet<String> = enum_names.iter().map(|s| s.to_string()).collect();
    let mut extern_blocks: Vec<String> = Vec::new();
    for ty in &visible_types {
        // Wrap each extern block in `#[cfg(feature = "...")]` when the type carries a
        // cfg condition.  swift-bridge 0.1.59 cannot parse `#[cfg(...)]` *inside* an
        // extern block, but `#[cfg(...)] extern "Rust" { ... }` at the module item
        // level is valid Rust: the Rust compiler strips the cfg-disabled item before
        // the proc macro expands, so swift-bridge never sees the decl when the feature
        // is off.  Without this gate the proc macro unconditionally generates Swift
        // bridging glue for the type, but the underlying `pub struct T(pub CoreT)` is
        // `#[cfg(feature = "...")]`-gated in the shims — causing "no type named 'T' in
        // module 'RustBridge'" when the feature is disabled.
        let cfg_open = ty
            .cfg
            .as_deref()
            .map(|c| format!("    #[cfg({c})]\n"))
            .unwrap_or_default();
        let cfg_close = if ty.cfg.is_some() {
            // The closing brace is already inside the extern block string, so we only
            // need to close the `#[cfg]` scope at the right nesting level. Because
            // `#[cfg]` is an attribute on the whole `extern "Rust" { }` item (not a
            // standalone scope), no extra braces are needed — the attribute applies to
            // the next item statement.
            String::new()
        } else {
            String::new()
        };
        let raw_block = extern_block::emit_extern_block_for_type(
            ty,
            exclude_fields,
            &type_paths,
            &no_serde_names,
            &enum_names_owned,
        );
        // Indent the raw block by 4 spaces when it is wrapped in a cfg attribute, to
        // keep the resulting source readable.  The raw block already begins with "    "
        // (four-space indent inside `mod ffi`), so no additional indent is needed;
        // we just prepend the attribute on its own line.
        extern_blocks.push(format!("{cfg_open}{raw_block}{cfg_close}"));
        // For opaque types with methods, also emit constructor + method extern blocks.
        if ty.is_opaque && !ty.methods.iter().all(|m| m.sanitized) && !ty.methods.is_empty() {
            // Only emit the `create_<type>` constructor when the user provides an explicit
            // `client_constructor_body` override in alef.toml. The default
            // `(api_key, base_url)` signature only fits one stateful-client shape; for plugin
            // types or utilities it produces calls that don't match
            // the real Rust signature. Opaque types without an override are returned by
            // Rust APIs, not constructed in Swift.
            let has_ctor_override = config
                .swift
                .as_ref()
                .is_some_and(|c| c.client_constructor_body.contains_key(&ty.name));
            if has_ctor_override && let Some(ctor_block) = extern_block::emit_extern_block_for_type_constructor(ty) {
                extern_blocks.push(format!("{cfg_open}{ctor_block}"));
            }
            if let Some(method_block) =
                extern_block::emit_extern_block_for_type_methods(ty, &handle_returned_types, &enum_names)
            {
                extern_blocks.push(format!("{cfg_open}{method_block}"));
            }
        }
    }
    for en in &visible_enums {
        // Skip result-type enums from the bridge — they're first-class Swift enums
        // and don't need opaque swift-bridge types. Swift calls JSON decoders locally.
        // Wrap cfg-gated enums in `#[cfg(...)]` for the same reason as types above.
        if !result_type_enums.contains(&en.name) {
            let raw_block = extern_block::emit_extern_block_for_enum(en);
            if let Some(cfg) = en.cfg.as_deref() {
                extern_blocks.push(format!("    #[cfg({cfg})]\n{raw_block}"));
            } else {
                extern_blocks.push(raw_block);
            }
        }
    }
    if !visible_functions.is_empty() {
        // Non-cfg-gated functions go into the aggregate extern "Rust" block.
        // cfg-gated functions are emitted into per-cfg `#[cfg(...)] extern "Rust" { }`
        // blocks so that the Rust compiler strips the declaration before swift-bridge's
        // proc macro runs when the feature is off.  The shim body is also `#[cfg(...)]`-
        // gated (emitted below), so Swift never sees either the declaration or the
        // implementation when the feature is disabled.
        let non_cfg_fns: Vec<FunctionDef> = visible_functions
            .iter()
            .filter(|f| f.cfg.is_none())
            .map(|f| (*f).clone())
            .collect();
        if !non_cfg_fns.is_empty() {
            extern_blocks.push(extern_block::emit_extern_block_for_functions(
                &non_cfg_fns,
                &handle_returned_types,
                &enum_names_owned,
            ));
        }
        // Group cfg-gated functions by their cfg condition, then emit one
        // `#[cfg(...)] extern "Rust" { }` block per distinct condition.
        let mut cfg_groups: std::collections::BTreeMap<String, Vec<FunctionDef>> = std::collections::BTreeMap::new();
        for f in visible_functions.iter().filter(|f| f.cfg.is_some()) {
            cfg_groups.entry(f.cfg.clone().unwrap()).or_default().push((*f).clone());
        }
        for (cfg_cond, fns) in &cfg_groups {
            let block = extern_block::emit_extern_block_for_functions(fns, &handle_returned_types, &enum_names_owned);
            extern_blocks.push(format!("    #[cfg({cfg_cond})]\n{block}"));
        }
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        extern_blocks.push(trait_bridge::emit_extern_block_for_trait_bridge(
            trait_def,
            &visible_type_names,
        ));
    }
    // Inbound (extern "Swift") plugin bridges — Swift implements the Rust trait.
    // First the register/unregister entry points (extern "Rust"), then the Swift-side
    // type and method declarations (extern "Swift").
    //
    // FunctionParam bridges: register/unregister entry points are needed for the
    // global registry pattern. OptionsField bridges: no registry, the visitor is
    // wired via a make_<trait>_handle factory that returns the local opaque
    // type_alias wrapper (see `emit_inbound_wrapper` below).
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::FunctionParam {
            continue;
        }
        let reg_block = plugin_inbound::emit_extern_block_for_inbound_registration(trait_def, bridge_cfg);
        if !reg_block.is_empty() {
            extern_blocks.push(reg_block);
        }
    }
    // Both FunctionParam AND OptionsField need the extern "Swift" block — it declares
    // the Swift opaque type and per-method shims that Rust calls back into Swift.
    for (bridge_cfg, trait_def) in &active_bridges {
        extern_blocks.push(plugin_inbound::emit_extern_block_for_inbound(trait_def, bridge_cfg));
    }
    // OptionsField bridges additionally need:
    //   - an extern "Rust" factory (`make_{trait_snake}_handle`) so Swift can wrap
    //     a protocol-conforming class into a VisitorHandle opaque type.
    //   - an extern "Rust" options helper (`{options_snake}_from_json_with_{field}`)
    //     so Swift e2e tests can deserialise fixture JSON and attach the visitor in one step.
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        let (factory_extern, _factory_body) =
            plugin_inbound::emit_options_field_factory(trait_def, bridge_cfg, api, &source_crate);
        if !factory_extern.is_empty() {
            extern_blocks.push(factory_extern);
        }
        let (helper_extern, _helper_body) =
            plugin_inbound::emit_options_field_options_helper(bridge_cfg, api, &source_crate);
        if !helper_extern.is_empty() {
            extern_blocks.push(helper_extern);
        }
    }

    // Streaming adapters: emit an extern "Rust" block for each streaming adapter
    // that owns a client type.  The Swift host wrapper references these as
    // `RustBridge.{camelName}(client, …)`, so they must be declared in the
    // swift-bridge module or the Swift compiler will produce
    // "module 'RustBridge' has no member named …".
    if let Some(streaming_block) = extern_block::emit_extern_block_for_streaming_adapters(&config.adapters) {
        extern_blocks.push(streaming_block);
    }

    // Service-API extern "Rust" blocks (bridge-based via C-callback shims).
    // These declare opaque service types, constructors, configurators,
    // and entrypoints to swift-bridge. Callback registration functions are emitted
    // as plain C functions OUTSIDE the bridge module (see below).
    let service_api_blocks = super::gen_bindings::service_api::generate_rust_extern_blocks(api).unwrap_or_default();
    extern_blocks.extend(service_api_blocks);

    // Service-API callback registration functions (plain C, emitted outside the bridge module).
    // swift-bridge 0.1.59 cannot parse raw pointer types or `extern "C" fn` in extern "Rust" blocks.
    let service_api_callback_funcs: Vec<String> =
        super::gen_bindings::service_api::generate_rust_callback_c_functions(api).unwrap_or_default();

    // Collect serde-enabled non-opaque types that appear as method parameters.
    // These need their own `{type_snake}_from_json` free-function shims so Swift
    // e2e tests can deserialise fixture JSON into the strongly-typed request
    // objects expected by the swift-bridge wrappers.
    let extra_serde_param_types: Vec<&TypeDef> =
        json_bridge::collect_serde_param_types(api, &visible_types, &visible_functions, &[]);

    // Collect streaming item types that have serde derives.  The Swift streaming
    // wrapper uses `RustBridge.{itemType}FromJson(json)` — a Rust-side free
    // function — to deserialise each JSON chunk into the opaque type so that
    // callers can use the full swift-bridge method API on the result.
    // The `from_json` shim must be emitted even when the type is NOT a param type.
    let extra_serde_param_names: std::collections::HashSet<&str> =
        extra_serde_param_types.iter().map(|t| t.name.as_str()).collect();
    let streaming_item_types: Vec<&TypeDef> = {
        let streaming_item_names: std::collections::HashSet<&str> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.item_type.as_deref())
            .collect();
        visible_types
            .iter()
            .copied()
            .filter(|ty| streaming_item_names.contains(ty.name.as_str()))
            .filter(|ty| ty.has_serde && !ty.is_opaque && !ty.is_trait)
            // Skip types already covered by the other from_json shim sets.
            .filter(|ty| !extra_serde_param_names.contains(ty.name.as_str()))
            .collect()
    };

    // Any serde DTO that Swift's `intoRust()` will JSON-encode must have a matching
    // Rust-side `*_from_json` shim. The Swift side can JSON-encode when the
    // constructor extern is emitted but `emit_into_rust_direct_call` cannot build
    // a per-field conversion (e.g. Map/Json/Path/Bytes/Duration/Char,
    // `Optional<Vec>`, `Optional<Named>`, `Vec<Vec<...>>`, etc.) and so the binding
    // side falls through to the JSON roundtrip.
    //
    // Without this broadened emission, those types have no Rust-side `*_from_json`
    // symbol, so Swift's emitted `RustBridge.{type}FromJson(json)` call fails at
    // link time. The Swift userland `*FromJson` forwarder filter
    // (`gen_bindings::emit_from_json_forwarders`) already targets the strict
    // superset of "all serde-enabled, non-opaque, non-trait struct types"; broaden
    // the Rust crate emission to the same set so the symbol is always present.
    //
    // Dedupe against the previously collected emission sets to avoid emitting the
    // same `*_from_json` shim twice in the bridge module.
    let streaming_item_names: std::collections::HashSet<&str> =
        streaming_item_types.iter().map(|t| t.name.as_str()).collect();
    let json_fallback_types: Vec<&TypeDef> = visible_types
        .iter()
        .copied()
        .filter(|ty| ty.has_serde && !ty.is_opaque && !ty.is_trait)
        .filter(|ty| !extra_serde_param_names.contains(ty.name.as_str()))
        .filter(|ty| !streaming_item_names.contains(ty.name.as_str()))
        .collect();

    // Enums with serde derives also need a matching Rust-side `*_from_json` shim
    // because every Swift `intoRust()` extension on a Codable enum JSON-encodes
    // `self` and calls `RustBridge.{enumSnake}FromJson(json)` — there is no
    // positional constructor path for enums. Without these shims, e2e fixtures
    // that pass enum DTOs across the bridge fail at link time.
    let json_fallback_enums: Vec<&EnumDef> = visible_enums.iter().copied().filter(|e| e.has_serde).collect();

    out.push_str("#[swift_bridge::bridge]\nmod ffi {\n");
    for block in &extern_blocks {
        out.push_str(block);
    }
    if !extra_serde_param_types.is_empty() {
        out.push_str("    extern \"Rust\" {\n");
        for ty in &extra_serde_param_types {
            let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
            let type_name = &ty.name;
            json_bridge::emit_from_json_extern_decl(&mut out, &type_snake, type_name);
        }
        out.push_str("    }\n");
    }
    // Emit from_json extern blocks for streaming item types.
    // The streaming wrapper calls `RustBridge.{itemType}FromJson(json)` on the
    // Swift side to deserialise each JSON chunk into the opaque type.
    if !streaming_item_types.is_empty() {
        out.push_str("    extern \"Rust\" {\n");
        for ty in &streaming_item_types {
            let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
            let type_name = &ty.name;
            json_bridge::emit_from_json_extern_decl(&mut out, &type_snake, type_name);
        }
        out.push_str("    }\n");
    }
    // Emit from_json extern blocks for any other DTO that Swift will JSON-encode.
    // Now broadened to every serde-enabled, non-opaque, non-trait struct type so
    // every `RustBridge.{Type}FromJson(json)` call from the Swift userland has a
    // matching link symbol — see the `json_fallback_types` comment above.
    if !json_fallback_types.is_empty() {
        out.push_str("    extern \"Rust\" {\n");
        for ty in &json_fallback_types {
            let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
            let type_name = &ty.name;
            json_bridge::emit_from_json_extern_decl(&mut out, &type_snake, type_name);
        }
        out.push_str("    }\n");
    }
    // Enum from_json declarations — pair every serde-enabled enum with a
    // `{enum}_from_json(json: String) -> Result<EnumWrapper, String>` shim so the
    // Swift `extension Enum.intoRust()` JSON path links cleanly. The wrapper enum
    // is the swift-bridge newtype (`pub struct Foo(pub source::Foo)`), so the
    // shim wraps the deserialised source enum in the wrapper.
    // HOWEVER: result-type enums (trait bridge result types) are first-class Swift
    // enums that JSON-decode locally in Swift — they do NOT call the Rust-side
    // `*_from_json` function, so we skip emitting the extern declaration for them.
    // This avoids the undeclared-type error while keeping the type declaration itself
    // (emitted above in emit_extern_block_for_enum) so swift-bridge knows about it
    // when it appears in other function signatures.
    let json_fallback_enums_filtered: Vec<&EnumDef> = json_fallback_enums
        .iter()
        .filter(|en| !result_type_enums.contains(&en.name))
        .copied()
        .collect();
    if !json_fallback_enums_filtered.is_empty() {
        out.push_str("    extern \"Rust\" {\n");
        for en in &json_fallback_enums_filtered {
            let enum_snake = AsSnakeCase(en.name.as_str()).to_string();
            let enum_name = &en.name;
            json_bridge::emit_from_json_extern_decl(&mut out, &enum_snake, enum_name);
        }
        out.push_str("    }\n");
    }

    // Emit phantom Vec<T> references for all opaque types so swift-bridge-build
    // generates the __swift_bridge__$Vec_T$* C symbols that Swift needs.
    // Skip result-type enums — they are never declared in extern blocks, so they
    // cannot be referenced in phantom Vec declarations without triggering a
    // "Type must be declared with `type T`" swift-bridge parser error.
    //
    // cfg-gated types are split into per-type `#[cfg(...)] extern "Rust" { }` blocks
    // so that when the feature is disabled, swift-bridge never sees the phantom Vec
    // declaration (the proc macro would otherwise generate bridging glue for a type
    // whose `pub struct T` is absent, causing "no type named 'T'" compile errors).
    let vec_accessible_enums: Vec<&EnumDef> = visible_enums
        .iter()
        .filter(|en| !result_type_enums.contains(&en.name))
        .copied()
        .collect();
    // Skip types marked `#[swift_bridge(already_declared)]` (streaming-adapter
    // owners) from the phantom-Vec emission. swift-bridge does NOT synthesize a
    // `Vectorizable` conformance for already_declared types, so any
    // `Vec<DefaultClient>` reference would fail to compile with
    // "type 'T' does not conform to protocol 'Vectorizable'". A Vec of an
    // opaque streaming owner is not useful in practice — callers cannot
    // construct one without owning a live Rust handle.
    let already_declared =
        crate::backends::swift::gen_bindings::opaque_handles::collect_already_declared_owner_types(config);
    // Non-cfg-gated types go into a single aggregate phantom Vec extern block.
    let non_cfg_types: Vec<&TypeDef> = visible_types
        .iter()
        .copied()
        .filter(|t| t.cfg.is_none() && !already_declared.contains(&t.name))
        .collect();
    let non_cfg_enums: Vec<&EnumDef> = vec_accessible_enums
        .iter()
        .copied()
        .filter(|e| e.cfg.is_none())
        .collect();
    let vec_accessors_block = extern_block::emit_extern_block_for_vec_accessors(&non_cfg_types, &non_cfg_enums);
    if !vec_accessors_block.is_empty() {
        out.push_str(&vec_accessors_block);
    }
    // cfg-gated types each get their own `#[cfg(...)] extern "Rust" { }` block.
    for ty in visible_types
        .iter()
        .filter(|t| t.cfg.is_some() && !already_declared.contains(&t.name))
    {
        let cfg = ty.cfg.as_deref().unwrap();
        let block = extern_block::emit_extern_block_for_vec_accessors(&[ty], &[]);
        if !block.is_empty() {
            out.push_str(&format!("    #[cfg({cfg})]\n{block}"));
        }
    }
    for en in vec_accessible_enums.iter().filter(|e| e.cfg.is_some()) {
        let cfg = en.cfg.as_deref().unwrap();
        let block = extern_block::emit_extern_block_for_vec_accessors(&[], &[en]);
        if !block.is_empty() {
            out.push_str(&format!("    #[cfg({cfg})]\n{block}"));
        }
    }

    out.push_str("}\n\n");

    // Emit phantom Vec accessor implementations paired with extern declarations inside the bridge module.
    // swift-bridge-build generates Vec ABI symbols when it sees these implementations.
    // Skip already_declared types — see comment above the matching filter for the
    // phantom-extern emission. The two must stay in sync or the Rust impl would
    // be orphaned (no extern decl referring to it) or the extern would be
    // dangling (no impl backing it).
    let vec_impl_types: Vec<&TypeDef> = visible_types
        .iter()
        .copied()
        .filter(|t| !already_declared.contains(&t.name))
        .collect();
    let phantom_impl = extern_block::emit_phantom_vec_impl(&vec_impl_types, &vec_accessible_enums);
    if !phantom_impl.is_empty() {
        out.push_str(&phantom_impl);
    }

    // Emit service App wrapper structs and impls (must be emitted BEFORE callback functions so
    // the C functions can reference the App type and its methods).
    out.push_str(&service_app_wrappers::emit_service_app_wrappers(api, &source_crate));
    out.push('\n');

    // Service-API callback registration functions live in `src/extern_callbacks.rs`
    // because swift-bridge 0.1.59 pins syn 1.x, which cannot parse Rust 2024's
    // `#[unsafe(no_mangle)]` attribute. swift-bridge only parses `src/lib.rs`,
    // so the `mod` declaration is invisible to its parser while still pulling
    // in the file at cargo-build time.
    if !service_api_callback_funcs.is_empty() {
        out.push_str("mod extern_callbacks;\n\n");
    }

    for ty in &visible_types {
        out.push_str(&wrappers::emit_type_wrapper(
            ty,
            &source_crate,
            &type_paths,
            &enum_names,
            &no_serde_names,
            exclude_fields,
        ));
        out.push('\n');
        // For opaque types that expose methods, emit constructor + method shims.
        // Serde types (is_opaque=false) are read-only data objects returned by Rust; they
        // don't need a constructor shim because users never construct them directly.
        if ty.is_opaque && !ty.methods.iter().all(|m| m.sanitized) && !ty.methods.is_empty() {
            let custom_body = config
                .swift
                .as_ref()
                .and_then(|c| c.client_constructor_body.get(&ty.name))
                .map(String::as_str);
            // Only emit `create_<type>` when an explicit constructor body is configured.
            // See extern-block gate above for the rationale.
            if custom_body.is_some() {
                out.push_str(&wrappers::emit_type_constructor_shim(
                    ty,
                    &source_crate,
                    &type_paths,
                    custom_body,
                ));
                out.push('\n');
            }
            out.push_str(&wrappers::emit_type_method_shims(
                ty,
                &source_crate,
                &type_paths,
                &handle_returned_types,
                &enum_names,
            ));
            out.push('\n');
        }
    }
    for en in &visible_enums {
        out.push_str(&enums::emit_enum_wrapper(en, &source_crate, &type_paths));
        out.push('\n');
    }
    for f in &visible_functions {
        out.push_str(&shims::emit_function_shim(
            f,
            &source_crate,
            &type_paths,
            &unit_enum_names,
            &tagged_enum_names,
            &no_serde_names,
            &handle_returned_types,
        ));
        out.push('\n');
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        out.push_str(&trait_bridge::emit_trait_bridge_wrapper(
            trait_def,
            &source_crate,
            &unit_enum_names,
            &visible_type_names,
            &type_paths,
        ));
        out.push('\n');
    }

    // Inbound plugin trait bridges (Swift implements the Rust trait): emit the shared
    // error helper once, then per-trait wrapper structs + Plugin/Trait impls + register fns.
    if !active_bridges.is_empty() {
        out.push_str(&plugin_inbound::emit_plugin_error_helper(
            &source_crate,
            &config.error_type_name(),
            &config.error_constructor_expr(),
        ));
    }
    for (bridge_cfg, trait_def) in &active_bridges {
        // Emit the Rust-side wrapper struct + trait impl for ALL inbound bridges
        // (both FunctionParam and OptionsField). The wrapper is the same in both
        // modes — a struct holding a Swift handle that routes Rust trait calls back
        // into Swift via the extern "Swift" shims. FunctionParam bridges additionally
        // emit register/unregister fns; OptionsField bridges emit a factory + options
        // helper instead (see below).
        out.push_str(&plugin_inbound::emit_inbound_wrapper(
            trait_def,
            bridge_cfg,
            api,
            &source_crate,
            &type_paths,
            &config.error_type_name(),
            &config.error_constructor_expr(),
        ));
        out.push('\n');
    }
    // OptionsField: emit factory fn + options-helper fn bodies (the extern "Rust"
    // declarations were already pushed into extern_blocks above, inside the ffi module).
    // Also emit bidirectional From impls so the factory and helper bodies compile:
    //   - From<inner_path> for type_alias  (factory: VisitorHandle::from(__inner))
    //   - From<type_alias> for inner_path  (helper: <inner_path>::from(h))
    //   - From<core_options_path> for options_type  (helper: ConversionOptions::from(__core))
    let mut options_field_from_emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        let from_impls = plugin_inbound::emit_options_field_from_impls(
            bridge_cfg,
            api,
            &source_crate,
            &mut options_field_from_emitted,
        );
        if !from_impls.is_empty() {
            out.push_str(&from_impls);
            out.push('\n');
        }
        let (_factory_extern, factory_body) =
            plugin_inbound::emit_options_field_factory(trait_def, bridge_cfg, api, &source_crate);
        if !factory_body.is_empty() {
            out.push_str(&factory_body);
            out.push('\n');
        }
        let (_helper_extern, helper_body) =
            plugin_inbound::emit_options_field_options_helper(bridge_cfg, api, &source_crate);
        if !helper_body.is_empty() {
            out.push_str(&helper_body);
            out.push('\n');
        }
    }

    // Emit Rust free-function shims for streaming adapters.
    // The matching extern "Rust" declarations are emitted inside the ffi module above.
    // Each shim blocks on a Tokio runtime and drives the stream to completion (or first
    // error), returning Result<(), String> so swift-bridge maps it to a throwing Swift call.
    let streaming_shims = wrappers::emit_streaming_adapter_shims(&config.adapters, &source_crate);
    if !streaming_shims.is_empty() {
        out.push_str(&streaming_shims);
    }

    // Emit from_json shim implementations for extra serde param types.
    // These allow Swift e2e tests to deserialise fixture JSON into the strongly-typed
    // request objects (e.g. ChatCompletionRequest) that the swift-bridge wrappers require.
    for ty in &extra_serde_param_types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path_base =
            crate::codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        let source_path = if ty.has_lifetime_params {
            format!("{source_path_base}<'static>")
        } else {
            source_path_base
        };
        json_bridge::emit_from_json_shim(&mut out, &type_snake, type_name, &source_path, type_name);
    }

    // Emit from_json shim implementations for streaming item types.
    // The Swift streaming wrapper calls `RustBridge.{itemType}FromJson(json)` to
    // deserialise each JSON chunk into the opaque swift-bridge type.
    for ty in &streaming_item_types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path_base =
            crate::codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        let source_path = if ty.has_lifetime_params {
            format!("{source_path_base}<'static>")
        } else {
            source_path_base
        };
        json_bridge::emit_from_json_shim(&mut out, &type_snake, type_name, &source_path, type_name);
    }

    // Emit from_json shim implementations for any other DTO that Swift will JSON-encode
    // (matched extern declarations emitted in the ffi module above). Keeps the Swift
    // binding side's `intoRust()` JSON-fallback call linkable on the Rust side.
    for ty in &json_fallback_types {
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path_base =
            crate::codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        let source_path = if ty.has_lifetime_params {
            format!("{source_path_base}<'static>")
        } else {
            source_path_base
        };
        json_bridge::emit_from_json_shim(&mut out, &type_snake, type_name, &source_path, type_name);
    }

    // Enum from_json bodies — deserialise the source enum and wrap in the
    // swift-bridge wrapper newtype. Mirrors the struct path above.
    // Skip result-type enums: their extern declarations were filtered above.
    for en in &json_fallback_enums_filtered {
        let enum_snake = AsSnakeCase(en.name.as_str()).to_string();
        let enum_name = &en.name;
        let source_path =
            crate::codegen::generators::type_paths::resolve_type_path(enum_name, &source_crate, &type_paths);
        let map_expr = format!("{enum_name}::from");
        json_bridge::emit_from_json_shim(&mut out, &enum_snake, enum_name, &source_path, &map_expr);
    }

    out
}
