use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, all_error_lines_match, run_command};
use std::io::Write;
use std::path::Path;

pub struct RustValidator;

impl RustValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<Vec<(SnippetStatus, Option<String>)>> {
        let dir = match session {
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let bin_dir = dir.path().join("src/bin");
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::write(dir.path().join("Cargo.toml"), Self::cargo_manifest(snippets, session)?)?;

        let filenames = snippets
            .iter()
            .map(|snippet| {
                let stable_id = ValidationCache::key(snippet, level, session.map(|value| value.fingerprint.as_str()));
                format!("snippet_{stable_id}.rs")
            })
            .collect::<Vec<_>>();
        for (snippet, filename) in snippets.iter().zip(&filenames) {
            std::fs::write(bin_dir.join(filename), Self::wrap_if_fragment(&snippet.code))?;
        }

        let mut command = Self::batch_check_command(dir.path(), session);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(&filenames, success, &output))
    }

    /// The `cargo check` invocation a batch runs, extracted so the environment it inherits is
    /// directly assertable. `CARGO_TARGET_DIR` is the load-bearing part: the check project itself
    /// lives in a scratch directory that is deleted after every run, so without a target directory
    /// that outlives it, every run recompiled the session's path dependency and its whole
    /// transitive tree from cold. [`ValidationSession::apply_environment`] supplies it. ~keep
    fn batch_check_command(scratch: &Path, session: Option<&ValidationSession>) -> std::process::Command {
        let mut command = std::process::Command::new("cargo");
        command
            .args(["check", "--bins", "--keep-going", "--message-format=json"])
            .current_dir(scratch);
        if let Some(session) = session {
            session.apply_environment(&mut command);
        }
        #[cfg(test)]
        if session.is_none() {
            Self::apply_test_cargo_environment(&mut command);
        }
        command
    }

    #[cfg(test)]
    fn apply_test_cargo_environment(command: &mut std::process::Command) {
        // A stable checkout-local target makes these compile probes incremental. The tests that
        // execute them hold `RealCargoGuard`, so no two fixtures mutate it concurrently. ~keep
        command
            .env("CARGO_TARGET_DIR", Self::test_cargo_target_directory())
            .env("CARGO_BUILD_JOBS", "1");
    }

    #[cfg(test)]
    fn test_cargo_target_directory() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("rust-snippet-validator-probes")
    }

    fn batch_results(filenames: &[String], success: bool, output: &str) -> Vec<(SnippetStatus, Option<String>)> {
        let mut diagnostics = vec![Vec::new(); filenames.len()];
        let mut unmatched = Vec::new();
        for line in output.lines() {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                if !line.trim().is_empty() {
                    unmatched.push(line.to_string());
                }
                continue;
            };
            if message.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-message") {
                continue;
            }
            if message
                .get("message")
                .and_then(|value| value.get("level"))
                .and_then(serde_json::Value::as_str)
                != Some("error")
            {
                continue;
            }
            let Some(rendered) = message
                .get("message")
                .and_then(|value| value.get("rendered"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let spans = message
                .get("message")
                .and_then(|value| value.get("spans"))
                .and_then(serde_json::Value::as_array);
            let index = spans.and_then(|spans| {
                spans.iter().find_map(|span| {
                    let path = span.get("file_name")?.as_str()?;
                    filenames.iter().position(|filename| {
                        Path::new(path)
                            .file_name()
                            .is_some_and(|name| name == std::ffi::OsStr::new(filename))
                    })
                })
            });
            match index {
                Some(index) => diagnostics[index].push(rendered.to_string()),
                None => unmatched.push(rendered.to_string()),
            }
        }
        let has_snippet_diagnostic = diagnostics.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !has_snippet_diagnostic).then(|| {
            if unmatched.is_empty() {
                "cargo check failed without a snippet-specific diagnostic".to_string()
            } else {
                unmatched.join("\n")
            }
        });
        diagnostics
            .into_iter()
            .map(|messages| {
                if messages.is_empty() {
                    match &fallback {
                        Some(message) => (SnippetStatus::Fail, Some(message.clone())),
                        None => (SnippetStatus::Pass, None),
                    }
                } else {
                    (SnippetStatus::Fail, Some(messages.join("\n")))
                }
            })
            .collect()
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let source_dir = dir.path().join("src");
        std::fs::create_dir_all(&source_dir)?;
        std::fs::write(
            dir.path().join("Cargo.toml"),
            Self::cargo_manifest(&[snippet], session)?,
        )?;
        let code = Self::wrap_if_fragment(&snippet.code);
        let mut source_file = std::fs::File::create(source_dir.join("main.rs"))?;
        source_file.write_all(code.as_bytes())?;
        let args: &[&str] = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile | ValidationLevel::TypeCheck => &["check", "--quiet"],
            ValidationLevel::Run => &["run", "--quiet"],
        };
        let mut command = std::process::Command::new("cargo");
        command.args(args).current_dir(dir.path());
        if let Some(session) = session {
            session.apply_environment(&mut command);
        }
        #[cfg(test)]
        if session.is_none() {
            Self::apply_test_cargo_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn cargo_manifest(snippets: &[&Snippet], session: Option<&ValidationSession>) -> Result<String> {
        let dependency = session.map(Self::path_dependency).transpose()?.unwrap_or_default();
        let dependencies = session
            .map(Self::additional_dependencies)
            .transpose()?
            .unwrap_or_default();
        let declared = Self::declared_dependencies(snippets, session)?;
        Ok(format!(
            "[workspace]\n\n[package]\nname = \"snippet-check\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependency}{dependencies}{declared}"
        ))
    }

    /// Resolve `crate:<name>` snippet requirements into `[dependencies]` entries.
    ///
    /// Session configuration wins: a crate declared under `rust_dependencies` keeps the
    /// configured version and features and is not emitted twice.
    fn declared_dependencies(snippets: &[&Snippet], session: Option<&ValidationSession>) -> Result<String> {
        let configured = session.map(|session| &session.rust_dependencies);
        let mut names = std::collections::BTreeSet::new();
        for snippet in snippets {
            for requirement in &snippet.metadata.requires {
                let Some(name) = requirement.strip_prefix(crate::e2e::snippets::CRATE_REQUIREMENT_PREFIX) else {
                    continue;
                };
                if configured.is_some_and(|dependencies| dependencies.contains_key(name)) {
                    continue;
                }
                names.insert(name);
            }
        }
        let mut rendered = String::new();
        for name in names {
            if !Self::valid_dependency_name(name) {
                return Err(crate::snippets::error::Error::Other(format!(
                    "invalid Rust snippet crate requirement `{name}`"
                )));
            }
            let version = Self::pinned_dependency_version(name).ok_or_else(|| {
                crate::snippets::error::Error::Other(format!(
                    "Rust snippet requires crate `{name}` with no pinned version; declare it under `[docs.snippets.sessions.<target>.rust_dependencies.{name}]`"
                ))
            })?;
            let features = Self::pinned_dependency_features(name);
            if features.is_empty() {
                rendered.push_str(&format!("{name} = {version:?}\n"));
            } else {
                rendered.push_str(&format!(
                    "{name} = {{ version = {version:?}, features = {features:?} }}\n"
                ));
            }
        }
        Ok(rendered)
    }

    /// Versions Alef can supply itself, for crates its own snippet recipes emit. ~keep
    /// Anything else must come from session configuration, so an undeclared crate fails loudly
    /// instead of resolving to an arbitrary version.
    fn pinned_dependency_version(name: &str) -> Option<&'static str> {
        match name {
            "serde_json" => Some(crate::core::template_versions::cargo::SERDE_JSON),
            "tokio" => Some(crate::core::template_versions::cargo::TOKIO),
            "tokio-stream" => Some(crate::core::template_versions::cargo::TOKIO_STREAM),
            _ => None,
        }
    }

    /// Cargo features a pinned dependency needs for the code Alef's own recipes emit. ~keep
    /// `#[tokio::main]` lives behind tokio's `macros` and `rt-multi-thread` features, neither of
    /// which is on by default, so a bare `tokio = "1"` still fails to compile an async snippet.
    fn pinned_dependency_features(name: &str) -> &'static [&'static str] {
        match name {
            "tokio" => &["full"],
            _ => &[],
        }
    }

    fn path_dependency(session: &ValidationSession) -> Result<String> {
        let manifest = session
            .manifest
            .clone()
            .unwrap_or_else(|| session.working_directory.join("Cargo.toml"));
        let content = std::fs::read_to_string(&manifest)?;
        let value: toml::Value = toml::from_str(&content).map_err(|error| {
            crate::snippets::error::Error::Other(format!("parsing {}: {error}", manifest.display()))
        })?;
        let package = value
            .get("package")
            .and_then(|value| value.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                crate::snippets::error::Error::Other(format!("no package.name in {}", manifest.display()))
            })?;
        let crate_name = package.replace('-', "_");
        Ok(format!(
            "{crate_name} = {{ package = {package:?}, path = {:?}, features = {:?} }}\n",
            manifest
                .parent()
                .unwrap_or(&session.working_directory)
                .to_string_lossy(),
            session.rust_features
        ))
    }

    fn additional_dependencies(session: &ValidationSession) -> Result<String> {
        let mut dependencies = String::new();
        for (name, dependency) in &session.rust_dependencies {
            if !Self::valid_dependency_name(name) {
                return Err(crate::snippets::error::Error::Other(format!(
                    "invalid Rust snippet dependency name `{name}`"
                )));
            }
            let mut specification = toml::map::Map::new();
            specification.insert("version".into(), toml::Value::String(dependency.version.clone()));
            specification.insert(
                "features".into(),
                toml::Value::Array(dependency.features.iter().cloned().map(toml::Value::String).collect()),
            );
            specification.insert(
                "default-features".into(),
                toml::Value::Boolean(dependency.default_features),
            );
            dependencies.push_str(&format!("{name} = {}\n", toml::Value::Table(specification)));
        }
        Ok(dependencies)
    }

    fn valid_dependency_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    fn is_bare_signature(code: &str) -> bool {
        let trimmed = code.trim();
        trimmed.contains("fn ") && !trimmed.contains('{')
    }

    fn has_use_then_statements(code: &str) -> bool {
        let trimmed = code.trim();
        if !trimmed.starts_with("use ") {
            return false;
        }

        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with("use ") {
                continue;
            }

            if line.starts_with("let ")
                || line.starts_with("println!")
                || line.starts_with("eprintln!")
                || line.starts_with("assert")
                || line.starts_with("if ")
                || line.starts_with("for ")
                || line.starts_with("while ")
                || line.starts_with("match ")
                || line.starts_with("loop ")
                || line.starts_with("tokio::")
                || line.starts_with("std::")
                || line.starts_with("//")
            {
                return true;
            }

            return false;
        }

        false
    }

    fn split_uses(code: &str) -> (String, String) {
        let mut uses = Vec::new();
        let mut body = Vec::new();
        let mut past_uses = false;

        for line in code.lines() {
            let trimmed = line.trim();
            if !past_uses && (trimmed.starts_with("use ") || trimmed.is_empty()) {
                uses.push(line);
            } else {
                past_uses = true;
                body.push(line);
            }
        }

        (uses.join("\n"), body.join("\n"))
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if trimmed.contains("fn main()") {
            return code.to_string();
        }

        if Self::is_bare_signature(trimmed) {
            return format!("{code}\n\nfn main() {{}}");
        }

        if Self::has_use_then_statements(code) {
            let (uses, body) = Self::split_uses(code);
            return format!("{uses}\n\nfn main() {{\n{body}\n}}");
        }

        let has_top_level_items = trimmed.starts_with("use ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("enum ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("mod ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("const ")
            || trimmed.starts_with("static ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("extern ")
            || trimmed.starts_with("unsafe ");

        if has_top_level_items {
            format!("{code}\n\nfn main() {{}}")
        } else {
            format!("fn main() {{\n{code}\n}}")
        }
    }
}

impl SnippetValidator for RustValidator {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn is_available(&self) -> bool {
        which::which("cargo").is_ok()
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
    ) -> Option<Result<Vec<(SnippetStatus, Option<String>)>>> {
        (level != ValidationLevel::Run)
            .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    /// ~keep A dependency error is one whose only possible cause is that a name from outside the
    /// snippet could not be resolved at all: an unresolved import, an unlinked crate, a module
    /// file that is not on disk. Nothing else qualifies, because `runner::finalize_result`
    /// rewrites an accepted `Fail` into `Unavailable` + "run `alef build` first", which takes it
    /// out of the failure tally entirely.
    ///
    /// The pre-#215 list accepted `E0425` (cannot find value), `E0308` (mismatched types),
    /// `E0599` (no method), `E0609` (no field), `E0061` (wrong argument count) and the bare
    /// `could not compile` summary rustc prints on EVERY failed build. That made the classifier
    /// vacuous: 283 generated snippets in one consumer repo referenced a `result` binding the
    /// call emitter never bound, and all 283 were reported as an unbuilt artifact instead of as
    /// the codegen defect they were. Every code that can only fire after name resolution has
    /// already succeeded is a defect in the generated snippet and must stay `Fail`.
    ///
    /// Mirrors `typescript::is_dependency_error` (task #130) in requiring EVERY diagnostic to be
    /// a dependency diagnostic: output mixing a real defect with an unresolved import is not
    /// confidently an environment gap, so it fails with rustc's own text rather than shrugging.
    fn is_dependency_error(&self, output: &str) -> bool {
        // `E0432` unresolved import, `E0433` use of undeclared crate or module, `E0463` can't
        // find crate, `E0583` file not found for module. Each fires because a name the snippet
        // imports does not exist anywhere on the resolution path. ~keep
        const UNRESOLVED_DEPENDENCY_CODES: [&str; 4] = ["E0432", "E0433", "E0463", "E0583"];
        // rustc's end-of-run summary lines accompany every failure regardless of cause, so they
        // carry no classification signal. Counting `could not compile` as a dependency signal is
        // what accepted every failing build. ~keep
        const SUMMARY_MARKERS: [&str; 4] = [
            "aborting due to",
            "Some errors have",
            "For more information",
            "could not compile",
        ];

        all_error_lines_match(
            output,
            |line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("error") && !SUMMARY_MARKERS.iter().any(|marker| trimmed.contains(marker))
            },
            |line| {
                UNRESOLVED_DEPENDENCY_CODES.iter().any(|code| line.contains(code))
                    || line.contains("unresolved import")
                    || line.contains("can't find crate")
                    || line.contains("no matching package named")
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn session_manifest_links_the_configured_local_crate() {
        if which::which("cargo").is_err() {
            return;
        }
        let _cargo_guard = crate::test_support::RealCargoGuard::acquire();
        let project = tempfile::tempdir().expect("project directory");
        std::fs::create_dir_all(project.path().join("src")).expect("source directory");
        let manifest = project.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"sample-binding\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("package manifest");
        std::fs::write(project.path().join("src/lib.rs"), "pub const VALUE: usize = 1;\n").expect("package source");
        let mut session = ValidationSession {
            language: Language::Rust,
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        session.env.insert(
            "CARGO_TARGET_DIR".into(),
            RustValidator::test_cargo_target_directory()
                .to_string_lossy()
                .into_owned(),
        );
        let snippet = Snippet {
            id: None,
            path: "snippet.rs".into(),
            language: Language::Rust,
            title: None,
            code: "fn main() { assert_eq!(sample_binding::VALUE, 1); }".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "snippet.rs".into(),
                line: 1,
                block_index: 0,
            },
        };

        let (status, output) = RustValidator::validate_with_context(
            &snippet,
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");

        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    #[test]
    fn session_dependency_enables_declared_features() {
        let project = tempfile::tempdir().expect("project directory");
        let manifest = project.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"sample-binding\"\nversion = \"0.1.0\"\n\n[features]\ndefault = []\nnetwork = []\n",
        )
        .expect("package manifest");
        let session = ValidationSession {
            language: Language::Rust,
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: vec!["network".into()],
            rust_dependencies: BTreeMap::new(),
        };

        let dependency = RustValidator::path_dependency(&session).expect("dependency");
        assert!(dependency.contains("features = [\"network\"]"));
    }

    #[test]
    fn session_manifest_adds_explicit_dependencies() {
        let mut session = ValidationSession {
            language: Language::Rust,
            working_directory: std::path::PathBuf::from("fixture"),
            manifest: None,
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        session.rust_dependencies.insert(
            "async-runtime".into(),
            crate::core::config::output::DocsSnippetRustDependencyConfig {
                version: "1".into(),
                features: vec!["macros".into()],
                default_features: false,
            },
        );

        let dependencies = RustValidator::additional_dependencies(&session).expect("dependencies");
        assert!(dependencies.starts_with("async-runtime = {"));
        assert!(dependencies.contains("version = \"1\""));
        assert!(dependencies.contains("features = [\"macros\"]"));
        assert!(dependencies.contains("default-features = false"));
    }

    #[test]
    fn rejects_invalid_dependency_names() {
        let mut session = ValidationSession {
            language: Language::Rust,
            working_directory: std::path::PathBuf::from("fixture"),
            manifest: None,
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        session.rust_dependencies.insert(
            "invalid\n[package]".into(),
            crate::core::config::output::DocsSnippetRustDependencyConfig {
                version: "1".into(),
                features: Vec::new(),
                default_features: true,
            },
        );

        assert!(RustValidator::additional_dependencies(&session).is_err());
    }

    /// The defect: the batch check project is written into a scratch directory that is removed
    /// after the run, so with no target directory outliving it, `cargo check` rebuilt the session's
    /// path dependency and every transitive dependency from cold on every single run. The target
    /// directory must therefore sit outside the scratch tree and be keyed by the session
    /// fingerprint, so two sessions cannot compile into each other's artifacts. ~keep
    #[test]
    fn the_batch_check_reuses_a_persistent_target_directory_outside_the_scratch_tree() {
        let project = tempfile::tempdir().expect("project directory");
        let scratch = tempfile::tempdir().expect("scratch directory");
        let session = ValidationSession {
            language: Language::Rust,
            working_directory: project.path().to_path_buf(),
            manifest: None,
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let command = RustValidator::batch_check_command(scratch.path(), Some(&session));

        let target_dir = command
            .get_envs()
            .find_map(|(name, value)| (name == "CARGO_TARGET_DIR").then_some(value))
            .expect("the batch check must set CARGO_TARGET_DIR")
            .expect("CARGO_TARGET_DIR must have a value");
        assert_eq!(std::path::Path::new(target_dir), session.cargo_target_directory());
        assert!(
            !std::path::Path::new(target_dir).starts_with(scratch.path()),
            "a target directory inside the scratch tree is deleted with it: {}",
            std::path::Path::new(target_dir).display()
        );
    }

    #[test]
    fn real_cargo_tests_share_one_bounded_checkout_local_build_lane() {
        let scratch = tempfile::tempdir().expect("scratch directory");

        let command = RustValidator::batch_check_command(scratch.path(), None);

        let environment = command
            .get_envs()
            .filter_map(|(name, value)| Some((name.to_str()?, value?.to_str()?)))
            .collect::<BTreeMap<_, _>>();
        let expected_target = RustValidator::test_cargo_target_directory();
        assert_eq!(
            environment["CARGO_TARGET_DIR"],
            expected_target.to_str().expect("UTF-8 target directory")
        );
        assert_eq!(environment["CARGO_BUILD_JOBS"], "1");
    }

    /// Two sessions must never share compiled artifacts when their CONFIGURATION differs: they can
    /// link different path dependencies, different features and different dependency versions into
    /// a package with the same name. The separation is keyed on configuration, not on a
    /// working-tree digest -- see the content-only companion below for why. ~keep
    #[test]
    fn two_differently_configured_sessions_do_not_share_a_cargo_target_directory() {
        let project = tempfile::tempdir().expect("project directory");
        let first = ValidationSession {
            language: Language::Rust,
            working_directory: project.path().to_path_buf(),
            manifest: None,
            fingerprint: "fingerprint-one".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut second = first.clone();
        second.rust_features = vec!["extra".to_string()];

        assert_ne!(
            first.cargo_target_directory(),
            second.cargo_target_directory(),
            "a differing feature set can link different artifacts into a package of the same name"
        );
    }

    /// A source edit must re-run validation without discarding the compiler cache. Cargo tracks its
    /// own artifacts' staleness against its own inputs; handing it an empty directory on every
    /// content change bought no correctness and cost a full cold rebuild per edit -- measured at
    /// 4.1 GiB stranded per run in a consumer repo, with nothing ever sweeping them. ~keep
    #[test]
    fn a_content_only_change_keeps_the_cargo_target_directory() {
        let project = tempfile::tempdir().expect("project directory");
        let first = ValidationSession {
            language: Language::Rust,
            working_directory: project.path().to_path_buf(),
            manifest: None,
            fingerprint: "fingerprint-one".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut second = first.clone();
        second.fingerprint = "fingerprint-two".into();

        assert_eq!(
            first.cargo_target_directory(),
            second.cargo_target_directory(),
            "a working-tree edit must rekey the verdict cache, not strand the compiler cache"
        );
    }

    #[test]
    fn maps_json_diagnostics_to_the_owning_binary() {
        let filenames = vec!["snippet_first.rs".to_string(), "snippet_second.rs".to_string()];
        let output = serde_json::json!({
            "reason": "compiler-message",
            "message": {
                "level": "error",
                "rendered": "error: unknown value",
                "spans": [{"file_name": "src/bin/snippet_second.rs"}]
            }
        })
        .to_string();

        let results = RustValidator::batch_results(&filenames, false, &output);

        assert_eq!(results[0].0, SnippetStatus::Pass);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[1].1.as_deref(), Some("error: unknown value"));
    }

    #[test]
    fn validates_multiple_bins_in_one_batch() {
        if which::which("cargo").is_err() {
            return;
        }
        let _cargo_guard = crate::test_support::RealCargoGuard::acquire();
        let first = snippet("fn main() { let value: usize = 1; assert_eq!(value, 1); }");
        let second = snippet("fn main() { let value: usize = \"wrong\"; }");

        let results = RustValidator::validate_batch_with_context(
            &[&first, &second],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, SnippetStatus::Pass, "{:?}", results[0].1);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("mismatched types"))
        );
    }

    #[test]
    fn declared_crate_requirements_enter_the_snippet_manifest() {
        let mut declared = snippet("fn main() {}");
        declared.metadata.requires = vec!["crate:serde_json".into(), "feature:visitor".into()];

        let manifest = RustValidator::cargo_manifest(&[&declared], None).expect("manifest renders");

        assert!(manifest.contains("serde_json = \"1\""), "{manifest}");
        assert!(!manifest.contains("feature:visitor"), "{manifest}");
    }

    /// `#[tokio::main]` expands to a runtime builder and lives behind tokio's `macros` and
    /// `rt-multi-thread` features, neither of which is a default. A bare `tokio = "1"` line would
    /// satisfy the crate resolution and still fail the snippet, so the feature list is part of the
    /// contract, not a detail.
    #[test]
    fn a_declared_tokio_requirement_enters_the_manifest_with_its_features() {
        let mut declared = snippet("fn main() {}");
        declared.metadata.requires = vec!["crate:tokio".into()];

        let manifest = RustValidator::cargo_manifest(&[&declared], None).expect("manifest renders");

        assert!(
            manifest.contains("tokio = { version = \"1\", features = [\"full\"] }"),
            "tokio must be pinned with the features `#[tokio::main]` needs: {manifest}"
        );
    }

    /// `tokio-stream` is the package name Cargo has to resolve on the registry; the body writes
    /// the `tokio_stream` lib name Cargo derives from it. Getting the two the wrong way round
    /// still passes every generator-side check and fails only here, on resolution. ~keep
    #[test]
    fn a_declared_stream_requirement_enters_the_manifest_under_its_package_name() {
        let mut declared = snippet("fn main() {}");
        declared.metadata.requires = vec!["crate:tokio-stream".into()];

        let manifest = RustValidator::cargo_manifest(&[&declared], None).expect("manifest renders");

        assert!(
            manifest.contains("tokio-stream = \"0.1\""),
            "the streaming recipe's crate must be pinned under its hyphenated package name: {manifest}"
        );
    }

    /// The two halves of this contract live in different modules: `e2e::snippets` decides which
    /// crate requirement a body earns, and this validator decides what a requirement resolves to.
    /// A requirement no version table knows fails the whole snippet at manifest-render time. ~keep
    #[test]
    fn every_crate_requirement_the_generator_attaches_resolves_to_a_pinned_version() {
        for (marker, requirement) in crate::e2e::snippets::RUST_BODY_CRATE_REQUIREMENTS {
            let name = requirement
                .strip_prefix(crate::e2e::snippets::CRATE_REQUIREMENT_PREFIX)
                .unwrap_or_else(|| panic!("`{requirement}` (attached for `{marker}`) is not a crate requirement"));
            assert!(
                RustValidator::pinned_dependency_version(name).is_some(),
                "a Rust body containing `{marker}` is given `{requirement}`, but this validator \
                 cannot pin `{name}`, so the snippet's manifest never renders"
            );
        }
    }

    #[test]
    fn configured_dependencies_win_over_declared_crate_requirements() {
        let mut declared = snippet("fn main() {}");
        declared.metadata.requires = vec!["crate:serde_json".into()];
        let mut session = ValidationSession {
            language: Language::Rust,
            working_directory: std::path::PathBuf::from("fixture"),
            manifest: None,
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        session.rust_dependencies.insert(
            "serde_json".into(),
            crate::core::config::output::DocsSnippetRustDependencyConfig {
                version: "1.0.100".into(),
                features: Vec::new(),
                default_features: false,
            },
        );

        let configured = RustValidator::additional_dependencies(&session).expect("configured dependencies");
        let declared_dependencies =
            RustValidator::declared_dependencies(&[&declared], Some(&session)).expect("declared dependencies");

        assert!(configured.contains("version = \"1.0.100\""), "{configured}");
        assert_eq!(declared_dependencies, "");
    }

    #[test]
    fn unknown_crate_requirements_fail_instead_of_resolving_silently() {
        let mut declared = snippet("fn main() {}");
        declared.metadata.requires = vec!["crate:not-a-pinned-crate".into()];

        let error = RustValidator::cargo_manifest(&[&declared], None).expect_err("unpinned crate must fail");

        assert!(error.to_string().contains("not-a-pinned-crate"), "{error}");
    }

    #[test]
    fn declared_crate_requirement_makes_a_json_snippet_compile() {
        if which::which("cargo").is_err() {
            return;
        }
        let _cargo_guard = crate::test_support::RealCargoGuard::acquire();
        let code = "use serde_json::Value;\n\nfn main() {\n    let options: Value = serde_json::from_str(r#\"{\"width\": 80}\"#).unwrap();\n    assert_eq!(options[\"width\"], 80);\n}\n";
        let mut declared = snippet(code);
        declared.metadata.requires = vec!["crate:serde_json".into()];
        let undeclared = snippet(code);

        let (declared_status, declared_output) = RustValidator::validate_with_context(
            &declared,
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("declared snippet validates");
        let (undeclared_status, _) = RustValidator::validate_with_context(
            &undeclared,
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("undeclared snippet validates");

        assert_eq!(declared_status, SnippetStatus::Pass, "{declared_output:?}");
        assert_eq!(undeclared_status, SnippetStatus::Fail);
    }

    /// The undeclared half of the tokio defect, asserted on the compiler's own diagnostic rather
    /// than on a bare `Fail` — a snippet can fail for any number of reasons, and a status alone
    /// would not distinguish this defect from an unrelated break.
    ///
    /// ~keep The declared half is deliberately not asserted here: every check project is a fresh
    /// directory, so proving it compiles means building tokio's `full` tree from scratch, which
    /// overruns the toolchain timeout and would make this a stopwatch rather than a test.
    #[test]
    fn an_async_snippet_without_its_tokio_requirement_fails_on_the_missing_crate() {
        if which::which("cargo").is_err() {
            return;
        }
        let _cargo_guard = crate::test_support::RealCargoGuard::acquire();
        let code = "#[tokio::main]\nasync fn main() {\n    let value = 1u8;\n    println!(\"{value:?}\");\n}\n";

        let (status, output) = RustValidator::validate_with_context(
            &snippet(code),
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("undeclared snippet validates");

        assert_eq!(status, SnippetStatus::Fail);
        let output = output.unwrap_or_default();
        assert!(
            output.contains("tokio"),
            "the failure must name the unresolved tokio crate, not some unrelated break: {output}"
        );
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: "guide.md".into(),
            language: Language::Rust,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "guide.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }
}
