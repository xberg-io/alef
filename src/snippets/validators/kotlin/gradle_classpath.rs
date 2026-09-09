use crate::snippets::error::{Error, Result};
use crate::snippets::session::ValidationSession;
use crate::snippets::validators::run_command;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use super::KotlinValidator;

/// Gradle init-script content that resolves a project's real Kotlin compile classpath and its own
/// compiled-class output directories, printed one per line as `ALEF_CLASSPATH_ENTRY:<path>`.
///
/// Directory probing cannot cover this: AGP's compiled-output layout is variant- and
/// version-dependent (AGP 9.x lands classes at
/// `build/intermediates/built_in_kotlinc/<variant>/compile<Variant>Kotlin/classes`; older AGP used
/// `build/tmp/kotlin-classes/<variant>`), and probing cannot see a project's *dependency*
/// classpath — kotlinx-coroutines, Jackson, etc. — at all. Asking Gradle's own task model instead
/// of guessing a path handles both, and needs no change to the consumer's build file. ~keep
///
/// The script also unions in each project's `*RuntimeClasspath` configuration(s), not just the
/// matched compile tasks' own `libraries`/`classpath` property. A compile task's classpath mirrors
/// `compileClasspath`, which by Gradle's `api`/`implementation` design only exposes a dependency's
/// `api`-scoped transitive dependencies; an `implementation`-scoped transitive dependency — a real
/// artifact the module ships with and that shows up on `runtimeClasspath` — stays invisible there.
/// A doc snippet is not bound by the module's own internal api/implementation boundary the way the
/// module's own source is, so validation needs the runtime-visible classpath too. ~keep
const INIT_SCRIPT: &str = include_str!("assets/alef_classpath_init.gradle");

const INIT_SCRIPT_FILE_NAME: &str = "alef_classpath_init.gradle";
const CLASS_PATH_TASK: &str = "alefPrintClasspath";
const CLASS_PATH_ENTRY_PREFIX: &str = "ALEF_CLASSPATH_ENTRY:";
/// Printed once per matched `compile*Kotlin` task, independently of whether that task contributed
/// any classpath entries. Lets `run_gradle_class_path` tell a small-but-real classpath apart from
/// one where a task matched but its `libraries`/`classpath` property resolved to nothing -- see the
/// completeness check below. ~keep
const CLASS_PATH_TASK_MARKER_PREFIX: &str = "ALEF_CLASSPATH_TASK:";
const MANIFEST_NAMES: [&str; 2] = ["build.gradle.kts", "build.gradle"];

#[cfg(unix)]
const WRAPPER_NAME: &str = "gradlew";
#[cfg(windows)]
const WRAPPER_NAME: &str = "gradlew.bat";

type ClassPathCache = Mutex<HashMap<PathBuf, std::result::Result<OsString, String>>>;

/// Process-wide, keyed by the manifest's canonicalized path. A Gradle invocation costs whole
/// seconds even with a warm daemon, and batch validation resolves a session's classpath once per
/// batch — dozens of times in one `alef snippets check` run — so the first resolution is reused
/// for the rest of the process. ~keep
static CACHE: OnceLock<ClassPathCache> = OnceLock::new();

/// Resolves a Kotlin session's classpath: Gradle's own configuration model for a Gradle project
/// with a wrapper, directory probing (`KotlinValidator::class_path`) otherwise. A Gradle
/// invocation that fails falls back to probing too rather than failing the whole session —
/// probing can still find the project's own compiled output even when dependency resolution
/// cannot run (offline, misconfigured repository, ...); the failure is logged, not swallowed. ~keep
pub(super) fn resolve_class_path(manifest: &Path, session: &ValidationSession, timeout_secs: u64) -> Result<OsString> {
    if is_gradle_manifest(manifest) {
        match gradle_class_path(manifest, session, timeout_secs) {
            Some(Ok(class_path)) => return Ok(class_path),
            Some(Err(error)) => {
                tracing::warn!(
                    manifest = %manifest.display(),
                    error = %error,
                    "Gradle classpath resolution failed; falling back to directory probing"
                );
            }
            None => {}
        }
    }
    KotlinValidator::class_path(manifest)
}

fn is_gradle_manifest(manifest: &Path) -> bool {
    manifest
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| MANIFEST_NAMES.contains(&name))
}

fn gradle_wrapper(root: &Path) -> Option<PathBuf> {
    let wrapper = root.join(WRAPPER_NAME);
    wrapper.is_file().then_some(wrapper)
}

/// `None` when the project has no wrapper to invoke, so the caller falls straight through to
/// directory probing without a wasted attempt.
fn gradle_class_path(manifest: &Path, session: &ValidationSession, timeout_secs: u64) -> Option<Result<OsString>> {
    let root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let wrapper = gradle_wrapper(root)?;
    let key = std::fs::canonicalize(manifest).unwrap_or_else(|_| manifest.to_path_buf());
    let cache = CACHE.get_or_init(ClassPathCache::default);
    if let Some(cached) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return Some(cached.clone().map_err(Error::Other));
    }
    let resolved = run_gradle_class_path(&wrapper, root, session, timeout_secs);
    let stored = match &resolved {
        Ok(class_path) => Ok(class_path.clone()),
        Err(error) => Err(error.to_string()),
    };
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(key, stored);
    Some(resolved)
}

fn run_gradle_class_path(
    wrapper: &Path,
    root: &Path,
    session: &ValidationSession,
    timeout_secs: u64,
) -> Result<OsString> {
    let script = session.workspace_directory()?.join(INIT_SCRIPT_FILE_NAME);
    std::fs::write(&script, INIT_SCRIPT)?;
    let mut command = std::process::Command::new(wrapper);
    command
        .current_dir(root)
        // A closed stdin, not an inherited terminal, is what keeps this child from ever being able
        // to trigger a job-control stop: `run_command` puts it in its own background process
        // group, and a background-group process that reads its controlling terminal is stopped by
        // `SIGTTIN` rather than erroring -- observed in practice as the Gradle wrapper's JVM
        // sitting in kernel state `T` forever, immune to `SIGCONT` because it re-attempts the same
        // read the instant it resumes. Nothing this classpath resolution does legitimately needs
        // to read stdin. ~keep
        .stdin(std::process::Stdio::null())
        .args(["--init-script", &script.to_string_lossy(), CLASS_PATH_TASK, "-q"]);
    let (success, output) = run_command(&mut command, timeout_secs)?;
    let entries: Vec<PathBuf> = output
        .lines()
        .filter_map(|line| line.strip_prefix(CLASS_PATH_ENTRY_PREFIX))
        .map(PathBuf::from)
        .collect();
    if !success || entries.is_empty() {
        // `entries` is parsed from the full output above; only the reported prose is bounded. ~keep
        return Err(Error::Other(format!(
            "resolving Gradle classpath for {}: {}",
            root.display(),
            crate::snippets::diagnostics::bounded_text(&output)
        )));
    }
    let matched_tasks = output
        .lines()
        .filter(|line| line.starts_with(CLASS_PATH_TASK_MARKER_PREFIX))
        .count();
    // A matched task always contributes at least its own destination directory, so a resolution
    // that found `n` compile tasks but no more than `n` total entries got zero library files across
    // every one of them -- kotlin-stdlib alone rules that out for any real compilation. Accepting
    // this silently is exactly the "check that passed because it examined nothing" shape: the entry
    // list looks non-empty and downstream code has no way to tell it apart from a project that
    // genuinely has few dependencies. Treat it as a failed resolution so the caller falls back to
    // directory probing instead of compiling snippets against a classpath missing every dependency,
    // transitive or not. ~keep
    if matched_tasks > 0 && entries.len() <= matched_tasks {
        return Err(Error::Other(format!(
            "resolving Gradle classpath for {}: matched {matched_tasks} Kotlin compile task(s) but \
             resolved only {} classpath entries -- no dependency artifacts were found for any of them, \
             which is implausible for a real Kotlin compilation",
            root.display(),
            entries.len()
        )));
    }
    existing_class_path(root, entries)
}

fn existing_class_path(root: &Path, entries: Vec<PathBuf>) -> Result<OsString> {
    // Gradle reports destinations for unbuilt variants as well as resolved artifacts. Passing those
    // absent outputs to kotlinc triggers warnings under -Werror before checking snippet types. ~keep
    let existing: Vec<_> = entries.into_iter().filter(|entry| entry.exists()).collect();
    if existing.is_empty() {
        return Err(Error::Other(format!(
            "resolving Gradle classpath for {}: no existing classpath entries",
            root.display()
        )));
    }
    std::env::join_paths(existing)
        .map_err(|error| Error::Other(format!("building Gradle classpath for {}: {error}", root.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::Language;
    use std::collections::BTreeMap;

    /// The fixtures below are trivial shell scripts that echo a fixed line and exit, so a real
    /// resolution takes milliseconds. The bound only exists so a genuinely hung wrapper cannot
    /// wedge the suite; it is generous because process spawn under a fully parallel `cargo test`
    /// run has been observed to exceed a tight bound and fail the success-path assertions. ~keep
    const TEST_TIMEOUT_SECS: u64 = 120;

    fn session(working_directory: &Path, fingerprint: &str) -> ValidationSession {
        ValidationSession {
            language: Language::Kotlin,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            fingerprint: fingerprint.to_owned(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn only_gradle_manifest_file_names_are_recognized() {
        let cases = [
            ("build.gradle.kts", true),
            ("build.gradle", true),
            ("pubspec.yaml", false),
            ("Package.swift", false),
            ("local-fixture.jar", false),
        ];
        for (name, expected) in cases {
            assert_eq!(
                is_gradle_manifest(Path::new(name)),
                expected,
                "manifest file name {name}"
            );
        }
    }

    #[test]
    fn gradle_wrapper_is_found_only_when_present_at_the_project_root() {
        let root = tempfile::tempdir().expect("project root");

        assert!(gradle_wrapper(root.path()).is_none());

        std::fs::write(root.path().join(WRAPPER_NAME), "").expect("wrapper stub");
        assert_eq!(gradle_wrapper(root.path()), Some(root.path().join(WRAPPER_NAME)));
    }

    #[test]
    fn a_gradle_manifest_without_a_wrapper_falls_back_to_directory_probing() {
        let root = tempfile::tempdir().expect("project root");
        let classes = root.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "no-wrapper-fixture");

        let class_path = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("classpath resolves");

        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, contents).expect("script contents");
        let mut permissions = std::fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("script permissions");
    }

    #[cfg(unix)]
    #[test]
    fn a_gradle_wrapper_invocation_is_parsed_and_cached_across_repeated_resolutions() {
        let root = tempfile::tempdir().expect("project root");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        let invocation_log = root.path().join("invocations");
        let libraries = [root.path().join("one.jar"), root.path().join("two.jar")];
        for library in &libraries {
            std::fs::write(library, "fixture").expect("dependency artifact");
        }
        let wrapper = reported_paths(root.path(), &libraries);
        let mut script = std::fs::read_to_string(&wrapper).expect("wrapper script");
        script.push_str(&format!("echo invoked >> '{}'\n", invocation_log.display()));
        write_executable(&wrapper, &script);
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "wrapper-fixture");

        let first = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("first resolution");
        let second = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("second resolution");

        let expected = std::env::join_paths(libraries).expect("expected classpath");
        assert_eq!(first, expected);
        assert_eq!(second, expected);
        let invocations = std::fs::read_to_string(&invocation_log).unwrap_or_default();
        assert_eq!(
            invocations.lines().count(),
            1,
            "the wrapper must run once and be served from cache thereafter: {invocations:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_gradle_wrapper_falls_back_to_directory_probing_instead_of_failing_the_session() {
        let root = tempfile::tempdir().expect("project root");
        let classes = root.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        write_executable(
            &root.path().join(WRAPPER_NAME),
            "#!/bin/sh\necho 'error: could not resolve dependencies' >&2\nexit 1\n",
        );
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "failing-wrapper-fixture");

        let class_path = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("falls back to probing");

        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
    }

    /// Reproduces the reported defect directly: a Gradle response that matched real compile tasks
    /// but resolved no dependency artifacts for any of them (only their own destination
    /// directories) -- exactly what a task whose `libraries`/`classpath` property silently resolved
    /// to nothing would report. Before the completeness check this was accepted at face value
    /// (`entries.is_empty()` is false), so every snippet touching a transitive dependency compiled
    /// against a classpath that never had it. This must now be rejected and fall back to directory
    /// probing, which at least finds the project's own compiled output. ~keep
    #[cfg(unix)]
    #[test]
    fn a_gradle_response_with_matched_tasks_but_no_dependency_entries_is_rejected_as_incomplete() {
        let root = tempfile::tempdir().expect("project root");
        let classes = root.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        write_executable(
            &root.path().join(WRAPPER_NAME),
            "#!/bin/sh\necho \"ALEF_CLASSPATH_TASK:compileKotlin\"\necho \"ALEF_CLASSPATH_ENTRY:/fake/build/classes/kotlin/main\"\n",
        );
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "incomplete-wrapper-fixture");

        let class_path = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("falls back to probing");

        let resolved: Vec<_> = std::env::split_paths(&class_path).collect();
        assert_eq!(
            resolved,
            vec![classes],
            "the one-entry-per-task Gradle response must be discarded in favor of real directory \
             probing, not returned as-is: {resolved:?}"
        );
    }

    /// The positive counterpart of the completeness check above: a response with more entries than
    /// matched tasks (real dependency jars beyond each task's own destination directory) must be
    /// accepted and used as-is, not rejected as a false positive. ~keep
    #[cfg(unix)]
    #[test]
    fn a_gradle_response_with_dependency_entries_beyond_matched_tasks_is_accepted() {
        let root = tempfile::tempdir().expect("project root");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        let entries = [
            root.path().join("classes"),
            root.path().join("direct.jar"),
            root.path().join("transitive.jar"),
        ];
        std::fs::create_dir(&entries[0]).expect("classes directory");
        for library in &entries[1..] {
            std::fs::write(library, "fixture").expect("dependency artifact");
        }
        let wrapper = reported_paths(root.path(), &entries);
        let mut script = std::fs::read_to_string(&wrapper).expect("wrapper script");
        script.push_str("echo 'ALEF_CLASSPATH_TASK:compileKotlin'\n");
        write_executable(&wrapper, &script);
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "complete-wrapper-fixture");

        let class_path = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("resolves from Gradle");

        let expected = std::env::join_paths(entries).expect("expected classpath");
        assert_eq!(class_path, expected);
    }

    #[cfg(unix)]
    fn reported_paths(root: &Path, entries: &[PathBuf]) -> PathBuf {
        let wrapper = root.join(WRAPPER_NAME);
        let mut script = String::from("#!/bin/sh\n");
        for entry in entries {
            script.push_str(&format!("printf '%s\\n' 'ALEF_CLASSPATH_ENTRY:{}'\n", entry.display()));
        }
        write_executable(&wrapper, &script);
        wrapper
    }

    #[cfg(unix)]
    #[test]
    fn missing_variant_outputs_are_excluded_without_dropping_existing_dependencies() {
        let root = tempfile::tempdir().expect("project root");
        let classes = root.path().join("classes");
        let library = root.path().join("dependency.jar");
        std::fs::create_dir(&classes).expect("classes directory");
        std::fs::write(&library, "fixture").expect("dependency artifact");
        let wrapper = reported_paths(
            root.path(),
            &[
                classes.clone(),
                root.path().join("unbuilt/classes"),
                library.clone(),
                root.path().join("unbuilt/R.jar"),
            ],
        );
        let resolved = run_gradle_class_path(&wrapper, root.path(), &session(root.path(), "mixed"), TEST_TIMEOUT_SECS)
            .expect("existing dependency paths resolve");
        assert_eq!(
            std::env::split_paths(&resolved).collect::<Vec<_>>(),
            vec![classes, library]
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_entirely_missing_gradle_classpath_is_rejected() {
        let root = tempfile::tempdir().expect("project root");
        let wrapper = reported_paths(root.path(), &[root.path().join("missing.jar")]);
        let error = run_gradle_class_path(
            &wrapper,
            root.path(),
            &session(root.path(), "missing"),
            TEST_TIMEOUT_SECS,
        )
        .expect_err("nonexistent classpath cannot validate dependencies");
        assert!(error.to_string().contains("no existing classpath entries"), "{error}");
    }

    #[cfg(unix)]
    fn compile_fixture(source: &Path, library: &Path, classpath: Option<&std::ffi::OsStr>) -> (bool, String) {
        let mut command = std::process::Command::new("kotlinc");
        command.arg(source).arg("-Werror").arg("-d").arg(library);
        if let Some(classpath) = classpath {
            command.arg("-classpath").arg(classpath);
        }
        run_command(&mut command, TEST_TIMEOUT_SECS).expect("real Kotlin compiler executes")
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "requires installed kotlinc and JVM; run explicitly with --include-ignored"]
    fn filtered_gradle_classpath_preserves_real_type_and_dependency_checks() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        let root = tempfile::tempdir().expect("project root");
        let source = root.path().join("Library.kt");
        let library = root.path().join("library.jar");
        std::fs::write(
            &source,
            "package fixture\nobject Values { const val value: Int = 37 }\n",
        )
        .expect("library source");
        let (success, output) = compile_fixture(&source, &library, None);
        assert!(success, "real dependency compiles: {output}");
        let wrapper = reported_paths(root.path(), &[root.path().join("unbuilt/classes"), library]);
        let classpath = run_gradle_class_path(&wrapper, root.path(), &session(root.path(), "real"), TEST_TIMEOUT_SECS)
            .expect("classpath resolves");
        let cases = [
            ("val result: Int = fixture.Values.value", true, ""),
            ("val result: String = fixture.Values.value", false, "type mismatch"),
            ("val result = absent.Required.value", false, "unresolved reference"),
        ];
        for (code, expected, diagnostic) in cases {
            std::fs::write(&source, code).expect("snippet source");
            let (success, output) = compile_fixture(&source, &root.path().join("snippet.jar"), Some(&classpath));
            assert_eq!(success, expected, "{code}: {output}");
            assert!(
                output.to_lowercase().contains(diagnostic),
                "expected {diagnostic}: {output}"
            );
        }
    }

    /// The init script's compile-task match must use `=~` (find), not `==~` (full string match).
    /// Kotlin Multiplatform names its per-target compile tasks with the target *after* `Kotlin`
    /// (`compileKotlinJvm`, `compileKotlinIosArm64`, ...); a full-string match against
    /// `compile.*Kotlin` silently excludes every one of them, dropping any dependency -- transitive
    /// or not -- reachable only through that target's classpath. This cannot be exercised without a
    /// live Gradle + Kotlin Multiplatform project, so this pins the operator in the shipped asset
    /// instead: reverting it to `==~` would pass every other test in this file untouched while
    /// silently reintroducing the completeness gap. ~keep
    #[test]
    fn the_init_script_matches_compile_tasks_by_find_not_full_string_match() {
        assert!(
            INIT_SCRIPT.contains("it.name =~ /(?i)compile.*Kotlin/"),
            "the compile-task match must use `=~` (find) so multiplatform per-target task names \
             like `compileKotlinJvm` still match"
        );
        assert!(
            !INIT_SCRIPT.contains("it.name ==~"),
            "a full-string match (`==~`) would exclude Kotlin Multiplatform per-target compile task \
             names, silently dropping their classpaths"
        );
    }

    /// Pins the other half of the completeness check: the init script must actually emit the task
    /// marker `run_gradle_class_path` counts, for every matched task unconditionally (outside the
    /// try/catch blocks that guard the destination-directory and classpath lookups). Without this
    /// marker, `matched_tasks` is always zero and the Rust-side completeness check above can never
    /// fire regardless of what a real Gradle run returns. ~keep
    #[test]
    fn the_init_script_prints_a_task_marker_for_every_matched_compile_task() {
        assert!(
            INIT_SCRIPT.contains("println \"ALEF_CLASSPATH_TASK:\" + compileTask.name"),
            "every matched compile task must print its marker unconditionally, before either \
             try/catch block runs"
        );
    }

    /// A compile task's own `libraries`/`classpath` property mirrors `compileClasspath`, which by
    /// Gradle's `api`/`implementation` design excludes an `implementation`-scoped transitive
    /// dependency of a directly-declared library — even though that dependency is a real artifact
    /// the module ships with and that appears on `runtimeClasspath`. That is a "one category of
    /// dependency missing" gap, not the "matched a task but got nothing" shape the completeness
    /// check above catches: most classpath entries are still present, so `entries.len() >
    /// matched_tasks` and the check never fires, yet snippets exercising the runtime-only-visible
    /// dependency still fail as an unresolved reference. This cannot be exercised without a live
    /// Gradle + Android project (compile vs. runtime classpath visibility is a real dependency-graph
    /// distinction, not something a fake shell-script wrapper can fake), so this pins the fix in the
    /// shipped asset instead: reverting it would pass every other test in this file untouched while
    /// silently reintroducing the gap. ~keep
    #[test]
    fn the_init_script_also_unions_in_the_runtime_classpath() {
        assert!(
            INIT_SCRIPT.contains("endsWith('runtimeclasspath')"),
            "the runtime classpath configuration(s) must be located generically by name suffix, not \
             a hardcoded variant such as `debugRuntimeClasspath`, so this covers Android's \
             per-variant configurations, a plain JVM project's `runtimeClasspath`, and Kotlin \
             Multiplatform's per-target ones alike"
        );
        assert!(
            INIT_SCRIPT.contains("viewConfiguration.lenient(true)"),
            "runtime classpath resolution must be lenient so one unresolvable artifact cannot \
             silently drop every other real dependency the configuration would otherwise report"
        );
        assert!(
            INIT_SCRIPT.contains("if (matchedAnyTask)"),
            "runtime classpath resolution must be scoped to projects that actually have a matched \
             Kotlin compile task, not run unconditionally across every project in the build"
        );
    }
}
