//! Emits `Cargo.toml` and `build.rs` for the swift-bridge crate.

use crate::codegen::cfg as shared_cfg;
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;

/// Formats a features array for TOML output.
/// Uses multi-line format when `features.len() >= 3` or the rendered line exceeds 100 chars.
fn format_features_array(features: &[String]) -> String {
    if features.is_empty() {
        return String::new();
    }

    let quoted = features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>();
    let single_line = quoted.join(", ");
    let single_line_full = format!(", features = [{single_line}]");

    if features.len() >= 3 || single_line_full.len() > 100 {
        let mut multi_line = String::from(", features = [\n");
        for feature in &quoted {
            multi_line.push_str("    ");
            multi_line.push_str(feature);
            multi_line.push_str(",\n");
        }
        multi_line.push(']');
        multi_line
    } else {
        single_line_full
    }
}

/// Whether any bridged function or method is async, so the crate needs swift-bridge's `async`
/// feature (the runtime behind `async fn` in `extern "Rust"` blocks). A streaming adapter's start
/// and `next` bridge functions are `async fn` too, but its owner method is not necessarily async
/// in the IR, so the caller adds adapters separately. Only opaque, non-trait types get bridged
/// method declarations; a trait bridge's async methods go through a blocking shim. ~keep
fn api_has_async(api: &ApiSurface) -> bool {
    api.functions.iter().any(|f| f.is_async)
        || api
            .types
            .iter()
            .any(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| m.is_async && !m.sanitized && !m.is_static))
}

fn swift_bridge_dep(version: &str, needs_async: bool) -> String {
    if needs_async {
        format!("swift-bridge = {{ version = \"{version}\", features = [\"async\"] }}")
    } else {
        format!("swift-bridge = \"{version}\"")
    }
}

/// Emit the `Cargo.toml` content for the generated swift crate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_cargo_toml(
    crate_name: &str,
    core_dep_key: &str,
    _core_crate_dir: &str,
    version: &str,
    swift_bridge_ver: &str,
    swift_bridge_build_ver: &str,
    core_path: &str,
    features: &[String],
    extra_deps: &str,
    license: &str,
    has_streaming_adapters: bool,
    target_overrides: &[crate::core::config::languages::SwiftTargetDepOverride],
    api: &ApiSurface,
    excluded_default_features: &[String],
    ffi_dep_key: &str,
    ffi_dep_path: &str,
    ffi_features: &[String],
    ffi_target_overrides: &[crate::core::config::languages::SwiftTargetDepOverride],
    cargo_lints: &crate::core::config::CargoLintsConfig,
) -> String {
    let source_crate_name = core_dep_key;
    let features_block = if features.is_empty() {
        String::new()
    } else {
        format_features_array(features)
    };
    let package_rename_block = if core_dep_key != crate_name {
        format!(", package = \"{crate_name}\"")
    } else {
        String::new()
    };
    let streaming_deps = if has_streaming_adapters {
        "futures-util = \"0.3\"\n"
    } else {
        ""
    };
    let extra_deps_block = if extra_deps.trim().is_empty() {
        String::new()
    } else {
        format!("{extra_deps}\n")
    };
    // gated on `cfg(not(any(<override cfgs>)))` and each override emits its own
    // `[target.'cfg(...)'.dependencies]` block (similar to the FFI and Dart
    let core_dep_for_block = crate::scaffold::render_core_dep(
        source_crate_name,
        core_path,
        &format!("{features_block}{package_rename_block}"),
        version,
    );
    // Both the core-dep and FFI-dep override loops below emit
    // `[target.'cfg(...)'.dependencies]` tables into the same `[dependencies]`
    // section of the final manifest, so cargo-sort's table-order rule
    // (alphabetical by the raw cfg predicate string, plain byte-wise
    // comparison) applies across both groups together — sorting each group
    // independently and concatenating them is not enough. Collect every
    // target-cfg entry (core + FFI) into one list and sort it once, in
    // `target_blocks_section` below.
    let mut target_dep_entries: Vec<(String, String)> = Vec::new();
    if !target_overrides.is_empty() {
        // Gate the default dep on cfg(not(any(<overrides>))) to keep one and only
        let neg_cfg = if target_overrides.len() == 1 {
            target_overrides[0].cfg.clone()
        } else {
            let any = target_overrides
                .iter()
                .map(|o| o.cfg.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("any({any})")
        };
        target_dep_entries.push((format!("not({neg_cfg})"), core_dep_for_block.clone()));
        for entry in target_overrides {
            let feat_list = entry
                .features
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let feats_block = if feat_list.is_empty() {
                String::new()
            } else {
                format!(", features = [{feat_list}]")
            };
            let default_block = if entry.default_features {
                String::new()
            } else {
                ", default-features = false".to_string()
            };
            let entry_dep = crate::scaffold::render_core_dep(
                source_crate_name,
                core_path,
                &format!("{feats_block}{default_block}{package_rename_block}"),
                version,
            );
            target_dep_entries.push((entry.cfg.clone(), entry_dep));
        }
    }
    let mut dep_entries: Vec<String> = vec![
        format!("ahash = \"{}\"", tv::cargo::AHASH),
        format!("async-trait = \"{}\"", tv::cargo::ASYNC_TRAIT),
        format!("libc = \"{}\"", tv::cargo::LIBC),
        format!(
            "serde = {{ version = \"{}\", features = [\"derive\"] }}",
            tv::cargo::SERDE
        ),
        format!("serde_json = \"{}\"", tv::cargo::SERDE_JSON),
        swift_bridge_dep(swift_bridge_ver, api_has_async(api) || has_streaming_adapters),
        format!(
            "tokio = {{ version = \"{}\", features = [\"rt\", \"rt-multi-thread\", \"macros\"] }}",
            tv::cargo::TOKIO
        ),
    ];
    if !core_dep_for_block.is_empty() && target_overrides.is_empty() {
        dep_entries.push(core_dep_for_block.clone());
    }
    // NOTE: see `ffi_keep_alive_shim.rs.jinja` and `ResolvedCrateConfig::ffi_crate_path_from_swift_rust`.
    // When `ffi_features` is non-empty, drop the FFI crate's default features and enable exactly
    // this set — lets the swift shim exclude cross-compile-hostile features (e.g. `heic` via a
    // `full-no-heic` set) that the primary core dep's feature handling does not reach.
    let ffi_suffix = if ffi_features.is_empty() {
        String::new()
    } else {
        format!(", default-features = false{}", format_features_array(ffi_features))
    };
    // When `ffi_target_overrides` is set the FFI dep moves out of the flat
    // `[dependencies]` table entirely and into `target_dep_entries` below —
    // mirrors how the core dep's `target_overrides` loop above excludes
    // `core_dep_for_block` from `dep_entries` once overrides apply.
    if ffi_target_overrides.is_empty() {
        dep_entries.push(crate::scaffold::render_core_dep(
            ffi_dep_key,
            ffi_dep_path,
            &ffi_suffix,
            version,
        ));
    }
    if !ffi_target_overrides.is_empty() {
        let neg_cfg = if ffi_target_overrides.len() == 1 {
            ffi_target_overrides[0].cfg.clone()
        } else {
            let any = ffi_target_overrides
                .iter()
                .map(|o| o.cfg.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("any({any})")
        };
        let default_entry = crate::scaffold::render_core_dep(ffi_dep_key, ffi_dep_path, &ffi_suffix, version);
        target_dep_entries.push((format!("not({neg_cfg})"), default_entry));
        for entry in ffi_target_overrides {
            let feat_list = entry
                .features
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let feats_block = if feat_list.is_empty() {
                String::new()
            } else {
                format!(", features = [{feat_list}]")
            };
            // Order matches `ffi_suffix` above (`default-features` before
            // `features`), not the core-dep override loop's order, so a
            // single override entry reproduces the same dep-line shape as
            // the flat `ffi_features`-only case.
            let entry_suffix = if entry.default_features {
                feats_block
            } else {
                format!(", default-features = false{feats_block}")
            };
            let entry_dep = crate::scaffold::render_core_dep(ffi_dep_key, ffi_dep_path, &entry_suffix, version);
            target_dep_entries.push((entry.cfg.clone(), entry_dep));
        }
    }
    // The core-dep and FFI-dep loops above both contribute to
    // `target_dep_entries`; sort them together (not each group separately) so
    // the combined `[target.'cfg(...)'.dependencies]` table sequence matches
    // what `cargo-sort` expects regardless of which dependency a given block
    // is about.
    let target_blocks_section = crate::scaffold::join_sorted_target_dep_blocks(target_dep_entries);
    let target_blocks_section = if target_blocks_section.is_empty() {
        String::new()
    } else {
        format!("\n{target_blocks_section}")
    };
    if has_streaming_adapters {
        dep_entries.push(format!("futures-util = \"{}\"", tv::cargo::FUTURES_UTIL));
    }
    for line in extra_deps.lines() {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            dep_entries.push(trimmed.to_string());
        }
    }
    crate::scaffold::sort_dependency_lines(&mut dep_entries);
    let dep_block = dep_entries.join("\n");
    let _ = streaming_deps;
    let _ = extra_deps_block;

    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    let mut cfg_features = shared_cfg::collect_cfg_features(api);
    // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`) must
    // still get a forwarding entry below -- alef-task #374, regression in
    // `cargo_excluded_features_tests.rs`. ~keep
    cfg_features.extend(excluded_default_features.iter().cloned());
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        // `[target.'cfg(...)'.dependencies]` block alone is insufficient
        let excluded: std::collections::HashSet<&str> = excluded_default_features.iter().map(String::as_str).collect();
        let mut lines: Vec<String> = Vec::with_capacity(cfg_features.len() + 1);
        let default_list: Vec<String> = cfg_features
            .iter()
            .filter(|name| !excluded.contains(name.as_str()))
            .map(|name| format!("\"{name}\""))
            .collect();
        lines.push(format!("default = [{}]", default_list.join(", ")));
        for name in &cfg_features {
            lines.push(format!(r#"{name} = ["{core_dep_key}/{name}"]"#));
        }
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    // The [lints.rust] block keeps cfg(frb_expand) in the allow-list (FRB-internal
    // cfg used during macro expansion). Cargo allows only one `[lints.rust]` table
    // per manifest, so a configured `[crates.cargo_lints.rust]` entry becomes an
    // extra sibling line under this same hand-written header rather than a second
    // one -- see `CargoLintsConfig::extra_rust_lines`. ~keep
    let mut lints_rust_lines =
        vec!["unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }".to_string()];
    lints_rust_lines.extend(cargo_lints.extra_rust_lines(&["unexpected_cfgs"]));
    let clippy_block = cargo_lints.clippy_block();
    let clippy_section = if clippy_block.is_empty() {
        String::new()
    } else {
        format!("\n\n{clippy_block}")
    };
    let lints_block = format!("[lints.rust]\n{}{clippy_section}", lints_rust_lines.join("\n"));

    format!(
        r#"# Generated by alef. Do not edit by hand.
[package]
name = "{crate_name}-swift"
version = "{version}"
edition = "2024"
license = "{license}"

# `ahash`, `async-trait`, `libc`, `serde`, `serde_json`, and `tokio` are all
# conditionally referenced by alef-emitted code: `ahash` only when the
# umbrella crate exposes `AHashMap<Cow<str>, _>` parameters (the conditional
# `__*_ahash` shim rebuilds), `async-trait` and `tokio` only when the API
# surface includes async streaming adapters and runtime spawn, `libc` only
# when service API C callback functions are emitted, `serde` and
# `serde_json` only when JSON DTO conversions are emitted. They are listed
# unconditionally in `[dependencies]` so the manifest is stable across
# regens, and ignored here so cargo-machete does not flag downstream crates
# whose API surface does not trigger those paths as unused.
[package.metadata.cargo-machete]
ignored = ["ahash", "async-trait", "libc", "serde", "serde_json", "tokio"]

[lib]
crate-type = ["cdylib", "staticlib"]
# The `extern "Swift"` block emits linker references that are only resolvable
# when the crate is linked into a Swift target. `cargo test --workspace` on
# pure-Rust runners (e.g. windows-latest) would otherwise fail with
# undefined `__swift_bridge__$*$alef_visit_*` symbols.
test = false
doctest = false
bench = false

{features_table}[dependencies]
{dep_block}
{target_blocks_section}
[build-dependencies]
swift-bridge-build = "{swift_bridge_build_ver}"

{lints_block}
"#
    )
}

/// Emit the `build.rs` content for the generated swift crate.
pub(crate) fn emit_build_rs() -> String {
    format!(
        "{}\n{}",
        crate::core::hash::SELF_MARKING_HEADER_LINE,
        r#"use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR unset"));
    let crate_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME unset");
    let bridges = vec!["src/lib.rs"];
    swift_bridge_build::parse_bridges(bridges).write_all_concatenated(out_dir, &crate_name);
    println!("cargo:rerun-if-changed=src/lib.rs");
}
"#
    )
}

#[cfg(test)]
mod manifest_tests;
