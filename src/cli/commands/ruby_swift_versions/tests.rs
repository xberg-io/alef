use super::*;
use crate::cli::commands::validate_versions::checks_pass;
use tempfile::TempDir;

fn config_toml(root: &Path, languages: &[&str], repository: Option<&str>) -> ResolvedCrateConfig {
    let manifest = root.join("Cargo.toml").to_string_lossy().replace('\\', "/");
    let languages_toml = languages
        .iter()
        .map(|language| format!("\"{language}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let repository_block = repository
        .map(|url| format!("[crates.package_metadata]\nrepository = \"{url}\"\n"))
        .unwrap_or_default();
    let source = format!(
        "[workspace]\nlanguages = [{languages_toml}]\n\
         [[crates]]\nname = \"demo_lib\"\nsources = [\"src/lib.rs\"]\nversion_from = \"{manifest}\"\n{repository_block}"
    );
    let parsed: crate::core::config::NewAlefConfig = toml::from_str(&source).expect("config parses");
    parsed.resolve().expect("config resolves").remove(0)
}

fn workspace(version: &str) -> TempDir {
    let temp = TempDir::new().expect("tempdir");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        format!("[package]\nname = \"demo_lib\"\nversion = \"{version}\"\nedition = \"2024\"\n"),
    )
    .expect("cargo manifest");
    temp
}

fn write_gemspec(root: &Path, gem_name: &str, version: &str) {
    let dir = root.join("packages/ruby");
    std::fs::create_dir_all(&dir).expect("ruby dir");
    std::fs::write(
        dir.join(format!("{gem_name}.gemspec")),
        format!(
            "Gem::Specification.new do |spec|\n  spec.name = \"{gem_name}\"\n  spec.version = \"{version}\"\nend\n"
        ),
    )
    .expect("gemspec");
}

fn write_root_package_swift(root: &Path, repository: &str, version: &str) {
    std::fs::write(
        root.join("Package.swift"),
        format!(
            "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(\n  \
             targets: [\n    .binaryTarget(\n      name: \"RustBridgeBinary\",\n      \
             url: \"{repository}/releases/download/v{version}/demo_lib-rs.artifactbundle.zip\",\n      \
             checksum: \"deadbeef\"\n    )\n  ]\n)\n"
        ),
    )
    .expect("Package.swift");
}

// ---------------------------------------------------------------------------
// gemspec
// ---------------------------------------------------------------------------

/// THE prove-the-check-fired test: a genuinely desynced `.gemspec` must fail, not vanish into a
/// vacuous pass, and the count examined must stay stable across the desync/resync cycle so a
/// caller can trust the verdict rather than just the printed word "ok".
#[test]
fn desynced_gemspec_fails_and_resynced_gemspec_passes() {
    let temp = workspace("1.0.0");
    write_gemspec(temp.path(), "demo_lib", "1.0.0");
    let config = config_toml(temp.path(), &["ruby"], None);

    let synced = collect(&config, temp.path(), "1.0.0");
    assert_eq!(synced.len(), 1, "exactly one gemspec must be examined: {synced:?}");
    assert!(checks_pass(&synced), "a synced gemspec must pass: {synced:?}");

    write_gemspec(temp.path(), "demo_lib", "9.9.9");
    let desynced = collect(&config, temp.path(), "1.0.0");
    assert_eq!(
        desynced.len(),
        1,
        "desyncing the version string must not change how many gemspecs are examined"
    );
    assert!(!checks_pass(&desynced), "a desynced gemspec must fail: {desynced:?}");
    assert_eq!(desynced[0].found.as_deref(), Some("9.9.9"));

    write_gemspec(temp.path(), "demo_lib", "1.0.0");
    let resynced = collect(&config, temp.path(), "1.0.0");
    assert_eq!(resynced.len(), 1);
    assert!(
        checks_pass(&resynced),
        "resyncing must make the gate pass again: {resynced:?}"
    );
}

/// The specific trap `prove-the-check-fired` calls out: a glob over the configured Ruby
/// directory that matches zero `.gemspec` files must not read the same as "all consistent". An
/// enabled-Ruby crate that ships no gemspec cannot be published to RubyGems at all.
#[test]
fn enabled_ruby_with_no_gemspec_does_not_silently_pass() {
    let temp = workspace("1.0.0");
    std::fs::create_dir_all(temp.path().join("packages/ruby")).expect("ruby dir");
    let config = config_toml(temp.path(), &["ruby"], None);

    let checks = collect(&config, temp.path(), "1.0.0");
    assert_eq!(
        checks.len(),
        1,
        "a missing gemspec must still surface exactly one FAILING check, not zero: {checks:?}"
    );
    assert!(
        !checks[0].matches,
        "a missing gemspec must be reported as a failure: {:?}",
        checks[0]
    );
    assert_eq!(
        checks[0].found, None,
        "no version can be reported when no gemspec exists"
    );
    assert!(
        !checks_pass(&checks),
        "an enabled Ruby crate with no gemspec must fail the gate"
    );
}

/// A prerelease canonical version must compare against the RubyGems-normalized form, exactly
/// like the existing `version.rb` checks in `validate_versions.rs`.
#[test]
fn gemspec_compares_against_the_rubygems_prerelease_form() {
    let temp = workspace("1.0.0-rc.1");
    write_gemspec(temp.path(), "demo_lib", "1.0.0.pre.rc.1");
    let config = config_toml(temp.path(), &["ruby"], None);

    let checks = collect(&config, temp.path(), "1.0.0-rc.1");
    assert_eq!(checks.len(), 1);
    assert!(
        checks[0].matches,
        "RubyGems prerelease form must match: {:?}",
        checks[0]
    );
}

/// Ruby disabled: the check must contribute nothing at all, whether or not a gemspec happens to
/// exist on disk -- absence of the language is not a failure.
#[test]
fn ruby_disabled_contributes_no_checks() {
    let temp = workspace("1.0.0");
    write_gemspec(temp.path(), "demo_lib", "9.9.9");
    let config = config_toml(temp.path(), &["python"], None);

    let checks = collect(&config, temp.path(), "1.0.0");
    assert!(
        checks.is_empty(),
        "a disabled language must contribute nothing regardless of files on disk: {checks:?}"
    );
}

/// A gemspec that exists but declares no `spec.version` line is unmanaged, not broken -- mirrors
/// `NoVersionField` semantics for every other manifest reader in `validate_versions.rs`.
#[test]
fn gemspec_without_a_version_line_is_skipped_not_failed() {
    let temp = workspace("1.0.0");
    let dir = temp.path().join("packages/ruby");
    std::fs::create_dir_all(&dir).expect("ruby dir");
    std::fs::write(dir.join("demo_lib.gemspec"), "Gem::Specification.new do |spec|\nend\n").expect("gemspec");
    let config = config_toml(temp.path(), &["ruby"], None);

    let checks = collect(&config, temp.path(), "1.0.0");
    assert!(
        checks.is_empty(),
        "a versionless gemspec must be skipped, not reported as a failure: {checks:?}"
    );
}

// ---------------------------------------------------------------------------
// Package.swift
// ---------------------------------------------------------------------------

/// The Swift counterpart of the desync/resync proof above.
#[test]
fn desynced_package_swift_release_url_fails_and_resynced_passes() {
    let temp = workspace("2.0.0");
    write_root_package_swift(temp.path(), "https://github.com/example/demo-lib", "2.0.0");
    let config = config_toml(temp.path(), &["swift"], Some("https://github.com/example/demo-lib"));

    let synced = collect(&config, temp.path(), "2.0.0");
    assert_eq!(
        synced.len(),
        1,
        "exactly one Package.swift check must be examined: {synced:?}"
    );
    assert!(checks_pass(&synced));

    write_root_package_swift(temp.path(), "https://github.com/example/demo-lib", "1.0.0");
    let desynced = collect(&config, temp.path(), "2.0.0");
    assert_eq!(desynced.len(), 1);
    assert!(
        !checks_pass(&desynced),
        "a stale release URL version must fail: {desynced:?}"
    );
    assert_eq!(desynced[0].found.as_deref(), Some("1.0.0"));

    write_root_package_swift(temp.path(), "https://github.com/example/demo-lib", "2.0.0");
    let resynced = collect(&config, temp.path(), "2.0.0");
    assert_eq!(resynced.len(), 1);
    assert!(checks_pass(&resynced));
}

/// The Swift equivalent of the gemspec trap: Swift enabled with a repository configured (so
/// alef's own scaffolder would have written a root `Package.swift`) but the file is absent must
/// fail, not silently pass because there was nothing to compare.
#[test]
fn enabled_swift_with_configured_repository_but_no_package_swift_does_not_silently_pass() {
    let temp = workspace("2.0.0");
    let config = config_toml(temp.path(), &["swift"], Some("https://github.com/example/demo-lib"));

    let checks = collect(&config, temp.path(), "2.0.0");
    assert_eq!(
        checks.len(),
        1,
        "a missing Package.swift must still surface exactly one FAILING check: {checks:?}"
    );
    assert!(!checks[0].matches);
    assert_eq!(checks[0].found, None);
    assert!(!checks_pass(&checks));
}

/// Swift enabled but no repository configured: alef itself never scaffolds a root
/// `Package.swift` in that case (see `scaffold_swift`), so this must contribute nothing even
/// though the file is absent -- absence here is correct, not a regression.
#[test]
fn swift_enabled_without_a_configured_repository_contributes_no_checks() {
    let temp = workspace("2.0.0");
    let config = config_toml(temp.path(), &["swift"], None);

    let checks = collect(&config, temp.path(), "2.0.0");
    assert!(
        checks.is_empty(),
        "no repository configured means alef never scaffolds Package.swift, so there is nothing to check: {checks:?}"
    );
}

/// Swift disabled entirely: nothing contributed, even with a stray root `Package.swift` and a
/// configured repository sitting on disk.
#[test]
fn swift_disabled_contributes_no_checks() {
    let temp = workspace("2.0.0");
    write_root_package_swift(temp.path(), "https://github.com/example/demo-lib", "9.9.9");
    let config = config_toml(temp.path(), &["python"], Some("https://github.com/example/demo-lib"));

    let checks = collect(&config, temp.path(), "2.0.0");
    assert!(
        checks.is_empty(),
        "a disabled language must contribute nothing: {checks:?}"
    );
}

/// An unresolved `__ALEF_SWIFT_VERSION__` placeholder (freshly scaffolded, never synced) must be
/// skipped rather than reported as a mismatch against the canonical version.
#[test]
fn package_swift_placeholder_version_is_skipped_not_failed() {
    let temp = workspace("2.0.0");
    std::fs::write(
        temp.path().join("Package.swift"),
        "url: \"https://github.com/example/demo-lib/releases/download/v__ALEF_SWIFT_VERSION__/demo_lib-rs.artifactbundle.zip\",\n",
    )
    .expect("Package.swift");
    let config = config_toml(temp.path(), &["swift"], Some("https://github.com/example/demo-lib"));

    let checks = collect(&config, temp.path(), "2.0.0");
    assert!(
        checks.is_empty(),
        "an unresolved placeholder must not be reported as a version mismatch: {checks:?}"
    );
}

// ---------------------------------------------------------------------------
// blocked_on_publish tolerance
// ---------------------------------------------------------------------------

/// Neither new check ever sets `blocked_on_publish` itself -- there is no registry-resolution
/// blocker for a source-tree literal like a `.gemspec` or `Package.swift`. What matters is that
/// they compose correctly with `checks_pass`'s existing tolerance for a check that IS blocked
/// elsewhere in the same run, so a repo mid-release (some other manifest legitimately blocked on
/// publish) does not have its gemspec/Package.swift checks swallowed by that tolerance, nor does
/// a genuinely blocked check get failed by these checks passing alongside it.
#[test]
fn new_checks_never_set_blocked_on_publish_and_compose_with_a_blocked_check_elsewhere() {
    let temp = workspace("1.0.0");
    write_gemspec(temp.path(), "demo_lib", "1.0.0");
    let config = config_toml(temp.path(), &["ruby"], None);
    let gemspec_checks = collect(&config, temp.path(), "1.0.0");
    assert_eq!(gemspec_checks.len(), 1);
    assert_eq!(gemspec_checks[0].blocked_on_publish, None);

    let blocked = VersionCheck {
        label: "test_apps/rust/Cargo.lock#app".to_string(),
        found: Some("0.1.0".to_string()),
        matches: false,
        blocked_on_publish: Some("demo-crate-rs@1.0.0".to_string()),
    };

    let mut mid_release = gemspec_checks;
    mid_release.push(blocked);
    assert!(
        checks_pass(&mid_release),
        "a passing gemspec beside a publish-blocked lockfile entry must still pass the gate: {mid_release:?}"
    );

    // Now make the gemspec itself genuinely wrong: the blocked check must not launder it.
    write_gemspec(temp.path(), "demo_lib", "9.9.9");
    let mut real_failure = collect(&config, temp.path(), "1.0.0");
    real_failure.push(VersionCheck {
        label: "test_apps/rust/Cargo.lock#app".to_string(),
        found: Some("0.1.0".to_string()),
        matches: false,
        blocked_on_publish: Some("demo-crate-rs@1.0.0".to_string()),
    });
    assert!(
        !checks_pass(&real_failure),
        "a genuinely desynced gemspec must still fail the gate alongside a blocked check: {real_failure:?}"
    );
}

// ---------------------------------------------------------------------------
// end-to-end wiring: both `alef verify` and `alef validate versions` share
// `validate_versions::collect_checks`, so proving it there proves it for both.
// ---------------------------------------------------------------------------

/// The actual regression this task closes: `collect_checks` (shared by `pipeline::verify_versions`
/// and `validate_versions::run`) must include the gemspec and Package.swift checks, not just this
/// module's own `collect`.
#[test]
fn collect_checks_wires_in_gemspec_and_package_swift() {
    let temp = workspace("3.0.0");
    write_gemspec(temp.path(), "demo_lib", "9.9.9");
    write_root_package_swift(temp.path(), "https://github.com/example/demo-lib", "8.8.8");
    let config = config_toml(
        temp.path(),
        &["ruby", "swift"],
        Some("https://github.com/example/demo-lib"),
    );

    let checks = crate::cli::commands::validate_versions::collect_checks(&config, temp.path(), "3.0.0");
    let gemspec = checks
        .iter()
        .find(|check| check.label.ends_with(".gemspec"))
        .expect("gemspec check must be present in the shared enumerator's output");
    assert!(
        !gemspec.matches,
        "the desynced gemspec must be reported as a failure via collect_checks"
    );

    let swift = checks
        .iter()
        .find(|check| check.label == "Package.swift")
        .expect("Package.swift check must be present in the shared enumerator's output");
    assert!(
        !swift.matches,
        "the desynced Package.swift must be reported as a failure via collect_checks"
    );
}
