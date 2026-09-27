//! A regenerated manifest may raise a dependency version, never lower one.
//!
//! Binding-crate manifests are `generated_header: true` and rewritten in full on every run
//! (see [`crate::scaffold::scaffold`]). Every dependency requirement in them comes from a
//! literal baked into the emitter — some routed through [`crate::core::template_versions`]
//! and therefore kept current by Renovate, most not. A literal outside that one file can
//! never be bumped by anything, so it drifts behind the ecosystem indefinitely, and each
//! regeneration writes the stale value back over whatever the consumer had. The observed
//! shape is a consumer on `base64 = "0.23"` receiving `base64 = "0.22"` and hand-reverting
//! it after every regen.
//!
//! Bumping the offending constant does not close this: it is one instance of a class that
//! recurs on the next release the emitter does not track. The floor below closes the class
//! instead — before a rendered manifest is handed back, every dependency requirement it
//! declares is compared against the requirement the manifest already on disk declares for
//! the same crate, and the higher of the two wins. An emitter can then only ever move a
//! version forward.
//!
//! Two consequences worth stating, because both are deliberate:
//!
//! - **Rendered output is now a function of disk state, not of config alone.** That is the
//!   entire point — the consumer's committed manifest is an input. It converges in one
//!   pass: after the first regeneration the on-disk requirement and the rendered one agree,
//!   so the next regeneration is a no-op and `alef verify` reports fresh.
//! - **The floor is silent when it cannot rule.** A requirement with no lower bound (`*`,
//!   `<2`), an unparseable requirement, an entry carrying no `version` key at all
//!   (`foo.workspace = true`, a path-only dep), or a manifest that is not valid TOML all
//!   leave the rendered value exactly as the emitter wrote it. Guessing at an ordering
//!   between requirements that are not ordered would be worse than emitting the literal.
//!
//! ~keep

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use std::path::{Path, PathBuf};

/// Dependency tables a Cargo manifest may declare, at the document root and again beneath
/// each `[target.'cfg(...)']` entry.
const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// Directory the relative paths carried by a [`GeneratedFile`] resolve against.
///
/// Mirrors the resolution `crate::scaffold::workspace_dep_specs` already uses, so the floor
/// reads the same tree the rest of the scaffold layer does.
fn manifest_base_dir(config: &ResolvedCrateConfig) -> Option<PathBuf> {
    config.workspace_root.clone().or_else(|| std::env::current_dir().ok())
}

/// Raise every dependency requirement in each emitted `Cargo.toml` to the requirement the
/// consumer's committed manifest already declares, wherever the committed one is higher.
pub(crate) fn apply_version_floors(files: &mut [GeneratedFile], config: &ResolvedCrateConfig) {
    for file in files.iter_mut() {
        if file.path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            continue;
        }
        let path = file.path.clone();
        file.content = floor_manifest(&file.content, config, &path);
    }
}

/// [`apply_version_floors`] for a single manifest a backend renders outside the scaffold
/// file set (the wasm crate builds its own [`GeneratedFile`]).
pub(crate) fn floor_manifest(rendered: &str, config: &ResolvedCrateConfig, relative_path: &Path) -> String {
    let Some(base_dir) = manifest_base_dir(config) else {
        return rendered.to_string();
    };
    let Ok(existing) = std::fs::read_to_string(base_dir.join(relative_path)) else {
        return rendered.to_string();
    };
    floor_against_exempting(rendered, &existing, &explicitly_pinned_crates(config))
}

/// Crate names this crate's config pins explicitly through
/// `[crates.<lang>.dependency_versions]`.
///
/// ~keep The floor and an explicit override are two answers to the same question, and the
/// floor would win every time: it only ever raises, so a deliberate pin *below* what the
/// consumer has committed is reverted on every run and never converges -- the precise silent
/// discard `[crates.<lang>.dependency_versions]` exists to end. An explicit pin is a stated
/// intent about one dependency; the floor is a guess about all of them, so the pin wins.
///
/// ~keep Unioned across every managed language rather than matched to the manifest's own
/// language: a `Cargo.toml`'s owning language is not recoverable from the `GeneratedFile`
/// path without re-deriving each renderer's directory convention, and the over-reach is
/// bounded -- it can only ever mean "alef emits the literal it was told to" for a dependency
/// the consumer has already pinned by name somewhere in this same crate.
fn explicitly_pinned_crates(config: &ResolvedCrateConfig) -> std::collections::HashSet<String> {
    crate::core::managed_versions::MANAGED_LANGUAGES
        .iter()
        .flat_map(|language| config.dependency_version_overrides(*language).keys())
        .filter_map(|key| key.strip_prefix("cargo:"))
        .map(str::to_string)
        .collect()
}

/// [`floor_against_exempting`] with nothing exempted.
///
/// ~keep Test-only since the exemption set landed: every production caller reaches the floor
/// through [`floor_manifest`], which always has a config to derive explicit pins from. Kept
/// so the floor's own table-driven tests stay about the floor rather than about the exemption.
#[cfg(test)]
pub(crate) fn floor_against(rendered: &str, existing: &str) -> String {
    floor_against_exempting(rendered, existing, &std::collections::HashSet::new())
}

/// Return `rendered` with every dependency requirement raised to the one `existing`
/// declares for the same crate, wherever `existing`'s is strictly higher, leaving every
/// dependency named in `exempt` exactly as rendered.
///
/// `toml_edit` is format-preserving and the document is only re-serialised when something
/// actually moved, so a manifest with nothing to raise is returned byte-for-byte and the
/// emitted comments, blank lines and key order survive untouched.
pub(crate) fn floor_against_exempting(
    rendered: &str,
    existing: &str,
    exempt: &std::collections::HashSet<String>,
) -> String {
    let (Ok(mut generated_doc), Ok(existing_doc)) = (
        rendered.parse::<toml_edit::DocumentMut>(),
        existing.parse::<toml_edit::DocumentMut>(),
    ) else {
        return rendered.to_string();
    };

    let mut raised = 0usize;
    for table in DEPENDENCY_TABLES {
        raised += raise_table(generated_doc.get_mut(table), existing_doc.get(table), exempt);
    }
    raised += raise_target_tables(&mut generated_doc, &existing_doc, exempt);

    if raised == 0 {
        return rendered.to_string();
    }
    generated_doc.to_string()
}

/// Apply the floor to the dependency tables nested under each `[target.'cfg(...)']` entry.
///
/// Kept separate from [`floor_against_exempting`] because the wasm crate declares its `getrandom`
/// trio only under `cfg(target_arch = "wasm32")` — a floor that walked the document root
/// alone would leave exactly those requirements unprotected.
fn raise_target_tables(
    generated_doc: &mut toml_edit::DocumentMut,
    existing_doc: &toml_edit::DocumentMut,
    exempt: &std::collections::HashSet<String>,
) -> usize {
    let Some(target_item) = generated_doc.get_mut("target") else {
        return 0;
    };
    let Some(targets) = target_item.as_table_like_mut() else {
        return 0;
    };
    let target_keys: Vec<String> = targets.iter().map(|(key, _)| key.to_string()).collect();
    let existing_targets = existing_doc.get("target").and_then(|item| item.as_table_like());

    let mut raised = 0usize;
    for target_key in target_keys {
        let existing_target = existing_targets.and_then(|tables| tables.get(&target_key));
        let Some(generated_target) = targets.get_mut(&target_key) else {
            continue;
        };
        let Some(generated_target) = generated_target.as_table_like_mut() else {
            continue;
        };
        for table in DEPENDENCY_TABLES {
            let existing_table = existing_target
                .and_then(|item| item.as_table_like())
                .and_then(|tables| tables.get(table));
            raised += raise_table(generated_target.get_mut(table), existing_table, exempt);
        }
    }
    raised
}

/// Raise every entry of one dependency table, returning how many entries moved.
fn raise_table(
    generated: Option<&mut toml_edit::Item>,
    existing: Option<&toml_edit::Item>,
    exempt: &std::collections::HashSet<String>,
) -> usize {
    let (Some(generated), Some(existing)) = (generated, existing) else {
        return 0;
    };
    let (Some(generated), Some(existing)) = (generated.as_table_like_mut(), existing.as_table_like()) else {
        return 0;
    };

    let names: Vec<String> = generated.iter().map(|(name, _)| name.to_string()).collect();
    let mut raised = 0usize;
    for name in names {
        if exempt.contains(&name) {
            tracing::debug!(
                dependency = %name,
                "leaving an emitted dependency requirement alone: it is pinned explicitly in \
                 [crates.<lang>.dependency_versions]"
            );
            continue;
        }
        let Some(committed) = existing.get(&name).and_then(version_requirement) else {
            continue;
        };
        let Some(entry) = generated.get_mut(&name) else {
            continue;
        };
        let Some(emitted) = version_requirement(entry) else {
            continue;
        };
        let (Some(committed_floor), Some(emitted_floor)) = (requirement_floor(&committed), requirement_floor(&emitted))
        else {
            continue;
        };
        if committed_floor <= emitted_floor {
            continue;
        }
        if set_version_requirement(entry, &committed) {
            tracing::debug!(
                dependency = %name,
                emitted = %emitted,
                committed = %committed,
                "raised an emitted dependency requirement to the one the committed manifest declares"
            );
            raised += 1;
        }
    }
    raised
}

/// The version requirement a dependency entry declares, whether written as a bare string
/// (`base64 = "0.22"`) or as a table (`base64 = { version = "0.22", features = [...] }`).
///
/// `None` for an entry that declares no version at all — `foo.workspace = true`, a
/// path-only or git-only dep — which is exactly the case where there is nothing to order.
fn version_requirement(entry: &toml_edit::Item) -> Option<String> {
    if let Some(requirement) = entry.as_str() {
        return Some(requirement.to_string());
    }
    entry.as_table_like()?.get("version")?.as_str().map(str::to_string)
}

/// Write `requirement` into a dependency entry, preserving the entry's shape and the
/// surrounding whitespace/comment decoration. Reports whether it landed.
fn set_version_requirement(entry: &mut toml_edit::Item, requirement: &str) -> bool {
    if entry.is_str() {
        let Some(value) = entry.as_value_mut() else {
            return false;
        };
        replace_preserving_decor(value, requirement);
        return true;
    }
    let Some(table) = entry.as_table_like_mut() else {
        return false;
    };
    let Some(version) = table.get_mut("version") else {
        return false;
    };
    let Some(version) = version.as_value_mut() else {
        return false;
    };
    replace_preserving_decor(version, requirement);
    true
}

fn replace_preserving_decor(value: &mut toml_edit::Value, requirement: &str) {
    let decor = value.decor().clone();
    *value = toml_edit::Value::from(requirement);
    *value.decor_mut() = decor;
}

/// The lowest concrete version a Cargo requirement admits, or `None` when it admits no
/// lower bound at all.
///
/// Two requirements are only comparable through their lower bounds: `"0.22"` and `"0.23"`
/// are caret requirements over disjoint ranges, and it is the floor that says which one the
/// consumer moved forward to. `*` and `<2` bound nothing from below, so they are reported
/// unorderable rather than silently treated as `0.0.0` — that would let a `<2` already on
/// disk be "raised" past by every emitted literal.
fn requirement_floor(requirement: &str) -> Option<semver::Version> {
    let parsed = semver::VersionReq::parse(requirement).ok()?;
    let mut floor: Option<semver::Version> = None;
    for comparator in &parsed.comparators {
        let bounded_below = matches!(
            comparator.op,
            semver::Op::Exact | semver::Op::Greater | semver::Op::GreaterEq | semver::Op::Caret | semver::Op::Tilde
        );
        if !bounded_below {
            continue;
        }
        let candidate = semver::Version {
            major: comparator.major,
            minor: comparator.minor.unwrap_or(0),
            patch: comparator.patch.unwrap_or(0),
            pre: comparator.pre.clone(),
            build: semver::BuildMetadata::EMPTY,
        };
        if floor.as_ref().is_none_or(|current| candidate > *current) {
            floor = Some(candidate);
        }
    }
    floor
}

#[cfg(test)]
mod tests;
