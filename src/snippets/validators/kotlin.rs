use crate::core::tool_command;
use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

mod gradle_classpath;

/// Package prefix that isolates one batched snippet's top-level declarations from every other
/// snippet's. Kotlin resolves top-level `fun`/`val` per *package*, across files, so two snippets
/// each declaring `fun main()` in the same package redeclare each other — a failure the
/// per-snippet path never had. A distinct file name is not enough; a distinct package is. ~keep
const BATCH_PACKAGE_PREFIX: &str = "alef_snippet_";

/// kotlinc diagnostic headers, as `<path>:<line>:<column>: <severity>: <message>`.
const DIAGNOSTIC_MARKERS: [(&str, DiagnosticSeverity); 2] = [
    (": error: ", DiagnosticSeverity::Error),
    (": warning: ", DiagnosticSeverity::Warning),
];

/// Column-zero, path-less lines kotlinc emits about the run as a whole (`warning: unable to find
/// kotlin-stdlib`, `error: could not find ...`). They end a diagnostic block rather than continuing
/// it, so they are not glued onto whichever snippet reported last. ~keep
const DIAGNOSTIC_TERMINATORS: [&str; 3] = ["error:", "warning:", "info:"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiagnosticSeverity {
    Error,
    Warning,
}

struct KotlinDiagnostic {
    path: String,
    severity: DiagnosticSeverity,
    text: String,
}

/// One snippet's compilation unit inside a batch. Unlike Java, Kotlin puts no constraint on a
/// file's name or location, so the file name itself is the attribution token.
struct KotlinBatchUnit {
    file_name: String,
    source: String,
}

pub struct KotlinValidator;

impl KotlinValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        if level == ValidationLevel::Run {
            return None;
        }
        let units = Self::plan_batch(snippets, level, session)?;
        Some(Self::compile_batch(&units, level, timeout_secs, session))
    }

    /// Builds one compilation unit per snippet, or declines the batch.
    ///
    /// A snippet that declares its own `package` keeps it, because rewriting a declared package
    /// would break the same-package references inside it. Two such snippets sharing a package can
    /// genuinely redeclare each other's top-level members, so the batch is declined and the runner
    /// falls back to one process per snippet, which cannot collide at all. ~keep
    fn plan_batch(
        snippets: &[&Snippet],
        level: ValidationLevel,
        session: Option<&ValidationSession>,
    ) -> Option<Vec<KotlinBatchUnit>> {
        let fingerprint = session.map(|value| value.fingerprint.as_str());
        let mut declared_packages: HashMap<String, String> = HashMap::new();
        let mut units = Vec::with_capacity(snippets.len());
        for snippet in snippets {
            let code = snippet.code.trim();
            let identifier = ValidationCache::key(snippet, level, fingerprint);
            let source = match Self::declared_package(code) {
                Some(package) => {
                    let owner = declared_packages.entry(package).or_insert_with(|| identifier.clone());
                    if owner != &identifier {
                        return None;
                    }
                    format!("{code}\n")
                }
                None => Self::with_package(code, &format!("{BATCH_PACKAGE_PREFIX}{identifier}")),
            };
            units.push(KotlinBatchUnit {
                file_name: format!("snippet_{identifier}.kt"),
                source,
            });
        }
        Some(units)
    }

    fn compile_batch(
        units: &[KotlinBatchUnit],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let dir = match session {
            Some(value) => value.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let mut sources = Vec::with_capacity(units.len());
        let mut seen = HashSet::new();
        for unit in units {
            let path = dir.path().join(&unit.file_name);
            std::fs::write(&path, &unit.source)?;
            if seen.insert(unit.file_name.clone()) {
                sources.push(path);
            }
        }
        let mut command = Self::batch_command(&sources, &dir.path().join("out"), level, timeout_secs, session)?;
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(
            units,
            level == ValidationLevel::TypeCheck,
            success,
            &output,
        ))
    }

    fn batch_command(
        sources: &[PathBuf],
        classes: &Path,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<std::process::Command> {
        let mut command = tool_command("kotlinc");
        if level == ValidationLevel::TypeCheck {
            command.arg("-Werror");
        }
        command.arg("-nowarn");
        if let Some(session) = session
            && let Some(manifest) = session.manifest.as_deref()
        {
            let class_path = gradle_classpath::resolve_class_path(manifest, session, timeout_secs)?;
            command.args(["-classpath", class_path.to_string_lossy().as_ref()]);
        }
        command.arg("-d").arg(classes).args(sources);
        if let Some(value) = session {
            value.apply(&mut command);
        }
        Ok(command)
    }

    fn batch_results(units: &[KotlinBatchUnit], warnings_fail: bool, success: bool, output: &str) -> BatchValidation {
        let (diagnostics, mut unmatched) = Self::split_diagnostics(output);
        let mut attributed = vec![Vec::<String>::new(); units.len()];
        for diagnostic in diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Warning && !warnings_fail {
                continue;
            }
            let owners = Self::owning_units(units, &diagnostic.path);
            if owners.is_empty() {
                unmatched.push(diagnostic.text);
                continue;
            }
            for owner in owners {
                attributed[owner].push(diagnostic.text.clone());
            }
        }
        let attributed_any = attributed.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !attributed_any).then(|| {
            if unmatched.is_empty() {
                "kotlinc failed without a snippet-specific diagnostic".to_string()
            } else {
                unmatched.join("\n")
            }
        });
        attributed
            .into_iter()
            .map(|messages| match (messages.is_empty(), &fallback) {
                (true, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (true, None) => (SnippetStatus::Pass, None),
                (false, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            })
            .collect()
    }

    fn split_diagnostics(output: &str) -> (Vec<KotlinDiagnostic>, Vec<String>) {
        let mut diagnostics: Vec<KotlinDiagnostic> = Vec::new();
        let mut other = Vec::new();
        let mut open = false;
        for line in output.lines() {
            if let Some((path, severity)) = Self::diagnostic_header(line) {
                diagnostics.push(KotlinDiagnostic {
                    path: path.to_owned(),
                    severity,
                    text: line.to_owned(),
                });
                open = true;
            } else if DIAGNOSTIC_TERMINATORS.iter().any(|marker| line.starts_with(marker)) {
                other.push(line.to_owned());
                open = false;
            } else if let Some(last) = diagnostics.last_mut().filter(|_| open) {
                last.text.push('\n');
                last.text.push_str(line);
            } else {
                other.push(line.to_owned());
            }
        }
        (diagnostics, other)
    }

    /// Splits `<path>:<line>:<column>: <severity>: ...`. Both position segments are stripped and
    /// checked, so a message body that happens to contain `: error: ` cannot be mistaken for a
    /// header and attribute a diagnostic to a path that is not a snippet file. ~keep
    fn diagnostic_header(line: &str) -> Option<(&str, DiagnosticSeverity)> {
        for (marker, severity) in DIAGNOSTIC_MARKERS {
            let Some(index) = line.find(marker) else {
                continue;
            };
            let (head, column) = line[..index].rsplit_once(':')?;
            let (path, number) = head.rsplit_once(':')?;
            if !path.is_empty() && Self::is_position(column) && Self::is_position(number) {
                return Some((path, severity));
            }
        }
        None
    }

    fn is_position(value: &str) -> bool {
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
    }

    fn owning_units(units: &[KotlinBatchUnit], path: &str) -> Vec<usize> {
        let Some(name) = Path::new(path).file_name().and_then(std::ffi::OsStr::to_str) else {
            return Vec::new();
        };
        units
            .iter()
            .enumerate()
            .filter(|(_, unit)| unit.file_name == name)
            .map(|(index, _)| index)
            .collect()
    }

    fn declared_package(code: &str) -> Option<String> {
        code.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("package ")?;
            Some(rest.trim().trim_end_matches(';').trim().to_owned())
        })
    }

    fn with_package(code: &str, package: &str) -> String {
        let lines: Vec<&str> = code.lines().collect();
        let insertion = lines
            .iter()
            .position(|line| Self::is_declaration_line(line))
            .unwrap_or(lines.len());
        let mut rendered = String::new();
        for line in &lines[..insertion] {
            rendered.push_str(line);
            rendered.push('\n');
        }
        rendered.push_str(&format!("package {package}\n\n"));
        rendered.push_str(&lines[insertion..].join("\n"));
        rendered.push('\n');
        rendered
    }

    /// Whether a line is real code rather than part of the leading blank/comment/`@file:` run a
    /// `package` declaration is still allowed to follow. `@file:` annotations must precede the
    /// package declaration, so inserting above them would not compile. ~keep
    fn is_declaration_line(line: &str) -> bool {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with('*')
            && !trimmed.starts_with("@file:")
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(value) => value.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let file = dir.path().join("snippet.kt");
        std::fs::write(&file, snippet.code.trim())?;
        let mut command = tool_command("kotlinc");
        if level == ValidationLevel::TypeCheck {
            command.arg("-Werror");
        }
        if level == ValidationLevel::Run {
            command.arg("-include-runtime");
        } else {
            command.arg("-nowarn");
        }
        if let Some(session) = session
            && let Some(manifest) = session.manifest.as_deref()
        {
            let class_path = gradle_classpath::resolve_class_path(manifest, session, timeout_secs)?;
            command.args(["-classpath", class_path.to_string_lossy().as_ref()]);
        }
        command
            .arg("-d")
            .arg(if level == ValidationLevel::Run {
                dir.path().join("out.jar")
            } else {
                dir.path().join("out")
            })
            .arg(&file);
        if let Some(value) = session {
            value.apply(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// Directory-probing fallback used when the manifest names no Gradle wrapper, or when a
    /// Gradle-task-based resolution (`gradle_classpath::resolve_class_path`) fails or times out.
    ///
    /// Every subdirectory name below is discovered rather than assumed. AGP's own output layout
    /// has moved twice already -- `build/tmp/kotlin-classes/<variant>`, then
    /// `build/intermediates/javac/<variant>/classes`, then
    /// `build/intermediates/built_in_kotlinc/<variant>/compile<Variant>Kotlin/classes` in AGP 9 --
    /// and a probe hardcoded to one variant name (`main`, `debug`) silently stops matching the
    /// moment a project uses any other flavor or build type, or a newer AGP relocates its output
    /// again. Globbing the variant segment instead of naming it is what keeps this fallback
    /// finding a project's own generated classes across that churn, not any particular consumer's
    /// project shape. ~keep
    fn class_path(manifest: &std::path::Path) -> Result<std::ffi::OsString> {
        if manifest.is_dir() || manifest.extension().is_some_and(|extension| extension == "jar") {
            return Ok(manifest.as_os_str().to_owned());
        }
        let root = manifest.parent().unwrap_or_else(|| std::path::Path::new("."));
        let build = root.join("build");
        let mut entries = Self::compiled_output_candidates(&build);
        entries.extend(Self::jar_candidates(&build.join("libs")));
        // A project's own `libs/` directory -- not `build/libs` -- is the conventional home for
        // vendored third-party jars referenced via `implementation files('libs/foo.jar')`. It is
        // source, not build output, so `build/` probing alone never sees it; that is what left
        // every external artifact off a fallback-resolved classpath even when the project's own
        // generated classes were found. ~keep
        entries.extend(Self::jar_candidates(&root.join("libs")));
        if entries.is_empty() {
            entries.push(root.to_path_buf());
        }
        std::env::join_paths(entries).map_err(|error| {
            crate::snippets::error::Error::Other(format!(
                "building Kotlin classpath for {}: {error}",
                manifest.display()
            ))
        })
    }

    /// This module's own compiled Kotlin/Java class output, wherever the installed AGP version (or
    /// no AGP at all) happens to have put it for whichever variant was last built.
    fn compiled_output_candidates(build: &Path) -> Vec<PathBuf> {
        let mut entries = Self::subdirectories(&build.join("classes/kotlin"));
        entries.extend(Self::subdirectories(&build.join("classes/java")));
        for variant in Self::subdirectories(&build.join("intermediates/javac")) {
            let classes = variant.join("classes");
            if classes.is_dir() {
                entries.push(classes);
            }
        }
        for variant in Self::subdirectories(&build.join("intermediates/built_in_kotlinc")) {
            for task in Self::subdirectories(&variant) {
                let classes = task.join("classes");
                if classes.is_dir() {
                    entries.push(classes);
                }
            }
        }
        entries.extend(Self::subdirectories(&build.join("tmp/kotlin-classes")));
        entries
    }

    /// Every existing immediate subdirectory of `parent`, in a stable order. A missing `parent` is
    /// not an error here -- most of these candidate roots only exist for the AGP layout and
    /// variant actually in use, and a fallback probe has to try all of them.
    fn subdirectories(parent: &Path) -> Vec<PathBuf> {
        let Ok(read_dir) = std::fs::read_dir(parent) else {
            return Vec::new();
        };
        let mut directories: Vec<PathBuf> = read_dir
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        directories.sort();
        directories
    }

    /// Every `.jar` file directly inside `directory`, in a stable order.
    fn jar_candidates(directory: &Path) -> Vec<PathBuf> {
        let Ok(read_dir) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut jars: Vec<PathBuf> = read_dir
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "jar"))
            .collect();
        jars.sort();
        jars
    }
}

impl SnippetValidator for KotlinValidator {
    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn is_available(&self) -> bool {
        which::which("kotlinc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, None)
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, session)
    }

    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        Self::validate_batch_with_context(snippets, level, timeout_secs, session)
    }

    fn requires_session_exclusivity(&self) -> bool {
        true
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::TypeCheck
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("unresolved reference") || output.contains("expecting an element")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Whether `kotlinc` runs, not merely resolves: a version-manager shim spawns fine then exits
    /// non-zero, so a PATH-only check leaves the skip below unreachable and fires the assert
    /// everywhere Kotlin is absent. ~keep
    fn kotlinc_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("kotlinc")
                .arg("-version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn session_manifest_is_used_as_a_real_classpath() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        if !kotlinc_is_runnable() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let source = root.path().join("LocalFixture.kt");
        let library = root.path().join("local-fixture.jar");
        std::fs::write(
            &source,
            "package localfixture\nobject Values { const val value: Int = 1 }\n",
        )
        .expect("Kotlin fixture source");
        let compiled = std::process::Command::new("kotlinc")
            .arg(&source)
            .args(["-d"])
            .arg(&library)
            .status();
        let compiled = match compiled {
            Ok(status) => status,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("kotlinc runs: {error}"),
        };
        assert!(compiled.success());
        let session = ValidationSession {
            language: Language::Kotlin,
            working_directory: root.path().to_path_buf(),
            manifest: Some(library),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = KotlinValidator::validate_with_context(
            &snippet("import localfixture.Values\nfun main() { println(Values.value) }"),
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    #[test]
    fn build_manifest_resolves_compiled_class_directory() {
        let project = tempfile::tempdir().expect("project directory");
        let classes = project.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = project.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");

        let class_path = KotlinValidator::class_path(&manifest).expect("classpath");
        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
    }

    /// The defect this fallback exists to close: a probe hardcoded to one AGP layout and one
    /// variant name found only whichever intermediate directory happened to match and silently
    /// dropped everything else, including the module's own newest-layout generated output and any
    /// vendored dependency jar -- a real run then classified every snippet that referenced either
    /// as an unresolved dependency, without ever having looked at a real classpath. Both a modern
    /// (AGP 9-shaped) generated-output directory and a vendored jar are asserted present, not
    /// merely that resolution returned *something*: a classpath containing only the stale
    /// `build/classes/kotlin/main` guess would satisfy a weaker assertion while still missing both.
    /// ~keep
    #[test]
    fn the_fallback_finds_a_modern_agp_output_directory_and_a_vendored_jar_together() {
        let project = tempfile::tempdir().expect("project directory");
        let generated_classes = project
            .path()
            .join("build/intermediates/built_in_kotlinc/debug/compileDebugKotlin/classes");
        std::fs::create_dir_all(&generated_classes).expect("generated classes directory");
        let vendored_directory = project.path().join("libs");
        std::fs::create_dir_all(&vendored_directory).expect("vendored libs directory");
        let vendored_jar = vendored_directory.join("external-artifact.jar");
        std::fs::write(&vendored_jar, b"").expect("vendored jar");
        let manifest = project.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");

        let class_path = KotlinValidator::class_path(&manifest).expect("classpath");

        let resolved: Vec<_> = std::env::split_paths(&class_path).collect();
        assert!(
            resolved.contains(&generated_classes),
            "the module's own generated output must be on the classpath: {resolved:?}"
        );
        assert!(
            resolved.contains(&vendored_jar),
            "a vendored external artifact must be on the classpath: {resolved:?}"
        );
    }

    /// Attribution is asserted without a toolchain so the mapping itself is pinned rather than
    /// kotlinc's willingness to run: a batch that blames one snippet for another's error is the one
    /// failure mode that makes batching worse than the per-snippet path it replaces. ~keep
    #[test]
    fn a_kotlinc_diagnostic_is_attributed_only_to_the_snippet_that_owns_its_file() {
        let units = [
            batch_unit("snippet_first.kt"),
            batch_unit("snippet_second.kt"),
            batch_unit("snippet_third.kt"),
        ];
        let output = "/scratch/snippet_second.kt:2:22: error: unresolved reference 'bogusValue'.\nfun main() { println(bogusValue) }\n                     ^^^^^^^^^^\n";

        let results = KotlinValidator::batch_results(&units, false, false, output);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(
            results[1].1.as_deref(),
            Some(
                "/scratch/snippet_second.kt:2:22: error: unresolved reference 'bogusValue'.\nfun main() { println(bogusValue) }\n                     ^^^^^^^^^^"
            )
        );
    }

    /// A kotlinc run that failed with nothing attributable must fail every snippet carrying the
    /// real output — never silently pass the batch. ~keep
    #[test]
    fn an_unattributable_kotlinc_failure_fails_every_snippet_with_the_real_output() {
        let units = [batch_unit("snippet_first.kt"), batch_unit("snippet_second.kt")];

        let results = KotlinValidator::batch_results(&units, false, false, "error: invalid argument: -nonsense\n");

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.0, SnippetStatus::Fail);
            assert_eq!(result.1.as_deref(), Some("error: invalid argument: -nonsense"));
        }
    }

    #[test]
    fn run_level_declines_batching() {
        let first = snippet("fun main() { println(\"one\") }");
        let second = snippet("fun main() { println(\"two\") }");

        let declined = KotlinValidator::validate_batch_with_context(&[&first, &second], ValidationLevel::Run, 5, None);

        assert!(declined.is_none());
    }

    #[test]
    fn a_batch_is_declined_when_two_snippets_share_a_declared_package() {
        let first = snippet("package shared.fixture\nfun alpha() = 1");
        let second = snippet("package shared.fixture\nfun beta() = 2");

        let declined =
            KotlinValidator::validate_batch_with_context(&[&first, &second], ValidationLevel::Compile, 5, None);

        assert!(declined.is_none());
    }

    #[test]
    fn a_batch_returns_exactly_one_result_per_snippet_in_input_order() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        if !kotlinc_is_runnable() {
            return;
        }
        let snippets = [
            snippet("fun main() { println(\"first\") }"),
            snippet("fun main() { println(\"second\") }"),
            snippet("fun main() { println(\"third\") }"),
            snippet("fun main() { println(\"fourth\") }"),
        ];
        let batch = snippets.iter().collect::<Vec<_>>();

        let results = KotlinValidator::validate_batch_with_context(
            &batch,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("compile level batches")
        .expect("batch validation runs");

        assert_eq!(results.len(), 4);
        for (index, result) in results.iter().enumerate() {
            assert_eq!(result.0, SnippetStatus::Pass, "snippet {index}: {:?}", result.1);
        }
    }

    /// Every snippet here declares `fun main()`. Kotlin resolves top-level declarations per package
    /// across files, so compiling them into one shared package would fail *all four* with a
    /// redeclaration error the per-snippet path never had — this asserts they pass together, which
    /// a per-snippet package is the only thing making true. ~keep
    #[test]
    fn a_batch_fails_only_the_broken_snippet() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        if !kotlinc_is_runnable() {
            return;
        }
        let snippets = [
            snippet("fun main() { println(\"first\") }"),
            snippet("fun main() { println(bogusValue) }"),
            snippet("fun main() { println(\"third\") }"),
        ];
        let batch = snippets.iter().collect::<Vec<_>>();

        let results = KotlinValidator::validate_batch_with_context(
            &batch,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("compile level batches")
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("bogusValue")),
            "the failure must name the broken snippet's own symbol: {:?}",
            results[1].1
        );
    }

    /// Two snippets declaring the same top-level `val` are the collision Kotlin's per-package
    /// resolution makes possible and the per-snippet path never had. Both must pass. ~keep
    #[test]
    fn snippets_declaring_the_same_top_level_member_do_not_collide_in_one_batch() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        if !kotlinc_is_runnable() {
            return;
        }
        let snippets = [
            snippet("val configured: Int = 1\nfun main() { println(configured) }"),
            snippet("val configured: Int = 2\nfun main() { println(configured) }"),
        ];
        let batch = snippets.iter().collect::<Vec<_>>();

        let results = KotlinValidator::validate_batch_with_context(
            &batch,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("compile level batches")
        .expect("batch validation runs");

        assert_eq!(results.len(), 2);
        for (index, result) in results.iter().enumerate() {
            assert_eq!(result.0, SnippetStatus::Pass, "snippet {index}: {:?}", result.1);
        }
    }

    #[test]
    fn an_auto_assigned_package_is_inserted_below_a_file_annotation() {
        let source = KotlinValidator::with_package("@file:JvmName(\"Example\")\nfun main() { }", "alef_snippet_x");

        assert_eq!(
            source,
            "@file:JvmName(\"Example\")\npackage alef_snippet_x\n\nfun main() { }\n"
        );
    }

    fn batch_unit(file_name: &str) -> KotlinBatchUnit {
        KotlinBatchUnit {
            file_name: file_name.to_owned(),
            source: String::new(),
        }
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.kt"),
            language: Language::Kotlin,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.kt"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
