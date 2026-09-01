//! A version const is only "centralized" if Renovate can actually see it.
//!
//! `src/core/template_versions.rs` is the one file `renovate.json`'s `customManager`
//! scans, so moving a literal there is what keeps a generated manifest's dependency
//! current. But the marker and the const have to line up with a regex, and when they do
//! not the const is silently frozen at whatever it was written as — indistinguishable, from
//! the outside, from a const nobody has needed to bump. That is how `PYO3` sat untracked,
//! and it is the failure the `base64` downgrade surfaced: the regex's `[A-Z_]+` excluded
//! every const name containing a digit.
//!
//! These tests check the apparatus rather than the values: they assert that each marker
//! actually reaches the const beneath it.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// `depName`s the `customManager` deliberately does not reach.
///
/// Empty as of the `widen` range strategy. `phpunit/phpunit` and `guzzlehttp/guzzle` used to
/// live here: both are compound `||` constraints spanning several majors on purpose, and under
/// the default `replace` strategy an auto-bump rewrites such a chain to its LAST clause alone,
/// collapsing the span that is the whole point of them. Keeping the marker unreachable was the
/// cheap defence. `renovate.json` now scopes `rangeStrategy: "widen"` to exactly those depNames,
/// which appends a clause instead of replacing the chain, so the marker is safe to reach and the
/// prose that used to sit between marker and `pub const` has moved above the marker.
///
/// Anything appearing here again means a constant went silently un-bumpable. The entry is a list
/// rather than a fix so a *new* unreachable marker still fails the test below. ~keep
const DELIBERATELY_UNREACHABLE: [&str; 0] = [];

/// Floor on how many markers the manager must reach, well under the real count so routine
/// additions do not churn it, and well over zero so a regex that matched nothing could not
/// pass every set-difference assertion in this file vacuously.
const MINIMUM_TRACKED_DEPENDENCIES: usize = 80;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn custom_manager_pattern() -> String {
    let raw = std::fs::read_to_string(repository_root().join("renovate.json")).expect("read renovate.json");
    let config: serde_json::Value = serde_json::from_str(&raw).expect("renovate.json is valid JSON");
    config["customManagers"][0]["matchStrings"][0]
        .as_str()
        .expect("the customManager declares a matchString")
        .to_string()
}

fn template_versions_source() -> String {
    std::fs::read_to_string(repository_root().join("src/core/template_versions.rs")).expect("read template_versions.rs")
}

/// The `depName`s the `customManager` regex actually captures.
fn tracked_dependency_names(pattern: &str, source: &str) -> BTreeSet<String> {
    let matcher = regex::Regex::new(pattern).expect("the customManager regex compiles");
    matcher
        .captures_iter(source)
        .filter_map(|captures| captures.name("depName").map(|name| name.as_str().to_string()))
        .collect()
}

/// The anti-vacuity control. Every assertion below is a set difference, and an empty
/// tracked set would make the "unreachable" list look complete while nothing was tracked
/// at all. Pin a floor and two concrete members.
#[test]
fn the_custom_manager_regex_matches_the_bulk_of_the_version_table() {
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &template_versions_source());

    assert!(
        tracked.len() > MINIMUM_TRACKED_DEPENDENCIES,
        "the customManager should reach nearly every marked const, reached {}: {tracked:?}",
        tracked.len()
    );
    assert!(tracked.contains("tracing"), "a known-good marker must be tracked");
}

/// The regression. `BASE64` and `PYO3` are the constants whose names carry digits; both
/// were invisible to a `[A-Z_]+` const-name class, so Renovate never proposed a bump and
/// the emitted `base64 = "0.22"` could only ever go stale.
#[test]
fn version_consts_whose_names_contain_digits_are_tracked() {
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &template_versions_source());

    for dependency in ["base64", "pyo3", "pyo3-async-runtimes"] {
        assert!(
            tracked.contains(dependency),
            "`{dependency}` must be reachable by the customManager; a const name with a digit \
             in it is exactly the case the regex used to drop. Tracked: {tracked:?}"
        );
    }
}

/// The general invariant, so a future hoist cannot repeat this quietly: a marker sitting
/// directly above a `pub const` has to reach it.
#[test]
fn every_marker_directly_above_a_const_reaches_that_const() {
    let source = template_versions_source();
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &source);
    let marker = regex::Regex::new(r"^\s*// renovate:.*\bdepName=(\S+)").expect("marker regex compiles");

    let lines: Vec<&str> = source.lines().collect();
    let mut unreachable = Vec::new();
    for pair in lines.windows(2) {
        let Some(captures) = marker.captures(pair[0]) else {
            continue;
        };
        if !pair[1].trim_start().starts_with("pub const") {
            continue;
        }
        let dependency = captures.get(1).expect("depName group").as_str();
        if !tracked.contains(dependency) {
            unreachable.push(format!("{dependency} ({})", pair[1].trim()));
        }
    }

    assert!(
        unreachable.is_empty(),
        "these markers sit directly above a `pub const` but the customManager does not reach them, \
         so the constants are frozen: {unreachable:?}"
    );
}

/// The complement, and the reason the list above is a list rather than a fix: every marker
/// the manager cannot reach must be one that is known and intended. A new one showing up
/// here means a constant went silently un-bumpable.
#[test]
fn the_only_unreachable_markers_are_the_known_compound_constraints() {
    let source = template_versions_source();
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &source);
    // Anchored and applied per line so the module doc's illustrative
    // `// renovate: datasource=... depName=...` is not read as a real marker. ~keep
    let marker = regex::Regex::new(r"^\s*// renovate:.*\bdepName=(\S+)").expect("marker regex compiles");

    let declared: BTreeSet<String> = source
        .lines()
        .filter_map(|line| marker.captures(line))
        .map(|captures| captures[1].to_string())
        .collect();
    let unreachable: BTreeSet<String> = declared.difference(&tracked).cloned().collect();
    let expected: BTreeSet<String> = DELIBERATELY_UNREACHABLE
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    assert_eq!(
        unreachable, expected,
        "the set of markers the customManager cannot reach changed"
    );
}

/// Why a version const carries no `// renovate:` marker at all. Every entry in
/// `KNOWN_UNMARKED_VERSION_CONSTS` below falls into exactly one of these; grouping by
/// category keeps the allowlist skimmable instead of a flat, rotting list of names.
#[derive(Debug, Clone, Copy)]
enum UnmarkedReason {
    /// Not a dependency version at all -- a filename, or one of alef's own internal
    /// release/contract identifiers -- so there is nothing for Renovate to bump.
    NotAVersion,
    /// A toolchain floor, SDK/target level, or language-version constraint that a human
    /// deliberately reviews rather than auto-bumps.
    ToolchainFloor,
    /// The value's own syntax defeats the customManager regex even if a marker were added.
    StructurallyUnmatched,
    /// A compatibility floor where auto-bumping previously broke publishing; held below
    /// latest on purpose.
    DeliberateFloor,
    /// Renovate has no datasource for this package's registry.
    NoDatasource,
    /// A marker was added and then removed because of an open upstream Renovate bug:
    /// custom-regex plus `ruby` versioning computes no range strategy
    /// (renovatebot/renovate#28090). Do not re-add it.
    UpstreamRenovateBug,
    /// One const's value is rendered for two different upstream packages, so a single
    /// marker would drive updates off the wrong one.
    SharedAcrossPackages,
    /// The version is paired with a checksum const that Renovate cannot recompute, so an
    /// automated version bump would leave the digest pointing at the previous artifact and
    /// silently break the integrity check the pair exists to provide. Both halves move together,
    /// by hand, or neither moves.
    CoupledToDigest,
}

/// One version const with no marker, why, and enough specific detail that the next
/// reader does not have to re-derive it from git blame.
struct UnmarkedVersionConst {
    name: &'static str,
    reason: UnmarkedReason,
    detail: &'static str,
}

/// The complete, reasoned set of `pub const NAME: &str` version consts in
/// `template_versions.rs` that deliberately carry no `// renovate:` marker. This is the
/// guard `DELIBERATELY_UNREACHABLE` above does not provide: that list only covers consts
/// that HAVE a marker the customManager cannot reach. A const with no marker at all is
/// invisible to every test above it, so a new one added without either a marker or an
/// entry here would freeze silently -- the exact failure mode this file exists to catch. ~keep
const KNOWN_UNMARKED_VERSION_CONSTS: &[UnmarkedVersionConst] = &[
    UnmarkedVersionConst {
        name: "PIE_VERSION",
        reason: UnmarkedReason::CoupledToDigest,
        detail: "paired with PIE_PHAR_SHA256; an auto-bump without the matching digest would \
                 defeat the checksum f768d438a added",
    },
    UnmarkedVersionConst {
        name: "PIE_PHAR_SHA256",
        reason: UnmarkedReason::NotAVersion,
        detail: "SHA-256 of the pie.phar asset, not a version -- updated by hand with PIE_VERSION",
    },
    UnmarkedVersionConst {
        name: "NODE_ENGINE",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "npm `engines` floor, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "NAPI_AUTO_DTS_FILENAME",
        reason: UnmarkedReason::NotAVersion,
        detail: "a filename (`napi build`'s auto-derived `.d.ts` output name)",
    },
    UnmarkedVersionConst {
        name: "MATURIN_BUILD_REQUIRES",
        reason: UnmarkedReason::StructurallyUnmatched,
        detail: "a PEP 508 requirement string (`maturin>=1.0,<2.0`), not a bare version",
    },
    UnmarkedVersionConst {
        name: "RB_SYS",
        reason: UnmarkedReason::StructurallyUnmatched,
        detail: "value embeds escaped quotes, which defeats the customManager capture group",
    },
    UnmarkedVersionConst {
        name: "SORBET_RUNTIME",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RAKE_COMPILER",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RSPEC_SCAFFOLD",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RSPEC_E2E",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RUBOCOP_SCAFFOLD",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RUBOCOP_E2E",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RUBOCOP_PERFORMANCE",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RUBOCOP_RSPEC_SCAFFOLD",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "RUBOCOP_RSPEC_E2E",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "STEEP",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "FARADAY",
        reason: UnmarkedReason::UpstreamRenovateBug,
        detail: "Ruby gem pessimistic (`~>`) pin; marker removed in acb09bdd8, renovatebot/renovate#28090",
    },
    UnmarkedVersionConst {
        name: "MAVEN_CORE",
        reason: UnmarkedReason::DeliberateFloor,
        detail: "auto-bumping this floor to the newest maven-core release broke publishing in 0.48.2",
    },
    UnmarkedVersionConst {
        name: "GLEAM_HTTPC_VERSION_RANGE",
        reason: UnmarkedReason::SharedAcrossPackages,
        detail: "one range rendered for both `gleam_httpc` and `gleam_http`; a marker would track only one",
    },
    UnmarkedVersionConst {
        name: "DART_SDK_CONSTRAINT",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "Dart SDK constraint, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "JAVA_JVM_TARGET",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "JVM bytecode target for the Java backend, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "KOTLIN_JVM_TARGET",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "JVM bytecode target for the Kotlin/JVM backend, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "JVM_TARGET",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "deprecated alias for JAVA_JVM_TARGET/KOTLIN_JVM_TARGET, same toolchain-floor reasoning",
    },
    UnmarkedVersionConst {
        name: "SWIFT_MIN_MACOS",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "Swift minimum macOS target, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "SWIFT_MIN_IOS",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "Swift minimum iOS target, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "ANDROID_COMPILE_SDK",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "Android compileSdk level, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "ANDROID_MIN_SDK",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "Android minSdk level, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "ANDROID_JVM_TARGET",
        reason: UnmarkedReason::ToolchainFloor,
        detail: "Android JVM target level, human-reviewed rather than auto-bumped",
    },
    UnmarkedVersionConst {
        name: "REXTENDR",
        reason: UnmarkedReason::NoDatasource,
        detail: "Renovate has no `cran` datasource",
    },
    UnmarkedVersionConst {
        name: "ALEF_REV",
        reason: UnmarkedReason::NotAVersion,
        detail: "alef's own release tag; set only by `task set-version`, never by Renovate",
    },
    UnmarkedVersionConst {
        name: "CODEGEN_FORMAT_VERSION",
        reason: UnmarkedReason::NotAVersion,
        detail: "alef's own internal codegen-format contract version, not a third-party dependency",
    },
    UnmarkedVersionConst {
        name: "HANDLE_ABI_VERSION",
        reason: UnmarkedReason::NotAVersion,
        detail: "alef's own internal handle-ABI contract version, not a third-party dependency",
    },
];

/// Floor on how many `pub const NAME: &str` declarations the scan below must find, well
/// under the real count (140 as of this writing) so routine additions do not churn it, and
/// well over zero so a broken declaration regex -- one that matched nothing -- could not
/// make every assertion in the test below pass vacuously by comparing two empty sets.
const MINIMUM_VERSION_CONST_DECLARATIONS: usize = 130;

/// Matches a real `pub const NAME: &str = ...;` declaration, anchored so only leading
/// whitespace may precede `pub`. That anchor is what keeps this from matching this very
/// module's own top-of-file doc comment, which illustrates the pattern with a literal
/// `` pub const NAME: &str = "..." `` inside a `//!` line -- the same illustrative-marker
/// trap `every_marker_directly_above_a_const_reaches_that_const` above is anchored to avoid,
/// applied here to the const side instead of the marker side. ~keep
fn version_const_declaration_pattern() -> regex::Regex {
    regex::Regex::new(r"^\s*pub const ([A-Za-z0-9_]+): &str").expect("const-declaration regex compiles")
}

/// Every `pub const NAME: &str` declaration in the file, paired with whether a
/// `// renovate:` marker sits on the line directly above it.
fn version_const_marker_status(source: &str) -> Vec<(String, bool)> {
    let const_pattern = version_const_declaration_pattern();
    let marker = regex::Regex::new(r"^\s*// renovate:.*\bdepName=(\S+)").expect("marker regex compiles");
    let lines: Vec<&str> = source.lines().collect();

    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let captures = const_pattern.captures(line)?;
            let name = captures[1].to_string();
            let has_marker = index > 0 && marker.is_match(lines[index - 1]);
            Some((name, has_marker))
        })
        .collect()
}

/// The gap `DELIBERATELY_UNREACHABLE` does not cover: that list only pins markers the
/// customManager cannot reach, which presupposes a marker exists. A const with NO marker
/// at all -- the common case, 30 of ~140 today -- is invisible to every test above this
/// one. Without this test, a new unmarked const is indistinguishable from an intentional
/// exception: it just freezes at whatever it was written as. This test fails in both
/// directions: a new unmarked const not on the allowlist, and an allowlisted const that
/// has since grown a real marker (a stale entry here would hide the next real regression).
#[test]
fn every_unmarked_version_const_is_a_known_and_reasoned_exception() {
    let source = template_versions_source();
    let declarations = version_const_marker_status(&source);

    assert!(
        declarations.len() > MINIMUM_VERSION_CONST_DECLARATIONS,
        "the `pub const NAME: &str` scan should reach nearly every version const in the file, \
         reached {}; see MINIMUM_VERSION_CONST_DECLARATIONS's doc comment for why this floor exists",
        declarations.len()
    );

    let expected_unmarked: BTreeSet<String> = KNOWN_UNMARKED_VERSION_CONSTS
        .iter()
        .map(|entry| entry.name.to_string())
        .collect();
    assert_eq!(
        expected_unmarked.len(),
        KNOWN_UNMARKED_VERSION_CONSTS.len(),
        "KNOWN_UNMARKED_VERSION_CONSTS has a duplicate `name` entry"
    );

    let actual_unmarked: BTreeSet<String> = declarations
        .into_iter()
        .filter(|(_, has_marker)| !has_marker)
        .map(|(name, _)| name)
        .collect();

    let newly_unmarked: Vec<&String> = actual_unmarked.difference(&expected_unmarked).collect();
    let now_marked: Vec<String> = expected_unmarked
        .difference(&actual_unmarked)
        .map(|name| {
            let entry = KNOWN_UNMARKED_VERSION_CONSTS
                .iter()
                .find(|entry| entry.name == name.as_str())
                .expect("name came from KNOWN_UNMARKED_VERSION_CONSTS");
            format!(
                "{} (was allowlisted as {:?}: {})",
                entry.name, entry.reason, entry.detail
            )
        })
        .collect();

    assert!(
        newly_unmarked.is_empty() && now_marked.is_empty(),
        "the set of unmarked version consts in template_versions.rs changed.\n\
         Newly unmarked (no marker, and not in KNOWN_UNMARKED_VERSION_CONSTS): {newly_unmarked:?} \
         -- add a `// renovate: datasource=... depName=...` marker directly above the const, or \
         add an entry to KNOWN_UNMARKED_VERSION_CONSTS in this file explaining why it must stay \
         unmarked.\n\
         No longer unmarked (listed in KNOWN_UNMARKED_VERSION_CONSTS but a marker now sits above \
         them): {now_marked:?} -- remove the stale entry; leaving it there would hide the next \
         real regression."
    );
}
