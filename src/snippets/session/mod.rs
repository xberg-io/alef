mod before_hooks;
mod fingerprint;
#[cfg(test)]
mod hook_reuse_tests;
#[cfg(test)]
mod preparation_error_tests;
mod purge;
mod toolchain_cache;

use before_hooks::{HookOutcomes, run_before_once};
use fingerprint::{session_fingerprint, session_toolchain_key};
use purge::{cleanup_legacy_scratch_directories, purge_abandoned_scratch, purge_stale_session_scratch};
use toolchain_cache::{ToolchainCaches, purge_stale_toolchain_caches};

pub use toolchain_cache::DEFAULT_TOOLCHAIN_CACHE_GENERATIONS;

use crate::snippets::error::{Error, Result};
use crate::snippets::types::Language;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub language: Language,
    pub working_directory: PathBuf,
    pub manifest: Option<PathBuf>,
    pub before: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub include_paths: Vec<PathBuf>,
    pub rust_features: Vec<String>,
    pub rust_dependencies: BTreeMap<String, crate::core::config::output::DocsSnippetRustDependencyConfig>,
}

#[derive(Debug, Clone)]
pub struct ValidationSession {
    /// Carried from the [`SessionSpec`] so the scratch destination can be one decision rather than
    /// per-runner behaviour: [`crate::snippets::scratch::scratch_root`] needs the language, and a
    /// validator holds only a `ValidationSession`. ~keep
    pub language: Language,
    pub working_directory: PathBuf,
    pub manifest: Option<PathBuf>,
    pub fingerprint: String,
    pub env: BTreeMap<String, String>,
    pub include_paths: Vec<PathBuf>,
    pub rust_features: Vec<String>,
    pub rust_dependencies: BTreeMap<String, crate::core::config::output::DocsSnippetRustDependencyConfig>,
}

pub(crate) struct SessionPreparation {
    pub sessions: HashMap<String, ValidationSession>,
    pub errors: HashMap<String, SessionPreparationError>,
}

/// A session's preparation failure, classified so a downstream snippet is never reported as
/// failed validation for a reason it never actually reached.
///
/// `ordering` is the split this type exists to carry: a `before` hook builds this language's
/// artifacts (`cargo build --release -p <crate>-jni`, `pnpm run build:all`, ...) before any of
/// its snippets can validate, and when that hook itself outlives `timeout_secs` -- readily hit on
/// a loaded machine, or right after `--clean` wiped the artifacts it exists to rebuild -- that is
/// a build that has not finished yet, not a defect in any snippet and not a broken session. Every
/// other preparation failure (a missing manifest, a missing working directory, a `before` hook
/// that ran to completion and then failed on its own terms) is a real configuration problem and
/// keeps `ordering` false. See `runner::session_prep::session_preparation_result`, the only
/// reader of this field. ~keep
#[derive(Debug, Clone)]
pub(crate) struct SessionPreparationError {
    pub message: String,
    pub ordering: bool,
}

/// The in-tree root every fingerprint-keyed session scratch directory is nested under, relative to
/// a session's `working_directory`.
pub(super) const SESSION_SCRATCH_ROOT: &str = ".alef/snippets/sessions";

/// The stable, persistent, cross-run scratch directory for a session's fingerprint, nested under
/// its `working_directory`. Shared between `ValidationSession::workspace_directory` (which
/// creates it) and `purge_session_scratch_root` (which needs the identical path before a
/// `ValidationSession` exists to compute it from). ~keep
pub(super) fn workspace_scratch_directory(working_directory: &Path, fingerprint: &str) -> PathBuf {
    working_directory.join(SESSION_SCRATCH_ROOT).join(fingerprint)
}

/// Whether a language's per-session scratch lives outside `working_directory` entirely, which is
/// the same question as "does this language have a live in-tree scratch directory at all".
///
/// Java is the only such language: alef's own Java backend points Maven's `<sourceDirectory>` at
/// `${project.basedir}`, so `JavaValidator` resolves its scratch through
/// [`ValidationSession::external_workspace_directory`] instead. That pairing is held in place by
/// `JavaValidator`'s `session_scratch_is_never_written_under_the_working_directory` regression
/// test, which fails if the validator ever writes under `working_directory` again.
pub(super) const fn keeps_scratch_outside_working_directory(language: Language) -> bool {
    matches!(language, Language::Java)
}

impl ValidationSession {
    pub fn workspace_directory(&self) -> Result<PathBuf> {
        let directory = workspace_scratch_directory(&self.working_directory, &self.fingerprint);
        crate::core::cache_dir::ensure_cache_dir(&directory)?;
        Ok(directory)
    }

    /// The persistent, fingerprint-keyed scratch directory for a session whose build tool globs
    /// its whole project directory for sources, not just a `src/` subtree — alef's own Java
    /// backend sets Maven's `<sourceDirectory>` to `${project.basedir}` (see the generated
    /// `packages/java/pom.xml`) because it emits sources at the package root rather than under
    /// `src/main/java/`. That means every path under a Java session's `working_directory`,
    /// `.alef/` included, is a live compiler input: `mvn package` would compile scratch
    /// `.java` files into the shipped artifact, and `maven-source-plugin`/`javadoc` would bundle
    /// them too. This directory lives under the OS temp root instead, so it can never be
    /// swept up by the consumer's own build. Classpath resolution is unaffected because
    /// `JavaValidator` resolves classpath entries as absolute paths from the manifest,
    /// independent of where the scratch source and class files are compiled from. ~keep
    pub fn external_workspace_directory(&self) -> Result<PathBuf> {
        let directory = std::env::temp_dir()
            .join("alef-snippets/sessions")
            .join(&self.fingerprint);
        std::fs::create_dir_all(&directory)?;
        Ok(directory)
    }

    /// Where this session's per-snippet scratch is allocated. Delegates to
    /// [`crate::snippets::scratch::scratch_root`] so a runner and the preparation-time sweep can
    /// never disagree about the destination. ~keep
    #[must_use]
    pub fn scratch_root(&self) -> PathBuf {
        crate::snippets::scratch::scratch_root(self.language, &self.working_directory, self.manifest.as_deref())
    }

    /// Allocates a self-removing scratch directory for this session.
    ///
    /// # Errors
    ///
    /// Returns an error when the scratch root cannot be created or a unique directory cannot be
    /// allocated inside it.
    pub fn scratch_dir(&self) -> Result<crate::snippets::scratch::ScratchDir> {
        crate::snippets::scratch::ScratchDir::for_session(self)
    }

    pub fn apply(&self, command: &mut std::process::Command) {
        command.current_dir(&self.working_directory);
        self.apply_environment(command);
    }

    pub fn apply_environment(&self, command: &mut std::process::Command) {
        let caches = self.cache_directories();
        command.env("GOCACHE", &caches.go_build);
        command.env("ZIG_GLOBAL_CACHE_DIR", &caches.zig_global);
        command.env("CARGO_TARGET_DIR", &caches.cargo_target);
        for (name, value) in &self.env {
            let path = std::path::Path::new(value);
            let value = if TOOLCHAIN_CACHE_VARIABLES.contains(&name.as_str()) && path.is_relative() {
                self.working_directory.join(path).into_os_string()
            } else {
                value.into()
            };
            command.env(name, value);
        }
    }

    /// The persistent directory `cargo` compiles the snippet check project into.
    ///
    /// Every rust snippet batch writes its check project into a fresh scratch directory, so
    /// without a target directory that outlives it, `cargo check` recompiled the session's path
    /// dependency and its entire transitive tree on every single run — minutes of work whose
    /// inputs had not changed. Keyed by [`Self::toolchain_cache_key`] like the other toolchain
    /// caches, so two sessions can never compile into each other's artifacts. ~keep
    #[must_use]
    pub fn cargo_target_directory(&self) -> PathBuf {
        self.cache_directories().cargo_target
    }

    /// The key naming this session's persistent toolchain cache directories — its session
    /// *configuration*, deliberately not its working-tree [`Self::fingerprint`]. See
    /// `fingerprint::session_toolchain_key` for why a compiler cache must outlive a source edit
    /// that a verdict cache must not. ~keep
    #[must_use]
    pub fn toolchain_cache_key(&self) -> String {
        session_toolchain_key(self)
    }

    fn cache_directories(&self) -> ToolchainCaches {
        toolchain_cache::cache_directories(self)
    }
}

/// Environment variables naming a per-session toolchain cache. A configured override for one of
/// these is resolved against the session's `working_directory` when it is relative, because a
/// toolchain resolves it against its own process working directory otherwise. ~keep
const TOOLCHAIN_CACHE_VARIABLES: &[&str] = &["GOCACHE", "ZIG_GLOBAL_CACHE_DIR", "CARGO_TARGET_DIR"];

/// A spec that resolved successfully, paired with the target name and spec it came from, awaiting
/// the purge and then activation.
type ResolvedSession<'a> = (&'a String, &'a SessionSpec, ValidationSession);

/// Prepares every configured session, activating (running `before` hooks for) all of them.
///
/// A thin wrapper over [`prepare_sessions_isolated_with_activation_filter`] for callers that have
/// no cache-hit information to filter activation by -- every test in this module, and any future
/// caller that wants the unconditional behaviour. ~keep
#[cfg(test)]
pub(crate) fn prepare_sessions_isolated(specs: &HashMap<String, SessionSpec>, timeout_secs: u64) -> SessionPreparation {
    prepare_sessions_isolated_with_activation_filter(
        specs,
        timeout_secs,
        DEFAULT_TOOLCHAIN_CACHE_GENERATIONS,
        &|_, _| true,
    )
}

/// Prepares every configured session in three phases, because the middle one is not per-session.
///
/// Phase one resolves each spec far enough to know its fingerprint, without running any `before`
/// hook. Phase two then purges the in-tree session scratch root — and, under `generations`, the
/// toolchain cache root — of every working directory, using the *complete* set of live keys for
/// that directory, which is why it cannot be folded back into a per-session step: two targets can
/// legitimately share one `working_directory`, and a per-session purge would delete its sibling's
/// live scratch. Phase three runs the `before` hooks against the already-purged tree, but only for
/// a session `needs_activation` still says needs it.
///
/// `needs_activation(target, session)` runs after phase one, once a session's fingerprint is known,
/// so a caller can compare that fingerprint against a result cache before committing to the
/// hook's cost. It is `Sync` because phase three's sessions run on separate rayon worker threads.
/// Returning `false` skips only that session's `before` commands; the session is still resolved,
/// purged for, and inserted into the returned map with a real fingerprint, so cache lookups keyed
/// on it still work identically to a session that did activate.
///
/// Phases one and three each run their own work concurrently, but the phase boundary itself stays
/// a barrier: the purge still needs every fingerprint before it removes anything, and no `before`
/// hook may run before the purge is complete. ~keep
pub(crate) fn prepare_sessions_isolated_with_activation_filter(
    specs: &HashMap<String, SessionSpec>,
    timeout_secs: u64,
    generations: usize,
    needs_activation: &(dyn Fn(&str, &ValidationSession) -> bool + Sync),
) -> SessionPreparation {
    let mut sessions = HashMap::new();
    let mut errors = HashMap::new();
    let mut resolved = Vec::new();
    for (target, spec, outcome) in resolve_sessions(specs, timeout_secs) {
        match outcome {
            Ok(session) => resolved.push((target, spec, session)),
            Err(error) => record_preparation_error(&mut errors, target, spec, &error),
        }
    }
    purge_stale_session_scratch(&resolved);
    purge_stale_toolchain_caches(&resolved, timeout_secs, generations);
    let mut outcomes = activate_sessions(&resolved, timeout_secs, needs_activation);
    outcomes.sort_by_key(|(index, _)| *index);
    for ((target, spec, session), (_, outcome)) in resolved.into_iter().zip(outcomes) {
        match outcome {
            Ok(()) => {
                sessions.insert(target.clone(), session);
            }
            Err(error) => record_preparation_error(&mut errors, target, spec, &error),
        }
    }
    SessionPreparation { sessions, errors }
}

/// Phase one, run concurrently across targets: every session's fingerprint is a full content hash
/// of its working tree, so sixteen configured languages meant sixteen whole-tree walks strictly one
/// after another before any of them could be purged, let alone validated. Resolution touches only
/// its own spec's scratch, and every removal it performs already tolerates a concurrent one. ~keep
fn resolve_sessions(
    specs: &HashMap<String, SessionSpec>,
    timeout_secs: u64,
) -> Vec<(&String, &SessionSpec, Result<ValidationSession>)> {
    let span = tracing::Span::current();
    specs
        .iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(target, spec)| (target, spec, span.in_scope(|| resolve_session(spec, timeout_secs))))
        .collect()
}

/// Phase three, run concurrently across *working directories* only. A `before` hook builds its
/// working directory in place (`pnpm build`, `cargo build --release`, `mvn package`, `swift
/// build`), so two sessions sharing one must still run theirs one after another — but two sessions
/// in different directories share nothing, and running those back to back put sixteen full builds
/// on the critical path before a single snippet was validated. ~keep
///
/// Returns `(index into `resolved`, outcome)` pairs rather than writing into a shared collection,
/// so grouping cannot disturb which session an outcome belongs to.
fn activate_sessions(
    resolved: &[ResolvedSession<'_>],
    timeout_secs: u64,
    needs_activation: &(dyn Fn(&str, &ValidationSession) -> bool + Sync),
) -> Vec<(usize, Result<()>)> {
    let span = tracing::Span::current();
    activation_groups(resolved)
        .into_par_iter()
        .map(|indices| {
            span.in_scope(|| {
                let mut ran = HookOutcomes::default();
                indices
                    .into_iter()
                    .map(|index| {
                        let (target, spec, session) = &resolved[index];
                        (
                            index,
                            activate_session(target, spec, session, timeout_secs, &mut ran, needs_activation),
                        )
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect()
}

fn activation_groups(resolved: &[ResolvedSession<'_>]) -> Vec<Vec<usize>> {
    let mut groups: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();
    for (index, (_, spec, _)) in resolved.iter().enumerate() {
        groups.entry(spec.working_directory.as_path()).or_default().push(index);
    }
    groups.into_values().collect()
}

fn record_preparation_error(
    errors: &mut HashMap<String, SessionPreparationError>,
    target: &str,
    spec: &SessionSpec,
    error: &Error,
) {
    // A `before` hook is the step that builds this language's artifacts, and `Error::Timeout` is
    // the only way `activate_session` can reach this function without first collapsing the error
    // into `Error::Other` -- see the `before` loop there. That means `Error::Timeout` here can
    // only mean one thing: the build step itself did not finish in time, which is an ordering
    // problem, not a broken session or a broken snippet. ~keep
    let ordering = matches!(error, Error::Timeout { .. });
    let message = if ordering {
        format!(
            "preparing snippet validation target `{target}`: the {} session's `before` hook (the step that \
             builds this language's artifacts) {error}. Run `alef build` before validating, or raise \
             `docs.snippets.timeout_secs`; `alef all --clean` discards previously built artifacts, which makes \
             this more likely.",
            spec.language
        )
    } else {
        format!("preparing snippet validation target `{target}`: {error}")
    };
    // Every snippet targeting this session ends up `Unavailable` or `Error` (see
    // `runner::session_prep::session_preparation_result`) with no other signal that the
    // *target*, not the individual snippets, is what broke — this had zero `tracing::` calls
    // before, so a whole language's worth of results going dark was silent beyond the final
    // summary counts. ~keep
    tracing::error!(
        target = %target,
        language = %spec.language,
        error = %error,
        ordering,
        "snippet validation session preparation failed"
    );
    errors.insert(target.to_owned(), SessionPreparationError { message, ordering });
}

/// Validates a spec and derives its fingerprint. Deliberately runs no `before` hook: the hook must
/// not see a scratch root that `purge_stale_session_scratch` has not swept yet, and that sweep
/// needs every session's fingerprint first.
fn resolve_session(spec: &SessionSpec, timeout_secs: u64) -> Result<ValidationSession> {
    let language = spec.language;
    ensure_directory(&spec.working_directory, language)?;
    cleanup_legacy_scratch_directories(&spec.working_directory, timeout_secs)?;
    purge_abandoned_scratch(spec, timeout_secs);
    if let Some(manifest) = &spec.manifest
        && !manifest.is_file()
    {
        return Err(Error::Other(format!(
            "configured {language} snippet manifest does not exist: {}",
            manifest.display()
        )));
    }
    Ok(ValidationSession {
        language,
        working_directory: spec.working_directory.clone(),
        manifest: spec.manifest.clone(),
        fingerprint: session_fingerprint(spec)?,
        env: spec.env.clone(),
        include_paths: spec.include_paths.clone(),
        rust_features: spec.rust_features.clone(),
        rust_dependencies: spec.rust_dependencies.clone(),
    })
}

fn activate_session(
    target: &str,
    spec: &SessionSpec,
    session: &ValidationSession,
    timeout_secs: u64,
    ran: &mut HookOutcomes,
    needs_activation: &(dyn Fn(&str, &ValidationSession) -> bool + Sync),
) -> Result<()> {
    let language = spec.language;
    if needs_activation(target, session) {
        for command in &spec.before {
            run_before_once(command, spec, timeout_secs, ran).map_err(|error| match error {
                // A `before` hook's own timeout is propagated verbatim, not wrapped into
                // `Error::Other`, so `record_preparation_error` can still tell "the build step
                // never finished" apart from every other way session preparation can fail.
                // Wrapping it here would erase the one signal that distinguishes an ordering
                // problem from a broken session -- see `SessionPreparationError::ordering`. ~keep
                Error::Timeout { .. } => error,
                other => Error::Other(format!("preparing {language} snippet validation session: {other}")),
            })?;
        }
    } else {
        // Every snippet that would use this session already has a cache-hit result on file, so the
        // `before` hook -- the expensive part of preparation, a whole-package cold build in the
        // reported case -- has nothing to build for. Skipping it here, not by never calling this
        // function, keeps the session resolved with a real fingerprint and its toolchain cache
        // directories created below, so a cache lookup keyed on it still works identically to a
        // session that did activate. ~keep
        tracing::debug!(
            target = %target,
            language = %language,
            "skipping snippet validation session before hook: every claimed snippet is already cached"
        );
    }
    let caches = session.cache_directories();
    for directory in caches.directories() {
        crate::core::cache_dir::ensure_cache_dir(directory).map_err(|error| {
            Error::Other(format!(
                "creating snippet toolchain cache {}: {error}",
                directory.display()
            ))
        })?;
    }
    caches.mark_used();
    Ok(())
}

fn ensure_directory(path: &Path, language: Language) -> Result<()> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "configured {language} snippet working directory does not exist: {}",
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_before_command_once_per_language() {
        let directory = tempfile::tempdir().expect("temp directory");
        let marker = directory.path().join("prepared");
        let mut specs = HashMap::new();
        specs.insert(
            "python".into(),
            SessionSpec {
                language: Language::Python,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec![format!("test ! -e prepared && touch {}", marker.display())],
                env: BTreeMap::new(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(marker.exists());
        assert!(prepared.errors.is_empty());
        assert_eq!(prepared.sessions.len(), 1);
    }

    #[test]
    fn scratch_cleanup_errors_name_the_working_directory() {
        let directory = tempfile::tempdir().expect("temp directory");
        let missing = directory.path().join("removed");

        let error = cleanup_legacy_scratch_directories(&missing, 5).expect_err("missing root must fail");

        let message = error.to_string();
        assert!(message.contains("reading snippet working directory"));
        assert!(message.contains(&missing.display().to_string()));
    }

    /// A failed target must be visible beyond the final summary counts: every snippet aimed at it
    /// silently becomes `SnippetStatus::Error` downstream (see
    /// `runner::session_preparation_error`), and before this there was no `tracing::` call
    /// anywhere in this module to explain why. ~keep
    #[tracing_test::traced_test]
    #[test]
    fn rejects_missing_configured_manifest() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut specs = HashMap::new();
        specs.insert(
            "typescript".into(),
            SessionSpec {
                language: Language::TypeScript,
                working_directory: directory.path().to_path_buf(),
                manifest: Some(directory.path().join("missing.json")),
                before: Vec::new(),
                env: BTreeMap::new(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);
        let error = prepared.errors.get("typescript").expect("missing manifest is rejected");
        assert!(logs_contain("snippet validation session preparation failed"));
        assert!(logs_contain("typescript"));

        assert!(error.message.contains("manifest does not exist"));
        assert!(
            !error.ordering,
            "a missing manifest is a configuration error, not an ordering problem: {}",
            error.message
        );
    }

    /// Wrap a POSIX `before` hook so it survives whichever shell [`shell_command`] picks.
    ///
    /// Hooks are consumer-authored shell text run verbatim, and on Windows that shell is `cmd`,
    /// which cannot parse `!`, `[ ... ]` or `$(( ))` and which splits a `;`-sequenced line on
    /// spaces -- `touch a; attempts=0; while [ $attempts -lt 500 ]` reached `touch` as one
    /// argument list and died on `-lt`. The orchestration these tests exercise (purge ordering,
    /// concurrent scheduling) is platform-independent; only the instrument is POSIX, so the
    /// instrument is handed to `sh` explicitly there.
    ///
    /// `script` must therefore contain no `cmd` metacharacter -- `& | < > ( ) ^ "` -- because
    /// `cmd` parses the outer line before `sh` ever sees it. Single quotes are not special to
    /// `cmd` and survive to `sh` intact. ~keep
    fn posix_hook(script: &str) -> String {
        debug_assert!(
            !script.contains(['&', '|', '<', '>', '(', ')', '^', '"']),
            "a `cmd` metacharacter in {script:?} would be parsed before `sh` sees the hook"
        );
        if cfg!(windows) {
            format!("sh -c '{script}'")
        } else {
            script.to_owned()
        }
    }

    /// Render `path` the way the `sh` in [`posix_hook`] can read it on either platform.
    ///
    /// `sh` treats a backslash as an escape, so a Windows path handed to it verbatim arrives as
    /// `C:UsersRUNNER1...` and every `[ -e ... ]` against it answers "absent" whatever is on
    /// disk -- a test that then passes has examined nothing. MSYS `sh` accepts a drive-letter
    /// path spelled with forward slashes. ~keep
    fn sh_path(path: &Path) -> String {
        path.display().to_string().replace('\\', "/")
    }

    /// The regression this closes: a `before` hook that builds the whole module from
    /// `working_directory` (`npm run build`, for a TypeScript session — java no longer takes this
    /// path at all; see `external_workspace_directory`) runs once, before any of *this* run's
    /// snippets are written — so the only way it can trip over bad scratch source content is a
    /// leftover from a *previous* run's per-snippet validate call, which nothing ever cleaned up.
    /// One bad leftover then failed session preparation and stamped every snippet in the session
    /// as `SnippetStatus::Error`, turning one bad snippet into a whole language going dark. The
    /// `before` command below does not know the fingerprint-derived workspace path in advance
    /// (neither does a real consumer's `npm run build`), so it searches for the leftover instead
    /// of asserting a literal path — exactly what a stale-content bug would trip over. ~keep
    #[test]
    fn stale_workspace_scratch_files_are_purged_before_before_hooks_run() {
        let directory = tempfile::tempdir().expect("temp directory");
        let spec = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: vec![posix_hook("find . -name snippet.ts -exec false {} +")],
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let fingerprint = session_fingerprint(&spec).expect("fingerprint");
        let workspace = workspace_scratch_directory(directory.path(), &fingerprint);
        std::fs::create_dir_all(&workspace).expect("workspace directory");
        let stale_file = workspace.join("snippet.ts");
        std::fs::write(&stale_file, "this does not compile: :::").expect("stale scratch file");
        // A subdirectory must survive the purge: it stands in for a compiled-artifact cache
        // (`target/classes`, `.nuget/packages`, ...) that is deliberately reused across runs. ~keep
        let cache_subdir = workspace.join("dist");
        std::fs::create_dir_all(&cache_subdir).expect("cache subdirectory");
        std::fs::write(cache_subdir.join("snippet.js"), b"cached").expect("cached artifact");

        let mut specs = HashMap::new();
        specs.insert("typescript".to_string(), spec);
        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(
            prepared.errors.is_empty(),
            "the `before` hook must run against an already-purged workspace: {:?}",
            prepared.errors
        );
        assert!(!stale_file.exists(), "the stale scratch file must be purged");
        assert!(
            cache_subdir.join("snippet.js").exists(),
            "cache subdirectories must survive the purge"
        );
    }

    /// The java incident this closes: `packages/java/.alef/snippets/sessions/` had accumulated
    /// four fingerprint-keyed directories dated across three separate days. alef's Java backend
    /// points Maven's `<sourceDirectory>` at `${project.basedir}`, so the session's own
    /// `mvn package` `before` hook compiled all four leftovers together and `javac` rejected them
    /// with `duplicate class: Example` — session preparation failed and all 283 java snippets were
    /// skipped. `JavaValidator` has written its scratch outside `working_directory` since the
    /// `external_workspace_directory` fix, so those directories were pre-fix leftovers that
    /// nothing swept: `--clean` only bypasses caches, and the per-fingerprint purge only ever
    /// looked inside the *current* fingerprint's directory. The `before` hook below globs the way
    /// Maven does rather than asserting a literal path, because a real consumer's hook does not
    /// know the fingerprint either. ~keep
    #[test]
    fn a_stale_session_directory_from_a_previous_run_cannot_break_the_current_one() {
        let directory = tempfile::tempdir().expect("temp directory");
        let stale = workspace_scratch_directory(directory.path(), "fingerprint-from-a-previous-run");
        std::fs::create_dir_all(&stale).expect("stale session directory");
        std::fs::write(stale.join("Example.java"), "public class Example {}").expect("stale source");
        std::fs::write(stale.join("Example.class"), b"stale").expect("stale class file");

        let mut specs = HashMap::new();
        specs.insert(
            "java".to_string(),
            SessionSpec {
                language: Language::Java,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec![posix_hook("find . -name Example.java -exec false {} +")],
                env: BTreeMap::new(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(
            prepared.errors.is_empty(),
            "a previous run's leftovers must not reach this run's `before` hook: {:?}",
            prepared.errors
        );
        assert!(
            !stale.exists(),
            "a stale session directory must not survive into the next run"
        );
    }

    /// A stale fingerprint's directory is removed outright while the live fingerprint's is kept
    /// and only swept of stray top-level files — the compiled-artifact caches in its
    /// subdirectories are deliberately reused across runs and must survive. ~keep
    #[test]
    fn a_stale_fingerprint_is_removed_while_the_live_one_keeps_its_caches() {
        let directory = tempfile::tempdir().expect("temp directory");
        let spec = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let live = workspace_scratch_directory(directory.path(), &session_fingerprint(&spec).expect("fingerprint"));
        std::fs::create_dir_all(live.join("dist")).expect("live cache directory");
        std::fs::write(live.join("dist/cached.js"), b"cached").expect("cached artifact");
        std::fs::write(live.join("snippet.ts"), "this does not compile: :::").expect("stale scratch file");
        let stale = workspace_scratch_directory(directory.path(), "fingerprint-from-a-previous-run");
        std::fs::create_dir_all(&stale).expect("stale session directory");
        std::fs::write(stale.join("snippet.ts"), "this does not compile: :::").expect("stale scratch file");

        let mut specs = HashMap::new();
        specs.insert("typescript".to_string(), spec);
        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
        assert!(!stale.exists(), "a stale fingerprint's directory must be removed");
        assert!(
            live.join("dist/cached.js").exists(),
            "the live fingerprint's caches must survive"
        );
        assert!(
            !live.join("snippet.ts").exists(),
            "the live fingerprint's stray scratch files must still be swept"
        );
    }

    /// Two targets can legitimately share one `working_directory` while differing in a way that
    /// changes the fingerprint. The purge therefore has to be computed over *all* of a directory's
    /// live fingerprints at once: a per-session purge would let whichever target ran second delete
    /// the first one's live scratch, turning the stale-session fix into a fresh collision. ~keep
    #[test]
    fn sibling_sessions_sharing_a_working_directory_keep_each_others_scratch() {
        let directory = tempfile::tempdir().expect("temp directory");
        let base = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut node = base.clone();
        node.env = BTreeMap::from([("ALEF_SESSION".into(), "node".into())]);
        let mut wasm = base;
        wasm.env = BTreeMap::from([("ALEF_SESSION".into(), "wasm".into())]);
        let fingerprints = [
            session_fingerprint(&node).expect("node fingerprint"),
            session_fingerprint(&wasm).expect("wasm fingerprint"),
        ];
        assert_ne!(fingerprints[0], fingerprints[1]);
        for fingerprint in &fingerprints {
            let workspace = workspace_scratch_directory(directory.path(), fingerprint);
            std::fs::create_dir_all(workspace.join("dist")).expect("cache directory");
            std::fs::write(workspace.join("dist/cached.js"), b"cached").expect("cached artifact");
        }

        let specs = HashMap::from([("node".to_string(), node), ("wasm".to_string(), wasm)]);
        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
        for fingerprint in &fingerprints {
            let cached = workspace_scratch_directory(directory.path(), fingerprint).join("dist/cached.js");
            assert!(
                cached.exists(),
                "a sibling session's live scratch must survive: {}",
                cached.display()
            );
        }
    }

    fn waiting_spec(language: Language, working_directory: &Path, own: &Path, sibling: &Path) -> SessionSpec {
        SessionSpec {
            language,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            before: vec![posix_hook(&format!(
                "touch {own}; attempts=0; while [ $attempts -lt {ACTIVATION_PROBE_ATTEMPTS} ]; do \
                 if [ -e {sibling} ]; then exit 0; fi; sleep 0.01; attempts=`expr $attempts + 1`; done; exit 1",
                own = sh_path(own),
                sibling = sh_path(sibling),
            ))],
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    /// How long a `before` hook waits for its sibling, in 10ms attempts. Long enough to absorb
    /// thread-pool startup on a loaded machine, short enough that a sequential regression fails the
    /// test in seconds instead of hanging it.
    const ACTIVATION_PROBE_ATTEMPTS: usize = 500;

    /// Two sessions in different working directories share nothing, and their `before` hooks are
    /// the expensive part of preparation (`pnpm build`, `mvn package`, ...). Each hook here refuses
    /// to return until it has seen the other one start, so a sequential phase three cannot satisfy
    /// both: the first would exhaust its attempts and fail preparation. ~keep
    #[test]
    fn before_hooks_in_different_working_directories_run_concurrently() {
        let first = tempfile::tempdir().expect("first directory");
        let second = tempfile::tempdir().expect("second directory");
        let first_marker = first.path().join("started");
        let second_marker = second.path().join("started");
        let specs = HashMap::from([
            (
                "typescript".to_string(),
                waiting_spec(Language::TypeScript, first.path(), &first_marker, &second_marker),
            ),
            (
                "python".to_string(),
                waiting_spec(Language::Python, second.path(), &second_marker, &first_marker),
            ),
        ]);

        let prepared = prepare_sessions_isolated(&specs, 30);

        assert!(
            prepared.errors.is_empty(),
            "both `before` hooks must be in flight at once: {:?}",
            prepared.errors
        );
        assert_eq!(prepared.sessions.len(), 2);
    }

    /// The other half of the constraint: two sessions that share a `working_directory` build the
    /// same tree in place, so their hooks must still run one after another. Each hook claims a
    /// marker for the duration of its run and fails if it finds the marker already claimed. ~keep
    #[test]
    fn before_hooks_sharing_a_working_directory_do_not_overlap() {
        let directory = tempfile::tempdir().expect("temp directory");
        let claim = directory.path().join("activating");
        let exclusive = format!(
            "test ! -e {claim} && touch {claim} && sleep 0.3 && rm {claim}",
            claim = claim.display()
        );
        let spec = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: vec![exclusive],
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut sibling = spec.clone();
        sibling.env = BTreeMap::from([("ALEF_SESSION".into(), "sibling".into())]);
        let specs = HashMap::from([("node".to_string(), spec), ("wasm".to_string(), sibling)]);

        let prepared = prepare_sessions_isolated(&specs, 30);

        assert!(
            prepared.errors.is_empty(),
            "sessions sharing a working directory must not build it concurrently: {:?}",
            prepared.errors
        );
        assert_eq!(prepared.sessions.len(), 2);
    }

    #[test]
    fn applies_environment_to_setup_and_validation_commands() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut specs = HashMap::new();
        specs.insert(
            "zig".into(),
            SessionSpec {
                language: Language::Zig,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec!["test \"$ALEF_SESSION_CACHE\" = configured".into()],
                env: BTreeMap::from([("ALEF_SESSION_CACHE".into(), "configured".into())]),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);
        assert!(prepared.errors.is_empty());
        let session = prepared.sessions.get("zig").expect("zig session");
        let mut command = std::process::Command::new("true");
        session.apply(&mut command);

        assert_eq!(
            command.get_envs().next(),
            Some(("ALEF_SESSION_CACHE".as_ref(), Some("configured".as_ref())))
        );
    }

    #[test]
    fn reuses_a_stable_workspace_for_a_prepared_session() {
        let directory = tempfile::tempdir().expect("temp directory");
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "neutral-fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let first = session.workspace_directory().expect("first workspace");
        std::fs::write(first.join("compiler-output"), "cached").expect("compiler output");
        let second = session.workspace_directory().expect("second workspace");

        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_to_string(second.join("compiler-output")).unwrap(),
            "cached"
        );
    }

    /// `external_workspace_directory` exists because alef's own Java backend emits sources at
    /// the package root and points Maven's `<sourceDirectory>` at `${project.basedir}` (see
    /// `packages/java/pom.xml`), making every path under a session's `working_directory` a live
    /// compiler input. Unlike `workspace_directory`, it must never resolve under
    /// `working_directory` at all, while still being stable and reused across calls for the same
    /// fingerprint so compiled-artifact caching still works.
    #[test]
    fn external_workspace_directory_stays_outside_the_working_directory_and_is_stable() {
        let directory = tempfile::tempdir().expect("temp directory");
        let fingerprint = format!(
            "external-workspace-fixture-{}",
            directory.path().to_string_lossy().replace(['/', '\\', ':'], "_")
        );
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint,
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let first = session
            .external_workspace_directory()
            .expect("first external workspace");
        assert!(
            !first.starts_with(directory.path()),
            "external workspace must never be nested under working_directory: {}",
            first.display()
        );
        std::fs::write(first.join("compiler-output"), "cached").expect("compiler output");
        let second = session
            .external_workspace_directory()
            .expect("second external workspace");

        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_to_string(second.join("compiler-output")).unwrap(),
            "cached"
        );
        let _ = std::fs::remove_dir_all(&first);
    }

    #[test]
    fn provides_absolute_isolated_toolchain_directories() {
        let directory = tempfile::tempdir().expect("temp directory");
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "neutral-fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let scratch = session.scratch_dir().expect("isolated scratch directory");
        assert!(scratch.path().starts_with(directory.path().join(".alef/snippets/tmp")));
        let mut command = std::process::Command::new("true");
        session.apply_environment(&mut command);
        let values = command
            .get_envs()
            .filter_map(|(name, value)| value.map(|value| (name.to_string_lossy().into_owned(), value.to_owned())))
            .collect::<BTreeMap<_, _>>();

        for name in TOOLCHAIN_CACHE_VARIABLES {
            assert!(std::path::Path::new(&values[*name]).is_absolute(), "{name}");
        }
        assert_eq!(
            std::path::Path::new(&values["CARGO_TARGET_DIR"]),
            session.cargo_target_directory()
        );
    }
}
