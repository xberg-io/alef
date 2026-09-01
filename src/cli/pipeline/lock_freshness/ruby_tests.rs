//! Coverage for [`check_generated_gemfile_lock_freshness`] / [`stale_gemfile_lock_findings`],
//! the RubyGems sibling of the cargo/composer checks. Like the composer fixtures there is no
//! path-dependency indirection: the constraint being compared lives in the one `Gemfile` alef
//! generated, so the fixtures here only need that file and a `Gemfile.lock` beside it.

use super::*;

const GEMFILE_DIR_RELATIVE: &str = "e2e/ruby";
const GEM_DEPENDENCY: &str = "demo_gem";
const GEM_STALE_REQUIREMENT: &str = "1.3.0";
const GEM_FRESH_PIN: &str = "1.2.3";

fn gemfile_dir(root: &Path) -> PathBuf {
    root.join(GEMFILE_DIR_RELATIVE)
}

/// Matches `crate::e2e::codegen::ruby::project::render_gemfile`'s registry-mode shape: a bare
/// exact pin, single-quoted.
fn write_gemfile(root: &Path, requirement: &str) -> PathBuf {
    let dir = gemfile_dir(root);
    std::fs::create_dir_all(&dir).expect("create gemfile dir");
    let manifest = dir.join("Gemfile");
    std::fs::write(
        &manifest,
        format!("source 'https://rubygems.org'\n\ngem '{GEM_DEPENDENCY}', '{requirement}'\ngem 'rspec'\n"),
    )
    .expect("write Gemfile");
    manifest
}

fn write_gemfile_lock(root: &Path, locked_version: &str) {
    std::fs::write(
        gemfile_dir(root).join("Gemfile.lock"),
        format!(
            "GEM\n  remote: https://rubygems.org/\n  specs:\n    {GEM_DEPENDENCY} ({locked_version})\n    \
             rspec (3.12.0)\n      rspec-core (~> 3.12.0)\n\nPLATFORMS\n  ruby\n\nDEPENDENCIES\n  \
             {GEM_DEPENDENCY} (= {locked_version})\n  rspec\n\nBUNDLED WITH\n   2.4.10\n"
        ),
    )
    .expect("write Gemfile.lock");
}

/// The regression: `Gemfile` requires an exact version the committed `Gemfile.lock` pins a
/// different one for -- exactly the shape that fails `bundle install --deployment`. Before this
/// module alef reported nothing and exited 0.
#[test]
fn stale_gemfile_lock_findings_reports_a_requirement_no_locked_version_satisfies() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, GEM_STALE_REQUIREMENT);
    write_gemfile_lock(root, GEM_FRESH_PIN);

    let findings = stale_gemfile_lock_findings(&gemfile_dir(root));

    assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.dependency, GEM_DEPENDENCY);
    assert_eq!(finding.requirement, GEM_STALE_REQUIREMENT);
    assert_eq!(finding.locked_version, GEM_FRESH_PIN);
    assert_eq!(finding.lock, gemfile_dir(root).join("Gemfile.lock"));
    assert_eq!(finding.declared_in, gemfile_dir(root).join("Gemfile"));
}

/// The control that stops "always fail" from satisfying this suite: a lock pinning the exact
/// version the Gemfile requires must produce nothing at all.
#[test]
fn stale_gemfile_lock_findings_accepts_a_lock_that_matches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, GEM_FRESH_PIN);
    write_gemfile_lock(root, GEM_FRESH_PIN);

    let findings = stale_gemfile_lock_findings(&gemfile_dir(root));

    assert!(
        findings.is_empty(),
        "a lock matching the Gemfile must be reported clean: {findings:?}"
    );
}

/// The one-sided rule: a gem absent from the lock's `specs:` block is never reported.
#[test]
fn stale_gemfile_lock_findings_ignores_a_dependency_absent_from_the_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, GEM_STALE_REQUIREMENT);
    std::fs::write(
        gemfile_dir(root).join("Gemfile.lock"),
        "GEM\n  remote: https://rubygems.org/\n  specs:\n    other_gem (2.0.0)\n\nPLATFORMS\n  ruby\n\n\
         DEPENDENCIES\n  other_gem\n\nBUNDLED WITH\n   2.4.10\n",
    )
    .expect("write Gemfile.lock");

    let findings = stale_gemfile_lock_findings(&gemfile_dir(root));

    assert!(
        findings.is_empty(),
        "a gem missing from the lock is not evidence of staleness: {findings:?}"
    );
}

/// A `path:` dependency resolves locally; it is never a RubyGems version pin the lock's own
/// `specs:` block would need to satisfy, matching the cargo check's git/path exclusion.
#[test]
fn stale_gemfile_lock_findings_ignores_a_path_dependency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let dir = gemfile_dir(root);
    std::fs::create_dir_all(&dir).expect("create gemfile dir");
    std::fs::write(
        dir.join("Gemfile"),
        format!("source 'https://rubygems.org'\n\ngem '{GEM_DEPENDENCY}', path: '../../packages/ruby'\n"),
    )
    .expect("write Gemfile");
    write_gemfile_lock(root, GEM_FRESH_PIN);

    let findings = stale_gemfile_lock_findings(&dir);

    assert!(
        findings.is_empty(),
        "a path dependency carries no RubyGems version pin to compare: {findings:?}"
    );
}

/// RubyGems' pessimistic operator: `~> 1.2` accepts anything up to (but not including) `2.0.0`
/// -- a lock at `1.9.0` must be reported clean, proving the two-component bump-the-major
/// semantics `ruby_pessimistic_matches` implements.
#[test]
fn stale_gemfile_lock_findings_accepts_a_pessimistic_two_component_range() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, "~> 1.2");
    write_gemfile_lock(root, "1.9.0");

    let findings = stale_gemfile_lock_findings(&gemfile_dir(root));

    assert!(
        findings.is_empty(),
        "~> 1.2 must accept 1.9.0 (only the major is bumped): {findings:?}"
    );
}

/// The over-correction guard: `~> 1.2` must still reject a version that crossed the major
/// boundary the pessimistic operator is supposed to block.
#[test]
fn stale_gemfile_lock_findings_rejects_a_pessimistic_range_violation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, "~> 1.2");
    write_gemfile_lock(root, "2.0.0");

    let findings = stale_gemfile_lock_findings(&gemfile_dir(root));

    assert_eq!(
        findings.len(),
        1,
        "~> 1.2 must reject 2.0.0, which crossed the major-version boundary: {findings:?}"
    );
}

/// Alef never authors a lockfile. A generated directory without one is a consumer choice, not a
/// defect, and must not fail the run.
#[test]
fn stale_gemfile_lock_findings_skips_a_directory_with_no_committed_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, GEM_STALE_REQUIREMENT);

    let findings = stale_gemfile_lock_findings(&gemfile_dir(root));

    assert!(
        findings.is_empty(),
        "a directory with no lock has nothing to check: {findings:?}"
    );
}

/// The run-level entry point: it must select `Gemfile` out of the generated path set, and the
/// error it returns must name the dependency, both versions, the lock, and the remedy.
#[test]
fn check_generated_gemfile_lock_freshness_names_the_dependency_and_the_remedy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_gemfile(root, GEM_STALE_REQUIREMENT);
    write_gemfile_lock(root, GEM_FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    let error = check_generated_gemfile_lock_freshness(&generated, root).expect("a stale lock must fail the run");
    let message = format!("{error:#}");

    assert!(
        message.contains(GEM_DEPENDENCY),
        "message must name the dependency: {message}"
    );
    assert!(
        message.contains(GEM_STALE_REQUIREMENT),
        "message must name the Gemfile requirement: {message}"
    );
    assert!(
        message.contains(GEM_FRESH_PIN),
        "message must name the locked version: {message}"
    );
    assert!(
        message.contains("bundle update"),
        "message must name the remedy: {message}"
    );
    assert!(
        message.contains(&gemfile_dir(root).join("Gemfile.lock").display().to_string()),
        "message must name the lock: {message}"
    );
}

/// Control for the entry point, matching the pattern above: a lock that resolves must return
/// `None` so the run keeps its zero exit.
#[test]
fn check_generated_gemfile_lock_freshness_passes_a_resolvable_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_gemfile(root, GEM_FRESH_PIN);
    write_gemfile_lock(root, GEM_FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    assert!(
        check_generated_gemfile_lock_freshness(&generated, root).is_none(),
        "a resolvable lock must not fail the run"
    );
}

/// A generated path set containing no `Gemfile` at all must not walk anything.
#[test]
fn check_generated_gemfile_lock_freshness_ignores_non_manifest_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gemfile(root, GEM_STALE_REQUIREMENT);
    write_gemfile_lock(root, GEM_FRESH_PIN);
    let generated: HashSet<PathBuf> = [gemfile_dir(root).join("spec/basic_spec.rb")].into_iter().collect();

    assert!(check_generated_gemfile_lock_freshness(&generated, root).is_none());
}

/// Coverage for [`check_generated_gemfile_lock_freshness_tolerating_pending_publish`]'s
/// exemption -- the Ruby sibling of the cargo/node/uv/php `pending_publish` modules.
mod pending_publish {
    use super::*;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::e2e::{E2eConfig, PackageRef, RegistryConfig};

    /// A crate whose `[crates.e2e.registry.packages.ruby]` explicitly names `pkg_name` at
    /// `pkg_version` -- the only shape [`registry_self_dependency`] ever vouches for.
    fn resolved_cfg_with_ruby_registry_package(pkg_name: &str, pkg_version: &str) -> ResolvedCrateConfig {
        let e2e = E2eConfig {
            registry: RegistryConfig {
                packages: [(
                    "ruby".to_string(),
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
        let manifest = write_gemfile(root, GEM_STALE_REQUIREMENT);
        write_gemfile_lock(root, GEM_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_gemfile_lock_freshness(&generated, root).is_some(),
            "control: the plain check has no pending-publish exemption and must still fail here"
        );
    }

    #[test]
    fn tolerating_variant_warns_instead_of_failing_when_the_requirement_matches_the_configured_registry_package() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_gemfile(root, GEM_STALE_REQUIREMENT);
        write_gemfile_lock(root, GEM_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_ruby_registry_package(GEM_DEPENDENCY, GEM_STALE_REQUIREMENT);

        let result =
            check_generated_gemfile_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));
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
        let manifest = write_gemfile(root, GEM_STALE_REQUIREMENT);
        write_gemfile_lock(root, GEM_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_gemfile_lock_freshness_tolerating_pending_publish(&generated, root, None).is_some(),
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
        let manifest = write_gemfile(root, GEM_STALE_REQUIREMENT);
        write_gemfile_lock(root, GEM_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_ruby_registry_package("unrelated_gem", "9.9.9");

        assert!(
            check_generated_gemfile_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg))
                .is_some(),
            "a third-party lock drift unrelated to this crate's own registry self-dependency \
             must still fail the run"
        );
    }
}
