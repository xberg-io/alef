//! Coverage for [`check_generated_composer_lock_freshness`] / [`stale_composer_lock_findings`],
//! the Composer/PHP sibling of the cargo/node/uv checks. Like the node/uv fixtures there is no
//! path-dependency indirection: the constraints being compared live in the one `composer.json`
//! alef generated, so the fixtures here only need that file and a `composer.lock` beside it.

use super::*;

const COMPOSER_DIR_RELATIVE: &str = "e2e/php";
const COMPOSER_DEPENDENCY: &str = "demo-vendor/sample-pkg";
const COMPOSER_STALE_REQUIREMENT: &str = "^1.3.0";
const COMPOSER_FRESH_PIN: &str = "1.2.3";

fn composer_dir(root: &Path) -> PathBuf {
    root.join(COMPOSER_DIR_RELATIVE)
}

fn write_composer_json(root: &Path, requirement: &str) -> PathBuf {
    let dir = composer_dir(root);
    std::fs::create_dir_all(&dir).expect("create composer dir");
    let manifest = dir.join("composer.json");
    std::fs::write(
        &manifest,
        format!(
            "{{\n  \"name\": \"demo-vendor/e2e-php\",\n  \"require-dev\": {{\n    \
             \"{COMPOSER_DEPENDENCY}\": \"{requirement}\"\n  }}\n}}\n"
        ),
    )
    .expect("write composer.json");
    manifest
}

fn write_composer_lock(root: &Path, locked_version: &str) {
    std::fs::write(
        composer_dir(root).join("composer.lock"),
        format!(
            "{{\n  \"packages\": [],\n  \"packages-dev\": [\n    {{\n      \"name\": \
             \"{COMPOSER_DEPENDENCY}\",\n      \"version\": \"v{locked_version}\"\n    }}\n  ]\n}}\n"
        ),
    )
    .expect("write composer.lock");
}

/// The regression: `composer.json` requires a caret range the committed `composer.lock` pins no
/// version satisfying -- exactly the shape that fails `composer install` (and
/// `composer validate --strict` in CI). Before this module alef reported nothing and exited 0.
#[test]
fn stale_composer_lock_findings_reports_a_requirement_no_locked_version_satisfies() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
    write_composer_lock(root, COMPOSER_FRESH_PIN);

    let findings = stale_composer_lock_findings(&composer_dir(root));

    assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.dependency, COMPOSER_DEPENDENCY);
    assert_eq!(finding.bucket, "require-dev");
    assert_eq!(finding.requirement, COMPOSER_STALE_REQUIREMENT);
    assert_eq!(finding.locked_versions, vec![COMPOSER_FRESH_PIN.to_string()]);
    assert_eq!(finding.lock, composer_dir(root).join("composer.lock"));
    assert_eq!(finding.declared_in, composer_dir(root).join("composer.json"));
}

/// The control that stops "always fail" from satisfying this suite: a lock pinning a version the
/// caret range does accept must produce nothing at all.
#[test]
fn stale_composer_lock_findings_accepts_a_lock_that_satisfies_the_requirement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
    write_composer_lock(root, "1.3.5");

    let findings = stale_composer_lock_findings(&composer_dir(root));

    assert!(
        findings.is_empty(),
        "a lock that resolves must be reported clean: {findings:?}"
    );
}

/// The one-sided rule: a requirement whose package is not in the lock at all is never reported
/// -- this also covers the `php`/`ext-*` platform pseudo-packages `composer.lock` never lists.
#[test]
fn stale_composer_lock_findings_ignores_a_dependency_absent_from_the_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
    std::fs::write(
        composer_dir(root).join("composer.lock"),
        "{\n  \"packages\": [],\n  \"packages-dev\": [\n    { \"name\": \"other/pkg\", \"version\": \"2.0.0\" }\n  ]\n}\n",
    )
    .expect("write composer.lock");

    let findings = stale_composer_lock_findings(&composer_dir(root));

    assert!(
        findings.is_empty(),
        "a package missing from the lock is not evidence of staleness: {findings:?}"
    );
}

/// Composer's tilde disagrees with Cargo's for the two-component form: `~1.2` means
/// `>=1.2.0,<2.0.0` in Composer (bumps the FIRST component), not `<1.3.0` the way Cargo's own
/// tilde would. A lock at `1.9.0` must satisfy `~1.2`, proving `composer_constraint_matches`
/// implements Composer's own semantics rather than delegating to `semver::VersionReq`'s tilde.
#[test]
fn stale_composer_lock_findings_uses_composers_own_two_component_tilde_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, "~1.2");
    write_composer_lock(root, "1.9.0");

    let findings = stale_composer_lock_findings(&composer_dir(root));

    assert!(
        findings.is_empty(),
        "Composer's ~1.2 must accept 1.9.0 (only the major is bumped): {findings:?}"
    );
}

/// A multi-clause constraint (`||`, comma, or a space-separated AND range) is not one this reader
/// confidently judges and must be skipped rather than risk a false positive.
#[test]
fn stale_composer_lock_findings_ignores_an_unsupported_multi_clause_constraint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, ">=1.0 <2.0");
    write_composer_lock(root, "5.0.0");

    let findings = stale_composer_lock_findings(&composer_dir(root));

    assert!(
        findings.is_empty(),
        "a compound range this reader cannot confidently judge must not be reported: {findings:?}"
    );
}

/// Alef never authors a lockfile. A generated directory without one is a consumer choice, not a
/// defect, and must not fail the run.
#[test]
fn stale_composer_lock_findings_skips_a_directory_with_no_committed_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, COMPOSER_STALE_REQUIREMENT);

    let findings = stale_composer_lock_findings(&composer_dir(root));

    assert!(
        findings.is_empty(),
        "a directory with no lock has nothing to check: {findings:?}"
    );
}

/// The run-level entry point: it must select `composer.json` out of the generated path set, and
/// the error it returns must name the dependency, both specifiers, the lock, and the remedy.
#[test]
fn check_generated_composer_lock_freshness_names_the_dependency_and_the_remedy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
    write_composer_lock(root, COMPOSER_FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    let error = check_generated_composer_lock_freshness(&generated, root).expect("a stale lock must fail the run");
    let message = format!("{error:#}");

    assert!(
        message.contains(COMPOSER_DEPENDENCY),
        "message must name the dependency: {message}"
    );
    assert!(
        message.contains(COMPOSER_STALE_REQUIREMENT),
        "message must name the composer.json requirement: {message}"
    );
    assert!(
        message.contains("composer update"),
        "message must name the remedy: {message}"
    );
    assert!(
        message.contains(&composer_dir(root).join("composer.lock").display().to_string()),
        "message must name the lock: {message}"
    );
}

/// Control for the entry point, matching the pattern above: a lock that resolves must return
/// `None` so the run keeps its zero exit.
#[test]
fn check_generated_composer_lock_freshness_passes_a_resolvable_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
    write_composer_lock(root, "1.3.5");
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    assert!(
        check_generated_composer_lock_freshness(&generated, root).is_none(),
        "a resolvable lock must not fail the run"
    );
}

/// A generated path set containing no `composer.json` at all must not walk anything, even when a
/// registered directory exists elsewhere.
#[test]
fn check_generated_composer_lock_freshness_ignores_non_manifest_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
    write_composer_lock(root, COMPOSER_FRESH_PIN);
    let generated: HashSet<PathBuf> = [composer_dir(root).join("tests/BasicTest.php")].into_iter().collect();

    assert!(check_generated_composer_lock_freshness(&generated, root).is_none());
}

/// Coverage for [`check_generated_composer_lock_freshness_tolerating_pending_publish`]'s
/// exemption -- the PHP sibling of the cargo/node/uv `pending_publish` modules.
mod pending_publish {
    use super::*;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::e2e::{E2eConfig, PackageRef, RegistryConfig};

    /// A crate whose `[crates.e2e.registry.packages.php]` explicitly names `pkg_name` at
    /// `pkg_version` -- the only shape [`registry_self_dependency`] ever vouches for.
    fn resolved_cfg_with_php_registry_package(pkg_name: &str, pkg_version: &str) -> ResolvedCrateConfig {
        let e2e = E2eConfig {
            registry: RegistryConfig {
                packages: [(
                    "php".to_string(),
                    PackageRef {
                        name: Some(pkg_name.to_string()),
                        version: Some(pkg_version.to_string()),
                        ..PackageRef::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..RegistryConfig::default()
            },
            ..E2eConfig::default()
        };
        ResolvedCrateConfig {
            e2e: Some(e2e),
            ..ResolvedCrateConfig::default()
        }
    }

    /// Control proving the exemption does real work: without it, this exact shape must still
    /// fail.
    #[test]
    fn plain_check_still_fails_on_a_pending_publish_disagreement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
        write_composer_lock(root, COMPOSER_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_composer_lock_freshness(&generated, root).is_some(),
            "control: the plain check has no pending-publish exemption and must still fail here"
        );
    }

    #[test]
    fn tolerating_variant_warns_instead_of_failing_when_the_requirement_matches_the_configured_registry_package() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
        write_composer_lock(root, COMPOSER_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_php_registry_package(COMPOSER_DEPENDENCY, COMPOSER_STALE_REQUIREMENT);

        let result =
            check_generated_composer_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));
        assert!(
            result.is_none(),
            "a disagreement fully explained by this crate's own configured registry \
             self-dependency must not fail the run: {result:?}"
        );
    }

    /// Without a resolved config, nothing can be classified as pending -- must behave exactly
    /// like the plain check.
    #[test]
    fn tolerating_variant_without_resolved_cfg_behaves_like_the_plain_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
        write_composer_lock(root, COMPOSER_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_composer_lock_freshness_tolerating_pending_publish(&generated, root, None).is_some(),
            "no resolved config means no exemption is possible; this must still fail"
        );
    }

    /// The false-negative guard: a genuinely stale THIRD-PARTY pin has nothing to do with this
    /// crate's own registry self-dependency and must still fail even when a resolved config is
    /// supplied.
    #[test]
    fn tolerating_variant_still_fails_on_a_disagreement_not_explained_by_pending_publish() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_composer_json(root, COMPOSER_STALE_REQUIREMENT);
        write_composer_lock(root, COMPOSER_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_php_registry_package("demo-vendor/unrelated", "^9.9.9");

        assert!(
            check_generated_composer_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg))
                .is_some(),
            "a third-party lock drift unrelated to this crate's own registry self-dependency \
             must still fail the run"
        );
    }
}
