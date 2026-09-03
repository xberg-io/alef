use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct GoValidator;

const ISOLATED_GO_MODULE: &str = "module snippet\n\ngo 1.21\n";
const BATCH_FILE_NAME: &str = "snippet.go";

/// Every snippet is its own `package main` with its own `func main`, so they cannot share one
/// package directory — `go build` would reject the batch with `main redeclared in this block`
/// before compiling any snippet's actual code. Each snippet gets a directory of its own under a
/// single module instead, and one `go build ./...` covers them all. ~keep
const BATCH_DIRECTORY_PREFIX: &str = "snippet_batch_";

const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the go toolchain failed without a snippet-specific diagnostic";
const BATCH_RUN_UNSUPPORTED: &str = "Go batch validation does not cover the run level";

impl GoValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let dir = match session {
            Some(session) => ScratchDir::for_session(session)?,
            None => ScratchDir::isolated()?,
        };
        let mut files = Vec::with_capacity(snippets.len());
        let mut directories = Vec::with_capacity(snippets.len());
        for (index, snippet) in snippets.iter().enumerate() {
            let directory_name = format!("{BATCH_DIRECTORY_PREFIX}{index}");
            let package_directory = dir.path().join(&directory_name);
            std::fs::create_dir_all(&package_directory)?;
            std::fs::write(
                package_directory.join(BATCH_FILE_NAME),
                Self::wrap_if_fragment(&snippet.code),
            )?;
            files.push(format!("{directory_name}/{BATCH_FILE_NAME}"));
            directories.push(directory_name);
        }
        if session.is_none() && level != ValidationLevel::Syntax {
            std::fs::write(dir.path().join("go.mod"), ISOLATED_GO_MODULE)?;
        }
        let mut command = Self::batch_command(level, &files)?;
        Self::apply_build_cache(&mut command, dir.path());
        Self::apply_dependency_caches(&mut command);
        command.current_dir(dir.path());
        if let Some(session) = session {
            session.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(&files, &directories, success, &output))
    }

    fn batch_command(level: ValidationLevel, files: &[String]) -> Result<std::process::Command> {
        Ok(match level {
            ValidationLevel::Syntax => {
                let mut command = std::process::Command::new("gofmt");
                command.args(["-e", "-l"]).args(files);
                command
            }
            ValidationLevel::Compile => {
                let mut command = std::process::Command::new("go");
                command.args(["build", "./..."]);
                command
            }
            ValidationLevel::TypeCheck => {
                let mut command = std::process::Command::new("go");
                command.args(["vet", "./..."]);
                command
            }
            ValidationLevel::Run => {
                return Err(crate::snippets::error::Error::Other(BATCH_RUN_UNSUPPORTED.to_string()));
            }
        })
    }

    /// Attributes toolchain output back to the snippet that owns it. Diagnostics name their file
    /// (`snippet_batch_2/snippet.go:4:9: …`), while `go build`/`go vet` precede a failing package's
    /// diagnostics with a `# <import path>` header whose last component is that snippet's
    /// directory — so a line carrying no path of its own stays with the package last announced. ~keep
    fn batch_results(files: &[String], directories: &[String], success: bool, output: &str) -> BatchValidation {
        let mut diagnostics = vec![Vec::new(); files.len()];
        let mut unmatched = Vec::new();
        let mut current = None;
        for line in output.lines() {
            let trimmed = line.trim();
            let normalized_trimmed = trimmed.replace('\\', "/");
            if trimmed.is_empty() || files.contains(&normalized_trimmed) {
                continue;
            }
            if let Some(header) = trimmed.strip_prefix("# ") {
                current = Self::package_owner(directories, header);
                continue;
            }
            match Self::file_owner(files, line).or(current) {
                Some(index) => diagnostics[index].push(line.to_string()),
                None => unmatched.push(line.to_string()),
            }
        }
        let attributed = diagnostics.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !attributed).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        });
        diagnostics
            .into_iter()
            .map(|messages| match (messages.is_empty(), &fallback) {
                (true, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (true, None) => (SnippetStatus::Pass, None),
                (false, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            })
            .collect()
    }

    /// The batch's file keys are always minted with `/` (`validate_batch_with_context`), while the
    /// Go toolchain names files with the host separator — `snippet_batch_1\snippet.go:5:27:` on
    /// Windows. Matching the raw line there leaves the diagnostic unowned, and `batch_results`
    /// charges an unowned diagnostic on a failing command to EVERY snippet, so the working
    /// snippets get reported as broken. `go vet` emits no `# <import path>` header for analysis
    /// findings, so the package fallback cannot rescue the attribution either. ~keep
    fn file_owner(files: &[String], line: &str) -> Option<usize> {
        let normalized = line.replace('\\', "/");
        files.iter().position(|file| normalized.contains(file.as_str()))
    }

    fn package_owner(directories: &[String], import_path: &str) -> Option<usize> {
        let name = import_path.trim_matches(['[', ']']).rsplit('/').next()?;
        directories.iter().position(|directory| directory == name)
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(session) => ScratchDir::for_session(session)?,
            None => ScratchDir::isolated()?,
        };
        let file = dir.path().join("snippet.go");
        std::fs::write(&file, Self::wrap_if_fragment(&snippet.code))?;
        if session.is_none() && level != ValidationLevel::Syntax {
            std::fs::write(dir.path().join("go.mod"), ISOLATED_GO_MODULE)?;
        }
        let mut command = match level {
            ValidationLevel::Syntax => {
                let mut command = std::process::Command::new("gofmt");
                command.args(["-e", "-l"]).arg(&file);
                command
            }
            ValidationLevel::Compile => {
                let mut command = std::process::Command::new("go");
                command.args(["build", "-o", "/dev/null"]).arg(&file);
                command
            }
            ValidationLevel::TypeCheck => {
                let mut command = std::process::Command::new("go");
                command.arg("vet").arg(&file);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new("go");
                command.arg("run").arg(&file);
                command
            }
        };
        Self::apply_build_cache(&mut command, dir.path());
        Self::apply_dependency_caches(&mut command);
        match session {
            Some(value) => {
                command.current_dir(Self::project_directory(value));
                value.apply_environment(&mut command);
            }
            None => {
                command.current_dir(dir.path());
            }
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// Give the go toolchain a build cache inside the snippet's own temp directory.
    ///
    /// ~keep `go build`/`vet`/`run` refuse to start without one: `run_command`'s
    /// `sanitize_environment` allowlist carries neither `HOME` nor `GOCACHE`, and go then exits
    /// with "build cache is required, but could not be located: GOCACHE is not defined and $HOME
    /// is not defined" before compiling anything. Applied before `apply_environment` so a session
    /// that configures its own `GOCACHE` still wins and keeps its cache warm across snippets.
    fn apply_build_cache(command: &mut std::process::Command, dir: &std::path::Path) {
        command.env("GOCACHE", dir.join("go-build-cache"));
    }

    fn apply_dependency_caches(command: &mut std::process::Command) {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let (go_path, module_cache) =
            Self::dependency_cache_paths(std::env::var_os("GOPATH"), std::env::var_os("GOMODCACHE"), home);
        if let Some(path) = go_path {
            command.env("GOPATH", path);
        }
        if let Some(path) = module_cache {
            command.env("GOMODCACHE", path);
        }
    }

    fn dependency_cache_paths(
        go_path: Option<std::ffi::OsString>,
        module_cache: Option<std::ffi::OsString>,
        home: Option<std::ffi::OsString>,
    ) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
        let go_path = go_path
            .map(std::path::PathBuf::from)
            .or_else(|| home.as_deref().map(std::path::Path::new).map(|path| path.join("go")));
        let module_cache = module_cache
            .map(std::path::PathBuf::from)
            .or_else(|| go_path.as_ref().map(|path| path.join("pkg/mod")));
        (go_path, module_cache)
    }

    fn project_directory(session: &ValidationSession) -> &std::path::Path {
        session
            .manifest
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(&session.working_directory)
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if trimmed.starts_with("package ") {
            return code.to_string();
        }

        let (imports, body) = Self::split_imports(trimmed);
        let body_trimmed = body.trim();
        let only_comments = !body_trimmed.is_empty()
            && body_trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));

        if body_trimmed.is_empty() || only_comments {
            let imports_block = if imports.trim().is_empty() {
                String::new()
            } else {
                format!("{imports}\n\n")
            };
            return format!("package main\n\n{imports_block}func main() {{\n{body_trimmed}\n_ = 0\n}}\n");
        }

        let imports_block = if imports.trim().is_empty() {
            String::new()
        } else {
            format!("{imports}\n\n")
        };
        format!("package main\n\n{imports_block}func main() {{\n{body}\n}}\n")
    }

    fn split_imports(code: &str) -> (String, String) {
        let mut imports = Vec::new();
        let mut body = Vec::new();
        let mut lines = code.lines().peekable();

        while let Some(line) = lines.peek() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                imports.push(*line);
                lines.next();
                continue;
            }
            if trimmed.starts_with("import (") {
                for import_line in lines.by_ref() {
                    imports.push(import_line);
                    if import_line.trim() == ")" {
                        break;
                    }
                }
                continue;
            }
            if let Some(stripped) = trimmed.strip_prefix("import ") {
                let stripped = stripped.trim();
                if stripped.starts_with('"') || stripped.starts_with('`') {
                    imports.push(*line);
                    lines.next();
                    continue;
                }
            }
            break;
        }
        for line in lines {
            body.push(line);
        }
        (imports.join("\n"), body.join("\n"))
    }
}

/// Apple's current linker spells a missing library `ld: library 'name' not found`, with no `-l`
/// anywhere in the line. Matching the two literal fragments in order -- rather than a bare
/// `not found`, which also appears in "file not found" and "framework not found" -- keeps this
/// from swallowing unrelated errors. ~keep
fn apple_quoted_library_not_found(output: &str) -> bool {
    output.lines().any(|line| {
        line.split_once("library '").is_some_and(|(_, rest)| {
            rest.split_once('\'')
                .is_some_and(|(_, tail)| tail.starts_with(" not found"))
        })
    })
}

impl SnippetValidator for GoValidator {
    fn language(&self) -> Language {
        Language::Go
    }

    fn is_available(&self) -> bool {
        which::which("go").is_ok() || which::which("gofmt").is_ok()
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

    /// `Run` is declined: `go run` executes one program and its stdout, exit status and side
    /// effects belong to that snippet alone. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        (level != ValidationLevel::Run)
            .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn missing_session_artifacts(
        &self,
        session: &ValidationSession,
        _level: ValidationLevel,
    ) -> Vec<std::path::PathBuf> {
        crate::snippets::validators::session_artifacts::missing_go_library_directories(session)
    }

    /// ~keep `undefined: x` is Go's diagnostic for any unresolved identifier — a package the
    /// module never provided and a local the generated snippet forgot to bind produce the same
    /// text — so it is the ambiguous shape task #130 rejected for `TS2304`, and accepting it
    /// turns a codegen defect into an `Unavailable` shrug (see `runner::finalize_result`). The
    /// package-resolution diagnostics name the module itself and stay.
    ///
    /// A missing FFI shared library manifests through the *linker*, not the compiler: `go build`
    /// parses and type-checks the generated cgo package cleanly (linking is the last stage, and
    /// only starts once every earlier stage succeeded), then fails once it reaches the artifact
    /// `alef build` produces — the one the package's own `#cgo LDFLAGS: -l<name>` directive names
    /// — and that artifact is not on disk yet. GNU `ld` reports that as `cannot find -l<name>`;
    /// Apple's classic linker as `library not found for -l<name>`; Apple's *current* linker
    /// (Xcode 15+, the default `ld` on macOS today) as `ld: library '<name>' not found` -- a
    /// quoted form naming no `-l`, which is why the other three patterns all miss it. That form
    /// was found by running the real gate against a real consumer tree, not by reading docs:
    /// every synthetic fixture had used one of the older spellings. LLVM's `lld` (reached via
    /// `-fuse-ld=lld`, e.g. a CI toolchain tuned for faster links) as `unable to find library
    /// -l<name>` (`lld/ELF/Driver.cpp`'s `searchLibraryBaseName`) — a third, distinct phrasing
    /// task #505 added after confirming neither of the other two patterns already covered it.
    /// Each is usually followed by a `collect2: error: ld returned 1 exit status` (GNU) or
    /// `clang: error: linker command failed` (Apple/lld) summary line, deliberately NOT matched
    /// here — like rustc's `aborting due to`/`could not compile` summary lines elsewhere in this
    /// module, it carries no root-cause signal and must never gate a match on its own.
    ///
    /// Unconditional on the library name, matching the precedent already set by
    /// `SwiftValidator::is_dependency_error`'s `no such module` and `ZigValidator`'s `unable to
    /// find`: this method receives only raw toolchain text, with no access to the specific
    /// artifact name this session expects (`ffi_lib_name` is per-consumer config resolved deep in
    /// `core::config`, never threaded through the validator trait), so nothing in this file can
    /// verify the missing name against ground truth. What the text alone DOES prove: reaching this
    /// exact linker diagnostic means compilation already succeeded, so the failure is categorically
    /// a missing artifact, never a snippet defect — that holds no matter whose library is missing.
    /// What stays excluded on purpose: `undefined reference to`/`Undefined symbols for
    /// architecture` mean the library WAS found and loaded and some symbol inside it does not
    /// match — a real defect (a stale build, an ABI mismatch, or the generator emitting the wrong
    /// symbol name), never a build-ordering problem, so that shape keeps its `Fail`. ~keep
    fn is_dependency_error(&self, output: &str) -> bool {
        let missing_library = (output.contains("cannot find -l")
            || output.contains("library not found for -l")
            || output.contains("unable to find library -l")
            || apple_quoted_library_not_found(output))
            && !output.contains("undefined reference to")
            && !output.contains("Undefined symbols for architecture");
        missing_library
            || output.contains("cannot find package")
            || output.contains("no required module")
            || output.contains("cannot find module providing package")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Whether `go` runs, not merely resolves: a version-manager shim spawns fine then exits
    /// non-zero, so a PATH-only check leaves the skip below unreachable and fires the assert
    /// everywhere Go is absent. ~keep
    fn go_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("go")
                .arg("version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    /// Whether `gofmt` runs, not merely resolves: same shim hazard as [`go_is_runnable`], for the
    /// batch syntax check that shells out to `gofmt` directly rather than `go`. ~keep
    fn gofmt_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("gofmt")
                .arg("-h")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn derives_dependency_caches_when_go_variables_are_not_exported() {
        let (go_path, module_cache) = GoValidator::dependency_cache_paths(None, None, Some("/home/sample".into()));

        assert_eq!(go_path, Some(PathBuf::from("/home/sample/go")));
        assert_eq!(module_cache, Some(PathBuf::from("/home/sample/go/pkg/mod")));
    }

    #[test]
    fn explicit_dependency_caches_override_home_defaults() {
        let (go_path, module_cache) = GoValidator::dependency_cache_paths(
            Some("/cache/go-path".into()),
            Some("/cache/modules".into()),
            Some("/home/sample".into()),
        );

        assert_eq!(go_path, Some(PathBuf::from("/cache/go-path")));
        assert_eq!(module_cache, Some(PathBuf::from("/cache/modules")));
    }

    #[test]
    fn compiles_a_snippet_under_the_sanitized_environment() {
        if !go_is_runnable() {
            return;
        }
        let snippet = snippet("package main\n\nfunc main() {}\n");

        let (status, output) =
            GoValidator::validate_with_context(&snippet, ValidationLevel::Compile, TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("validation runs");

        assert_eq!(
            status,
            SnippetStatus::Pass,
            "go must compile under the sanitized environment; with neither HOME nor GOCACHE it refuses to \
             start with \"build cache is required\" before compiling anything: {output:?}"
        );
    }

    #[test]
    fn session_manifest_resolves_a_local_module_outside_the_working_directory() {
        if !go_is_runnable() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let working = root.path().join("working");
        let project = root.path().join("project");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::create_dir_all(project.join("localpkg")).expect("local package directory");
        std::fs::write(project.join("go.mod"), "module example.test/local\n\ngo 1.24\n").expect("go manifest");
        std::fs::write(project.join("localpkg/value.go"), "package localpkg\nconst Value = 1\n").expect("go package");
        let snippet =
            snippet("package main\nimport \"example.test/local/localpkg\"\nfunc main() { _ = localpkg.Value }");
        let session = ValidationSession {
            language: Language::Go,
            working_directory: working,
            manifest: Some(project.join("go.mod")),
            fingerprint: "fixture".into(),
            env: BTreeMap::from([(
                "GOCACHE".into(),
                root.path().join("go-cache").to_string_lossy().into_owned(),
            )]),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = GoValidator::validate_with_context(
            &snippet,
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    fn scratch_shape_session(project: &std::path::Path, fingerprint: &str) -> ValidationSession {
        ValidationSession {
            language: Language::Go,
            working_directory: project.to_path_buf(),
            manifest: None,
            fingerprint: fingerprint.into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    fn scratch_top_level_entries(project: &std::path::Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(project)
            .expect("read project directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect()
    }

    /// Regression: `validate_with_context` used to create its session-scoped scratch directory
    /// directly inside `project_directory(session)` (a tracked package source directory) via a
    /// bare `tempdir_in`, leaving a `.alef-snippet-*/` directory loose in `packages/go/` after
    /// every run. It must nest under that project's own `.alef/snippets/tmp` cache root instead. ~keep
    #[test]
    fn session_scratch_resolves_under_the_cache_root_not_the_project_directory() {
        if !go_is_runnable() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-shape-fixture");
        let snippet = snippet("package main\n\nfunc main() {}\n");

        let (status, output) = GoValidator::validate_with_context(
            &snippet,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory: {leftovers:?}"
        );
    }

    /// Pins cleanup on the failure path specifically: a snippet that fails `go build` must not
    /// leave its scratch directory behind under the project directory any more than a passing
    /// one does.
    #[test]
    fn session_scratch_is_removed_after_a_run_that_fails() {
        if !go_is_runnable() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-cleanup-fixture");
        let snippet = snippet("package main\n\nfunc main() { this does not compile }\n");

        let (status, _) = GoValidator::validate_with_context(
            &snippet,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Fail);

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory after a failing run: {leftovers:?}"
        );
        let scratch_root = project.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a failing snippet validation"
        );
    }

    #[test]
    fn batch_declines_run_so_each_snippet_executes_on_its_own() {
        let only = snippet("package main\n\nfunc main() {}\n");

        let declined = GoValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !go_is_runnable() {
            return;
        }
        let first = snippet("package main\n\nfunc main() { _ = 1 }\n");
        let second = snippet("package main\n\nfunc main() { _ = 2 }\n");
        let third = snippet("package main\n\nfunc main() { _ = 3 }\n");

        let results = GoValidator::validate_batch_with_context(
            &[&first, &second, &third],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None)
            ]
        );
    }

    /// Each snippet is its own `package main` with its own `func main`; sharing one package
    /// directory would fail the whole batch on `main redeclared` before any snippet's own code was
    /// judged. ~keep
    #[test]
    fn batch_build_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if !go_is_runnable() {
            return;
        }
        let first = snippet("package main\n\nfunc main() { _ = 1 }\n");
        let broken = snippet("package main\n\nfunc main() { undefinedBatchCall() }\n");
        let third = snippet("package main\n\nfunc main() { _ = 3 }\n");

        let results = GoValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("undefinedBatchCall")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn batch_vet_fails_only_the_snippet_the_toolchain_names() {
        if !go_is_runnable() {
            return;
        }
        let first = snippet("package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Printf(\"%d\\n\", 1) }\n");
        let broken = snippet("package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Printf(\"%d\\n\", \"text\") }\n");
        let third = snippet("package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Printf(\"%d\\n\", 3) }\n");

        let results = GoValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn batch_syntax_fails_only_the_snippet_gofmt_cannot_parse() {
        if !gofmt_is_runnable() {
            return;
        }
        let first = snippet("package main\n\nfunc main() { _ = 1 }\n");
        let broken = snippet("package main\n\nfunc main() { this is not go }\n");
        let third = snippet("package main\n\nfunc main() { _ = 3 }\n");

        let results = GoValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn batch_in_a_session_resolves_the_local_module_the_manifest_declares() {
        if !go_is_runnable() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let working = root.path().join("working");
        let project = root.path().join("project");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::create_dir_all(project.join("localpkg")).expect("local package directory");
        std::fs::write(project.join("go.mod"), "module example.test/local\n\ngo 1.24\n").expect("go manifest");
        std::fs::write(project.join("localpkg/value.go"), "package localpkg\nconst Value = 1\n").expect("go package");
        let session = ValidationSession {
            language: Language::Go,
            working_directory: working,
            manifest: Some(project.join("go.mod")),
            fingerprint: "batch-fixture".into(),
            env: BTreeMap::from([(
                "GOCACHE".into(),
                root.path().join("go-cache").to_string_lossy().into_owned(),
            )]),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let valid = snippet("package main\nimport \"example.test/local/localpkg\"\nfunc main() { _ = localpkg.Value }");
        let broken =
            snippet("package main\nimport \"example.test/local/localpkg\"\nfunc main() { _ = localpkg.Missing }");

        let results = GoValidator::validate_batch_with_context(
            &[&valid, &broken, &valid],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn package_header_lines_route_their_followers_to_the_owning_snippet() {
        let files = vec![
            "snippet_batch_0/snippet.go".to_string(),
            "snippet_batch_1/snippet.go".to_string(),
        ];
        let directories = vec!["snippet_batch_0".to_string(), "snippet_batch_1".to_string()];
        let output = "# snippet/snippet_batch_1\nvet: snippet_batch_1/snippet.go:3:15: undefined: missing\n";

        let results = GoValidator::batch_results(&files, &directories, false, output);

        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(
            results[1],
            (
                SnippetStatus::Fail,
                Some("vet: snippet_batch_1/snippet.go:3:15: undefined: missing".to_string())
            )
        );
    }

    /// A Windows Go toolchain names files with `\`, while the batch's own file keys are minted with
    /// `/`. Attributing by raw substring left the diagnostic unowned, and an unowned diagnostic on a
    /// failing command is charged to EVERY snippet — reporting the passing snippets as broken. The
    /// `# <import path>` fallback cannot cover for it: `go vet` emits no such header for analysis
    /// findings, which is why only the vet path regressed. ~keep
    #[test]
    fn a_windows_style_toolchain_path_attributes_only_to_the_snippet_it_names() {
        let files = vec![
            "snippet_batch_0/snippet.go".to_string(),
            "snippet_batch_1/snippet.go".to_string(),
        ];
        let directories = vec!["snippet_batch_0".to_string(), "snippet_batch_1".to_string()];
        let output = "vet: snippet_batch_1\\snippet.go:5:27: fmt.Printf format %d has arg of wrong type\n";

        let results = GoValidator::batch_results(&files, &directories, false, output);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(
            results[1].1.as_deref(),
            Some("vet: snippet_batch_1\\snippet.go:5:27: fmt.Printf format %d has arg of wrong type"),
            "the diagnostic must reach the named snippet with its own text unrewritten"
        );
    }

    /// `gofmt -l` lists unformatted files by name with no error attached. Normalizing the toolchain
    /// path in `file_owner` for attribution must not make a listing line printed with `\` look like
    /// a diagnostic, so the listing guard normalizes too.
    ///
    /// This test pins a regression the `file_owner` normalization could introduce, so it is only
    /// red when the listing guard's own normalization is reverted while `file_owner` keeps its.
    /// Reverting BOTH leaves it green (an unowned listing line on a successful command is simply
    /// dropped) — a full revert passing here is expected, not evidence the test is vacuous. ~keep
    #[test]
    fn windows_style_gofmt_listing_lines_are_not_read_as_failures() {
        let files = vec![
            "snippet_batch_0/snippet.go".to_string(),
            "snippet_batch_1/snippet.go".to_string(),
        ];
        let directories = vec!["snippet_batch_0".to_string(), "snippet_batch_1".to_string()];
        let output = "snippet_batch_0\\snippet.go\nsnippet_batch_1\\snippet.go\n";

        let results = GoValidator::batch_results(&files, &directories, true, output);

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// A header for a package outside the batch — a dependency of a snippet, say — must clear the
    /// package the parser is attributing to, or the diagnostics that follow it would be charged to
    /// whichever batched snippet happened to be announced last. ~keep
    #[test]
    fn a_header_for_a_package_outside_the_batch_stops_attributing_to_the_previous_one() {
        let files = vec![
            "snippet_batch_0/snippet.go".to_string(),
            "snippet_batch_1/snippet.go".to_string(),
        ];
        let directories = vec!["snippet_batch_0".to_string(), "snippet_batch_1".to_string()];
        let output = concat!(
            "# snippet/snippet_batch_0\n",
            "snippet_batch_0/snippet.go:3:2: declared and not used: value\n",
            "# example.test/dependency\n",
            "dependency.go:9:1: syntax error\n"
        );

        let results = GoValidator::batch_results(&files, &directories, false, output);

        assert_eq!(
            results[0],
            (
                SnippetStatus::Fail,
                Some("snippet_batch_0/snippet.go:3:2: declared and not used: value".to_string())
            )
        );
        assert_eq!(results[1], (SnippetStatus::Pass, None));
    }

    /// `gofmt -l` lists every file whose formatting differs, with no error attached and no effect
    /// on the exit status. Reading those bare filenames as diagnostics would fail well-formed
    /// snippets for being unformatted. ~keep
    #[test]
    fn gofmt_listing_lines_are_not_read_as_failures() {
        let files = vec![
            "snippet_batch_0/snippet.go".to_string(),
            "snippet_batch_1/snippet.go".to_string(),
        ];
        let directories = vec!["snippet_batch_0".to_string(), "snippet_batch_1".to_string()];
        let output = "snippet_batch_0/snippet.go\nsnippet_batch_1/snippet.go\n";

        let results = GoValidator::batch_results(&files, &directories, true, output);

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// A toolchain failure no snippet owns — an unusable module, a missing build cache — must fail
    /// every snippet carrying the real output rather than silently passing them all. ~keep
    #[test]
    fn a_toolchain_failure_naming_no_snippet_fails_the_whole_batch_with_the_real_output() {
        let files = vec![
            "snippet_batch_0/snippet.go".to_string(),
            "snippet_batch_1/snippet.go".to_string(),
        ];
        let directories = vec!["snippet_batch_0".to_string(), "snippet_batch_1".to_string()];
        let output = "go: go.mod file not found in current directory or any parent directory\n";

        let results = GoValidator::batch_results(&files, &directories, false, output);

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.0, SnippetStatus::Fail);
            assert_eq!(
                result.1.as_deref(),
                Some("go: go.mod file not found in current directory or any parent directory")
            );
        }
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.go"),
            language: Language::Go,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.go"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
