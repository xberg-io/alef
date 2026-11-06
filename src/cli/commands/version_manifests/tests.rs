use super::*;
use tempfile::TempDir;

fn config(root: &Path) -> ResolvedCrateConfig {
    let manifest = root.join("Cargo.toml").to_string_lossy().replace('\\', "/");
    let source = format!(
        "[workspace]\nlanguages = [\"csharp\", \"dart\", \"zig\"]\n\
         [[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\nversion_from = \"{manifest}\"\n"
    );
    let parsed: crate::core::config::NewAlefConfig = toml::from_str(&source).expect("config parses");
    parsed.resolve().expect("config resolves").remove(0)
}

fn workspace(version: &str) -> TempDir {
    let temp = TempDir::new().expect("tempdir");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[workspace]\nmembers = [\"crates/sample\", \"crates/helper\"]\n\
             [workspace.package]\nversion = \"{version}\"\n"
        ),
    )
    .expect("workspace manifest");
    for name in ["sample", "helper"] {
        let directory = temp.path().join("crates").join(name);
        std::fs::create_dir_all(&directory).expect("crate directory");
        std::fs::write(
            directory.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion.workspace = true\n"),
        )
        .expect("crate manifest");
    }
    temp
}

#[test]
fn discovers_nested_csproj_and_checks_all_version_fields() {
    let temp = workspace("1.2.3");
    let project = temp.path().join("packages/csharp/src/Sample/Sample.csproj");
    std::fs::create_dir_all(project.parent().expect("project parent")).expect("project directory");
    std::fs::write(
        &project,
        concat!(
            "<Project><PropertyGroup>",
            "<Version>1.2.3</Version>",
            "<AssemblyVersion>1.2.2</AssemblyVersion>",
            "<FileVersion>1.2.1</FileVersion>",
            "<InformationalVersion>1.2.0</InformationalVersion>",
            "</PropertyGroup></Project>",
        ),
    )
    .expect("csproj");

    let checks = collect(&config(temp.path()), temp.path(), "1.2.3");
    let project_checks: Vec<_> = checks
        .iter()
        .filter(|check| check.label.contains("Sample.csproj#"))
        .collect();
    assert_eq!(project_checks.len(), 4);
    assert!(
        project_checks
            .iter()
            .any(|check| check.label.ends_with("#Version") && check.matches)
    );
    for field in ["AssemblyVersion", "FileVersion", "InformationalVersion"] {
        assert!(
            project_checks
                .iter()
                .any(|check| check.label.ends_with(&format!("#{field}")) && !check.matches)
        );
    }
}

/// Write a csproj whose four version fields carry exactly what the generator
/// emits for `canonical`, then return its checks.
fn generator_shaped_csproj_checks(canonical: &str) -> Vec<VersionCheck> {
    let temp = workspace(canonical);
    let project = temp.path().join("packages/csharp/src/Sample/Sample.csproj");
    std::fs::create_dir_all(project.parent().expect("project parent")).expect("project directory");
    let assembly = crate::core::version::to_dotnet_assembly_version(canonical);
    std::fs::write(
        &project,
        format!(
            "<Project><PropertyGroup>\
             <Version>{canonical}</Version>\
             <AssemblyVersion>{assembly}</AssemblyVersion>\
             <FileVersion>{assembly}</FileVersion>\
             <InformationalVersion>{canonical}</InformationalVersion>\
             </PropertyGroup></Project>",
        ),
    )
    .expect("csproj");

    collect(&config(temp.path()), temp.path(), canonical)
        .into_iter()
        .filter(|check| check.label.contains("Sample.csproj#"))
        .collect()
}

#[test]
fn generated_four_component_assembly_version_is_not_reported_as_a_mismatch() {
    let checks = generator_shaped_csproj_checks("1.17.0");
    assert_eq!(checks.len(), 4);
    for check in &checks {
        assert!(
            check.matches,
            "generator output must validate clean, but {} reported {:?}",
            check.label, check.found
        );
    }
}

#[test]
fn prerelease_assembly_version_matches_while_semver_fields_keep_the_prerelease() {
    let checks = generator_shaped_csproj_checks("1.9.0-rc.48");
    let find = |suffix: &str| {
        checks
            .iter()
            .find(|check| check.label.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing {suffix}"))
    };
    for suffix in ["#AssemblyVersion", "#FileVersion"] {
        let check = find(suffix);
        assert!(check.matches, "{suffix} should match: {:?}", check.found);
        assert_eq!(check.found.as_deref(), Some("1.9.0.0"));
    }
    for suffix in ["#Version", "#InformationalVersion"] {
        let check = find(suffix);
        assert!(check.matches, "{suffix} should match: {:?}", check.found);
        assert_eq!(check.found.as_deref(), Some("1.9.0-rc.48"));
    }
}

#[test]
fn semver_fields_still_reject_the_four_component_assembly_form() {
    let temp = workspace("1.17.0");
    let project = temp.path().join("packages/csharp/src/Sample/Sample.csproj");
    std::fs::create_dir_all(project.parent().expect("project parent")).expect("project directory");
    std::fs::write(
        &project,
        concat!(
            "<Project><PropertyGroup>",
            "<Version>1.17.0.0</Version>",
            "<InformationalVersion>1.17.0.0</InformationalVersion>",
            "</PropertyGroup></Project>",
        ),
    )
    .expect("csproj");

    let checks = collect(&config(temp.path()), temp.path(), "1.17.0");
    for suffix in ["#Version", "#InformationalVersion"] {
        let check = checks
            .iter()
            .find(|check| check.label.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing {suffix}"));
        assert!(
            !check.matches,
            "{suffix} carries full SemVer and must not accept the assembly form"
        );
    }
}

#[test]
fn checks_dart_and_zig_manifests() {
    let temp = workspace("2.0.0");
    std::fs::create_dir_all(temp.path().join("packages/dart")).expect("dart directory");
    std::fs::write(
        temp.path().join("packages/dart/pubspec.yaml"),
        "name: sample\nversion: 2.0.0 # release\n",
    )
    .expect("pubspec");
    std::fs::create_dir_all(temp.path().join("packages/zig")).expect("zig directory");
    std::fs::write(
        temp.path().join("packages/zig/build.zig.zon"),
        ".{\n    .name = \"sample\",\n    .version = \"1.9.0\",\n}\n",
    )
    .expect("zig manifest");

    let checks = collect(&config(temp.path()), temp.path(), "2.0.0");
    let dart = checks
        .iter()
        .find(|check| check.label.ends_with("pubspec.yaml"))
        .expect("dart check");
    let zig = checks
        .iter()
        .find(|check| check.label.ends_with("build.zig.zon"))
        .expect("zig check");
    assert!(dart.matches);
    assert!(!zig.matches);
}

#[test]
fn checks_each_local_package_in_every_cargo_lock() {
    let temp = workspace("3.4.5");
    std::fs::write(
        temp.path().join("crates/helper/Cargo.toml"),
        "[package]\nname = \"helper\"\nversion = \"4.0.0\"\n",
    )
    .expect("versioned helper manifest");
    let nested = temp.path().join("packages/elixir/native/sample");
    std::fs::create_dir_all(&nested).expect("lock directory");
    let lock = concat!(
        "version = 4\n\n",
        "[[package]]\nname = \"sample\"\nversion = \"3.4.4\"\n\n",
        "[[package]]\nname = \"helper\"\nversion = \"4.0.0\"\n\n",
        "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://example.invalid/index\"\n",
    );
    std::fs::write(nested.join("Cargo.lock"), lock).expect("nested lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
    let lock_checks: Vec<_> = checks
        .iter()
        .filter(|check| check.label.contains("Cargo.lock#"))
        .collect();
    assert_eq!(lock_checks.len(), 2);
    assert!(
        lock_checks
            .iter()
            .any(|check| check.label.ends_with("#sample") && !check.matches)
    );
    assert!(
        lock_checks
            .iter()
            .any(|check| check.label.ends_with("#helper") && check.matches)
    );
    assert!(lock_checks.iter().all(|check| !check.label.contains("//")));
}

/// Reproduces the false-positive MISMATCH observed in a consumer repo: a vendored/frozen
/// copy of a crate's `Cargo.toml` (e.g. under a Rustler `vendor/` tree carried for
/// offline builds) declares the same `name` as the live crate but at a stale,
/// explicit version. `cargo_manifest_versions` keys its map by name alone, so
/// without the `vendor` exclusion the vendored entry silently overwrites the live
/// one and every *other* Cargo.lock's genuinely-matching `sample` entry gets
/// compared against the wrong expected version.
#[test]
fn vendored_duplicate_package_name_does_not_poison_manifest_versions() {
    let temp = workspace("3.4.5");
    std::fs::create_dir_all(temp.path().join("vendor/sample")).expect("vendor directory");
    std::fs::write(
        temp.path().join("vendor/sample/Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"9.9.9\"\n",
    )
    .expect("vendored manifest");

    let nested = temp.path().join("packages/elixir/native/consumer");
    std::fs::create_dir_all(&nested).expect("lock directory");
    std::fs::write(
        nested.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n",
    )
    .expect("nested lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
    let sample = checks
        .iter()
        .find(|check| check.label.ends_with("consumer/Cargo.lock#sample"))
        .expect("sample check present");
    assert!(
        sample.matches,
        "live entry matching canonical must not be poisoned by the vendored copy: {sample:?}"
    );
    assert!(
        checks.iter().all(|check| !check.label.contains("vendor/")),
        "vendored Cargo.lock/Cargo.toml must not be walked at all: {checks:?}"
    );
}

/// Same mechanism as the vendor case, but for Mix's fetched-dependency cache
/// (`deps/`), which is how the consumer repo's `<crate>_nif` false positive actually
/// occurred — a Hex-fetched copy of the elixir package under `deps/` bundles its own
/// frozen `Cargo.toml` at the last-published version.
#[test]
fn deps_fetched_duplicate_package_name_does_not_poison_manifest_versions() {
    let temp = workspace("3.4.5");
    std::fs::create_dir_all(temp.path().join("test_apps/elixir/deps/sample_pkg/native/sample"))
        .expect("deps directory");
    std::fs::write(
        temp.path()
            .join("test_apps/elixir/deps/sample_pkg/native/sample/Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"1.8.1\"\n",
    )
    .expect("deps-fetched manifest");

    let nested = temp.path().join("packages/elixir/native/consumer");
    std::fs::create_dir_all(&nested).expect("lock directory");
    std::fs::write(
        nested.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n",
    )
    .expect("nested lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
    let sample = checks
        .iter()
        .find(|check| check.label.ends_with("consumer/Cargo.lock#sample"))
        .expect("sample check present");
    assert!(
        sample.matches,
        "live entry matching canonical must not be poisoned by the deps-fetched copy: {sample:?}"
    );
}

/// Guard against the lazy fix: a genuinely stale local Cargo.lock (no vendor/deps
/// involvement, a real path dependency that hasn't been `cargo update`d) must keep
/// failing even in the presence of an unrelated vendored duplicate name elsewhere.
#[test]
fn genuine_drift_outside_vendor_still_fails() {
    let temp = workspace("3.4.5");
    std::fs::create_dir_all(temp.path().join("vendor/sample")).expect("vendor directory");
    std::fs::write(
        temp.path().join("vendor/sample/Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"9.9.9\"\n",
    )
    .expect("vendored manifest");

    let nested = temp.path().join("e2e/rust");
    std::fs::create_dir_all(&nested).expect("lock directory");
    std::fs::write(
        nested.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.0\"\n",
    )
    .expect("stale nested lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
    let sample = checks
        .iter()
        .find(|check| check.label.ends_with("e2e/rust/Cargo.lock#sample"))
        .expect("sample check present");
    assert!(
        !sample.matches,
        "genuinely stale lockfile entry must still be reported as a mismatch: {sample:?}"
    );
    assert_eq!(sample.found.as_deref(), Some("3.4.0"));
    assert_eq!(
        sample.blocked_on_publish, None,
        "drift with no pending-release dependency is a chore, not a publish blocker: {sample:?}"
    );
}

/// Write a `.git` marker of the shape a linked worktree or submodule checkout
/// carries: a FILE pointing at the owning repository's gitdir.
fn write_git_marker(checkout_root: &Path, gitdir: &str) {
    std::fs::create_dir_all(checkout_root).expect("checkout root");
    std::fs::write(checkout_root.join(".git"), format!("gitdir: {gitdir}\n")).expect("git marker file");
}

/// A linked `git worktree` checked out inside the repo (e.g. under `.worktrees/`)
/// is a different checkout at a different commit, and nothing in `.gitmodules`
/// claims it as part of this repo. Its manifests must not be walked at all —
/// neither to raise mismatches of their own nor to overwrite the live entry for a
/// package name they happen to share.
#[test]
fn unregistered_nested_worktree_is_excluded_from_the_version_map() {
    let temp = workspace("3.4.5");
    std::fs::write(
        temp.path().join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n",
    )
    .expect("root lock");
    let worktree_root = temp.path().join(".worktrees/scratch-lane");
    write_git_marker(&worktree_root, "../../.git/worktrees/scratch-lane");
    std::fs::write(
        worktree_root.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"9.0.0\"\n",
    )
    .expect("worktree manifest");
    std::fs::write(
        worktree_root.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.0\"\n",
    )
    .expect("worktree lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
    assert!(
        checks.iter().all(|check| !check.label.contains(".worktrees")),
        "no check should be produced for anything under the unregistered worktree: {checks:?}"
    );
    let sample = checks
        .iter()
        .find(|check| check.label == "Cargo.lock#sample")
        .expect("root sample check present");
    assert!(
        sample.matches,
        "the live root lockfile must not be poisoned by the worktree's stale manifest: {sample:?}"
    );
}

/// The other half of the same discriminator: a submodule checkout carries the very
/// same `.git` marker file as a stray worktree, but `.gitmodules` registers its
/// path, making it a declared part of this repo's version surface. Both scans must
/// still descend into it — the `#subcrate` check can only exist if the submodule's
/// `Cargo.toml` reached the manifest map *and* its `Cargo.lock` was walked.
#[test]
fn gitmodules_registered_submodule_is_still_validated() {
    let temp = workspace("3.4.5");
    std::fs::write(
        temp.path().join(".gitmodules"),
        "[submodule \"subcrate\"]\n\tpath = libs/subcrate\n\turl = https://example.invalid/subcrate.git\n",
    )
    .expect("gitmodules");
    let submodule_root = temp.path().join("libs/subcrate");
    write_git_marker(&submodule_root, "../../.git/modules/subcrate");
    std::fs::write(
        submodule_root.join("Cargo.toml"),
        "[package]\nname = \"subcrate\"\nversion = \"3.4.5\"\n",
    )
    .expect("submodule manifest");
    std::fs::write(
        submodule_root.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"subcrate\"\nversion = \"3.4.0\"\n",
    )
    .expect("submodule lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
    let subcrate = checks
        .iter()
        .find(|check| check.label == "libs/subcrate/Cargo.lock#subcrate")
        .expect("registered submodule must still be validated");
    assert!(
        !subcrate.matches,
        "genuine drift inside a registered submodule must still be reported: {subcrate:?}"
    );
    assert_eq!(subcrate.found.as_deref(), Some("3.4.0"));
}

fn init_git_repo(root: &Path) {
    let status = crate::test_support::git_command(root)
        .args(["init", "-q"])
        .status()
        .expect("git init");
    assert!(status.success(), "git init must succeed for tracked-discovery tests");
}

fn git_add(root: &Path, relative: &str) {
    let status = crate::test_support::git_command(root)
        .args(["add", "--", relative])
        .status()
        .expect("git add");
    assert!(
        status.success(),
        "git add must succeed for tracked-discovery tests: {relative}"
    );
}

/// A workspace at `1.15.1` whose ruby native crate is committed at the canonical version and
/// whose gem-build staging tree holds a gitignored copy of the very same two manifests, frozen
/// at the previous release. This is the exact shape a consumer reported: `packages/ruby/tmp/`
/// is `tracked=no, ignored=yes`, and glob order puts it *after* the tracked original, so the
/// stale copy is the last writer into the name-keyed manifest map.
fn workspace_with_gem_staging_copies() -> TempDir {
    let temp = workspace("1.15.1");
    init_git_repo(temp.path());
    std::fs::write(temp.path().join(".gitignore"), "packages/ruby/tmp/\n").expect("gitignore");

    let native = temp.path().join("packages/ruby/ext/sample_rb/native");
    std::fs::create_dir_all(&native).expect("native directory");
    std::fs::write(
        native.join("Cargo.toml"),
        "[package]\nname = \"sample-rb\"\nversion = \"1.15.1\"\n",
    )
    .expect("native manifest");
    std::fs::write(
        native.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample-rb\"\nversion = \"1.15.1\"\n",
    )
    .expect("native lock");

    let staged = temp.path().join("packages/ruby/tmp/ruby/stage/ext/sample_rb/native");
    std::fs::create_dir_all(&staged).expect("stage directory");
    std::fs::write(
        staged.join("Cargo.toml"),
        "[package]\nname = \"sample-rb\"\nversion = \"1.15.0\"\n",
    )
    .expect("staged manifest");
    std::fs::write(
        staged.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"sample-rb\"\nversion = \"1.15.0\"\n",
    )
    .expect("staged lock");

    for relative in [
        ".gitignore",
        "Cargo.toml",
        "crates/sample/Cargo.toml",
        "crates/helper/Cargo.toml",
        "packages/ruby/ext/sample_rb/native/Cargo.toml",
        "packages/ruby/ext/sample_rb/native/Cargo.lock",
    ] {
        git_add(temp.path(), relative);
    }
    temp
}

/// Defect 1: build staging is not part of the repository's version surface. Every row it
/// produced described a stale copy of a file whose tracked original is correct.
#[test]
fn gitignored_gem_staging_tree_is_not_scanned() {
    let temp = workspace_with_gem_staging_copies();

    let checks = collect(&config(temp.path()), temp.path(), "1.15.1");

    assert!(
        checks.iter().all(|check| !check.label.contains("/tmp/")),
        "untracked build staging must not be discovered at all: {checks:?}"
    );
    assert!(
        checks
            .iter()
            .any(|check| check.label == "packages/ruby/ext/sample_rb/native/Cargo.lock#sample-rb"),
        "the tracked original must still be checked: {checks:?}"
    );
}

/// Defect 2: the staged `Cargo.toml` declares the same package name as the tracked one, and
/// `cargo_manifest_versions` keys its map by name alone, so the ignored copy decided the
/// *expected* value for the tracked lockfile — which then failed while literally reading the
/// canonical version, in a run where every other canonical row printed `ok`.
#[test]
fn manifest_reading_the_canonical_version_is_never_reported_as_a_mismatch() {
    let temp = workspace_with_gem_staging_copies();

    let checks = collect(&config(temp.path()), temp.path(), "1.15.1");

    let tracked = checks
        .iter()
        .find(|check| check.label == "packages/ruby/ext/sample_rb/native/Cargo.lock#sample-rb")
        .expect("tracked native lock check present");
    assert!(
        tracked.matches,
        "a manifest whose version equals the workspace version must pass: {tracked:?}"
    );
    assert!(
        checks
            .iter()
            .all(|check| check.found.as_deref() != Some("1.15.1") || check.matches),
        "no row that reads the canonical version may be a mismatch: {checks:?}"
    );
}

/// The enhancement: an e2e app that depends on the *published* crate at the version being
/// released cannot have its lockfile refreshed until that release exists on the registry —
/// `cargo` cannot resolve `sample = "3.4.5"` while the index tops out at `3.4.4`. The row is
/// still a mismatch, but it is a publish blocker rather than a chore somebody forgot, and the
/// output has to be able to say which.
#[test]
fn drift_blocked_on_an_unpublished_release_is_labelled_as_such() {
    let temp = workspace("3.4.5");
    let app = temp.path().join("test_apps/rust");
    std::fs::create_dir_all(&app).expect("app directory");
    std::fs::write(
        app.join("Cargo.toml"),
        "[package]\nname = \"sample-e2e-rust\"\nversion = \"3.4.5\"\n\n\
         [dependencies]\nsample_alias = { package = \"sample\", version = \"3.4.5\" }\n",
    )
    .expect("app manifest");
    std::fs::write(
        app.join("Cargo.lock"),
        "version = 4\n\n\
         [[package]]\nname = \"sample\"\nversion = \"3.4.4\"\n\
         source = \"registry+https://example.invalid/index\"\n\n\
         [[package]]\nname = \"sample-e2e-rust\"\nversion = \"3.4.0\"\n",
    )
    .expect("app lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");

    let app_check = checks
        .iter()
        .find(|check| check.label == "test_apps/rust/Cargo.lock#sample-e2e-rust")
        .expect("app lock check present");
    assert!(!app_check.matches, "the row is still a mismatch: {app_check:?}");
    assert_eq!(
        app_check.blocked_on_publish.as_deref(),
        Some("sample@3.4.5"),
        "the row must name the release it waits on: {app_check:?}"
    );
}

/// The inverse guard, so the label cannot degrade into "every mismatch is somebody else's
/// problem": the same drift, with the dependency resolved from a local path instead of the
/// registry, is refreshable today and must stay plain drift.
#[test]
fn drift_resolvable_from_a_path_dependency_is_not_labelled_unresolvable() {
    let temp = workspace("3.4.5");
    let app = temp.path().join("test_apps/rust");
    std::fs::create_dir_all(&app).expect("app directory");
    std::fs::write(
        app.join("Cargo.toml"),
        "[package]\nname = \"sample-e2e-rust\"\nversion = \"3.4.5\"\n\n\
         [dependencies]\nsample = { version = \"3.4.5\", path = \"../../crates/sample\" }\n",
    )
    .expect("app manifest");
    std::fs::write(
        app.join("Cargo.lock"),
        "version = 4\n\n\
         [[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n\n\
         [[package]]\nname = \"sample-e2e-rust\"\nversion = \"3.4.0\"\n",
    )
    .expect("app lock");

    let checks = collect(&config(temp.path()), temp.path(), "3.4.5");

    let app_check = checks
        .iter()
        .find(|check| check.label == "test_apps/rust/Cargo.lock#sample-e2e-rust")
        .expect("app lock check present");
    assert!(!app_check.matches, "the row is still a mismatch: {app_check:?}");
    assert_eq!(
        app_check.blocked_on_publish, None,
        "a lockfile cargo can refresh right now is plain drift: {app_check:?}"
    );
}

#[cfg(windows)]
#[test]
fn glob_root_keeps_the_windows_verbatim_disk_prefix_literal() {
    let root = Path::new(r"\\?\C:\workspace[one]\project");
    assert_eq!(escaped_glob_root(root), r"\\?\C:\workspace[[]one[]]\project");
}

#[cfg(not(windows))]
#[test]
fn glob_root_escapes_all_posix_metacharacters() {
    let root = Path::new("/tmp/workspace[one]?*/project");
    assert_eq!(escaped_glob_root(root), "/tmp/workspace[[]one[]][?][*]/project");
}

#[test]
fn canonical_roots_discover_exactly_the_tracked_lock_with_metacharacters() {
    let temp = TempDir::new().expect("temporary directory");
    let directory = temp.path().join("workspace[one]");
    std::fs::create_dir(&directory).expect("workspace directory");
    let root = directory.canonicalize().expect("canonical workspace root");
    #[cfg(windows)]
    assert!(matches!(
        root.components().next(),
        Some(std::path::Component::Prefix(prefix)) if prefix.kind().is_verbatim()
    ));
    init_git_repo(&root);
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"1.2.0\"\n",
    )
    .expect("manifest");
    let lock = "version = 4\n[[package]]\nname = \"sample\"\nversion = \"1.2.0\"\n";
    std::fs::write(root.join("Cargo.lock"), lock).expect("tracked lock");
    let untracked = root.join("untracked");
    std::fs::create_dir(&untracked).expect("untracked directory");
    std::fs::write(untracked.join("Cargo.lock"), lock).expect("untracked lock");
    for path in ["Cargo.toml", "Cargo.lock"] {
        git_add(&root, path);
    }
    let tracked = crate::cli::git::tracked_paths_under(&root).expect("tracked paths");
    assert_eq!(tracked.len(), 2);
    let locks = discover_cargo_locks(&root, "1.2.0", Some(&tracked));
    assert_eq!(
        locks.len(),
        1,
        "the root lock must be discovered without the untracked copy"
    );
    assert_eq!(locks[0].path, root.join("Cargo.lock"));
    assert_eq!(locks[0].packages.len(), 1);
}
