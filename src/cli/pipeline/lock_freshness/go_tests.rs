//! Coverage for [`check_generated_go_sum_freshness`] / [`stale_go_sum_findings`], the Go sibling
//! of the checks above. Unlike every other ecosystem in this module family, Go has no
//! version-range concept: `go.mod` pins one exact version and `go.sum` is a flat checksum ledger,
//! so the fixtures here check ledger PRESENCE for the exact pin, not range satisfaction.

use super::*;

const GO_DIR_RELATIVE: &str = "e2e/go";
const GO_MODULE: &str = "example.com/demo/sample-pkg";
const GO_STALE_VERSION: &str = "v1.3.0";
const GO_FRESH_PIN: &str = "v1.2.3";

fn go_dir(root: &Path) -> PathBuf {
    root.join(GO_DIR_RELATIVE)
}

/// Matches `crate::e2e::codegen::go::project::render_go_mod`'s registry-mode shape: a `require
/// (...)` block naming the module under test at an exact version, plus a fixed indirect-deps
/// block this reader must not misread as a top-level requirement.
fn write_go_mod(root: &Path, required_version: &str) -> PathBuf {
    let dir = go_dir(root);
    std::fs::create_dir_all(&dir).expect("create go dir");
    let manifest = dir.join("go.mod");
    std::fs::write(
        &manifest,
        format!(
            "module example.com/demo/sample-pkg-e2e\n\ngo 1.26\n\nrequire (\n\t{GO_MODULE} \
             {required_version}\n\tgithub.com/stretchr/testify v1.11.1\n)\n\nrequire (\n\t\
             github.com/davecgh/go-spew v1.1.1 // indirect\n)\n"
        ),
    )
    .expect("write go.mod");
    manifest
}

/// A `go.mod` whose only `require` entry is covered by a `replace` directive -- alef's own
/// local-mode shape.
fn write_go_mod_with_replace(root: &Path, required_version: &str) -> PathBuf {
    let dir = go_dir(root);
    std::fs::create_dir_all(&dir).expect("create go dir");
    let manifest = dir.join("go.mod");
    std::fs::write(
        &manifest,
        format!(
            "module example.com/demo/sample-pkg-e2e/e2e\n\ngo 1.26\n\nrequire {GO_MODULE} \
             {required_version}\n\nreplace {GO_MODULE} => ../../packages/go\n"
        ),
    )
    .expect("write go.mod");
    manifest
}

fn write_go_sum(root: &Path, locked_version: &str) {
    std::fs::write(
        go_dir(root).join("go.sum"),
        format!(
            "{GO_MODULE} {locked_version} h1:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead=\n\
             {GO_MODULE} {locked_version}/go.mod h1:cafebabecafebabecafebabecafebabecafebabecafeb=\n\
             github.com/stretchr/testify v1.11.1 h1:deadbeef00000000000000000000000000000000000=\n\
             github.com/stretchr/testify v1.11.1/go.mod h1:cafebabe00000000000000000000000000000=\n"
        ),
    )
    .expect("write go.sum");
}

/// The regression: `go.mod` requires an exact version the committed `go.sum` ledger has no
/// checksum entry for -- exactly the shape that fails `go build -mod=readonly` with "missing
/// go.sum entry". Before this module alef reported nothing and exited 0 (no Go lock-freshness
/// gate existed at all).
#[test]
fn stale_go_sum_findings_reports_a_require_with_no_matching_ledger_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_go_mod(root, GO_STALE_VERSION);
    write_go_sum(root, GO_FRESH_PIN);

    let findings = stale_go_sum_findings(&go_dir(root));

    assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.dependency, GO_MODULE);
    assert_eq!(finding.requirement, GO_STALE_VERSION);
    assert_eq!(finding.locked_versions, vec![GO_FRESH_PIN.to_string()]);
    assert_eq!(finding.lock, go_dir(root).join("go.sum"));
    assert_eq!(finding.declared_in, go_dir(root).join("go.mod"));
}

/// The control that stops "always fail" from satisfying this suite: a ledger carrying the exact
/// required version must produce nothing at all. Also proves the fixed `testify`
/// indirect-dependency block is read without producing spurious findings of its own.
#[test]
fn stale_go_sum_findings_accepts_a_ledger_that_matches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_go_mod(root, GO_FRESH_PIN);
    write_go_sum(root, GO_FRESH_PIN);

    let findings = stale_go_sum_findings(&go_dir(root));

    assert!(
        findings.is_empty(),
        "a ledger matching go.mod must be reported clean: {findings:?}"
    );
}

/// The one-sided rule, matching every other reader in this module family: a module the ledger
/// has never recorded at all is not reported -- indistinguishable from a require added moments
/// ago, before the first `go mod download`.
#[test]
fn stale_go_sum_findings_ignores_a_module_absent_from_the_ledger() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_go_mod(root, GO_STALE_VERSION);
    std::fs::write(
        go_dir(root).join("go.sum"),
        "github.com/other/module v9.0.0 h1:aaaa=\ngithub.com/other/module v9.0.0/go.mod h1:bbbb=\n",
    )
    .expect("write go.sum");

    let findings = stale_go_sum_findings(&go_dir(root));

    assert!(
        findings.is_empty(),
        "a module the ledger has never recorded is not evidence of staleness: {findings:?}"
    );
}

/// A `replace`-covered module resolves locally and is never expected to carry a `go.sum` entry
/// at all, matching the cargo check's own path/git exclusion.
#[test]
fn stale_go_sum_findings_ignores_a_replaced_module() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_go_mod_with_replace(root, GO_STALE_VERSION);
    // No go.sum entry for GO_MODULE at all -- exactly the local-replace shape, where the
    // ledger legitimately never sees it.
    std::fs::write(
        go_dir(root).join("go.sum"),
        "github.com/stretchr/testify v1.11.1 h1:deadbeef=\ngithub.com/stretchr/testify v1.11.1/go.mod h1:cafe=\n",
    )
    .expect("write go.sum");

    let findings = stale_go_sum_findings(&go_dir(root));

    assert!(
        findings.is_empty(),
        "a replace-covered module must never be checked against the ledger: {findings:?}"
    );
}

/// The over-correction guard alongside the replace test: a replace directive for one module must
/// not blanket-suppress a genuine drift on a DIFFERENT, non-replaced require in the same file.
#[test]
fn stale_go_sum_findings_still_reports_a_non_replaced_modules_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let dir = go_dir(root);
    std::fs::create_dir_all(&dir).expect("create go dir");
    std::fs::write(
        dir.join("go.mod"),
        format!(
            "module example.com/demo/sample-pkg-e2e\n\ngo 1.26\n\nrequire (\n\t{GO_MODULE} \
             {GO_FRESH_PIN}\n\tgithub.com/other/dep {GO_STALE_VERSION}\n)\n\nreplace {GO_MODULE} => \
             ../../packages/go\n"
        ),
    )
    .expect("write go.mod");
    std::fs::write(
        dir.join("go.sum"),
        format!("github.com/other/dep {GO_FRESH_PIN} h1:aaaa=\ngithub.com/other/dep {GO_FRESH_PIN}/go.mod h1:bbbb=\n"),
    )
    .expect("write go.sum");

    let findings = stale_go_sum_findings(&dir);

    assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
    assert_eq!(findings[0].dependency, "github.com/other/dep");
}

/// Alef never authors `go.sum`. A generated directory without one is a consumer choice (or a run
/// before the first `go mod download`), not a defect, and must not fail the run.
#[test]
fn stale_go_sum_findings_skips_a_directory_with_no_committed_ledger() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_go_mod(root, GO_STALE_VERSION);

    let findings = stale_go_sum_findings(&go_dir(root));

    assert!(
        findings.is_empty(),
        "a directory with no go.sum has nothing to check: {findings:?}"
    );
}

/// The run-level entry point: it must select `go.mod` out of the generated path set, and the
/// error it returns must name the module, both versions, the lock, and the remedy.
#[test]
fn check_generated_go_sum_freshness_names_the_dependency_and_the_remedy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_go_mod(root, GO_STALE_VERSION);
    write_go_sum(root, GO_FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    let error = check_generated_go_sum_freshness(&generated, root).expect("a stale ledger must fail the run");
    let message = format!("{error:#}");

    assert!(message.contains(GO_MODULE), "message must name the module: {message}");
    assert!(
        message.contains(GO_STALE_VERSION),
        "message must name the required version: {message}"
    );
    assert!(
        message.contains(GO_FRESH_PIN),
        "message must name the recorded version: {message}"
    );
    assert!(
        message.contains("go mod download"),
        "message must name the remedy: {message}"
    );
    assert!(
        message.contains(&go_dir(root).join("go.sum").display().to_string()),
        "message must name the lock: {message}"
    );
}

/// Control for the entry point, matching the pattern above: a ledger that resolves must return
/// `None` so the run keeps its zero exit.
#[test]
fn check_generated_go_sum_freshness_passes_a_resolvable_ledger() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let manifest = write_go_mod(root, GO_FRESH_PIN);
    write_go_sum(root, GO_FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    assert!(
        check_generated_go_sum_freshness(&generated, root).is_none(),
        "a resolvable ledger must not fail the run"
    );
}

/// A generated path set containing no `go.mod` at all must not walk anything.
#[test]
fn check_generated_go_sum_freshness_ignores_non_manifest_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_go_mod(root, GO_STALE_VERSION);
    write_go_sum(root, GO_FRESH_PIN);
    let generated: HashSet<PathBuf> = [go_dir(root).join("basic_test.go")].into_iter().collect();

    assert!(check_generated_go_sum_freshness(&generated, root).is_none());
}

/// Coverage for [`check_generated_go_sum_freshness_tolerating_pending_publish`]'s exemption --
/// the Go sibling of the cargo/node/uv/php/ruby/dart `pending_publish` modules.
mod pending_publish {
    use super::*;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::e2e::{E2eConfig, PackageRef, RegistryConfig};

    /// A crate whose `[crates.e2e.registry.packages.go]` explicitly names `pkg_name` at
    /// `pkg_version`. One of two shapes [`registry_self_dependency`] vouches for -- see
    /// [`resolved_cfg_with_go_registry_module`] for the `module`-keyed shape real Go configs
    /// actually use (Go has no `name` concept; alef.toml configures Go packages by `module`).
    fn resolved_cfg_with_go_registry_package(pkg_name: &str, pkg_version: &str) -> ResolvedCrateConfig {
        let e2e = E2eConfig {
            registry: RegistryConfig {
                packages: [(
                    "go".to_string(),
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

    /// A crate whose `[crates.e2e.registry.packages.go]` sets only `module` (no `name`) --
    /// the real shape every Go `alef.toml` in the wild actually uses (e.g.
    /// `module = "github.com/example-org/example-crate/packages/go/v3"`). Regression fixture for
    /// the bug where [`registry_self_dependency`] read only `name`, making this exemption
    /// structurally unreachable for Go.
    fn resolved_cfg_with_go_registry_module(module: &str, pkg_version: &str) -> ResolvedCrateConfig {
        let e2e = E2eConfig {
            registry: RegistryConfig {
                packages: [(
                    "go".to_string(),
                    PackageRef {
                        module: Some(module.to_string()),
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
        let manifest = write_go_mod(root, GO_STALE_VERSION);
        write_go_sum(root, GO_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_go_sum_freshness(&generated, root).is_some(),
            "control: the plain check has no pending-publish exemption and must still fail here"
        );
    }

    #[test]
    fn tolerating_variant_warns_instead_of_failing_when_the_requirement_matches_the_configured_registry_package() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_go_mod(root, GO_STALE_VERSION);
        write_go_sum(root, GO_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        // The registry config's version is stored verbatim ("already v-prefixed"), matching
        // `normalize_go_version`'s no-op path for a version that already starts with `v`.
        let resolved_cfg = resolved_cfg_with_go_registry_package(GO_MODULE, GO_STALE_VERSION);

        let result = check_generated_go_sum_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));
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
        let manifest = write_go_mod(root, GO_STALE_VERSION);
        write_go_sum(root, GO_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_go_sum_freshness_tolerating_pending_publish(&generated, root, None).is_some(),
            "no resolved config means no exemption is possible; this must still fail"
        );
    }

    /// The false-negative guard: a genuinely stale THIRD-PARTY module has nothing to do with this
    /// crate's own registry self-dependency and must still fail even when a resolved config is
    /// supplied.
    #[test]
    fn tolerating_variant_still_fails_on_a_disagreement_not_explained_by_pending_publish() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_go_mod(root, GO_STALE_VERSION);
        write_go_sum(root, GO_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_go_registry_package("example.com/unrelated", "v9.9.9");

        assert!(
            check_generated_go_sum_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg))
                .is_some(),
            "a third-party ledger drift unrelated to this crate's own registry self-dependency \
             must still fail the run"
        );
    }

    /// The real downstream regression itself: `[crates.e2e.registry.packages.go]` set by
    /// `module` alone (no `name` -- the real shape, e.g.
    /// `module = "github.com/example-org/example-crate/packages/go/v3"`) must still be recognized
    /// as this crate's own pending self-dependency. Before the `module`-fallback fix,
    /// `registry_self_dependency` read only `name`, so this config resolved to `None` and this
    /// exact finding hard-failed instead of being tolerated.
    #[test]
    fn tolerating_variant_warns_instead_of_failing_when_the_requirement_matches_a_module_configured_self_dependency() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_go_mod(root, GO_STALE_VERSION);
        write_go_sum(root, GO_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_go_registry_module(GO_MODULE, GO_STALE_VERSION);

        let result = check_generated_go_sum_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));
        assert!(
            result.is_none(),
            "a disagreement fully explained by this crate's own `module`-configured registry \
             self-dependency must not fail the run: {result:?}"
        );
    }

    /// The over-correction guard alongside the previous test: a `module`-configured self-dependency
    /// must not blanket-suppress a genuinely stale, UNRELATED module drift at a different version.
    #[test]
    fn tolerating_variant_with_module_configured_self_dependency_still_fails_on_an_unrelated_drift() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_go_mod(root, GO_STALE_VERSION);
        write_go_sum(root, GO_FRESH_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_go_registry_module("example.com/unrelated", "v9.9.9");

        assert!(
            check_generated_go_sum_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg))
                .is_some(),
            "a third-party ledger drift unrelated to this crate's own `module`-configured registry \
             self-dependency must still fail the run"
        );
    }
}
