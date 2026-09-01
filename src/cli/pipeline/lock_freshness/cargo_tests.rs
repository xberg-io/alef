//! Coverage for the generated-manifest / committed-lock disagreement check.
//!
//! Every fixture reproduces the reported shape structurally: an alef-generated Rust e2e crate
//! that is its own workspace root and reaches its real registry requirements through a *path*
//! dependency it does not own. That indirection is the whole point — it is why the pre-existing
//! `relock_lockfiles_beside_changed_manifests` hook (keyed on "did alef rewrite this manifest")
//! cannot see the breakage, and why nothing observed lock freshness at all before this module.
//!
//! No cargo invocation anywhere: the fixtures are plain files and the check is pure.

use super::*;

const E2E_RELATIVE_DIR: &str = "e2e/rust";

/// Package name of the crate under test, and of the lock entry the fixtures move.
const REGISTRY_DEPENDENCY: &str = "sample-json";

/// The lock pins this; the manifests below require one minor above it in the stale fixtures.
const STALE_PIN: &str = "1.25.0";
const FRESH_PIN: &str = "1.26.0";
const REQUIREMENT: &str = "1.26";

/// Root workspace crate that the generated e2e crate depends on by path.
fn write_root_manifest(root: &Path, dependencies: &str) {
    std::fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{dependencies}"),
    )
    .expect("write root Cargo.toml");
}

/// The alef-generated e2e crate: its own workspace root, depending on the crate under test by
/// path exactly as `crate::e2e::codegen::rust::cargo_toml` emits it.
fn write_generated_e2e_manifest(root: &Path) -> PathBuf {
    let dir = root.join(E2E_RELATIVE_DIR);
    std::fs::create_dir_all(&dir).expect("create e2e dir");
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[workspace]\n\n[package]\nname = \"sample-core-e2e-rust\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [dependencies]\nsample_core = { package = \"sample-core\", path = \"../..\" }\n",
    )
    .expect("write generated e2e Cargo.toml");
    manifest
}

/// Same as [`write_generated_e2e_manifest`], but the path dependency edge requests `features`
/// explicitly -- the shape needed to exercise [`activated_optional_dependencies`]'s feature
/// closure.
fn write_generated_e2e_manifest_with_features(root: &Path, features: &[&str]) -> PathBuf {
    let dir = root.join(E2E_RELATIVE_DIR);
    std::fs::create_dir_all(&dir).expect("create e2e dir");
    let manifest = dir.join("Cargo.toml");
    let features_toml = if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|feature| format!("\"{feature}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    };
    std::fs::write(
        &manifest,
        format!(
            "[workspace]\n\n[package]\nname = \"sample-core-e2e-rust\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nsample_core = {{ package = \"sample-core\", path = \"../..\"{features_toml} }}\n"
        ),
    )
    .expect("write generated e2e Cargo.toml");
    manifest
}

/// A committed lock beside the generated manifest, pinning the registry dependency at `pin`.
fn write_lock(root: &Path, pin: &str) {
    std::fs::write(
        root.join(E2E_RELATIVE_DIR).join("Cargo.lock"),
        format!(
            "version = 4\n\n\
             [[package]]\nname = \"sample-core-e2e-rust\"\nversion = \"0.1.0\"\n\n\
             [[package]]\nname = \"sample-core\"\nversion = \"0.1.0\"\n\n\
             [[package]]\nname = \"{REGISTRY_DEPENDENCY}\"\nversion = \"{pin}\"\n"
        ),
    )
    .expect("write Cargo.lock");
}

fn e2e_dir(root: &Path) -> PathBuf {
    root.join(E2E_RELATIVE_DIR)
}

/// The regression: the generated manifest is byte-identical to what alef would emit, nothing
/// alef owns changed, and the lock still cannot satisfy a requirement the path dependency
/// declares. `cargo metadata --locked` fails here; before this module alef exited 0.
#[test]
fn stale_lock_findings_reports_a_requirement_no_locked_version_satisfies() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.dependency, REGISTRY_DEPENDENCY);
    assert_eq!(finding.requirement, REQUIREMENT);
    assert_eq!(finding.locked_versions, vec![STALE_PIN.to_string()]);
    assert_eq!(finding.lock, e2e_dir(root).join("Cargo.lock"));
    assert_eq!(
        finding.declared_in,
        root.join("Cargo.toml"),
        "the requirement is declared in the path dependency, not in the manifest alef generated"
    );
}

/// The control that stops "always fail" from satisfying this suite: the identical fixture with a
/// lock that does satisfy the requirement must produce nothing at all.
#[test]
fn stale_lock_findings_accepts_a_lock_that_satisfies_every_requirement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, FRESH_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a lock that resolves must be reported clean: {findings:?}"
    );
}

/// The one-sided rule: a requirement whose package is not in the lock at all is never reported.
/// Absence is ambiguous (trimmed dev-dependencies, `[patch]`, platform gating) and reporting it
/// would turn healthy trees red; only a contradiction cargo itself would reject is a finding.
#[test]
fn stale_lock_findings_ignores_a_dependency_absent_from_the_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(root, "[dependencies]\nsample-absent = \"2\"\n");
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a package missing from the lock is not evidence of staleness: {findings:?}"
    );
}

/// The common real-world spelling: the path dependency inherits its requirement from the
/// workspace root it is itself the root of. Resolving inheritance is what keeps this check from
/// being blind on most consumer repos.
#[test]
fn stale_lock_findings_resolves_a_workspace_inherited_requirement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[workspace]\nmembers = []\n\n[workspace.dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n\n\
             [dependencies]\n{REGISTRY_DEPENDENCY} = {{ workspace = true }}\n"
        ),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert_eq!(
        findings.len(),
        1,
        "expected the inherited requirement, got: {findings:?}"
    );
    assert_eq!(findings[0].dependency, REGISTRY_DEPENDENCY);
    assert_eq!(findings[0].requirement, REQUIREMENT);
}

/// A git dependency is locked by revision; the `version` field beside it is not a registry
/// requirement the lock's pinned version has to satisfy.
#[test]
fn stale_lock_findings_ignores_a_git_dependency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[dependencies]\n{REGISTRY_DEPENDENCY} = {{ git = \"https://example.invalid/sample.git\", version = \
             \"{REQUIREMENT}\" }}\n"
        ),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a git dependency carries no registry pin: {findings:?}"
    );
}

/// The A4 regression, reproducing the crawlberg incident structurally: `tower-http` is declared
/// `optional = true` and only reachable behind a feature (`api`) that neither the generated
/// manifest's requested features nor the path dependency's default feature set activates. Before
/// this guard, alef read the bare `version` off the optional entry and reported a hard failure on
/// a lock `cargo metadata --locked` genuinely accepts.
#[test]
fn stale_lock_findings_ignores_an_unreached_optional_dependency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[features]\ndefault = []\nnetworking = [\"dep:{REGISTRY_DEPENDENCY}\"]\n\n\
             [dependencies]\n{REGISTRY_DEPENDENCY} = {{ version = \"{REQUIREMENT}\", optional = true }}\n"
        ),
    );
    write_generated_e2e_manifest_with_features(root, &[]);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "an optional dependency no requested or default feature activates must not be reported: {findings:?}"
    );
}

/// The over-correction guard for A4: an optional dependency IS reported once the generated
/// manifest actually requests the feature that activates it -- the fix must not blanket-ignore
/// every optional dependency, only the unreachable ones.
#[test]
fn stale_lock_findings_reports_a_reached_optional_dependency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[features]\ndefault = []\nnetworking = [\"dep:{REGISTRY_DEPENDENCY}\"]\n\n\
             [dependencies]\n{REGISTRY_DEPENDENCY} = {{ version = \"{REQUIREMENT}\", optional = true }}\n"
        ),
    );
    write_generated_e2e_manifest_with_features(root, &["networking"]);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert_eq!(
        findings.len(),
        1,
        "an optional dependency activated by a requested feature must still be checked: {findings:?}"
    );
    assert_eq!(findings[0].dependency, REGISTRY_DEPENDENCY);
}

/// A control alongside the two above: an optional dependency activated through the path
/// dependency's own DEFAULT feature set (no explicit `features = [...]` needed on the edge) must
/// also be reported -- the default-feature closure has to be resolved, not only explicit
/// requests.
#[test]
fn stale_lock_findings_reports_an_optional_dependency_reached_via_default_features() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[features]\ndefault = [\"dep:{REGISTRY_DEPENDENCY}\"]\n\n\
             [dependencies]\n{REGISTRY_DEPENDENCY} = {{ version = \"{REQUIREMENT}\", optional = true }}\n"
        ),
    );
    write_generated_e2e_manifest_with_features(root, &[]);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert_eq!(
        findings.len(),
        1,
        "an optional dependency activated by the target's own default features must still be checked: {findings:?}"
    );
}

/// Alef never authors a lockfile. A generated crate without one is a consumer choice, not a
/// defect, and must not fail the run.
#[test]
fn stale_lock_findings_skips_a_directory_with_no_committed_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a directory with no lock has nothing to check: {findings:?}"
    );
}

/// The run-level entry point: it must select manifests out of the generated path set, and the
/// error it returns must name the dependency, the lock, and the command that fixes it.
#[test]
fn check_generated_lock_freshness_names_the_dependency_and_the_remedy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    let manifest = write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);
    let generated: HashSet<PathBuf> = [manifest, root.join(E2E_RELATIVE_DIR).join("tests/basic_test.rs")]
        .into_iter()
        .collect();

    let error = check_generated_lock_freshness(&generated).expect("a stale lock must fail the run");
    let message = format!("{error:#}");

    assert!(
        message.contains(REGISTRY_DEPENDENCY),
        "message must name the dependency: {message}"
    );
    assert!(
        message.contains(STALE_PIN),
        "message must name the stale pin: {message}"
    );
    assert!(
        message.contains(REQUIREMENT),
        "message must name the requirement: {message}"
    );
    assert!(
        message.contains("cargo update"),
        "message must name the remedy: {message}"
    );
    assert!(
        message.contains(&e2e_dir(root).join("Cargo.lock").display().to_string()),
        "message must name the lock: {message}"
    );
}

/// Control for the entry point, matching the `stale_lock_findings` control above: a resolvable
/// lock must return `None` so the run keeps its zero exit.
#[test]
fn check_generated_lock_freshness_passes_a_resolvable_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    let manifest = write_generated_e2e_manifest(root);
    write_lock(root, FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    assert!(
        check_generated_lock_freshness(&generated).is_none(),
        "a resolvable lock must not fail the run"
    );
}

/// A generated path set containing no Rust manifest at all must not walk anything.
#[test]
fn check_generated_lock_freshness_ignores_non_manifest_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);
    let generated: HashSet<PathBuf> = [root.join(E2E_RELATIVE_DIR).join("tests/basic_test.rs")]
        .into_iter()
        .collect();

    assert!(check_generated_lock_freshness(&generated).is_none());
}

/// Coverage for [`check_generated_lock_freshness_tolerating_pending_publish`]'s exemption for a
/// disagreement fully explained by this crate's own not-yet-published release version -- the
/// `html-to-markdown` incident this exists for: `test_apps/rust/Cargo.toml` requires
/// `html-to-markdown-rs` at the exact version being released, but the registry still only has the
/// previous one, so `cargo update` cannot resolve it until publish. Reuses
/// [`registry_dependencies_on_local_crates`]'s exact shape (a registry, not path, dependency on a
/// local crate pinned at exactly its own canonical version) rather than inventing a new fixture
/// pattern.
mod pending_publish {
    use super::*;

    const CANONICAL: &str = "3.12.0";
    const PUBLISHED: &str = "3.11.6";
    const LOCAL_CRATE: &str = "sample-core";
    const GENERATED_RELATIVE_DIR: &str = "test_apps/rust";

    fn write_root_crate(root: &Path, version: &str) {
        std::fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = \"{LOCAL_CRATE}\"\nversion = \"{version}\"\nedition = \"2024\"\n"),
        )
        .expect("write root Cargo.toml");
    }

    /// A `test_apps`/e2e manifest requiring the crate under test at `required_version` through
    /// the registry -- no `path = `, matching `test_apps/rust/Cargo.toml`'s real shape exactly
    /// (`registry_dependencies_on_local_crates` skips any dependency declared with `path`).
    fn write_test_app_manifest(root: &Path, required_version: &str) -> PathBuf {
        let dir = root.join(GENERATED_RELATIVE_DIR);
        std::fs::create_dir_all(&dir).expect("create test_apps dir");
        let manifest = dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            format!(
                "[workspace]\n\n[package]\nname = \"{LOCAL_CRATE}-e2e-rust\"\nversion = \"0.0.0\"\nedition = \
                 \"2024\"\n\n[dependencies]\n{LOCAL_CRATE} = {{ version = \"{required_version}\" }}\n"
            ),
        )
        .expect("write test-app Cargo.toml");
        manifest
    }

    /// A committed lock pinning `locked_version`, marked registry-sourced -- `unpublished_dependency`
    /// only counts a resolved entry that carries a `source` at all, exactly like a real
    /// `cargo update`-produced lock.
    fn write_test_app_lock(root: &Path, locked_version: &str) {
        std::fs::write(
            root.join(GENERATED_RELATIVE_DIR).join("Cargo.lock"),
            format!(
                "version = 3\n\n[[package]]\nname = \"{LOCAL_CRATE}\"\nversion = \"{locked_version}\"\nsource = \
                 \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            ),
        )
        .expect("write test-app Cargo.lock");
    }

    /// Control proving the exemption is doing real work: without it, this exact shape must still
    /// fail -- otherwise the tolerating variant's `None` below would prove nothing.
    #[test]
    fn plain_check_still_fails_on_a_pending_publish_disagreement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_root_crate(root, CANONICAL);
        let manifest = write_test_app_manifest(root, CANONICAL);
        write_test_app_lock(root, PUBLISHED);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_lock_freshness(&generated).is_some(),
            "control: the plain check has no pending-publish exemption and must still fail here"
        );
    }

    #[test]
    fn tolerating_variant_warns_instead_of_failing_on_a_pending_publish_disagreement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_root_crate(root, CANONICAL);
        let manifest = write_test_app_manifest(root, CANONICAL);
        write_test_app_lock(root, PUBLISHED);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        let result = check_generated_lock_freshness_tolerating_pending_publish(&generated, root, Some(CANONICAL));
        assert!(
            result.is_none(),
            "a disagreement fully explained by this crate's own not-yet-published version must not fail the \
             run: {result:?}"
        );
    }

    /// Without a canonical version, nothing can be classified as pending -- must behave exactly
    /// like the plain check rather than silently exempting everything.
    #[test]
    fn tolerating_variant_without_canonical_behaves_like_the_plain_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_root_crate(root, CANONICAL);
        let manifest = write_test_app_manifest(root, CANONICAL);
        write_test_app_lock(root, PUBLISHED);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_lock_freshness_tolerating_pending_publish(&generated, root, None).is_some(),
            "no canonical version means no exemption is possible; this must still fail"
        );
    }

    /// The false-negative guard: a genuinely stale THIRD-PARTY pin (the `tower-http`-shaped
    /// incident, reusing the exact fixture from `check_generated_lock_freshness_names_the_dependency_and_the_remedy`
    /// above) has nothing to do with this crate's own pending release and must still fail even
    /// when a canonical version is supplied -- the exemption must not blanket-suppress every
    /// finding just because generation happens to know its own version.
    #[test]
    fn tolerating_variant_still_fails_on_a_disagreement_not_explained_by_pending_publish() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_root_manifest(
            root,
            &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
        );
        let manifest = write_generated_e2e_manifest(root);
        write_lock(root, STALE_PIN);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_lock_freshness_tolerating_pending_publish(&generated, root, Some(CANONICAL)).is_some(),
            "a third-party lock drift unrelated to this crate's own version must still fail the run"
        );
    }
}
