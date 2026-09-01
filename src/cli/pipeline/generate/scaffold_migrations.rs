use crate::core::backend::GeneratedFile;
use anyhow::Context as _;
use std::path::Path;

/// Every in-place repair applied to a pre-existing, create-once (`generated_header: false`)
/// scaffold file this run's `files` set still names.
///
/// Called once from [`super::write_scaffold_files_report`], after its per-file write loop and
/// before it returns. Each repair below targets a file the write-guard permanently refuses to
/// overwrite once it exists on disk (a markable extension with no marker, or one that cannot
/// carry a marker at all), so a generator fix to that file's content can never reach an
/// already-scaffolded repo through the normal write path -- these self-contained, independently
/// idempotent patches are the only way such a fix ever lands there. See each individual
/// `crate::scaffold::migrate_*` call's own doc for the specific defect it closes and why an
/// in-place text patch, never a full regenerate-and-overwrite, is the only safe shape for a file
/// a consumer may have hand-edited. Each helper below is called in a fixed, load-bearing order --
/// see `migrate_zig_build_config`'s doc for the one documented ordering dependency. ~keep
pub(super) fn apply_pending_migrations(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    migrate_zig_build_config(files, base_dir)?;
    migrate_dart_placeholder_test_file(files, base_dir)?;
    migrate_swift_placeholder_test_file(files, base_dir)?;
    migrate_dart_pubignore_file(files, base_dir)?;
    migrate_wasm_package_json_file(files, base_dir)?;
    migrate_node_package_json_service_file(files, base_dir)?;
    migrate_zig_example_file(files, base_dir)?;
    migrate_kotlin_build_gradle_file(files, base_dir)?;
    migrate_php_composer_files(files, base_dir)?;
    migrate_java_checkstyle_file(files, base_dir)?;
    migrate_wasm_cargo_config_unconditional(base_dir)?;
    migrate_poly_toml_unconditional(base_dir)?;

    Ok(())
}

// `packages/zig/build.zig` is a `generated_header: false` seed on a markable (`.zig`)
// extension, so the ownership guard above permanently refuses to overwrite it once it
// exists -- by design, since consumers legitimately hand-edit it. A generator fix to its
// content therefore never reaches an existing repo through the normal write path at all,
// whatever `overwrite` says, so the one known-bad shape (test module compiling the
// generated `src/<module>.zig`, which carries zero `test` blocks) is repaired in place
// here instead. Runs AFTER the write loop, not before: the repair repoints the test
// module at `test/<module>_test.zig`, which the same batch seeds create-only, and a repo
// can legitimately have the bad `build.zig` with no `test/` directory at all
// (a consumer repo is in exactly that state). Repairing first would leave any run that
// failed between the two steps pointing at a nonexistent root source file -- trading
// silent coverage loss for a build graph that will not resolve. ~keep
fn migrate_zig_build_config(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(build_zig) = files
        .iter()
        .find(|file| file.path == Path::new("packages/zig/build.zig"))
    {
        crate::scaffold::migrate_build_zig_test_target(base_dir)
            .context("failed to migrate pre-existing packages/zig/build.zig test target")?;
        // Same reachability gap, same file: a `build.zig` seeded before `scaffold_zig` derived
        // the FFI crate directory from `[crates.output] ffi` still searches the directory
        // guessed from the crate name, so every `@cInclude` in the binding fails to resolve.
        // The corrected default is read out of this run's freshly generated content rather
        // than re-derived from config, so the two can never disagree. ~keep
        crate::scaffold::migrate_zig_build_ffi_include_default(base_dir, &build_zig.content)
            .context("failed to migrate pre-existing packages/zig/build.zig ffi include default")?;
    }
    Ok(())
}

// Same reachability gap, same shape of fix, for Dart: `packages/dart/test/*_test.dart` is
// also `generated_header: false` on a markable (`.dart`) extension, so the vacuous
// `expect(1 + 1, equals(2))` placeholder this scaffold used to always emit can never be
// replaced by `scaffold_dart_test`'s real assertion through the normal write path either,
// on any pre-existing repo. Unlike the zig repair, this one only ever fires when the
// on-disk file still matches the *exact* old placeholder shape byte-for-byte -- see
// `migrate_dart_placeholder_test`'s doc -- so a hand-written suite is never at risk. This
// run's freshly generated content for that path (already computed by `scaffold_dart_test`
// above, using the real API surface) is what gets written when the shape matches. ~keep
fn migrate_dart_placeholder_test_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(dart_test_file) = files.iter().find(|file| {
        file.path.starts_with("packages/dart/test")
            && file
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_test.dart"))
    }) {
        crate::scaffold::migrate_dart_placeholder_test(base_dir, &dart_test_file.path, &dart_test_file.content)
            .context("failed to migrate pre-existing packages/dart/test/*_test.dart placeholder")?;
    }
    Ok(())
}

// Same reachability gap again, for Swift: `packages/swift/Tests/<Name>Tests/<Name>Tests.swift`
// is `generated_header: false` on a markable (`.swift`) extension, so the vacuous
// `XCTAssertTrue(true)` placeholder this scaffold used to always emit can never be replaced
// by `scaffold_swift_test`'s real assertion through the normal write path on any
// pre-existing repo (a consumer repo is in exactly that state). Fires only on the vacuity
// signature -- one `XCTAssert`-family call, one `func test`, and that call is the tautology
// -- so a hand-written suite is never at risk; see `migrate_swift_placeholder_test`'s doc.
// This run's freshly generated content for that path (already computed by
// `scaffold_swift_test` above, against the real API surface) is what gets written when the
// signature matches.
//
// Singular by construction, so `find` cannot silently skip a second candidate: this
// function's `files` come from one `crate::scaffold::scaffold(api, config, languages)` call
// for a single crate, and `scaffold_swift` emits exactly one `Tests/<module>Tests` file per
// call (`module` is the crate's one `config.swift_module()`). A multi-crate workspace runs
// this whole path once per crate, each with its own distinct module directory. Placed after
// the write loop for the same reason as the zig repair. ~keep
fn migrate_swift_placeholder_test_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(swift_test_file) = files.iter().find(|file| {
        file.path.starts_with("packages/swift/Tests")
            && file
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("Tests.swift"))
    }) {
        crate::scaffold::migrate_swift_placeholder_test(base_dir, &swift_test_file.path, &swift_test_file.content)
            .context("failed to migrate pre-existing packages/swift/Tests/*Tests.swift placeholder")?;
    }
    Ok(())
}

// `packages/dart/.pubignore` is `generated_header: false` on a markable (`.pubignore` is not
// a `CommentStyle`-recognised extension, but see `migrate_dart_pubignore`'s doc for why an
// exact byte match is still safe here). A repo scaffolded before the fix that stopped
// excluding native FFI libraries from the pub.dev tarball keeps silently stripping them from
// every release; see `migrate_dart_pubignore`'s doc for the full defect. ~keep
fn migrate_dart_pubignore_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(pubignore_file) = files
        .iter()
        .find(|file| file.path == Path::new("packages/dart/.pubignore"))
    {
        crate::scaffold::migrate_dart_pubignore(base_dir, &pubignore_file.path, &pubignore_file.content)
            .context("failed to migrate pre-existing packages/dart/.pubignore")?;
    }
    Ok(())
}

// `crates/<crate>-wasm/package.json` is `generated_header: false`; a repo scaffolded before a
// later scaffold_wasm fix keeps shipping whatever defect that fix closed forever. Each repair
// this bundles is self-contained and independently idempotent; see `migrate_wasm_package_json`'s
// doc. ~keep
fn migrate_wasm_package_json_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(wasm_pkg_file) = files.iter().find(|file| {
        file.path
            .to_str()
            .is_some_and(|path| path.ends_with("-wasm/package.json"))
    }) {
        crate::scaffold::migrate_wasm_package_json(base_dir, &wasm_pkg_file.path)
            .context("failed to migrate pre-existing crates/*-wasm/package.json")?;
    }
    Ok(())
}

// `crates/<crate>-node/package.json` (the main napi-rs package, not the per-platform
// `npm/<platform>/package.json` sub-packages) is `generated_header: false`; a service-API
// crate scaffolded before the fix that exposed a `./service` subpath keeps shipping
// `service.cjs` unreachable via `require`/`import`. Matched by parent-directory name (ending
// in `-node`) rather than the bare filename, so the platform sub-package manifests --
// nested one level deeper under `npm/<platform>/` -- are never candidates. ~keep
fn migrate_node_package_json_service_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(node_pkg_file) = files.iter().find(|file| {
        file.path.file_name() == Some(std::ffi::OsStr::new("package.json"))
            && file
                .path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-node"))
    }) {
        crate::scaffold::migrate_node_package_json_service_export(base_dir, &node_pkg_file.path)
            .context("failed to migrate pre-existing crates/*-node/package.json service export")?;
    }
    Ok(())
}

// `packages/zig/examples/example.zig` is `generated_header: false`; a repo scaffolded before
// the Zig 0.16 rewrite (`cc7f824b0`) keeps shipping an example that no longer compiles under
// the pinned toolchain. Fires only on an exact match against the one known pre-0.16 shape --
// see `migrate_zig_example`'s doc. ~keep
fn migrate_zig_example_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if let Some(zig_example_file) = files
        .iter()
        .find(|file| file.path == Path::new("packages/zig/examples/example.zig"))
    {
        crate::scaffold::migrate_zig_example(base_dir, &zig_example_file.path, &zig_example_file.content)
            .context("failed to migrate pre-existing packages/zig/examples/example.zig")?;
    }
    Ok(())
}

// `packages/kotlin/build.gradle.kts` is `generated_header: false`; a repo scaffolded before
// either of the two independent fixes (`srcDir(".")` output-overlap breaking
// `publishToMavenCentral`, the missing mavenPublishing trailing comma churning against
// ktlint) keeps carrying one or both. Self-contained (no `replacement` needed, the file's own
// path is fixed); see `migrate_kotlin_build_gradle`'s doc. ~keep
fn migrate_kotlin_build_gradle_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if files
        .iter()
        .any(|file| file.path == Path::new("packages/kotlin/build.gradle.kts"))
    {
        crate::scaffold::migrate_kotlin_build_gradle(base_dir)
            .context("failed to migrate pre-existing packages/kotlin/build.gradle.kts")?;
    }
    Ok(())
}

// `composer.json` (root and/or `{pkg_dir}`) is `generated_header: false`; a repo scaffolded
// before `ddde77260` ("widen the scaffolded PHPUnit constraint to the declared PHP floor")
// keeps a `phpunit/phpunit` constraint that cannot resolve against the declared PHP >=8.2
// floor on 8.2/8.3. Run over every emitted composer.json path this run (there are at most
// two -- root and package-dir, see `scaffold_php`), each independently guarded by
// `migrate_php_composer_phpunit_constraint`'s own exact-match + php-ext marker check. ~keep
fn migrate_php_composer_files(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    for composer_file in files.iter().filter(|file| {
        file.path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "composer.json")
    }) {
        crate::scaffold::migrate_php_composer_phpunit_constraint(base_dir, &composer_file.path)
            .context("failed to migrate pre-existing composer.json phpunit constraint")?;
    }
    Ok(())
}

// `packages/java/checkstyle.xml` is `generated_header: false`; a repo scaffolded before
// either LineLength bump (`a95defbf5` 120->140, `6382afdf6` 140->200) fails `mvn verify` on
// every alef-emitted FFM call shim that needs more columns than the stale ceiling allows.
// Self-contained; see `migrate_java_checkstyle_line_length`'s doc. ~keep
fn migrate_java_checkstyle_file(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<()> {
    if files
        .iter()
        .any(|file| file.path == Path::new("packages/java/checkstyle.xml"))
    {
        crate::scaffold::migrate_java_checkstyle_line_length(base_dir, Path::new("packages/java/checkstyle.xml"))
            .context("failed to migrate pre-existing packages/java/checkstyle.xml LineLength ceiling")?;
    }
    Ok(())
}

// `.cargo/config.toml`'s wasm-only fallback (no `[scaffold.cargo]` configured) is unusual:
// `scaffold()` only pushes it into `files` when the path does *not already exist*, so once a
// repo has one it drops out of `files` entirely and this can never be gated the way every
// migration above is (on the file's presence in this run's `files`). The migrator is
// therefore called unconditionally on every run; it is self-guarding via an exact byte match
// against the one known pre-fix constant, so it is a no-op on any non-wasm project, any
// `[scaffold.cargo]`-driven config, or a file that doesn't exist at all. ~keep
fn migrate_wasm_cargo_config_unconditional(base_dir: &Path) -> anyhow::Result<()> {
    crate::scaffold::migrate_wasm_cargo_config_allow_multiple_definition(base_dir)
        .context("failed to migrate pre-existing .cargo/config.toml wasm32 rustflags")?;
    Ok(())
}

// `poly.toml`'s managed merge unions and prunes array values but never retracts a whole
// table alef stops emitting, so this repairs the known stale tables left behind. Called
// unconditionally, self-guarding like the repair above -- see
// `migrate_poly_toml_drop_snippet_hook`'s doc for the full defect, and
// `migrate_poly_toml_drop_unrunnable_snapshot_hooks`'s doc for the second, independent
// instance of the same defect (the `rubocop`/`steep`/`dart-analyze`/`dart-e2e-analyze`
// hooks `8ed9ad8d4` retracted from generation without a matching repair). ~keep
fn migrate_poly_toml_unconditional(base_dir: &Path) -> anyhow::Result<()> {
    crate::scaffold::migrate_poly_toml_drop_snippet_hook(base_dir)
        .context("failed to migrate pre-existing poly.toml alef-snippets pre-commit hook")?;
    crate::scaffold::migrate_poly_toml_drop_unrunnable_snapshot_hooks(base_dir).context(
        "failed to migrate pre-existing poly.toml rubocop/steep/dart-analyze/dart-e2e-analyze pre-commit hooks",
    )?;
    Ok(())
}
