//! Best-effort repair of a Rust-emitting binding crate's `[features]` table.
//!
//! `codegen::cfg::warn_on_undeclared_binding_cfg_features` warns when generated source for
//! a native Rust wrapper forwards a `#[cfg(feature = "X")]` gate the binding
//! crate's own `Cargo.toml` does not declare, and prescribes re-running `alef scaffold`. That
//! manifest is `generated_header: true`, so a fresh scaffold with no prior file on disk already
//! writes the right `[features]` table (both `scaffold_ruby_cargo` and `scaffold_elixir_cargo`
//! derive it from `codegen::cfg::collect_cfg_features`) -- but once the file exists,
//! `write_scaffold_files_report`'s ownership guard only overwrites it wholesale when it can prove
//! alef authored the bytes on disk. A manifest that predates the marker scheme, or one whose
//! marker a formatter/hand-edit moved past the guard's scan window, is refused forever: the
//! prescribed remedy becomes a permanent no-op. This module closes that gap with a narrower,
//! always-safe operation instead of widening the guard: inserting a missing forwarding row is
//! purely additive and cannot corrupt, reorder, or drop anything else already in the file, so it
//! runs on its own write path and does not need the guard's proof of full-file authorship. ~keep
//!
//! `Dart` (`packages/dart/rust/Cargo.toml`, via `backends::dart::gen_rust_crate::emit`) is
//! exactly as exposed to this same staleness as Ruby/Elixir: its manifest is also
//! `generated_header: true` and also derives `[features]` from `collect_cfg_features` on the
//! same `ApiSurface` used for its `lib.rs`, so a cfg-gated free function reexported into that
//! `lib.rs` after the manifest was first scaffolded hits the identical guard-refusal gap (alef
//! #154: a consumer's committed `packages/dart/rust/Cargo.toml` never picked up the `tokenizer`/
//! `tower` features `lib.rs` already forwards for `count_tokens`/`count_request_tokens`/
//! `record_cost_usd`, producing `unexpected cfg condition value` once `frb_generated.rs`
//! re-emits those same gates). Dart was simply never added to `managed_manifests` below when
//! this repair was written for Ruby/Elixir; nothing about the mechanism is Ruby/Elixir-specific. ~keep

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::collections::HashSet;
use std::path::PathBuf;

/// One Rust-emitting binding manifest this repair covers: the language it belongs to, its
/// manifest path, and the Cargo dependency-table key its own generator uses for the core crate
/// dependency -- the same key each `<feature> = ["{key}/<feature>"]` forwarding row must use, or
/// the row points at a dependency-table entry the manifest does not have. Native wrappers key
/// their forwarding rows off the raw, unmodified crate name; Dart keys off `[crates.dart]
/// core_crate_override` when configured, otherwise the crate name with `-` replaced by `_` (see
/// `backends::dart::gen_rust_crate::dart_core_dep_key`'s doc).
///
/// The fourth element is that language's own `excluded_default_features`, read from the same
/// config field its scaffolder reads. The repair has to honour it for the same reason the
/// scaffolder does: a name the config deliberately keeps out of `default` must stay out, or this
/// pass re-enables exactly what the scaffolder just excluded and the two disagree on disk. The
/// final flag excludes Dart from the native-only `wrapper_default_features` policy. ~keep
fn managed_manifests(config: &ResolvedCrateConfig) -> Vec<(Language, PathBuf, String, HashSet<&str>, bool)> {
    fn excluded(names: Option<&[String]>) -> HashSet<&str> {
        names.unwrap_or_default().iter().map(String::as_str).collect()
    }

    vec![
        (
            Language::Python,
            PathBuf::from(format!("crates/{}-py/Cargo.toml", config.core_crate_dir())),
            config.name.clone(),
            HashSet::new(),
            true,
        ),
        (
            Language::Node,
            PathBuf::from(format!("crates/{}-node/Cargo.toml", config.core_crate_dir())),
            config.name.clone(),
            excluded(config.node.as_ref().map(|c| c.excluded_default_features.as_slice())),
            true,
        ),
        (
            Language::Ruby,
            super::ruby_native_manifest_path(config),
            config.name.clone(),
            excluded(config.ruby.as_ref().map(|c| c.excluded_default_features.as_slice())),
            true,
        ),
        (
            Language::Php,
            PathBuf::from(format!("crates/{}-php/Cargo.toml", config.core_crate_dir())),
            config.name.clone(),
            excluded(config.php.as_ref().map(|c| c.excluded_default_features.as_slice())),
            true,
        ),
        (
            Language::Elixir,
            PathBuf::from(super::elixir_native_crate_dir(config)).join("Cargo.toml"),
            config.name.clone(),
            excluded(config.elixir.as_ref().map(|c| c.excluded_default_features.as_slice())),
            true,
        ),
        (
            Language::Dart,
            crate::backends::dart::gen_rust_crate::dart_native_manifest_path(config),
            crate::backends::dart::gen_rust_crate::dart_core_dep_key(config),
            excluded(config.dart.as_ref().map(|c| c.excluded_default_features.as_slice())),
            false,
        ),
    ]
}

/// The surface a binding backend actually emits from, given the surface this repair is handed.
///
/// [`crate::codegen::binding_projection`] drops every `binding_excluded` item before any binding
/// consumer sees the IR, so a manifest written by a backend declares features gated on the
/// *projected* surface. This repair is called with the raw source surface and must use that same
/// projection or it can propose forwarding rows for features nothing in the generated crate
/// references. ~keep
///
/// The per-language carrier prune (alef #480) is deliberately **not** reproduced here, and that
/// is load-bearing rather than an omission: it only sets `binding_excluded` on a field, and
/// `codegen::cfg::collect_cfg_gates` reads every `field.cfg` without consulting
/// `binding_excluded` at all. Both sides therefore see the same feature set whether or not the
/// prune ran, so they stay in lock-step by construction and this pass needs no language
/// dimension. Teaching `collect_cfg_gates` to skip excluded fields would break that and this
/// function would then have to take the target language too. ~keep
/// Add every cfg-forwarded feature the generated source for `languages` references but an
/// existing binding manifest does not yet declare, preserving the rest of each manifest exactly.
///
/// Skips a language absent from `languages` (scaffolding one language must not touch another's
/// manifest) and a manifest that does not exist yet on disk (a fresh `alef scaffold` run creates
/// it correctly the first time, via `scaffold_ruby_cargo`/`scaffold_elixir_cargo`'s own
/// `collect_cfg_features` call -- there is nothing here to repair). Failures are logged and
/// skipped rather than propagated: this is best-effort maintenance on top of an already-completed
/// scaffold run, not a required step it should abort.
///
/// Returns the manifest paths this call actually changed, for the caller's own reporting.
pub(crate) fn repair_missing_cfg_binding_features(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> Vec<PathBuf> {
    if let Err(error) = crate::core::config::validation::validate_wrapper_default_features(config) {
        tracing::warn!(%error, "refusing to repair native wrapper features from invalid configuration");
        return Vec::new();
    }
    let mut repaired = Vec::new();
    let projected = crate::codegen::binding_projection::project(api);
    for (language, relative_manifest, core_dep_key, excluded_default_features, preserve_wrapper_defaults) in
        managed_manifests(config)
    {
        if !languages.contains(&language) {
            continue;
        }
        // Contained before the read, not after: `std::fs::write` below follows a symlinked
        // ancestor exactly as the scaffold migrations' temporary files do, and `relative_manifest`
        // is config-derived (`ruby_native_manifest_path`, `elixir_native_crate_dir`, the Dart
        // crate directory). Logged and skipped rather than propagated, matching this function's
        // best-effort contract. ~keep
        let root = crate::codegen::cfg::workspace_root(config);
        let manifest_path =
            match crate::cli::pipeline::generate::write::contained_output_path(&root, &relative_manifest) {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!(
                        language = %language,
                        manifest = %relative_manifest.display(),
                        %error,
                        "refusing to repair a binding manifest that does not resolve inside the workspace root"
                    );
                    continue;
                }
            };
        let Ok(existing) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let core_declared_features = crate::codegen::cfg::core_crate_declared_features(config);
        match crate::codegen::cfg::merge_missing_cfg_features(
            &existing,
            &projected,
            if preserve_wrapper_defaults {
                &config.wrapper_default_features
            } else {
                &[]
            },
            &core_dep_key,
            &core_declared_features,
            &excluded_default_features,
        ) {
            Ok(Some(patched)) => match std::fs::write(&manifest_path, &patched) {
                Ok(()) => {
                    tracing::info!(
                        language = %language,
                        manifest = %manifest_path.display(),
                        "added missing cfg-forwarded feature(s) to this binding crate's [features] table"
                    );
                    repaired.push(manifest_path);
                }
                Err(error) => tracing::warn!(
                    language = %language,
                    manifest = %manifest_path.display(),
                    %error,
                    "failed to write repaired [features] table"
                ),
            },
            Ok(None) => {}
            Err(error) => tracing::warn!(
                language = %language,
                manifest = %manifest_path.display(),
                %error,
                "failed to repair [features] table"
            ),
        }
    }
    repaired
}
