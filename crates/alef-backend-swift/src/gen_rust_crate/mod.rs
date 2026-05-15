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
pub(crate) mod plugin_inbound;
pub(crate) mod shims;
pub(crate) mod trait_bridge;
pub(crate) mod type_bridge;
pub(crate) mod wrappers;

use alef_codegen::generators::type_paths::build_type_path_lookup;
use alef_core::backend::GeneratedFile;
use alef_core::config::extras::Language;
use alef_core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef_core::ir::{ApiSurface, EnumDef, FunctionDef, TypeDef};
use alef_core::template_versions;
use std::collections::HashSet;
use std::path::PathBuf;

/// Top-level entry point: emit all three files for the swift-bridge crate.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let base = PathBuf::from("packages/swift/rust");
    let crate_name = &api.crate_name;
    let version = &api.version;

    let swift_bridge_ver = crate::naming::swift_bridge_version(config);
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
    // The IR records `any(feature = "ocr", feature = "ocr-wasm")` as the cfg condition for
    // TesseractWasmBackend (inherited from `pub mod ocr` in lib.rs). The concrete type,
    // however, lives in `kreuzberg::ocr::TesseractWasmBackend` which requires `ocr-wasm`.
    // `ocr` is transitively enabled by `full`; ensure `ocr-wasm` is also included whenever
    // the OCR module would be active so the bridge compiles correctly.
    //
    // Only do this when the source crate actually exposes an `ocr-wasm` feature — otherwise
    // we would inject an unknown feature into Cargo.toml for crates that have no OCR module
    // at all (e.g. liter-llm). We probe by reading the on-disk Cargo.toml of the umbrella
    // crate.
    let mut features_owned: Vec<String>;
    let ocr_active = base_features.iter().any(|f| f == "ocr" || f == "full");
    let ocr_wasm_present = base_features.iter().any(|f| f == "ocr-wasm");
    let source_has_ocr_wasm = source_crate_has_feature(config, &core_crate_dir, "ocr-wasm");
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
        .any(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming));
    let extra_deps = render_extra_deps(config);
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

    Ok(vec![
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
    ])
}

fn render_extra_deps(config: &ResolvedCrateConfig) -> String {
    let deps = config.extra_deps_for_language(Language::Swift);
    if deps.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = deps
        .iter()
        .map(|(name, value)| match value {
            toml::Value::String(version) => format!("{name} = \"{version}\""),
            other => format!("{name} = {other}"),
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

/// Check whether the umbrella source crate exposes the given feature name in its
/// on-disk Cargo.toml. Used to gate auto-injection of optional features like
/// `ocr-wasm` that some crates expose and others do not.
fn source_crate_has_feature(config: &ResolvedCrateConfig, core_crate_dir: &str, feature: &str) -> bool {
    let root = match config.workspace_root.as_deref() {
        Some(p) => p.to_path_buf(),
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(_) => return false,
        },
    };
    let cargo_toml = root.join("crates").join(core_crate_dir).join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&cargo_toml) else {
        return false;
    };
    // Naive scan: look for `<feature> = [` or `<feature> = "..."` under [features]. Avoids
    // pulling in a TOML parser dep — the Cargo.toml format here is predictable.
    let needle_line_start = format!("{feature} =");
    let mut in_features = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && trimmed.starts_with(&needle_line_start) {
            return true;
        }
    }
    false
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
    out.push_str("#![allow(unused_variables, unreachable_code)]\n");
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
    out.push_str(")]\n\n");

    let visible_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait)
        .filter(|t| cfg_satisfied(t.cfg.as_deref(), configured_features))
        .collect();
    let visible_enums: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .filter(|e| cfg_satisfied(e.cfg.as_deref(), configured_features))
        .collect();

    // Set of enum names (not struct names) so wrappers can use the correct
    // conversion idiom: `T::from(val)` for enums, `T(val)` for struct newtypes.
    let enum_names: HashSet<&str> = visible_enums.iter().map(|e| e.name.as_str()).collect();

    // Union of all visible type names (structs + enums) that have swift-bridge wrapper newtypes
    // in the generated lib.rs. Used by trait bridge trampolines to decide whether a Named
    // return type should be wrapped (it has a newtype) or JSON-serialised (excluded/foreign type).
    let visible_type_names: HashSet<&str> = visible_types
        .iter()
        .map(|t| t.name.as_str())
        .chain(enum_names.iter().copied())
        .collect();

    // Set of type names that do NOT implement serde (Serialize + Deserialize).
    // These cannot be JSON-bridged and must use unimplemented!() when they appear
    // as inner Named types in Optional/Vec fields or return types.
    let no_serde_names: HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.has_serde)
        .map(|t| t.name.as_str())
        .collect();

    // api types that are returned by value from public functions/methods.  These appear as
    // `*mut T` opaque handles in the FFI; swift-bridge must declare them as the bare
    // Named type so the Swift side wraps them in the opaque class instead of receiving
    // a `RustString` (the JSON fallback).  Mirrors `compute_handle_returned_types` in
    // alef-backend-csharp's errors.rs.
    let handle_returned_types: HashSet<String> = type_bridge::compute_handle_returned_types(api);

    // Filter to only functions that can be fully bridged without emitting unimplemented!().
    // Unbridgeable functions (enum params, Vec<u8> tuple params, non-serde return types)
    // are silently excluded from both the extern block and the shim impl — callers in Swift
    // simply won't see these functions rather than panicking at runtime.
    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .filter(|f| shims::is_bridgeable_fn(f, &enum_names, &type_paths, &no_serde_names, &handle_returned_types))
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

    // Collect extern "Rust" blocks for the ffi module
    // Build a HashSet<String> from enum_names (&str) for the enum-aware bridge type helper.
    let enum_names_owned: std::collections::HashSet<String> = enum_names.iter().map(|s| s.to_string()).collect();
    let mut extern_blocks: Vec<String> = Vec::new();
    for ty in &visible_types {
        extern_blocks.push(extern_block::emit_extern_block_for_type(
            ty,
            exclude_fields,
            &type_paths,
            &no_serde_names,
            &enum_names_owned,
        ));
        // For opaque types with methods, also emit constructor + method extern blocks.
        if ty.is_opaque && !ty.methods.iter().all(|m| m.sanitized) && !ty.methods.is_empty() {
            // Only emit the `create_<type>` constructor when the user provides an explicit
            // `client_constructor_body` override in alef.toml. The default
            // `(api_key, base_url)` signature only fits liter_llm-style clients; for plugin
            // types like `HwpxExtractor::new()` or utilities like
            // `TessdataManager::new(Option<PathBuf>)` it produces calls that don't match
            // the real Rust signature. Opaque types without an override are returned by
            // Rust APIs, not constructed in Swift.
            let has_ctor_override = config
                .swift
                .as_ref()
                .is_some_and(|c| c.client_constructor_body.contains_key(&ty.name));
            if has_ctor_override && let Some(ctor_block) = extern_block::emit_extern_block_for_type_constructor(ty) {
                extern_blocks.push(ctor_block);
            }
            if let Some(method_block) = extern_block::emit_extern_block_for_type_methods(ty, &handle_returned_types) {
                extern_blocks.push(method_block);
            }
        }
    }
    for en in &visible_enums {
        extern_blocks.push(extern_block::emit_extern_block_for_enum(en));
    }
    if !visible_functions.is_empty() {
        let visible: Vec<FunctionDef> = visible_functions.iter().map(|f| (*f).clone()).collect();
        extern_blocks.push(extern_block::emit_extern_block_for_functions(
            &visible,
            &handle_returned_types,
        ));
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
    for (bridge_cfg, trait_def) in &active_bridges {
        if bridge_cfg.bind_via != BridgeBinding::FunctionParam {
            continue;
        }
        extern_blocks.push(plugin_inbound::emit_extern_block_for_inbound(trait_def, bridge_cfg));
    }

    // Streaming adapters: emit an extern "Rust" block for each streaming adapter
    // that owns a client type.  The Swift host wrapper references these as
    // `RustBridge.{camelName}(client, …)`, so they must be declared in the
    // swift-bridge module or the Swift compiler will produce
    // "module 'RustBridge' has no member named …".
    if let Some(streaming_block) = extern_block::emit_extern_block_for_streaming_adapters(&config.adapters) {
        extern_blocks.push(streaming_block);
    }

    // Detect legacy extraction e2e types: when the api surface exposes
    // `ExtractionConfig`, `BatchBytesItem`, and `BatchFileItem` (all serde-enabled),
    // emit JSON factory shims so the e2e test layer can deserialise fixture JSON
    // into the corresponding opaque swift-bridge types. This is structural — no
    // crate-name hardcoding — and is a no-op for binding crates that don't expose
    // these specific types.
    let has_e2e_types = api_has_e2e_types(api);

    // Collect serde-enabled non-opaque types that appear as method parameters but
    // are NOT already covered by the kreuzberg e2e shims above. These need their
    // own `{type_snake}_from_json` free-function shims so Swift e2e tests can
    // deserialise fixture JSON into the strongly-typed request objects expected by
    // the swift-bridge wrappers.
    let e2e_type_names = ["ExtractionConfig", "BatchBytesItem", "BatchFileItem"];
    let extra_serde_param_types: Vec<&TypeDef> =
        collect_serde_param_types(api, &visible_types, &visible_functions, &e2e_type_names);

    // Collect streaming item types that have serde derives.  The Swift streaming
    // wrapper uses `RustBridge.{itemType}FromJson(json)` — a Rust-side free
    // function — to deserialise each JSON chunk into the opaque type so that
    // callers can use the full swift-bridge method API on the result.
    // The `from_json` shim must be emitted even when the type is NOT a param type.
    let extra_serde_param_names: std::collections::HashSet<&str> =
        extra_serde_param_types.iter().map(|t| t.name.as_str()).collect();
    let e2e_type_name_set: std::collections::HashSet<&str> = e2e_type_names.iter().copied().collect();
    let streaming_item_types: Vec<&TypeDef> = {
        let streaming_item_names: std::collections::HashSet<&str> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
            .filter_map(|a| a.item_type.as_deref())
            .collect();
        visible_types
            .iter()
            .copied()
            .filter(|ty| streaming_item_names.contains(ty.name.as_str()))
            .filter(|ty| ty.has_serde && !ty.is_opaque && !ty.is_trait)
            // Skip types already covered by the other from_json shim sets.
            .filter(|ty| !extra_serde_param_names.contains(ty.name.as_str()))
            .filter(|ty| !e2e_type_name_set.contains(ty.name.as_str()))
            .collect()
    };

    out.push_str("#[swift_bridge::bridge]\nmod ffi {\n");
    for block in &extern_blocks {
        out.push_str(block);
    }
    if has_e2e_types {
        out.push_str(concat!(
            "    extern \"Rust\" {\n",
            "\n",
            "        #[swift_bridge(swift_name = \"extractionConfigFromJson\")]\n",
            "        fn extraction_config_from_json(json: String) -> Result<ExtractionConfig, String>;\n",
            "        #[swift_bridge(swift_name = \"batchBytesItemFromJson\")]\n",
            "        fn batch_bytes_item_from_json(json: String) -> Result<BatchBytesItem, String>;\n",
            "        #[swift_bridge(swift_name = \"batchFileItemFromJson\")]\n",
            "        fn batch_file_item_from_json(json: String) -> Result<BatchFileItem, String>;\n",
            "    }\n",
        ));
    }
    if !extra_serde_param_types.is_empty() {
        out.push_str("    extern \"Rust\" {\n\n");
        for ty in &extra_serde_param_types {
            use heck::{AsSnakeCase, ToLowerCamelCase};
            let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
            let type_name = &ty.name;
            let swift_name = format!("{}_from_json", type_snake).to_lower_camel_case();
            out.push_str(&format!(
                "        #[swift_bridge(swift_name = \"{swift_name}\")]\n        fn {type_snake}_from_json(json: String) -> Result<{type_name}, String>;\n"
            ));
        }
        out.push_str("    }\n");
    }
    // Emit from_json extern blocks for streaming item types.
    // The streaming wrapper calls `RustBridge.{itemType}FromJson(json)` on the
    // Swift side to deserialise each JSON chunk into the opaque type.
    if !streaming_item_types.is_empty() {
        out.push_str("    extern \"Rust\" {\n\n");
        for ty in &streaming_item_types {
            use heck::{AsSnakeCase, ToLowerCamelCase};
            let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
            let type_name = &ty.name;
            let swift_name = format!("{}_from_json", type_snake).to_lower_camel_case();
            out.push_str(&format!(
                "        #[swift_bridge(swift_name = \"{swift_name}\")]\n        fn {type_snake}_from_json(json: String) -> Result<{type_name}, String>;\n"
            ));
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n\n");

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
            &enum_names,
            &no_serde_names,
            &handle_returned_types,
        ));
        out.push('\n');
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        out.push_str(&trait_bridge::emit_trait_bridge_wrapper(
            trait_def,
            &source_crate,
            &enum_names,
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
        // Match the inbound-extern gate above: only emit Rust-side wrappers +
        // register fns for `function_param` bridges. `options_field` bridges
        // are handled via options-builder methods, not via the inbound plugin
        // registry pattern.
        if bridge_cfg.bind_via != BridgeBinding::FunctionParam {
            continue;
        }
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

    // Emit Rust free-function shims for streaming adapters.
    // The matching extern "Rust" declarations are emitted inside the ffi module above.
    // Each shim blocks on a Tokio runtime and drives the stream to completion (or first
    // error), returning Result<(), String> so swift-bridge maps it to a throwing Swift call.
    let streaming_shims = wrappers::emit_streaming_adapter_shims(&config.adapters, &source_crate);
    if !streaming_shims.is_empty() {
        out.push_str(&streaming_shims);
    }

    // Emit JSON-factory shims for legacy extraction e2e types when present.
    // The matching extern declarations are emitted in the ffi module above.
    if has_e2e_types {
        emit_json_factory_shims(&source_crate, &mut out);
    }

    // Emit from_json shim implementations for extra serde param types.
    // These allow Swift e2e tests to deserialise fixture JSON into the strongly-typed
    // request objects (e.g. ChatCompletionRequest) that the swift-bridge wrappers require.
    for ty in &extra_serde_param_types {
        use heck::AsSnakeCase;
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path =
            alef_codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        out.push_str(&format!(
            "pub fn {type_snake}_from_json(json: String) -> Result<{type_name}, String> {{\n    \
             serde_json::from_str::<{source_path}>(&json)\n        \
             .map({type_name})\n        \
             .map_err(|e| e.to_string())\n}}\n\n"
        ));
    }

    // Emit from_json shim implementations for streaming item types.
    // The Swift streaming wrapper calls `RustBridge.{itemType}FromJson(json)` to
    // deserialise each JSON chunk into the opaque swift-bridge type.
    for ty in &streaming_item_types {
        use heck::AsSnakeCase;
        let type_snake = AsSnakeCase(ty.name.as_str()).to_string();
        let type_name = &ty.name;
        let source_path =
            alef_codegen::generators::type_paths::resolve_type_path(type_name, &source_crate, &type_paths);
        out.push_str(&format!(
            "pub fn {type_snake}_from_json(json: String) -> Result<{type_name}, String> {{\n    \
             serde_json::from_str::<{source_path}>(&json)\n        \
             .map({type_name})\n        \
             .map_err(|e| e.to_string())\n}}\n\n"
        ));
    }

    out
}

/// Returns `true` when the api surface exposes the legacy extraction e2e helper
/// types (`ExtractionConfig`, `BatchBytesItem`, `BatchFileItem`), all serde-enabled.
/// Used to gate emission of JSON-factory shims and Swift e2e wrapper helpers.
fn api_has_e2e_types(api: &ApiSurface) -> bool {
    let required = ["ExtractionConfig", "BatchBytesItem", "BatchFileItem"];
    required
        .iter()
        .all(|name| api.types.iter().any(|t| !t.is_trait && t.has_serde && t.name == *name))
}

/// Emits JSON factory functions for legacy extraction opaque swift-bridge types
/// (`extraction_config_from_json`, `batch_bytes_item_from_json`, `batch_file_item_from_json`).
/// Wired into `emit_lib_rs` only when the api surface exposes all three serde-enabled
/// types — see `api_has_e2e_types`. Crate-agnostic by structure.
fn emit_json_factory_shims(source_crate: &str, out: &mut String) {
    out.push_str(&crate::template_env::render(
        "json_factory_shims.rs.jinja",
        minijinja::context! {
            source_crate => source_crate,
        },
    ));
}

/// Returns `true` when the `cfg` condition is satisfied by `configured_features`.
///
/// Handles:
/// - `feature = "foo"` — simple single-feature gate
/// - `any (feature = "foo" , feature = "bar")` — OR of feature gates (alef IR format)
///
/// Returns `true` for `None` (no condition) and for any condition format that cannot
/// be parsed (safe default: include the type and let the compiler surface the error
/// only if the feature combination is truly incompatible).
///
/// NOTE: the alef IR sometimes records a broader cfg condition from a parent module
/// rather than the exact gate on the specific item.  When all conditions in an `any()`
/// are features (not target_arch etc.) we check whether ALL of them are present in
/// the configured features — not just any one.  This is conservative: it only excludes
/// types that cannot possibly compile given the configured features.
fn cfg_satisfied(cfg: Option<&str>, configured_features: &HashSet<&str>) -> bool {
    let Some(cfg_str) = cfg else {
        return true; // no condition → always visible
    };

    // `full` is the all-inclusive aggregate feature: every sub-feature is transitively
    // enabled when `full` is configured. Skip the cfg check entirely in that case.
    if configured_features.contains("full") {
        return true;
    }

    // Simple `feature = "foo"` form.
    if let Some(rest) = cfg_str.strip_prefix("feature = \"") {
        if let Some(feature_name) = rest.strip_suffix('"') {
            return configured_features.contains(feature_name);
        }
    }

    // `any (feature = "foo" , feature = "bar" , ...)` form produced by the alef IR extractor.
    // Extract the parenthesised content and check every listed feature.
    // We require ALL of the listed features to be absent before excluding a type — i.e. we
    // include the type if ANY of the listed features is configured.
    if let Some(inner) = cfg_str
        .strip_prefix("any (")
        .or_else(|| cfg_str.strip_prefix("any("))
        .and_then(|s| s.strip_suffix(')'))
    {
        // Split on `,` and parse each clause as `feature = "..."`.
        let feature_names: Vec<&str> = inner
            .split(',')
            .filter_map(|clause| {
                let trimmed = clause.trim();
                trimmed.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"'))
            })
            .collect();

        if !feature_names.is_empty() {
            // any() → include if at least one required feature is present.
            return feature_names.iter().any(|f| configured_features.contains(f));
        }
    }

    // For unrecognised formats, include the type (conservative default).
    true
}

/// Collect serde-enabled, non-opaque types from `visible_types` that appear as
/// parameters in either free functions or type methods, excluding those already
/// covered by the static kreuzberg e2e shims (`already_covered`).
///
/// These types need `{type_snake}_from_json` shims so Swift e2e tests can
/// deserialise fixture JSON into the strongly-typed request objects required by
/// swift-bridge wrappers (e.g. `ChatCompletionRequest` on `DefaultClient.chat`).
fn collect_serde_param_types<'a>(
    api: &'a ApiSurface,
    visible_types: &[&'a TypeDef],
    visible_functions: &[&FunctionDef],
    already_covered: &[&str],
) -> Vec<&'a TypeDef> {
    let covered: std::collections::HashSet<&str> = already_covered.iter().copied().collect();

    /// Return true if any param in `params` references the type named `name`.
    fn param_uses_type(params: &[alef_core::ir::ParamDef], name: &str) -> bool {
        params.iter().any(|p| p.ty.references_named(name))
    }

    visible_types
        .iter()
        .copied()
        .filter(|ty| {
            // Must be serde-enabled and non-opaque (serde types are the request/response structs).
            ty.has_serde && !ty.is_opaque && !ty.is_trait
        })
        .filter(|ty| !covered.contains(ty.name.as_str()))
        .filter(|ty| {
            let name = ty.name.as_str();
            // Check free-function params.
            let in_free_fn = visible_functions.iter().any(|f| param_uses_type(&f.params, name));
            // Check method params on all types in the API surface.
            let in_method = api
                .types
                .iter()
                .any(|t| t.methods.iter().any(|m| param_uses_type(&m.params, name)));
            in_free_fn || in_method
        })
        .collect()
}
