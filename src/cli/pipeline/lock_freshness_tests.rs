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

/// Coverage for [`check_generated_node_lock_freshness`] / [`stale_node_lock_findings`], the pnpm
/// sibling of the checks above. Unlike the Rust fixtures, there is no path-dependency indirection
/// to reproduce: the specifiers being compared live in the one `package.json` alef generated, so
/// the fixtures below only need that file and a `pnpm-lock.yaml` beside it.
mod node {
    use super::*;

    const NODE_DIR_RELATIVE: &str = "e2e/typescript";
    const NODE_DEPENDENCY: &str = "sample-pkg";
    const NODE_STALE_SPEC: &str = "1.3.0";
    const NODE_FRESH_SPEC: &str = "1.2.3";

    fn node_dir(root: &Path) -> PathBuf {
        root.join(NODE_DIR_RELATIVE)
    }

    /// The alef-generated e2e `package.json`, matching the shape
    /// `crate::e2e::codegen::typescript::config::render_package_json` emits: the dependency under
    /// test sits in `devDependencies`.
    fn write_package_json(root: &Path, specifier: &str) -> PathBuf {
        let dir = node_dir(root);
        std::fs::create_dir_all(&dir).expect("create node dir");
        let manifest = dir.join("package.json");
        std::fs::write(
            &manifest,
            format!(
                "{{\n  \"name\": \"sample-pkg-e2e-typescript\",\n  \"version\": \"0.1.0\",\n  \"private\": \
                 true,\n  \"devDependencies\": {{\n    \"{NODE_DEPENDENCY}\": \"{specifier}\"\n  }}\n}}\n"
            ),
        )
        .expect("write package.json");
        manifest
    }

    /// `lockfileVersion` 9's workspace-aware shape: dependency tables nest under `importers.".".*`.
    fn write_pnpm_lock_v9(root: &Path, locked_specifier: &str) {
        std::fs::write(
            node_dir(root).join("pnpm-lock.yaml"),
            format!(
                "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      \
                 {NODE_DEPENDENCY}:\n        specifier: {locked_specifier}\n        version: \
                 {locked_specifier}\n"
            ),
        )
        .expect("write pnpm-lock.yaml");
    }

    /// `lockfileVersion` 6's flat, non-workspace shape: dependency tables sit at the document root
    /// with no `importers` wrapper at all -- the fallback `locked_node_specifiers` must also read.
    fn write_pnpm_lock_v6(root: &Path, locked_specifier: &str) {
        std::fs::write(
            node_dir(root).join("pnpm-lock.yaml"),
            format!(
                "lockfileVersion: '6.0'\n\ndevDependencies:\n  {NODE_DEPENDENCY}:\n    specifier: \
                 {locked_specifier}\n    version: {locked_specifier}\n"
            ),
        )
        .expect("write pnpm-lock.yaml");
    }

    /// The regression: `package.json` was regenerated with a specifier the committed
    /// `pnpm-lock.yaml` does not record, exactly the shape that fails `pnpm install` under the
    /// default frozen lockfile in CI. Before this module alef reported nothing and exited 0.
    #[test]
    fn stale_node_lock_findings_reports_a_specifier_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);

        let findings = stale_node_lock_findings(&node_dir(root));

        assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.dependency, NODE_DEPENDENCY);
        assert_eq!(finding.bucket, "devDependencies");
        assert_eq!(finding.requirement, NODE_STALE_SPEC);
        assert_eq!(finding.locked_requirement, NODE_FRESH_SPEC);
        assert_eq!(finding.lock, node_dir(root).join("pnpm-lock.yaml"));
        assert_eq!(finding.declared_in, node_dir(root).join("package.json"));
    }

    /// The control that stops "always fail" from satisfying this suite: the identical fixture
    /// with a lock that already records the same specifier must produce nothing at all. This is
    /// the one that would NOT fail if `stale_node_lock_findings` were reverted to always compare
    /// unconditionally correctly -- it instead catches a reversion that made the comparison
    /// unconditionally report (e.g. dropping the equality check).
    #[test]
    fn stale_node_lock_findings_accepts_a_lock_that_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_FRESH_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);

        let findings = stale_node_lock_findings(&node_dir(root));

        assert!(
            findings.is_empty(),
            "a lock matching package.json must be reported clean: {findings:?}"
        );
    }

    /// `lockfileVersion` 6's flat shape (no `importers` wrapper) must be read too, not only 9's --
    /// this is the one that would fail if the `importers.".".*` fallback in
    /// `locked_node_specifiers` were the only shape read.
    #[test]
    fn stale_node_lock_findings_reports_a_mismatch_in_the_flat_lockfile_v6_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v6(root, NODE_FRESH_SPEC);

        let findings = stale_node_lock_findings(&node_dir(root));

        assert_eq!(
            findings.len(),
            1,
            "expected the flat lockfileVersion 6 shape to be read too, got: {findings:?}"
        );
        assert_eq!(findings[0].locked_requirement, NODE_FRESH_SPEC);
    }

    /// The one-sided rule, matching the cargo check's absence rule: a dependency package.json
    /// declares but the lock's own bucket never mentions is not reported. The lock here is
    /// non-empty (it pins an unrelated package) so this exercises the per-name lookup missing,
    /// not merely an empty bucket short-circuiting earlier.
    #[test]
    fn stale_node_lock_findings_ignores_a_dependency_absent_from_the_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        let dir = node_dir(root);
        std::fs::write(
            dir.join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      other-pkg:\n        \
             specifier: 2.0.0\n        version: 2.0.0\n",
        )
        .expect("write pnpm-lock.yaml");

        let findings = stale_node_lock_findings(&dir);

        assert!(
            findings.is_empty(),
            "a package missing from the lock's bucket is not evidence of drift: {findings:?}"
        );
    }

    /// `workspace:` specifiers are resolved through a workspace root this check never reads, so a
    /// text mismatch against the lock's recorded specifier must not be reported.
    #[test]
    fn stale_node_lock_findings_ignores_a_workspace_specifier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, "workspace:*");
        write_pnpm_lock_v9(root, "workspace:^1.0.0");

        let findings = stale_node_lock_findings(&node_dir(root));

        assert!(
            findings.is_empty(),
            "workspace: specifiers are not directly comparable: {findings:?}"
        );
    }

    /// `file:` specifiers (the wasm e2e app's local dependency mode) are excluded for the same
    /// reason `fingerprint.rs` excludes `node_modules`/`vendor` from its own hash: a locally
    /// linked dependency's content, and potentially the text pnpm records for it, can move for
    /// reasons a text diff here cannot verify.
    #[test]
    fn stale_node_lock_findings_ignores_a_file_specifier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, "file:../../..");
        write_pnpm_lock_v9(root, "file:../../../dist");

        let findings = stale_node_lock_findings(&node_dir(root));

        assert!(
            findings.is_empty(),
            "file: specifiers are not directly comparable: {findings:?}"
        );
    }

    /// The run-level entry point: it must select `package.json` out of the generated path set,
    /// and the error it returns must name the dependency, both specifiers, the lock, and the
    /// remedy.
    #[test]
    fn check_generated_node_lock_freshness_names_the_dependency_and_the_remedy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        let error = check_generated_node_lock_freshness(&generated, root).expect("a stale lock must fail the run");
        let message = format!("{error:#}");

        assert!(
            message.contains(NODE_DEPENDENCY),
            "message must name the dependency: {message}"
        );
        assert!(
            message.contains(NODE_STALE_SPEC),
            "message must name the package.json specifier: {message}"
        );
        assert!(
            message.contains(NODE_FRESH_SPEC),
            "message must name the locked specifier: {message}"
        );
        assert!(
            message.contains("pnpm install"),
            "message must name the remedy: {message}"
        );
        assert!(
            message.contains(&node_dir(root).join("pnpm-lock.yaml").display().to_string()),
            "message must name the lock: {message}"
        );
    }

    /// Control for the entry point, matching the pattern above: a lock whose specifier already
    /// matches must return `None` so the run keeps its zero exit. This is the assertion that
    /// would catch a regression turning this check into an unconditional failure.
    #[test]
    fn check_generated_node_lock_freshness_passes_a_matching_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_package_json(root, NODE_FRESH_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_node_lock_freshness(&generated, root).is_none(),
            "a matching lock must not fail the run"
        );
    }

    /// A generated path set containing no `package.json` at all must not walk anything.
    #[test]
    fn check_generated_node_lock_freshness_ignores_non_manifest_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [node_dir(root).join("src/index.ts")].into_iter().collect();

        assert!(check_generated_node_lock_freshness(&generated, root).is_none());
    }

    /// Coverage for [`check_generated_node_lock_freshness_tolerating_pending_publish`]'s
    /// exemption -- the npm sibling of the cargo `pending_publish` module above. Reproduces the
    /// `@xberg-io/html-to-markdown` shape: `test_apps/typescript/package.json` requires the
    /// crate's own published npm package at the exact range `[crates.e2e.registry.packages.node]`
    /// currently declares, but the registry has not published it yet.
    mod pending_publish {
        use super::*;
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::config::e2e::{E2eConfig, PackageRef, RegistryConfig};

        /// A crate whose `[crates.e2e.registry.packages.node]` explicitly names `pkg_name` at
        /// `pkg_version` -- the only shape [`registry_self_dependency`] ever vouches for.
        fn resolved_cfg_with_node_registry_package(pkg_name: &str, pkg_version: &str) -> ResolvedCrateConfig {
            let e2e = E2eConfig {
                registry: RegistryConfig {
                    packages: [(
                        "node".to_string(),
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

        const SIBLING_DEPENDENCY: &str = "sibling-pkg";
        const SIBLING_STALE_SPEC: &str = "^1.0.0";
        const SIBLING_LOCKED_SPEC: &str = "^2.0.0";

        /// A `package.json` declaring both the pending self-dependency and an unrelated sibling
        /// dependency in the same `devDependencies` bucket -- the tslp shape: one `package.json`
        /// pinning `@xberg-io/tree-sitter-language-pack` at the not-yet-published release version
        /// alongside `@types/node`/`vitest`/`rollup` specifiers the lock also disagrees with.
        fn write_package_json_with_sibling(root: &Path, self_specifier: &str, sibling_specifier: &str) -> PathBuf {
            let dir = node_dir(root);
            std::fs::create_dir_all(&dir).expect("create node dir");
            let manifest = dir.join("package.json");
            std::fs::write(
                &manifest,
                format!(
                    "{{\n  \"name\": \"sample-pkg-e2e-typescript\",\n  \"version\": \"0.1.0\",\n  \"private\": \
                     true,\n  \"devDependencies\": {{\n    \"{NODE_DEPENDENCY}\": \"{self_specifier}\",\n    \
                     \"{SIBLING_DEPENDENCY}\": \"{sibling_specifier}\"\n  }}\n}}\n"
                ),
            )
            .expect("write package.json");
            manifest
        }

        /// The sibling lockfile to [`write_package_json_with_sibling`]: both dependencies pinned
        /// at a specifier that disagrees with `package.json`.
        fn write_pnpm_lock_v9_with_sibling(root: &Path, self_locked: &str, sibling_locked: &str) {
            std::fs::write(
                node_dir(root).join("pnpm-lock.yaml"),
                format!(
                    "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      \
                     {NODE_DEPENDENCY}:\n        specifier: {self_locked}\n        version: {self_locked}\n      \
                     {SIBLING_DEPENDENCY}:\n        specifier: {sibling_locked}\n        version: \
                     {sibling_locked}\n"
                ),
            )
            .expect("write pnpm-lock.yaml");
        }

        /// The A5 regression: a lockfile blocked on this crate's own pending release also carries
        /// an unrelated sibling specifier drift (`@types/node`, `vitest`, `rollup` in the real
        /// incident). `pnpm install --lockfile-only` cannot resolve ANYTHING in this lockfile
        /// until the self-dependency publishes -- it fails on the self-dependency with
        /// `ERR_PNPM_NO_MATCHING_VERSION` before it ever reaches the sibling -- so a per-finding
        /// partition that still hard-fails the sibling prescribes a remedy the operator cannot
        /// run. The whole lock must be tolerated together.
        #[test]
        fn tolerating_variant_downgrades_sibling_findings_in_the_same_pending_lock() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();
            let manifest = write_package_json_with_sibling(root, NODE_STALE_SPEC, SIBLING_STALE_SPEC);
            write_pnpm_lock_v9_with_sibling(root, NODE_FRESH_SPEC, SIBLING_LOCKED_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
            let resolved_cfg = resolved_cfg_with_node_registry_package(NODE_DEPENDENCY, NODE_STALE_SPEC);

            let result =
                check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));

            assert!(
                result.is_none(),
                "a sibling drift sharing a lock with a pending self-dependency must also be tolerated, \
                 since pnpm cannot resolve any of it until the self-dependency publishes: {result:?}"
            );
        }

        /// The multi-lock boundary the A5 fix actually introduces: tolerance is scoped PER LOCK,
        /// not blanket across the whole run. Lock A (this crate's own generated directory) carries
        /// both the pending self-dependency row and a sibling drift, both correctly tolerated
        /// together per the test above. Lock B is a wholly separate generated `pnpm-lock.yaml`
        /// with its own unrelated sibling drift and NO self-dependency row at all. Downgrading
        /// lock A must not silently swallow lock B: the run must still fail, and the surviving
        /// error must name lock B's own drift, not lock A's (which is fully tolerated).
        #[test]
        fn tolerating_variant_still_fails_on_a_different_locks_unrelated_drift() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();

            let manifest_a = write_package_json_with_sibling(root, NODE_STALE_SPEC, SIBLING_STALE_SPEC);
            write_pnpm_lock_v9_with_sibling(root, NODE_FRESH_SPEC, SIBLING_LOCKED_SPEC);

            let other_dir = root.join("e2e/typescript-other");
            std::fs::create_dir_all(&other_dir).expect("create other node dir");
            let manifest_b = other_dir.join("package.json");
            std::fs::write(
                &manifest_b,
                format!(
                    "{{\n  \"name\": \"other-pkg-e2e-typescript\",\n  \"version\": \"0.1.0\",\n  \"private\": \
                     true,\n  \"devDependencies\": {{\n    \"{SIBLING_DEPENDENCY}\": \"{SIBLING_STALE_SPEC}\"\n  \
                     }}\n}}\n"
                ),
            )
            .expect("write package.json");
            std::fs::write(
                other_dir.join("pnpm-lock.yaml"),
                format!(
                    "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      \
                     {SIBLING_DEPENDENCY}:\n        specifier: {SIBLING_LOCKED_SPEC}\n        version: \
                     {SIBLING_LOCKED_SPEC}\n"
                ),
            )
            .expect("write pnpm-lock.yaml");

            let generated: HashSet<PathBuf> = [manifest_a, manifest_b].into_iter().collect();
            let resolved_cfg = resolved_cfg_with_node_registry_package(NODE_DEPENDENCY, NODE_STALE_SPEC);

            let error =
                check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg))
                    .expect(
                        "lock B's unrelated drift, sharing no lock with the pending self-dependency, must \
                             still fail the run",
                    );
            let message = format!("{error:#}");

            assert!(
                message.contains(&other_dir.join("pnpm-lock.yaml").display().to_string()),
                "message must name lock B: {message}"
            );
            assert!(
                !message.contains(&node_dir(root).join("pnpm-lock.yaml").display().to_string()),
                "lock A must be fully tolerated and absent from the surviving error: {message}"
            );
        }

        /// Control proving the exemption does real work: without it, this exact shape must still
        /// fail.
        #[test]
        fn plain_check_still_fails_on_a_pending_publish_disagreement() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();
            let manifest = write_package_json(root, NODE_STALE_SPEC);
            write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

            assert!(
                check_generated_node_lock_freshness(&generated, root).is_some(),
                "control: the plain check has no pending-publish exemption and must still fail here"
            );
        }

        #[test]
        fn tolerating_variant_warns_instead_of_failing_when_the_requirement_matches_the_configured_registry_package() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();
            let manifest = write_package_json(root, NODE_STALE_SPEC);
            write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
            let resolved_cfg = resolved_cfg_with_node_registry_package(NODE_DEPENDENCY, NODE_STALE_SPEC);

            let result =
                check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));
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
            let manifest = write_package_json(root, NODE_STALE_SPEC);
            write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

            assert!(
                check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, None).is_some(),
                "no resolved config means no exemption is possible; this must still fail"
            );
        }

        /// The false-negative guard: a genuinely stale THIRD-PARTY pin has nothing to do with this
        /// crate's own registry self-dependency and must still fail even when a resolved config is
        /// supplied -- the exemption must not blanket-suppress every finding just because
        /// generation happens to know its own registry package identity.
        #[test]
        fn tolerating_variant_still_fails_on_a_disagreement_not_explained_by_pending_publish() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();
            let manifest = write_package_json(root, NODE_STALE_SPEC);
            write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
            // The configured registry package name/version do NOT match the finding's own
            // dependency/requirement at all -- an unrelated self-dependency identity, so nothing
            // here explains this drift.
            let resolved_cfg = resolved_cfg_with_node_registry_package("unrelated-package", "9.9.9");

            assert!(
                check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg),)
                    .is_some(),
                "a third-party lock drift unrelated to this crate's own registry self-dependency \
                 must still fail the run"
            );
        }
    }
}

#[path = "lock_freshness_registered_manifest_tests.rs"]
mod registered_unmarkable_manifest_gap;

/// Coverage for [`check_generated_uv_lock_freshness`] / [`stale_uv_lock_findings`], the uv/Python
/// sibling of the checks above. Like the node fixtures there is no path-dependency indirection: the
/// specifiers being compared live in the one `pyproject.toml` alef generated, so the fixtures below
/// only need that file and a `uv.lock` beside it. `requires_dist_map`'s marker/extra filtering and
/// `parse_pep508_requirement`/`normalize_pep503_name` are exercised indirectly through every fixture
/// here rather than unit-tested in isolation, matching how `locked_node_specifiers` is covered above.
mod uv {
    use super::*;

    const UV_DIR_RELATIVE: &str = "e2e/python";
    const UV_DEPENDENCY: &str = "sample-pkg";
    const UV_STALE_SPEC: &str = ">=1.1.1";
    const UV_FRESH_SPEC: &str = ">=1.2.0";

    fn uv_dir(root: &Path) -> PathBuf {
        root.join(UV_DIR_RELATIVE)
    }

    /// The alef-generated e2e `pyproject.toml`, matching the shape
    /// `crate::e2e::codegen::python::config::render_pyproject` emits: a `[project.dependencies]`
    /// array holding a plain PEP 508 requirement string.
    fn write_pyproject(root: &Path, specifier: &str) -> PathBuf {
        let dir = uv_dir(root);
        std::fs::create_dir_all(&dir).expect("create uv dir");
        let manifest = dir.join("pyproject.toml");
        std::fs::write(
            &manifest,
            format!(
                "[project]\nname = \"sample-pkg-e2e\"\nversion = \"0.0.0\"\ndependencies = \
                 [\"{UV_DEPENDENCY}{specifier}\"]\n"
            ),
        )
        .expect("write pyproject.toml");
        manifest
    }

    /// The project-lock shape: the project's own `[[package]]` entry carries `[package.metadata]
    /// requires-dist`, keyed by name off `pyproject.toml`'s own `[project.name]`.
    fn write_uv_lock_project_shape(root: &Path, locked_specifier: &str) {
        std::fs::write(
            uv_dir(root).join("uv.lock"),
            format!(
                "version = 1\nrequires-python = \">=3.10\"\n\n\
                 [[package]]\nname = \"sample-pkg-e2e\"\nversion = \"0.0.0\"\nsource = {{ virtual = \".\" }}\n\
                 dependencies = [\n  {{ name = \"{UV_DEPENDENCY}\" }},\n]\n\n\
                 [package.metadata]\nrequires-dist = [{{ name = \"{UV_DEPENDENCY}\", specifier = \
                 \"{locked_specifier}\" }}]\n"
            ),
        )
        .expect("write uv.lock");
    }

    /// The standalone-script-lock shape: no per-package `[[package]]` entry for the project at all
    /// (there is no project -- a script lock has nothing to attach `[package.metadata]` to), only a
    /// top-level `[manifest] requirements` array carrying the same `{ name, specifier }` shape. This
    /// is the fallback `locked_uv_requirements` must also read.
    fn write_uv_lock_manifest_shape(root: &Path, locked_specifier: &str) {
        std::fs::write(
            uv_dir(root).join("uv.lock"),
            format!(
                "version = 1\nrequires-python = \">=3.10\"\n\n\
                 [manifest]\nrequirements = [{{ name = \"{UV_DEPENDENCY}\", specifier = \
                 \"{locked_specifier}\" }}]\n"
            ),
        )
        .expect("write uv.lock");
    }

    /// The regression: `pyproject.toml` was regenerated with a specifier the committed `uv.lock`
    /// does not record, exactly the shape that fails `uv sync --locked` under the default frozen
    /// lockfile in CI. Before this module alef reported nothing and exited 0.
    #[test]
    fn stale_uv_lock_findings_reports_a_specifier_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_pyproject(root, UV_STALE_SPEC);
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);

        let findings = stale_uv_lock_findings(&uv_dir(root));

        assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.dependency, UV_DEPENDENCY);
        assert_eq!(finding.requirement, UV_STALE_SPEC);
        assert_eq!(finding.locked_requirement, UV_FRESH_SPEC);
        assert_eq!(finding.lock, uv_dir(root).join("uv.lock"));
        assert_eq!(finding.declared_in, uv_dir(root).join("pyproject.toml"));
    }

    /// The control that stops "always fail" from satisfying this suite: the identical fixture with
    /// a lock that already records the same specifier must produce nothing at all. This is the one
    /// that would NOT fail if `stale_uv_lock_findings` were reverted to always compare unconditionally
    /// -- it instead catches a reversion that made the comparison unconditionally report.
    #[test]
    fn stale_uv_lock_findings_accepts_a_lock_that_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_pyproject(root, UV_FRESH_SPEC);
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);

        let findings = stale_uv_lock_findings(&uv_dir(root));

        assert!(
            findings.is_empty(),
            "a lock matching pyproject.toml must be reported clean: {findings:?}"
        );
    }

    /// The one-sided rule, matching the cargo and node checks' absence rule: a dependency
    /// `pyproject.toml` declares but the lock's recorded copy never mentions is not reported. The
    /// lock here is non-empty (it pins an unrelated package) so this exercises the per-name lookup
    /// missing, not merely an empty map short-circuiting earlier.
    #[test]
    fn stale_uv_lock_findings_ignores_a_dependency_absent_from_the_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_pyproject(root, UV_STALE_SPEC);
        std::fs::write(
            uv_dir(root).join("uv.lock"),
            "version = 1\nrequires-python = \">=3.10\"\n\n\
             [[package]]\nname = \"sample-pkg-e2e\"\nversion = \"0.0.0\"\nsource = { virtual = \".\" }\n\n\
             [package.metadata]\nrequires-dist = [{ name = \"other-pkg\", specifier = \">=2.0\" }]\n",
        )
        .expect("write uv.lock");

        let findings = stale_uv_lock_findings(&uv_dir(root));

        assert!(
            findings.is_empty(),
            "a package missing from the lock's recorded copy is not evidence of drift: {findings:?}"
        );
    }

    /// The standalone-script-lock fallback shape must be read too, not only the project shape --
    /// this is the one that would fail if the `[manifest] requirements` fallback in
    /// `locked_uv_requirements` were never reached (e.g. dropped, or only tried when the project
    /// shape's own array is literally absent rather than merely mapping to nothing).
    #[test]
    fn stale_uv_lock_findings_reports_a_mismatch_in_the_manifest_requirements_fallback_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_pyproject(root, UV_STALE_SPEC);
        write_uv_lock_manifest_shape(root, UV_FRESH_SPEC);

        let findings = stale_uv_lock_findings(&uv_dir(root));

        assert_eq!(
            findings.len(),
            1,
            "expected the manifest.requirements fallback shape to be read too, got: {findings:?}"
        );
        assert_eq!(findings[0].locked_requirement, UV_FRESH_SPEC);
    }

    /// A dependency declared with an environment marker is conditional on something this reader
    /// does not evaluate; comparing its specifier text against the lock's recorded copy would not
    /// be reliable evidence of drift, so it must not be reported even when the texts do differ.
    #[test]
    fn stale_uv_lock_findings_ignores_a_marker_conditional_dependency() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let dir = uv_dir(root);
        std::fs::create_dir_all(&dir).expect("create uv dir");
        std::fs::write(
            dir.join("pyproject.toml"),
            format!(
                "[project]\nname = \"sample-pkg-e2e\"\nversion = \"0.0.0\"\ndependencies = \
                 [\"{UV_DEPENDENCY}{UV_STALE_SPEC}; python_version < '3.11'\"]\n"
            ),
        )
        .expect("write pyproject.toml");
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);

        let findings = stale_uv_lock_findings(&uv_dir(root));

        assert!(
            findings.is_empty(),
            "a marker-conditional requirement is not directly comparable: {findings:?}"
        );
    }

    /// A name declared in `[tool.uv.sources]` has its resolution overridden (path, git, URL,
    /// workspace, or an alternate index) -- exactly `render_pyproject`'s own `Local` dependency
    /// mode, which writes the bare unconstrained name here and the real source in that table.
    /// Comparing it against the lock's registry-shaped specifier text would be a false positive.
    #[test]
    fn stale_uv_lock_findings_ignores_a_source_overridden_dependency() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let dir = uv_dir(root);
        std::fs::create_dir_all(&dir).expect("create uv dir");
        std::fs::write(
            dir.join("pyproject.toml"),
            format!(
                "[project]\nname = \"sample-pkg-e2e\"\nversion = \"0.0.0\"\ndependencies = \
                 [\"{UV_DEPENDENCY}\"]\n\n[tool.uv]\nsources.{UV_DEPENDENCY} = {{ path = \"../..\" }}\n"
            ),
        )
        .expect("write pyproject.toml");
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);

        let findings = stale_uv_lock_findings(&uv_dir(root));

        assert!(
            findings.is_empty(),
            "a [tool.uv.sources]-overridden dependency is not directly comparable: {findings:?}"
        );
    }

    /// The run-level entry point: it must select `pyproject.toml` out of the generated path set,
    /// and the error it returns must name the dependency, both specifiers, the lock, and the
    /// remedy.
    #[test]
    fn check_generated_uv_lock_freshness_names_the_dependency_and_the_remedy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_pyproject(root, UV_STALE_SPEC);
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        let error = check_generated_uv_lock_freshness(&generated).expect("a stale lock must fail the run");
        let message = format!("{error:#}");

        assert!(
            message.contains(UV_DEPENDENCY),
            "message must name the dependency: {message}"
        );
        assert!(
            message.contains(UV_STALE_SPEC),
            "message must name the pyproject.toml specifier: {message}"
        );
        assert!(
            message.contains(UV_FRESH_SPEC),
            "message must name the locked specifier: {message}"
        );
        assert!(message.contains("uv lock"), "message must name the remedy: {message}");
        assert!(
            message.contains(&uv_dir(root).join("uv.lock").display().to_string()),
            "message must name the lock: {message}"
        );
    }

    /// Control for the entry point, matching the pattern above: a lock whose specifier already
    /// matches must return `None` so the run keeps its zero exit. This is the assertion that would
    /// catch a regression turning this check into an unconditional failure.
    #[test]
    fn check_generated_uv_lock_freshness_passes_a_matching_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_pyproject(root, UV_FRESH_SPEC);
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_uv_lock_freshness(&generated).is_none(),
            "a matching lock must not fail the run"
        );
    }

    /// A generated path set containing no `pyproject.toml` at all must not walk anything.
    #[test]
    fn check_generated_uv_lock_freshness_ignores_non_manifest_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_pyproject(root, UV_STALE_SPEC);
        write_uv_lock_project_shape(root, UV_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [uv_dir(root).join("tests/test_basic.py")].into_iter().collect();

        assert!(check_generated_uv_lock_freshness(&generated).is_none());
    }

    /// Coverage for [`check_generated_uv_lock_freshness_tolerating_pending_publish`]'s exemption
    /// -- the uv sibling of the cargo `pending_publish` module above, and the actual
    /// `html-to-markdown` incident this closes: `test_apps/python/pyproject.toml` requires
    /// `html-to-markdown>=3.12.0` while PyPI still only has `3.11.6` published.
    mod pending_publish {
        use super::*;
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::config::e2e::{E2eConfig, PackageRef, RegistryConfig};

        /// A crate whose `[crates.e2e.registry.packages.python]` explicitly names `pkg_name` at
        /// `pkg_version` -- the only shape [`registry_self_dependency`] ever vouches for.
        fn resolved_cfg_with_python_registry_package(pkg_name: &str, pkg_version: &str) -> ResolvedCrateConfig {
            let e2e = E2eConfig {
                registry: RegistryConfig {
                    packages: [(
                        "python".to_string(),
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
            let manifest = write_pyproject(root, UV_STALE_SPEC);
            write_uv_lock_project_shape(root, UV_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

            assert!(
                check_generated_uv_lock_freshness(&generated).is_some(),
                "control: the plain check has no pending-publish exemption and must still fail here"
            );
        }

        #[test]
        fn tolerating_variant_warns_instead_of_failing_when_the_requirement_matches_the_configured_registry_package() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();
            let manifest = write_pyproject(root, UV_STALE_SPEC);
            write_uv_lock_project_shape(root, UV_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
            // `UV_STALE_SPEC` already carries a PEP 508 comparator (">="), so
            // `normalize_python_version` passes it through unchanged -- this is exactly what
            // alef's own e2e generator would have written for this registry package.
            let resolved_cfg = resolved_cfg_with_python_registry_package(UV_DEPENDENCY, UV_STALE_SPEC);

            let result = check_generated_uv_lock_freshness_tolerating_pending_publish(&generated, Some(&resolved_cfg));
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
            let manifest = write_pyproject(root, UV_STALE_SPEC);
            write_uv_lock_project_shape(root, UV_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

            assert!(
                check_generated_uv_lock_freshness_tolerating_pending_publish(&generated, None).is_some(),
                "no resolved config means no exemption is possible; this must still fail"
            );
        }

        /// The false-negative guard: a genuinely stale THIRD-PARTY pin has nothing to do with this
        /// crate's own registry self-dependency and must still fail even when a resolved config is
        /// supplied -- the exemption must not blanket-suppress every finding just because
        /// generation happens to know its own registry package identity.
        #[test]
        fn tolerating_variant_still_fails_on_a_disagreement_not_explained_by_pending_publish() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path();
            let manifest = write_pyproject(root, UV_STALE_SPEC);
            write_uv_lock_project_shape(root, UV_FRESH_SPEC);
            let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
            // The configured registry package name/version do NOT match the finding's own
            // dependency/requirement at all -- an unrelated self-dependency identity, so nothing
            // here explains this drift.
            let resolved_cfg = resolved_cfg_with_python_registry_package("unrelated-package", ">=9.9.9");

            assert!(
                check_generated_uv_lock_freshness_tolerating_pending_publish(&generated, Some(&resolved_cfg)).is_some(),
                "a third-party lock drift unrelated to this crate's own registry self-dependency \
                 must still fail the run"
            );
        }
    }
}
