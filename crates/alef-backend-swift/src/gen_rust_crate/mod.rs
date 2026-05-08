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
pub(crate) mod shims;
pub(crate) mod trait_bridge;
pub(crate) mod type_bridge;
pub(crate) mod wrappers;

use alef_codegen::generators::type_paths::build_type_path_lookup;
use alef_core::backend::GeneratedFile;
use alef_core::config::{ResolvedCrateConfig, TraitBridgeConfig};
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
    let core_crate_dir = config.core_crate_for_language(alef_core::config::extras::Language::Swift);
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

    let base_features = config.features_for_language(alef_core::config::extras::Language::Swift);
    // The IR records `any(feature = "ocr", feature = "ocr-wasm")` as the cfg condition for
    // TesseractWasmBackend (inherited from `pub mod ocr` in lib.rs). The concrete type,
    // however, lives in `kreuzberg::ocr::TesseractWasmBackend` which requires `ocr-wasm`.
    // `ocr` is transitively enabled by `full`; ensure `ocr-wasm` is also included whenever
    // the OCR module would be active so the bridge compiles correctly.
    let mut features_owned: Vec<String>;
    let ocr_active = base_features.iter().any(|f| f == "ocr" || f == "full");
    let ocr_wasm_present = base_features.iter().any(|f| f == "ocr-wasm");
    let features: &[String] = if ocr_active && !ocr_wasm_present {
        features_owned = base_features.to_vec();
        features_owned.push("ocr-wasm".to_string());
        &features_owned
    } else {
        base_features
    };
    let exclude_functions: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let exclude_types: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
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
    let cargo_toml = cargo::emit_cargo_toml(
        crate_name,
        &core_dep_key,
        version,
        &swift_bridge_ver,
        swift_bridge_build_ver,
        &core_path,
        features,
        license,
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

    // Set of type names that do NOT implement serde (Serialize + Deserialize).
    // These cannot be JSON-bridged and must use unimplemented!() when they appear
    // as inner Named types in Optional/Vec fields or return types.
    let no_serde_names: HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.has_serde)
        .map(|t| t.name.as_str())
        .collect();

    // Filter to only functions that can be fully bridged without emitting unimplemented!().
    // Unbridgeable functions (enum params, Vec<u8> tuple params, non-serde return types)
    // are silently excluded from both the extern block and the shim impl — callers in Swift
    // simply won't see these functions rather than panicking at runtime.
    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .filter(|f| shims::is_bridgeable_fn(f, &enum_names, &type_paths, &no_serde_names))
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
    let mut extern_blocks: Vec<String> = Vec::new();
    for ty in &visible_types {
        extern_blocks.push(extern_block::emit_extern_block_for_type(
            ty,
            exclude_fields,
            &type_paths,
            &no_serde_names,
        ));
    }
    for en in &visible_enums {
        extern_blocks.push(extern_block::emit_extern_block_for_enum(en));
    }
    if !visible_functions.is_empty() {
        let visible: Vec<FunctionDef> = visible_functions.iter().map(|f| (*f).clone()).collect();
        extern_blocks.push(extern_block::emit_extern_block_for_functions(&visible));
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        extern_blocks.push(trait_bridge::emit_extern_block_for_trait_bridge(trait_def));
    }

    out.push_str("#[swift_bridge::bridge]\nmod ffi {\n");
    for block in &extern_blocks {
        out.push_str(block);
    }
    // JSON factory extern declarations — always emitted so the e2e test layer
    // can deserialise fixture JSON into opaque swift-bridge types.
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
        ));
        out.push('\n');
    }
    for (_bridge_cfg, trait_def) in &active_bridges {
        out.push_str(&trait_bridge::emit_trait_bridge_wrapper(
            trait_def,
            &source_crate,
            &enum_names,
        ));
        out.push('\n');
    }

    // Emit JSON-factory shims for types that are serde-compatible but cannot be
    // constructed from the e2e test layer using the swift-bridge init() path.
    // These are used by the generated e2e tests to deserialize JSON fixture
    // values into their opaque swift-bridge wrapper types.
    emit_json_factory_shims(&source_crate, &mut out);

    out
}

/// Emits JSON factory functions for the opaque swift-bridge types that
/// the e2e test layer needs to construct from JSON strings.
///
/// Functions emitted:
///   - `extraction_config_from_json(json: String) -> Result<ExtractionConfig, String>`
///   - `batch_bytes_item_from_json(json: String) -> Result<BatchBytesItem, String>`
///   - `batch_file_item_from_json(json: String) -> Result<BatchFileItem, String>`
///
/// These are declared in a separate `extern "Rust"` block (appended after code
/// generation) so they appear in the swift-bridge ffi module and are callable
/// from Swift as free functions.
fn emit_json_factory_shims(source_crate: &str, out: &mut String) {
    out.push_str("// JSON factory shims for e2e test layer.\n");
    out.push_str("// These let generated tests deserialise fixture JSON into opaque swift-bridge types.\n\n");

    // ExtractionConfig
    out.push_str(&format!(
        "pub fn extraction_config_from_json(json: String) -> Result<ExtractionConfig, String> {{\n\
             serde_json::from_str::<{source_crate}::ExtractionConfig>(&json)\n\
                 .map_err(|e| e.to_string())\n\
                 .map(ExtractionConfig)\n\
         }}\n\n"
    ));

    // BatchBytesItem
    out.push_str(&format!(
        "pub fn batch_bytes_item_from_json(json: String) -> Result<BatchBytesItem, String> {{\n\
             serde_json::from_str::<{source_crate}::BatchBytesItem>(&json)\n\
                 .map_err(|e| e.to_string())\n\
                 .map(BatchBytesItem)\n\
         }}\n\n"
    ));

    // BatchFileItem
    out.push_str(&format!(
        "pub fn batch_file_item_from_json(json: String) -> Result<BatchFileItem, String> {{\n\
             serde_json::from_str::<{source_crate}::BatchFileItem>(&json)\n\
                 .map_err(|e| e.to_string())\n\
                 .map(BatchFileItem)\n\
         }}\n\n"
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
