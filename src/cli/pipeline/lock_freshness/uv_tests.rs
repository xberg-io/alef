//! Coverage for [`check_generated_uv_lock_freshness`] / [`stale_uv_lock_findings`], the uv/Python
//! sibling of the cargo/node checks. Like the node fixtures there is no path-dependency
//! indirection: the specifiers being compared live in the one `pyproject.toml` alef generated, so
//! the fixtures here only need that file and a `uv.lock` beside it. `requires_dist_map`'s
//! marker/extra filtering and `parse_pep508_requirement`/`normalize_pep503_name` are exercised
//! indirectly through every fixture here rather than unit-tested in isolation, matching how
//! `locked_node_specifiers` is covered in the node suite.

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
/// -- the uv sibling of the cargo `pending_publish` module, and the actual
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
