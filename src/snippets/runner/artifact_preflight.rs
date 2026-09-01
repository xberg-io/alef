//! Decides, once per session rather than once per snippet, whether the level this run asks for is
//! satisfiable at all — and reports the answer instead of spending a compiler on it N times.
//!
//! `alef all` never builds the full per-language artifacts that `compile`/`typecheck`/`run`
//! validation needs; only `alef build` does (see `--skip-snippet-validation`'s own help text). A
//! consumer whose `docs.snippets.validation_level` is `compile` therefore reaches this stage, on
//! the default path, with the artifacts absent — and the stage then spawned one real toolchain per
//! snippet against them. Every one of those processes was going to fail for the same reason, so
//! the run could not produce a true positive: what it produced was hours of wall clock and a
//! failure count that measured the environment. One reported run reached 7,795 snippets at
//! `compile` and was still going long after every write stage had finished.
//!
//! The same fact is also what made the per-snippet fallback rediscover a *shared* missing artifact
//! N separate times. Scoping the question to the session — the unit that owns the artifact — is
//! what fixes both: one probe, one log line, N results.
//!
//! ## Why this is not the gate alef already deleted
//!
//! A static `enforce_build_dependency` pre-flight used to bail whenever a language had no
//! configured `sessions.<target>.before` step, which said nothing about whether that language
//! needed a build; it read config *shape*, its own doc admitted its verdict could not change, and
//! it was removed. Everything here is different in the ways that mattered:
//!
//! - The evidence is the filesystem, not the config: a path the session's own manifest names, and
//!   `alef build` fills, is absent (see `validators::session_artifacts`).
//! - The default answer is "satisfiable". A validator that declares no artifact — every one but
//!   the three that can read theirs off a manifest today — validates exactly as before, so an
//!   unimplemented probe costs nothing and a missing probe never manufactures a skip.
//! - The verdict is falsifiable by running `alef build`, which is precisely the remedy reported.
//!
//! ## Why a skip here is not a pass
//!
//! A skipped snippet is reported as `SnippetStatus::Unavailable` with `unresolved_dependency` set
//! — bit for bit the classification the per-snippet path already produced when it discovered the
//! same missing artifact the expensive way (`runner::finalize_result`). So it stays out of
//! `fully_verified`, still trips `RunSummary::checked_nothing`, still reaches
//! `output::unresolved_dependency_rollup`'s "run `alef build`" remediation, and still lands on
//! whichever side of `--strict` this pipeline already put it on. Detecting the fact earlier
//! changes what the run *costs*, never what it *concludes*. `preflight_skipped` is set on top so
//! the saving is visible as a count rather than dissolving into a bucket. ~keep

use super::RunnerConfig;
use super::levels::capped_level;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Snippet, SnippetStatus, ValidationLevel, ValidationResult};
use crate::snippets::validators::ValidatorRegistry;
use std::collections::HashMap;
use std::path::PathBuf;

/// How many missing artifact paths one report names before it stops listing them. A session that
/// is missing its whole build output can name many; the first few identify the build that has not
/// run, which is all the remedy needs.
const REPORTED_ARTIFACT_LIMIT: usize = 4;

/// The sessions this run cannot satisfy, keyed by session fingerprint.
///
/// Fingerprint, not config target name: `alef.toml` may alias two target names (a language
/// fallback and an explicit binding-package target) onto one `cwd`/manifest, which resolves to a
/// single physical workspace and a single set of artifacts. Keying by name would let one alias be
/// skipped and its twin spawn the very processes this exists to prevent — the same aliasing bug
/// `session_locks_by_fingerprint` already had to fix once. ~keep
pub(super) struct ArtifactPreflight {
    unsatisfiable: HashMap<String, MissingArtifacts>,
}

struct MissingArtifacts {
    language: crate::snippets::types::Language,
    paths: Vec<PathBuf>,
}

impl ArtifactPreflight {
    /// Probes every session at least one snippet in this run actually claims, and emits one `WARN`
    /// per unsatisfiable session naming the level, the snippet count, the missing artifacts and
    /// the remedy.
    ///
    /// `Syntax` is excluded outright: it resolves no imports and links nothing, so no artifact can
    /// be a precondition for it. Sessions are probed once each even when hundreds of snippets
    /// claim them, which is the whole point.
    pub(super) fn inspect(
        snippets: &[Snippet],
        registry: &ValidatorRegistry,
        config: &RunnerConfig,
        sessions: &HashMap<String, ValidationSession>,
    ) -> Self {
        if config.level == ValidationLevel::Syntax {
            return Self {
                unsatisfiable: HashMap::new(),
            };
        }
        let claimed = claimed_snippet_counts(snippets, registry, config, sessions);
        let mut unsatisfiable = HashMap::new();
        for (fingerprint, (session, snippet_count)) in claimed {
            let Some(validator) = registry.get(session.language) else {
                continue;
            };
            let paths = validator.missing_session_artifacts(session, config.level);
            if paths.is_empty() {
                continue;
            }
            report(session, config.level, snippet_count, &paths);
            unsatisfiable.insert(
                fingerprint,
                MissingArtifacts {
                    language: session.language,
                    paths,
                },
            );
        }
        Self { unsatisfiable }
    }

    /// The result for a snippet whose session cannot be satisfied, or `None` when it can.
    ///
    /// Callers must consult this only after the cheap short-circuits `validate_one` already
    /// performs — a cache hit, a `skip` annotation, a side-effect rejection, a missing toolchain.
    /// Those answers are true regardless of the artifacts and cost nothing, and overriding a
    /// cached real result with a skip would throw away work that already happened. `effective`
    /// is the snippet's own capped level, so a `syntax-only` annotation or a front-matter
    /// `level: syntax` contract keeps validating normally inside an otherwise unsatisfiable
    /// session. ~keep
    pub(super) fn skipped_result(
        &self,
        snippet: &Snippet,
        config: &RunnerConfig,
        session: Option<&ValidationSession>,
        effective: ValidationLevel,
    ) -> Option<ValidationResult> {
        if effective == ValidationLevel::Syntax {
            return None;
        }
        let missing = self.unsatisfiable.get(session?.fingerprint.as_str())?;
        let mut result = super::result(
            snippet,
            SnippetStatus::Unavailable,
            config.level,
            config.level,
            Some(message(missing, effective)),
            0,
        );
        result.unresolved_dependency = true;
        result.preflight_skipped = true;
        Some(result)
    }

    /// Whether a batch group for this session would be doomed, so `batch_level` can decline to
    /// form it. Without this the per-snippet path would be spared its N spawns while the batch
    /// path still made one — a single process, but one that reports N failures for a corpus
    /// nobody checked. ~keep
    pub(super) fn is_unsatisfiable(&self, session: Option<&ValidationSession>, effective: ValidationLevel) -> bool {
        effective != ValidationLevel::Syntax
            && session.is_some_and(|session| self.unsatisfiable.contains_key(session.fingerprint.as_str()))
    }
}

/// Every session claimed by at least one snippet that would actually be validated above `Syntax`,
/// paired with how many such snippets claim it.
///
/// Counting the snippets, rather than just collecting the sessions, is what lets the report say
/// how much this run is skipping — a bare "artifacts missing" line leaves the reader unable to
/// tell a one-snippet corner from the 7,795-snippet stall this exists for.
fn claimed_snippet_counts<'a>(
    snippets: &[Snippet],
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &'a HashMap<String, ValidationSession>,
) -> HashMap<String, (&'a ValidationSession, usize)> {
    let mut claimed: HashMap<String, (&ValidationSession, usize)> = HashMap::new();
    for snippet in snippets {
        let Some(session) = super::session_for(snippet, sessions) else {
            continue;
        };
        let Some(validator) = registry.get(snippet.language) else {
            continue;
        };
        if capped_level(snippet, config, validator) == ValidationLevel::Syntax {
            continue;
        }
        let entry = claimed.entry(session.fingerprint.clone()).or_insert((session, 0));
        entry.1 += 1;
    }
    claimed
}

/// The single log line that replaces the multi-hour stall. `warn`, not `info`: a run that silently
/// stops checking a language is exactly what an operator must be told about, even though this
/// pipeline treats the underlying build gap as expected. ~keep
fn report(session: &ValidationSession, level: ValidationLevel, snippet_count: usize, paths: &[PathBuf]) {
    tracing::warn!(
        language = %session.language,
        level = %level,
        snippet_count,
        missing_artifacts = %rendered(paths),
        working_directory = %session.working_directory.display(),
        "skipping snippet validation for this session: the build artifacts its manifest points at do not exist; \
         these snippets are reported unvalidated, not passed -- run `alef build` first, or pass \
         --skip-snippet-validation for a generate-only run"
    );
}

fn message(missing: &MissingArtifacts, effective: ValidationLevel) -> String {
    format!(
        "could not validate at {effective}: skipped before spawning a {} toolchain because this session's build \
         artifacts do not exist ({}) -- run `alef build` first",
        missing.language,
        rendered(&missing.paths)
    )
}

fn rendered(paths: &[PathBuf]) -> String {
    let listed = paths
        .iter()
        .take(REPORTED_ARTIFACT_LIMIT)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    match paths.len().checked_sub(REPORTED_ARTIFACT_LIMIT) {
        Some(remainder) if remainder > 0 => format!("{listed}, +{remainder} more"),
        _ => listed,
    }
}
