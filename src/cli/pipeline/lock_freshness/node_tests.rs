//! Coverage for [`check_generated_node_lock_freshness`] / [`stale_node_lock_findings`], the pnpm
//! sibling of the cargo checks. Unlike the Rust fixtures, there is no path-dependency indirection
//! to reproduce: the specifiers being compared live in the one `package.json` alef generated, so
//! the fixtures below only need that file and a `pnpm-lock.yaml` beside it.

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

    const WASM_DIR_RELATIVE: &str = "e2e/wasm";
    const WASM_DEPENDENCY: &str = "sample-pkg-wasm";
    const WASM_STALE_SPEC: &str = "1.3.0";
    const WASM_FRESH_SPEC: &str = "1.2.3";

    fn wasm_dir(root: &Path) -> PathBuf {
        root.join(WASM_DIR_RELATIVE)
    }

    /// The alef-generated wasm e2e `package.json`, matching `crate::e2e::codegen::wasm`'s own
    /// shape (see that module's counterpart to `render_package_json`): a sibling npm package
    /// under its own name, not `"node"`'s.
    fn write_wasm_package_json(root: &Path, specifier: &str) -> PathBuf {
        let dir = wasm_dir(root);
        std::fs::create_dir_all(&dir).expect("create wasm dir");
        let manifest = dir.join("package.json");
        std::fs::write(
            &manifest,
            format!(
                "{{\n  \"name\": \"sample-pkg-e2e-wasm\",\n  \"version\": \"0.1.0\",\n  \"private\": \
                 true,\n  \"devDependencies\": {{\n    \"{WASM_DEPENDENCY}\": \"{specifier}\"\n  }}\n}}\n"
            ),
        )
        .expect("write package.json");
        manifest
    }

    fn write_wasm_pnpm_lock(root: &Path, locked_specifier: &str) {
        std::fs::write(
            wasm_dir(root).join("pnpm-lock.yaml"),
            format!(
                "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      \
                 {WASM_DEPENDENCY}:\n        specifier: {locked_specifier}\n        version: \
                 {locked_specifier}\n"
            ),
        )
        .expect("write pnpm-lock.yaml");
    }

    /// Both `[crates.e2e.registry.packages.node]` and `[crates.e2e.registry.packages.wasm]`
    /// explicitly named -- the shape a crate publishing both an npm package and its wasm
    /// sibling (the html-to-markdown / tree-sitter-language-pack incident) actually has.
    fn resolved_cfg_with_node_and_wasm_registry_packages(
        node_name: &str,
        node_version: &str,
        wasm_name: &str,
        wasm_version: &str,
    ) -> ResolvedCrateConfig {
        let e2e = E2eConfig {
            registry: RegistryConfig {
                packages: [
                    (
                        "node".to_string(),
                        PackageRef {
                            name: Some(node_name.to_string()),
                            version: Some(node_version.to_string()),
                            ..PackageRef::default()
                        },
                    ),
                    (
                        "wasm".to_string(),
                        PackageRef {
                            name: Some(wasm_name.to_string()),
                            version: Some(wasm_version.to_string()),
                            ..PackageRef::default()
                        },
                    ),
                ]
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

    /// The reported defect (alef #A6): the `"wasm"` test_app has its own pending self-dependency
    /// row (its own not-yet-published version), the exact shape of `"node"`'s already-tolerated
    /// row but under a different package name. Before the fix, only `"node"`'s self-dependency
    /// was ever resolved, so this row fell through to `real` and hard-failed generation on a
    /// specifier the release itself cannot satisfy until it ships.
    #[test]
    fn tolerating_variant_downgrades_the_wasm_test_apps_own_pending_self_dependency() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_wasm_package_json(root, WASM_STALE_SPEC);
        write_wasm_pnpm_lock(root, WASM_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_node_and_wasm_registry_packages(
            NODE_DEPENDENCY,
            NODE_STALE_SPEC,
            WASM_DEPENDENCY,
            WASM_STALE_SPEC,
        );

        let result =
            check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));

        assert!(
            result.is_none(),
            "the wasm test_app's own pending self-dependency must be tolerated exactly like node's: {result:?}"
        );
    }

    /// The exact reported incident: both `test_apps/node` and `test_apps/wasm` carry the
    /// crate's own pending, not-yet-published version in the same run -- `"node"`'s row was
    /// already tolerated before this fix; `"wasm"`'s identical row, under its own package name,
    /// was not, and alone made `alef generate` exit 1. Both must be tolerated together.
    #[test]
    fn tolerating_variant_downgrades_both_the_node_and_wasm_test_apps_pending_self_dependencies_together() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let node_manifest = write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let wasm_manifest = write_wasm_package_json(root, WASM_STALE_SPEC);
        write_wasm_pnpm_lock(root, WASM_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [node_manifest, wasm_manifest].into_iter().collect();
        let resolved_cfg = resolved_cfg_with_node_and_wasm_registry_packages(
            NODE_DEPENDENCY,
            NODE_STALE_SPEC,
            WASM_DEPENDENCY,
            WASM_STALE_SPEC,
        );

        let result =
            check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg));

        assert!(
            result.is_none(),
            "both test_apps' own pending self-dependencies must be tolerated in the same run: {result:?}"
        );
    }

    /// The false-negative guard for the wasm test_app, mirroring
    /// `tolerating_variant_still_fails_on_a_disagreement_not_explained_by_pending_publish` for
    /// node: a genuinely stale THIRD-PARTY pin in the wasm test_app's lock must still fail even
    /// though a resolved config with a `"wasm"` registry package is supplied -- recognising the
    /// wasm self-dependency must not blanket-suppress every wasm finding.
    #[test]
    fn tolerating_variant_still_fails_on_the_wasm_test_apps_drift_not_explained_by_pending_publish() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_wasm_package_json(root, WASM_STALE_SPEC);
        write_wasm_pnpm_lock(root, WASM_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();
        // The configured wasm registry package name/version do NOT match this finding's own
        // dependency/requirement at all -- an unrelated self-dependency identity.
        let resolved_cfg = resolved_cfg_with_node_and_wasm_registry_packages(
            NODE_DEPENDENCY,
            NODE_STALE_SPEC,
            "unrelated-wasm-package",
            "9.9.9",
        );

        assert!(
            check_generated_node_lock_freshness_tolerating_pending_publish(&generated, root, Some(&resolved_cfg),)
                .is_some(),
            "a third-party lock drift in the wasm test_app unrelated to its own registry \
             self-dependency must still fail the run"
        );
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

#[path = "node_registered_manifest_tests.rs"]
mod registered_unmarkable_manifest_gap;
