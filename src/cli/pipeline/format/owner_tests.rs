//! Tests for [`super::formatting_owner`] and [`super::PolyCoverage`] -- the alef#478 seam.
//!
//! Every case here is a shape that produced a false `alef verify` drift finding in a real
//! consumer before this module existed. Coverage is injected rather than probed, so each branch
//! is provable without depending on whether the host running the suite has `poly`, and so the
//! "poly does not reach this path" branch can be stated as a fact rather than arranged by
//! finding a genuinely excluded file. ~keep

use super::*;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};

fn config_with(extra: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
{extra}
"#
    ))
    .expect("valid config");
    cfg.resolve().expect("resolvable").remove(0)
}

fn plain_config() -> ResolvedCrateConfig {
    config_with("")
}

const E2E_WITH_OVERRIDES: &str = r#"
[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "run"

[crates.e2e.format]
dart = "cd {dir} && dart format ."

[crates.e2e.registry]
output = "test_apps"
"#;

/// THE alef#478 FIX, stated as directly as it can be: a generated path poly's own pass does not
/// reach is owned by nobody, so the render IS the final bytes and the reader may compare them.
///
/// Before this, the drift check staged a temp sibling of such a file and ran poly over it --
/// and because poly's excludes are keyed on the file's NAME, the sibling escaped every one of
/// them. `crates/<crate>-ffi/Cargo.toml` (excluded by a consumer's `**/Cargo.toml`) came back
/// taplo-reformatted and was reported as drifted on every run, forever.
#[test]
fn a_path_poly_never_reaches_is_owned_by_nobody() {
    let base = std::path::Path::new("/repo");
    let path = base.join("packages/python/sample.py");
    let coverage = PolyCoverage::covering([]);

    assert_eq!(
        formatting_owner(&path, &plain_config(), base, &coverage),
        FormattingOwner::None,
        "poly does not reach this path, so nothing alef runs reshapes it"
    );
}

#[test]
fn a_path_poly_does_reach_is_owned_by_poly() {
    let base = std::path::Path::new("/repo");
    let path = base.join("packages/python/sample.py");
    let coverage = PolyCoverage::covering([path.clone()]);

    assert_eq!(
        formatting_owner(&path, &plain_config(), base, &coverage),
        FormattingOwner::Poly
    );
}

#[test]
fn zig_package_source_is_owned_by_zig_fmt_not_poly() {
    let base = std::path::Path::new("/repo");
    let config = plain_config();
    let coverage = PolyCoverage::covering([]);

    for relative in ["packages/zig/src/sample.zig", "packages/zig/build.zig"] {
        assert_eq!(
            formatting_owner(&base.join(relative), &config, base, &coverage),
            FormattingOwner::Residual("zig fmt"),
            "{relative} is persisted through the native Zig formatter"
        );
    }
}

#[test]
fn configured_zig_source_and_canonical_build_are_both_owned_by_zig_fmt() {
    let base = std::path::Path::new("/repo");
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[crates.output]
zig = "sdk/zig-package/src"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().expect("resolvable config").remove(0);
    let coverage = PolyCoverage::covering([]);

    for relative in ["sdk/zig-package/src/sample.zig", "packages/zig/build.zig"] {
        assert_eq!(
            formatting_owner(&base.join(relative), &config, base, &coverage),
            FormattingOwner::Residual("zig fmt"),
            "{relative} must share the writer's two-root Zig formatter ownership"
        );
    }
}

/// An unavailable probe must never be read as "poly covers nothing". Claiming
/// [`FormattingOwner::None`] there would assert "the render is the final bytes" on no evidence,
/// which is the silent-pass direction of the same defect. ~keep
#[test]
fn an_unavailable_probe_stays_conservative_rather_than_claiming_nothing_is_formatted() {
    let base = std::path::Path::new("/repo");
    let path = base.join("packages/python/sample.py");

    assert_eq!(
        formatting_owner(&path, &plain_config(), base, &PolyCoverage::unavailable()),
        FormattingOwner::Poly,
        "an unknown answer must route to the tier that still examines something"
    );
}

/// Elixir is the sharpest alef#478 case: [`POLY_EXCLUDE_GLOBS`] keeps poly off `.ex`/`.exs` on
/// BOTH sides, so the old check's temp copy was a no-op and it ended up demanding that disk
/// equal the raw emitter output -- while the writer had run `mix format` over it.
#[test]
fn elixir_source_is_owned_by_mix_format_not_poly() {
    let base = std::path::Path::new("/repo");
    let config = plain_config();
    let coverage = PolyCoverage::covering([]);

    for relative in ["packages/elixir/lib/sample.ex", "packages/elixir/mix.exs"] {
        assert_eq!(
            formatting_owner(&base.join(relative), &config, base, &coverage),
            FormattingOwner::Residual("mix format"),
            "{relative} is formatted solely by mix format"
        );
    }
}

#[test]
fn a_cargo_manifest_is_owned_by_cargo_sort() {
    let base = std::path::Path::new("/repo");

    assert_eq!(
        formatting_owner(
            &base.join("crates/sample-ffi/Cargo.toml"),
            &plain_config(),
            base,
            &PolyCoverage::covering([])
        ),
        FormattingOwner::Residual("cargo sort -n -w"),
    );
}

/// A `[crates.e2e.format]` override REPLACES the poly pass for its language
/// (`e2e::format::format_language` returns early), so it outranks poly coverage even when poly
/// would also walk the file.
#[test]
fn an_e2e_format_override_outranks_poly_coverage() {
    let base = std::path::Path::new("/repo");
    let path = base.join("e2e/dart/test/smoke_test.dart");
    let config = config_with(E2E_WITH_OVERRIDES);

    assert_eq!(
        formatting_owner(&path, &config, base, &PolyCoverage::covering([path.clone()])),
        FormattingOwner::E2eOverride("cd {dir} && dart format .".to_string()),
    );
}

/// The registry-mode tree (`[crates.e2e.registry] output`, conventionally `test_apps/`) is
/// generated from the same config and formatted by the same `format_language`, so it must be
/// classified identically. Reading only `effective_output()` would answer for whichever mode
/// this process happens to be in and silently drop the other tree. ~keep
#[test]
fn the_registry_output_tree_is_classified_like_the_plain_one() {
    let base = std::path::Path::new("/repo");
    let config = config_with(E2E_WITH_OVERRIDES);

    assert_eq!(
        formatting_owner(
            &base.join("test_apps/dart/test/smoke_test.dart"),
            &config,
            base,
            &PolyCoverage::covering([])
        ),
        FormattingOwner::E2eOverride("cd {dir} && dart format .".to_string()),
    );
}

#[test]
fn a_language_without_an_override_is_not_claimed_by_one() {
    let base = std::path::Path::new("/repo");
    let path = base.join("e2e/java/src/test/java/SmokeTest.java");
    let config = config_with(E2E_WITH_OVERRIDES);

    assert_eq!(
        formatting_owner(&path, &config, base, &PolyCoverage::covering([path.clone()])),
        FormattingOwner::Poly,
        "only the languages named in [crates.e2e.format] are override-owned"
    );
}

/// THE REGRESSION FOR A BUG THIS MODULE'S OWN DEVELOPMENT HIT: poly echoes back the root
/// spelling it was given, so a root of `.` yields `./packages/x/y.md`. A string-keyed lookup
/// against alef's own path values reported **0 of 197** candidate paths as poly-covered when the
/// true answer was 32 -- and a zero looks exactly like a real finding. ~keep
#[test]
fn reported_paths_are_normalized_before_they_enter_the_coverage_set() {
    let base = std::path::Path::new("/repo");
    let payload = r#"{
        "results": [
            {"path": "./packages/zig/README.md", "changed": false},
            {"path": "packages/zig/build.zig", "changed": true},
            {"path": "/repo/packages/zig/src/main.zig", "changed": false}
        ]
    }"#;

    let covered = parse_poly_coverage(payload, base).expect("payload parses");

    for relative in [
        "packages/zig/README.md",
        "packages/zig/build.zig",
        "packages/zig/src/main.zig",
    ] {
        assert!(
            covered.contains(&base.join(relative)),
            "{relative} must resolve to the same absolute path alef uses internally; got {covered:?}"
        );
    }
}

/// A `skipped` entry is poly declining the file, not covering it -- poly reports those inside
/// `results` as well as in the top-level `skipped` list.
#[test]
fn a_skipped_entry_is_not_coverage() {
    let base = std::path::Path::new("/repo");
    let payload = r#"{
        "results": [
            {"path": "./a.csproj", "changed": false, "skipped": "no matching engine for this file type"},
            {"path": "./b.py", "changed": false}
        ]
    }"#;

    let covered = parse_poly_coverage(payload, base).expect("payload parses");

    assert!(
        !covered.contains(&base.join("a.csproj")),
        "a declined file is not covered"
    );
    assert!(covered.contains(&base.join("b.py")));
}

/// A malformed payload must yield "unknown", never an empty coverage set: the two render
/// identically to a caller that only asks `covers()`, and the second silently reclassifies every
/// poly-owned path as unformatted. ~keep
#[test]
fn a_malformed_payload_is_unknown_not_empty() {
    assert!(
        parse_poly_coverage("not json at all", std::path::Path::new("/repo")).is_none(),
        "an unreadable payload must not masquerade as a complete answer"
    );
}

/// The exclude globs are produced in exactly one place, so the writer's pass and the reader's
/// coverage probe cannot disagree about which files poly was allowed to touch.
#[test]
fn every_exclude_glob_is_emitted_as_an_exclude_argument_pair() {
    let mut args = Vec::new();
    push_poly_format_excludes(&mut args);

    assert_eq!(args.len(), POLY_EXCLUDE_GLOBS.len() * 2);
    for glob in POLY_EXCLUDE_GLOBS {
        let position = args.iter().position(|arg| arg == glob).expect("glob present");
        assert_eq!(args[position - 1], "--exclude");
    }
}
