use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::config::warning_ack::{AcknowledgeableWarningCategory, WarningAcknowledgement};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};
use crate::core::warning_ack::{AckOutcome, AcknowledgementLedger};
use crate::e2e::codegen::{E2eCodegen, all_generators};
use crate::e2e::fixture::{Fixture, FixtureDocs, SideEffectClass};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub mod coverage;
mod exclusions;
pub mod language_filter;
pub(crate) mod ledger_paths;
pub mod migration;
pub(crate) mod mock_harness_guard;
mod mock_url_defaults;
pub mod ownership;
mod recipe_policy;
mod render_body;
mod sample_url_policy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetInclusion {
    Include,
    Exclude { missing_requirements: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct GeneratedSnippet {
    pub file: GeneratedFile,
    pub fixture_id: String,
    pub fixture_source: String,
    pub language: String,
    pub requirements: Vec<String>,
    pub side_effects: SideEffectClass,
}

pub const COVERAGE_MANIFEST: &str = ".alef-snippet-coverage.json";
pub const COVERAGE_MANIFEST_VERSION: u32 = 2;

/// True when `path`'s file name is alef's own snippet-coverage ledger.
///
/// The ledger is strict JSON, so it can never carry an `alef:hash:` provenance marker, and
/// `write_scaffold_files_report`'s write-time ownership guard falls back to the committed
/// `.alef-ownership.toml` record for exactly that reason. That record is populated by the
/// guard itself the first time it *creates* a path, but a ledger written before this
/// mechanism existed — or one whose only prior writes happened to leave content
/// byte-identical to disk, which records nothing by design (byte-equality is never proof of
/// authorship) — reaches this guard already `exists()` and unrecorded, and is refused
/// forever: refusing means the write never happens, so the record that would unblock the
/// *next* write is never established either.
///
/// This is deliberately not folded into a general "unmarkable extension" filename
/// allowlist the way `orphans.rs`'s `UNMARKABLE_ALEF_MANIFESTS` covers
/// `composer.json`/`package.json` for orphan reclaim: those names a human plausibly authors
/// independently of alef, so trusting the name alone there would risk silently overwriting
/// hand-written content. This ledger's dotfile name has no meaning or use to anything but
/// alef's own coverage bookkeeping — nothing else ever reads or writes it — so a name match
/// here is sufficient proof of exclusive alef authorship without weakening the guard's
/// protection for any other path. Consulted from
/// `cli::pipeline::generate::scaffold::write_scaffold_files_report`'s ownership check; once
/// it lets the write through once, the guard's own write-time registration records the path
/// durably and this predicate is never needed again for that tree. ~keep
pub fn is_snippet_coverage_manifest_path(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(COVERAGE_MANIFEST)
}

/// Requirement namespace for a Cargo crate a generated Rust snippet body names directly. ~keep
/// The Rust snippet validator resolves these into `[dependencies]` of the check project.
pub const CRATE_REQUIREMENT_PREFIX: &str = "crate:";

const SERDE_JSON_REQUIREMENT: &str = "crate:serde_json";

/// `rust/snippet_body.rs.jinja` emits `#[tokio::main]` for an async fixture, so the snippet ~keep
/// carries a tokio dependency the fixture's own config never declares. Without this requirement
/// the check project has no `tokio` in `[dependencies]` and every async Rust snippet fails to
/// resolve the attribute macro (E0433) before any of its actual content is checked.
const TOKIO_REQUIREMENT: &str = "crate:tokio";

/// `streaming_assertions::StreamingFieldResolver::collect_snippet` drains a Rust stream through ~keep
/// `tokio_stream::StreamExt`, so a streaming fixture's snippet names a crate no fixture config
/// declares — alef's own recipe is what put the path there. Spelled with the package's real
/// hyphenated name, because that is what `[dependencies]` has to resolve on the registry; the
/// `tokio_stream::` the body writes is the underscored lib name Cargo derives from it.
const TOKIO_STREAM_REQUIREMENT: &str = "crate:tokio-stream";

/// The crates alef's own Rust snippet recipes name in a body, keyed by the text that proves the
/// body names them. Fixture config never mentions these: the recipe emitted the path, so the
/// recipe owes the dependency. ~keep
pub(crate) const RUST_BODY_CRATE_REQUIREMENTS: &[(&str, &str)] = &[
    ("serde_json::", SERDE_JSON_REQUIREMENT),
    ("#[tokio::main]", TOKIO_REQUIREMENT),
    ("tokio_stream::", TOKIO_STREAM_REQUIREMENT),
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnippetCoverageKey {
    pub fixture_id: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedSnippetMetadata {
    pub key: SnippetCoverageKey,
    pub path: PathBuf,
    pub language: String,
    pub target: String,
    pub session: String,
    pub requires: Vec<String>,
    pub side_effect: SideEffectClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingSnippet {
    pub key: SnippetCoverageKey,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentedSnippetException {
    pub key: SnippetCoverageKey,
    pub reason: String,
    pub reference: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetCoverageLedger {
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub generated_paths: Vec<PathBuf>,
    #[serde(default)]
    pub generated_metadata: Vec<GeneratedSnippetMetadata>,
    pub expected: Vec<SnippetCoverageKey>,
    pub generated: Vec<SnippetCoverageKey>,
    pub missing: Vec<MissingSnippet>,
    pub documented_exceptions: Vec<DocumentedSnippetException>,
}

/// A snippet body the mock-harness guard refused, kept out of the coverage ledger. ~keep
///
/// This deliberately does not live on [`SnippetCoverageLedger`]: the ledger is the
/// serialized manifest, and a guard rejection is never a durable state a run may come to
/// rest in — it aborts generation. Carrying it on the in-memory report instead keeps the
/// on-disk manifest format unchanged while still giving the caller per-language,
/// per-marker attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetGuardRejection {
    pub key: SnippetCoverageKey,
    pub marker: String,
}

/// The guard's typed failure, so a caller can tell "this body leaked harness scaffolding" ~keep
/// apart from every other reason a recipe can fail to render. Without the type the two are
/// indistinguishable strings, and a `coverage_exceptions` entry authored for an unrelated
/// capability gap silently absorbs a leak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockHarnessLeak {
    pub marker: String,
    pub fixture_id: String,
    pub language: String,
}

impl std::fmt::Display for MockHarnessLeak {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "`{}` snippet for fixture `{}` leaks e2e mock-server scaffolding (`{}`); \
             a documentation snippet must construct its client the way a reader would",
            self.language, self.fixture_id, self.marker
        )
    }
}

impl std::error::Error for MockHarnessLeak {}

#[derive(Debug, Clone, Default)]
pub struct SnippetGenerationReport {
    pub snippets: Vec<GeneratedSnippet>,
    pub coverage: SnippetCoverageLedger,
    /// Always empty on a successful run: a non-empty value aborts generation. ~keep
    pub guard_rejections: Vec<SnippetGuardRejection>,
    /// Fixtures whose published snippets carry the reserved-domain placeholder, from either ~keep
    /// cause `render_body::PlaceholderSampleUrlLedger` keeps apart: no public address was
    /// configured for them, or the fixture's own `docs.sample_url` failed to produce one.
    ///
    /// Empty for a fixture that inherited `[crates.e2e.snippets].mock_only`, which states that
    /// no such address exists to configure -- and never empty merely because that flag is set,
    /// since it does not reach the second cause. See `sample_url_policy`'s module doc comment.
    ///
    /// Not a failure -- a project may have no public sample host, and refusing to generate
    /// would break every consumer that has lived with the placeholder. It is reported so the
    /// run states what it published rather than implying the snippets are runnable.
    pub placeholder_sample_url_fixtures: Vec<String>,
    /// Total number of `doc_snippet_reserved_domain` warning occurrences a configured
    /// `[[crates.e2e.snippets.acknowledged_warnings]]` entry matched and suppressed this run.
    /// Zero when no acknowledgements are configured or none matched -- see
    /// `crate::core::warning_ack::AcknowledgementLedger`. Task #540 requires this stay visible
    /// rather than the suppressed set going unreported. ~keep
    pub acknowledged_warning_count: usize,
    /// Project-root-relative paths that `[crates.e2e.snippets].curated_snippets` claims as
    /// hand-authored on purpose, resolved by [`coverage::resolve_curated_snippet_paths`].
    ///
    /// Project-root-relative rather than `output`-relative because hand-authored snippets
    /// characteristically sit BESIDE the generated tree rather than inside it; see that
    /// function for the measurement behind the choice.
    ///
    /// Deliberately not part of [`SnippetCoverageLedger`] -- `coverage.expected`/`generated`/
    /// `missing`/`documented_exceptions` are all keyed by fixture/language cell, and a curated
    /// file has no fixture behind it at all. Kept here instead so a caller can report "N
    /// curated, M generated" without conflating the two coverage dimensions.
    pub curated_paths: Vec<PathBuf>,
}

struct SnippetRenderContext<'a> {
    e2e: &'a E2eConfig,
    crate_config: &'a ResolvedCrateConfig,
    type_defs: &'a [TypeDef],
    enums: &'a [EnumDef],
    functions: &'a [crate::core::ir::FunctionDef],
    errors: &'a [crate::core::ir::ErrorDef],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentationLanguage {
    Binding(Language),
    Shell,
}

impl DocumentationLanguage {
    fn code_fence(self) -> &'static str {
        match self {
            Self::Binding(language) => crate::docs::naming::lang_code_fence(language),
            Self::Shell => "bash",
        }
    }

    fn canonical_name(self) -> &'static str {
        self.code_fence()
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Binding(language) => crate::docs::naming::lang_display_name(language),
            Self::Shell => "Shell",
        }
    }
}

#[expect(clippy::too_many_arguments, reason = "preserves the public snippet generation API")]
pub fn generate_snippets(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<Vec<GeneratedFile>> {
    Ok(generate_snippet_report(
        fixtures,
        languages,
        e2e,
        snippets,
        crate_config,
        type_defs,
        enums,
        functions,
        &[],
    )?
    .snippets
    .into_iter()
    .map(|snippet| snippet.file)
    .collect())
}

/// ~keep `errors` is `ApiSurface::errors`. Only a caller that holds it can let a snippet name the
/// exception class a specific error variant maps to (see `codegen::snippet_error_branch`); the
/// coverage-only paths, which never publish a rendered body, pass an empty slice and get the
/// generic catch-all, which is exactly what they rendered before.
#[expect(clippy::too_many_arguments, reason = "preserves the public snippet generation API")]
pub fn generate_snippet_report(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) -> Result<SnippetGenerationReport> {
    crate::with_extensions(|extensions| {
        let context = SnippetRenderContext {
            e2e,
            crate_config,
            type_defs,
            enums,
            functions,
            errors,
        };
        generate_snippet_report_with_extensions(fixtures, languages, snippets, &context, extensions)
    })
}

fn generate_snippet_report_with_extensions(
    fixtures: &[Fixture],
    languages: &[String],
    snippets: &SnippetConfig,
    context: &SnippetRenderContext<'_>,
    extensions: &[Box<dyn crate::Extension>],
) -> Result<SnippetGenerationReport> {
    validate_relative_path(Path::new(&snippets.output), "snippet output")?;
    // Resolve before anything renders: an unusable sample base URL, template or manifest must ~keep
    // fail the run, not reach published documentation as a broken address -- and a corpus that
    // both declares itself mock-only and configures a public host must fail before that
    // contradiction gets to decide a warning. See `sample_url_policy::SampleUrlPolicy::resolve`.
    let url_policy = sample_url_policy::SampleUrlPolicy::resolve(snippets, Path::new("."))?;
    // Pin the *previous* run's ownership record before this run computes, let alone writes,
    // anything. `e2e::run` hands the freshly computed ledger to the same write batch as the
    // snippets, and `.alef-snippet-coverage.json` sorts ahead of every sibling snippet directory
    // in that batch's `BTreeMap`, so reading it any later would read this run's intentions and
    // silently degrade `ownership::is_ledger_owned_snippet_path` to bare path identity. ~keep
    ownership::snapshot_pre_run_ledger(Path::new(&snippets.output));
    // Only `doc_snippet_reserved_domain` is meaningful at this config location; a
    // `virtual_field_path` entry configured under `[crates.e2e.snippets]` is rejected here
    // rather than reported as merely stale below -- see `WarningAckError::OutOfScope`. ~keep
    let mut ack_ledger = AcknowledgementLedger::new(
        &[AcknowledgeableWarningCategory::DocSnippetReservedDomain],
        snippets.acknowledged_warnings.clone(),
    )
    .map_err(|error| anyhow::anyhow!("invalid `[crates.e2e.snippets].acknowledged_warnings`: {error}"))?;
    let generators = snippet_generators(languages)?;
    let mut generated = BTreeMap::<PathBuf, GeneratedSnippet>::new();
    let mut guard_rejections = Vec::<SnippetGuardRejection>::new();
    let mut placeholder_sample_urls = render_body::PlaceholderSampleUrlLedger::default();
    let mut coverage = SnippetCoverageLedger {
        format_version: COVERAGE_MANIFEST_VERSION,
        ..SnippetCoverageLedger::default()
    };
    for fixture in fixtures {
        validate_requirements(fixture)?;
        validate_coverage_exceptions(fixture)?;
        validate_docs_paths(fixture, languages)?;
        // Resolved here, beside the other per-fixture validators, rather than inside the render
        // seam below: a render failure there is caught and recorded as a missing coverage cell,
        // so an unusable `docs.sample_url` would degrade into a silently absent snippet instead
        // of failing the run the way an unusable corpus-level base does. Also the natural place
        // for it -- the answer is per fixture, not per fixture/language. ~keep
        let sample_url = url_policy.for_fixture(fixture)?;
        for (language, generator) in &generators {
            // A function this language's `exclude_functions` (or the crate-wide
            // `[crates.exclude].functions` that `language_excludes` folds in) drops can
            // never be emitted here, so the cell must not enter `expected` at all --
            // pushing it and then failing to generate it is exactly the ledger/emitter
            // disagreement this check exists to prevent. See
            // `function_excluded_for_language`'s doc comment for why this reuses the
            // docs generator's exclusion accessor instead of re-deriving the rule. ~keep
            // A call declaring `skip_languages` for this language is already excluded from
            // the executable e2e suite by `crate::e2e::codegen::fixture_inclusion` --
            // consulting the same `call_skip_reason` seam here, rather than re-deriving the
            // check, is what keeps the snippet generator from attempting (and failing) a
            // render for a call the target language cannot represent at all. This is
            // deliberately narrower than a fixture-level `skip` directive, which opts a
            // fixture out of the executable harness only and must NOT suppress a
            // documentation snippet -- see `call_skip_reason`'s doc comment and
            // `documentation_rendering_is_independent_of_test_harness_skips`. ~keep
            // A function/method the IR itself marked `binding_excluded`
            // (`#[alef::skip]`/`#[doc(hidden)]`) is excluded from every non-Rust binding
            // regardless of `alef.toml` -- `function_excluded_for_language` only ever
            // consults `alef.toml`-configured lists and never sees this flag. See
            // `function_binding_excluded_for_language`'s doc comment. ~keep
            if crate::e2e::codegen::call_skip_reason(fixture, language, context.e2e).is_some()
                || exclusions::function_excluded_for_language(fixture, language, generator.language_name(), context)
                || exclusions::visitor_excluded_for_language(fixture, generator.language_name(), context)
                || exclusions::function_binding_excluded_for_language(
                    fixture,
                    language,
                    generator.language_name(),
                    context,
                )
            {
                continue;
            }
            let key = SnippetCoverageKey {
                fixture_id: fixture.id.clone(),
                language: language.to_string(),
            };
            coverage.expected.push(key.clone());
            let Some(docs) = fixture.docs.as_ref() else {
                coverage.missing.push(MissingSnippet {
                    key,
                    reason: "fixture has no documentation metadata".to_string(),
                });
                continue;
            };
            let capabilities = capabilities(language, snippets, context.crate_config);
            let capability_decision = snippet_inclusion(fixture, &capabilities);
            let exclusion_reason = match &capability_decision {
                SnippetInclusion::Exclude { missing_requirements } => {
                    Some(format!("missing requirements: {}", missing_requirements.join(", ")))
                }
                SnippetInclusion::Include => None,
            };
            if let Some(reason) = exclusion_reason {
                if let Some(exception) = coverage_exception(docs, language) {
                    coverage.documented_exceptions.push(DocumentedSnippetException {
                        key,
                        reason: exception.reason.clone(),
                        reference: exception.documentation.clone(),
                    });
                } else {
                    coverage.missing.push(MissingSnippet { key, reason });
                }
                continue;
            }
            let lang = parse_language(generator.language_name()).ok_or_else(|| {
                anyhow::anyhow!(
                    "e2e code generator `{}` has no documentation language mapping",
                    generator.language_name()
                )
            })?;
            let path = snippet_path(&snippets.output, docs, &fixture.id, language, lang)?;
            let rendered = match render_body::render_snippet_body(
                extensions,
                generator.as_ref(),
                fixture,
                language,
                context,
                &sample_url,
            ) {
                Ok(rendered) => rendered,
                Err(error) => {
                    // A guard rejection is a generator defect, not a documented limitation, so ~keep
                    // it is recorded separately and is deliberately *not* eligible for the
                    // coverage-exception branch below. Routing it there is what turned a
                    // rejected snippet into a silent deletion.
                    if let Some(leak) = error.downcast_ref::<MockHarnessLeak>() {
                        guard_rejections.push(SnippetGuardRejection {
                            key: key.clone(),
                            marker: leak.marker.clone(),
                        });
                        coverage.missing.push(MissingSnippet {
                            key,
                            reason: format!("{error:#}"),
                        });
                        continue;
                    }
                    if let Some(exception) = coverage_exception(docs, language) {
                        coverage.documented_exceptions.push(DocumentedSnippetException {
                            key,
                            reason: exception.reason.clone(),
                            reference: exception.documentation.clone(),
                        });
                    } else {
                        coverage.missing.push(MissingSnippet {
                            key,
                            reason: format!("{error:#}"),
                        });
                    }
                    continue;
                }
            };
            let render_body::RenderedSnippetBody {
                body,
                placeholder_class,
            } = rendered;
            placeholder_sample_urls.record(&mut ack_ledger, placeholder_class, &fixture.id, language);
            let content = render_snippet_markdown(&body, fixture, docs, language, lang);
            let requirements = snippet_requirements(fixture, language, &body);
            let file = GeneratedFile {
                path: path.clone(),
                content,
                generated_header: false,
            };
            let snippet = GeneratedSnippet {
                file,
                fixture_id: fixture.id.clone(),
                fixture_source: fixture.source.clone(),
                language: language.to_string(),
                requirements: requirements.clone(),
                side_effects: docs.side_effects,
            };
            if generated.insert(path.clone(), snippet).is_some() {
                bail!("snippet output collision at {}", path.display());
            }
            let relative_path = path
                .strip_prefix(&snippets.output)
                .context("generated snippet path escaped the configured output root")?
                .to_path_buf();
            coverage.generated_paths.push(relative_path.clone());
            coverage.generated_metadata.push(GeneratedSnippetMetadata {
                key: key.clone(),
                path: relative_path,
                language: lang.canonical_name().to_string(),
                target: language.to_string(),
                session: language.to_string(),
                requires: requirements,
                side_effect: docs.side_effects,
            });
            coverage.generated.push(key);
        }
    }
    // Abort before the caller reports coverage, prunes orphans, or writes anything: a run ~keep
    // that deleted the stale files and *then* failed would still have destroyed published
    // documentation.
    guard_rejections.sort_by(|left, right| left.key.cmp(&right.key));
    ensure_no_guard_rejections(&guard_rejections)?;
    coverage = coverage::normalize(coverage);
    coverage::validate(&coverage)?;
    let placeholder_sample_url_fixtures = placeholder_sample_urls.fixtures();
    placeholder_sample_urls.report(url_policy.base());
    // Requirement 2: a configured acknowledgement that matched nothing this run -- because the
    // warning was fixed, never fired for that identity/target, or was mistyped -- fails the run
    // here, before the caller writes any snippet to disk. This is deliberately not deferred the
    // way `deferred_error` defers generator failures in `e2e::mod.rs`: a stale acknowledgement
    // is a configuration defect the consumer must see and fix, not a transient render failure. ~keep
    let acknowledgement_report = ack_ledger
        .finish()
        .map_err(|error| anyhow::anyhow!("`[crates.e2e.snippets].acknowledged_warnings`: {error}"))?;
    render_body::report_acknowledged_warnings(&acknowledgement_report);
    // The project root is the process working directory here: `snippets.output` is itself an
    // unresolved project-root-relative configuration string, and every generated path above is
    // built by joining it. Curated globs share that base by construction. ~keep
    let curated_paths = coverage::resolve_curated_snippet_paths(Path::new("."), &snippets.curated_snippets)?;
    let generated_from_project_root: Vec<PathBuf> = coverage
        .generated_paths
        .iter()
        .map(|relative| Path::new(&snippets.output).join(relative))
        .collect();
    coverage::reject_generated_curated_paths(&curated_paths, &generated_from_project_root)?;
    if !snippets.curated_snippets.is_empty() {
        tracing::info!(
            target: "alef::e2e::snippets",
            "snippet coverage: {}",
            coverage::summary(curated_paths.len(), coverage.generated_paths.len())
        );
    }
    Ok(SnippetGenerationReport {
        snippets: generated.into_values().collect(),
        coverage,
        guard_rejections,
        placeholder_sample_url_fixtures,
        acknowledged_warning_count: acknowledgement_report.matched_count,
        curated_paths,
    })
}

/// Find a fixture's coverage exception for `language`.
///
/// Both the exception's declared key and `language` resolve through
/// [`crate::e2e::fixture::canonical_language`], so an exception declared under one accepted
/// spelling of a backend (e.g. `docs.coverage_exceptions = { c = ... }`) still applies when
/// that backend is configured under an alias spelling (e.g. running as `ffi`). Without this,
/// `"c"` and `"ffi"` silently stopped matching each other and the exception never fired. ~keep
fn coverage_exception<'a>(
    docs: &'a FixtureDocs,
    language: &str,
) -> Option<&'a crate::e2e::fixture::SnippetCoverageException> {
    let canonical = crate::e2e::fixture::canonical_language(language);
    docs.coverage_exceptions
        .iter()
        .find(|(key, _)| crate::e2e::fixture::canonical_language(key) == canonical)
        .map(|(_, exception)| exception)
}

/// The canonical e2e language name every registered code generator declares, used to
/// validate `docs.coverage_exceptions` keys against real backends rather than arbitrary
/// strings.
///
/// Delegates to [`crate::e2e::known_e2e_target_names`] -- the same enumeration
/// `validate_skip_languages` (`src/e2e/fixture.rs`) checks `skip.languages` ids against --
/// rather than re-deriving the generator list here. Two independently computed "is this a
/// real backend" enumerations is the same defect shape as two alias lists: nothing forces
/// them to agree, and only one needs to fall behind for a validator to start passing
/// nonsense through. ~keep
fn known_language_names() -> BTreeSet<String> {
    crate::e2e::known_e2e_target_names().into_iter().collect()
}

/// Render `known` as a human-readable list, expanding any alias groups (e.g. `"c"` also
/// accepts `"c_ffi"` and `"ffi"`) so a validation error can name every accepted spelling.
fn describe_known_languages(known: &BTreeSet<String>) -> String {
    known
        .iter()
        .map(|canonical| {
            let aliases = crate::e2e::fixture::language_alias_groups()
                .iter()
                .find(|(name, _)| *name == canonical.as_str())
                .map(|(_, aliases)| *aliases)
                .unwrap_or_default();
            if aliases.is_empty() {
                canonical.clone()
            } else {
                format!("{canonical} (also accepted: {})", aliases.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_coverage_exceptions(fixture: &Fixture) -> Result<()> {
    let Some(docs) = &fixture.docs else {
        return Ok(());
    };
    let known = known_language_names();
    for (language, exception) in &docs.coverage_exceptions {
        if language.trim().is_empty() || exception.reason.trim().is_empty() {
            bail!(
                "fixture `{}` has invalid coverage exception for language `{language}`: language and reason must be non-empty",
                fixture.id
            );
        }
        if !known.contains(crate::e2e::fixture::canonical_language(language)) {
            bail!(
                "fixture `{}` has a coverage exception for unknown language `{language}`; accepted language spellings: {}",
                fixture.id,
                describe_known_languages(&known)
            );
        }
        validate_documentation_reference(&exception.documentation).map_err(|error| {
            anyhow::anyhow!(
                "fixture `{}` has invalid coverage exception documentation for language `{language}`: {error}",
                fixture.id
            )
        })?;
    }
    Ok(())
}

fn validate_documentation_reference(reference: &str) -> Result<()> {
    if reference.trim() != reference || reference.is_empty() {
        bail!("reference must be non-empty and have no surrounding whitespace");
    }
    if reference.starts_with("https://") || reference.starts_with("http://") {
        if reference.chars().any(char::is_whitespace) {
            bail!("URL reference must not contain whitespace");
        }
        return Ok(());
    }
    validate_relative_path(Path::new(reference), "documentation reference")
}

/// Turn every guard rejection this run produced into one aborting, attributed error. ~keep
///
/// A rejection must never come to rest as a coverage gap: `missing` cells can be retired by
/// writing a `docs.coverage_exceptions` entry, and an exception authored for an unrelated
/// capability gap would then also retire a leak — deleting the snippet from the docs tree
/// with no signal at all. Failing here, before the caller prunes orphans or writes any
/// file, is what makes the denylist's failure mode loud instead of a silent deletion.
fn ensure_no_guard_rejections(rejections: &[SnippetGuardRejection]) -> Result<()> {
    if rejections.is_empty() {
        return Ok(());
    }
    let mut by_language: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    for rejection in rejections {
        by_language
            .entry(rejection.key.language.as_str())
            .or_default()
            .entry(rejection.marker.as_str())
            .or_default()
            .push(rejection.key.fixture_id.as_str());
    }
    let mut detail = String::new();
    for (language, markers) in &by_language {
        let language_total: usize = markers.values().map(Vec::len).sum();
        detail.push_str(&format!("\n  {language} ({language_total}):"));
        for (marker, fixtures) in markers {
            detail.push_str(&format!(
                "\n    `{marker}` ({}): {}",
                fixtures.len(),
                fixtures.join(", ")
            ));
        }
    }
    bail!(
        "{} documentation snippet(s) were rejected by the mock-harness guard. Fix the generator so \
         the snippet constructs its client the way a reader would — a `docs.coverage_exceptions` \
         entry cannot retire a guard rejection.{detail}",
        rejections.len()
    )
}

fn snippet_generators(languages: &[String]) -> Result<Vec<(&str, Box<dyn E2eCodegen>)>> {
    let mut available = BTreeMap::new();
    for generator in all_generators() {
        let name = generator.language_name();
        if available.insert(name, generator).is_some() {
            bail!("duplicate e2e code generator registered for snippet language `{name}`");
        }
    }
    let mut requested = BTreeSet::new();
    languages
        .iter()
        .map(|language| {
            let generator_name = generator_name(language);
            if !requested.insert(generator_name) {
                bail!("duplicate snippet language resolves to e2e code generator `{generator_name}`");
            }
            available
                .remove(generator_name)
                .map(|generator| (language.as_str(), generator))
                .ok_or_else(|| anyhow::anyhow!("no e2e code generator registered for snippet language `{language}`"))
        })
        .collect()
}

/// Resolve a configured e2e language to the generator name registered in [`all_generators`].
///
/// Delegates entirely to [`crate::e2e::fixture::canonical_language`] -- the single alias
/// table for backends with more than one accepted spelling -- rather than carrying its own
/// copy of the `"c"`/`"c_ffi"`/`"ffi"` and `"rust"`/`"core"`/`"rust_core"` groups.
fn generator_name(language: &str) -> &str {
    crate::e2e::fixture::canonical_language(language)
}

/// The CLI invocation that produces fixture snippets, embedded verbatim in every
/// snippet's provenance header by [`render_snippet_markdown`].
const SNIPPET_REGENERATE_COMMAND: &str = "alef e2e generate";

/// Render one fixture snippet as a self-marking Markdown document.
///
/// The provenance header comes from [`crate::docs::with_html_header`] — the same emitter
/// `readme::template` and `docs::render` use — rather than a second marker producer, so the
/// bytes the ownership guard reads back are byte-identical across every `.md` alef writes.
///
/// Why it is needed here at all: `write::marker_comment_style`'s doc excludes `.md` on the
/// stated grounds that "`readme::template` and `docs::render` both route content through
/// `docs::render::with_html_header`". That is true of READMEs and docs pages and false of
/// fixture snippets, which are assembled here and never touch `docs::render`. The result was
/// a `.md` that no side stamps: `generated_header` is `false`, `marker_header_syntax` is
/// `None` for `.md`, and the snippet output root sits outside any path
/// `cache::record_scaffold_owned_path` had recorded — so once a snippet existed on disk the
/// write guard could prove nothing and refused it forever (15,677 refusals in one consumer
/// repo, 9,139 in another). ~keep
///
/// Placement is load-bearing and has no slack. `with_html_header` puts the marker after the
/// YAML front matter (it must: Astro/Starlight imports these files as content and requires the
/// opening `---` to be the first bytes) with one blank line between, so with
/// `snippets/file.md.jinja`'s front matter the marker lands on line 10 at worst — the last line
/// `hash::content_has_alef_marker`'s 10-line scan window reads. **Adding a front-matter line
/// beyond that budget pushes the marker out of the window and silently restores the deadlock**;
/// the marker would still be in the file and nothing would read it. The front matter is 8 lines
/// when `level:` is present and 7 when it is omitted, so the omitted case has one line of slack
/// and the present case has none. `snippet_marker_lands_inside_the_read_side_scan_window`
/// and its control fail if that budget is spent. ~keep
fn render_snippet_markdown(
    body: &str,
    fixture: &Fixture,
    docs: &FixtureDocs,
    target: &str,
    language: DocumentationLanguage,
) -> String {
    let snippet_id = format!("fixture_{target}_{}", fixture.id);
    let requirements = snippet_requirements(fixture, target, body);
    let requires = serde_json::to_string(&requirements).unwrap_or_else(|_| "[]".to_string());
    let rendered = crate::e2e::template_env::render(
        "snippets/file.md.jinja",
        minijinja::context! {
            description => docs.description.as_deref().unwrap_or(&fixture.description),
            fence => language.code_fence(),
            id => snippet_id,
            language => language.canonical_name(),
            level => level_stamp(docs.side_effects),
            requires => requires,
            side_effect => side_effect_name(docs.side_effects),
            target => target,
            title => language.display_name(),
            // Generators end their body with a newline; the template adds its own before the
            // closing fence, so passing the body through verbatim put a blank line inside every
            // one of the thousands of generated code fences. ~keep
            body => body.trim_end(),
        },
    );
    crate::docs::with_html_header(rendered, SNIPPET_REGENERATE_COMMAND)
}

fn snippet_requirements(fixture: &Fixture, target: &str, body: &str) -> Vec<String> {
    let mut requirements = fixture.requirements.clone();
    if target == "rust" && fixture.visitor.is_some() && !requirements.iter().any(|value| value == "feature:visitor") {
        requirements.push("feature:visitor".to_string());
    }
    if generator_name(target) == "rust" {
        for (marker, requirement) in RUST_BODY_CRATE_REQUIREMENTS {
            if body.contains(marker) && !requirements.iter().any(|value| value == requirement) {
                requirements.push((*requirement).to_string());
            }
        }
    }
    requirements
}

/// The front-matter `level` a generated snippet declares. `effective_validation_level`
/// (`src/snippets/runner.rs`) folds this with the requested level by `min`, so any concrete
/// value here can only ever lower validation, never raise it.
///
/// `94d09809d` ("fix(e2e): typecheck fixture snippets") made this stamp unconditional, replacing
/// a `syntax` ceiling with `typecheck` specifically because fixtures with side effects the e2e
/// harness cannot safely execute unattended (network calls, process/install/server side effects)
/// were being validated no deeper than syntax. That protection is still needed for exactly those
/// fixtures. It was never needed for `Safe` ones, and stamping them anyway silently capped every
/// generated snippet at `typecheck` regardless of what the workspace and the snippet's own
/// capabilities could actually support. A `Safe` snippet OMITS the `level:` key entirely — an
/// absent key deserialises to `SnippetMetadata::level == None` exactly as an explicit `null` did
/// — so it has nothing to fold against `requested` and validates at whatever level the workspace
/// and validator achieve on their own.
///
/// The key is omitted rather than rendered `level: null` because these files are Astro content
/// entries, and Astro's collection schema types `level` as an optional STRING: an absent key
/// validates, an explicit YAML null does not (`Expected type "string", received "object"`). One
/// such entry fails the whole `astro build`, which took out 810 generated snippets in a consumer
/// docs site. Alef's own parser treats the two spellings identically, so nothing here is lost. ~keep
fn level_stamp(side_effects: SideEffectClass) -> Option<&'static str> {
    if side_effects == SideEffectClass::Safe {
        None
    } else {
        Some("typecheck")
    }
}

fn side_effect_name(side_effect: SideEffectClass) -> &'static str {
    match side_effect {
        SideEffectClass::Safe => "safe",
        SideEffectClass::Network => "network",
        SideEffectClass::Process => "process",
        SideEffectClass::Install => "install",
        SideEffectClass::Server => "server",
    }
}

fn validate_requirements(fixture: &Fixture) -> Result<()> {
    for requirement in &fixture.requirements {
        let valid = requirement.split_once(':').is_some_and(|(kind, value)| {
            matches!(kind, "feature" | "model" | "service" | "credential") && !value.is_empty() && !value.contains(':')
        });
        if !valid {
            bail!("fixture `{}` has invalid requirement token `{requirement}`", fixture.id);
        }
    }
    Ok(())
}

pub fn snippet_inclusion(fixture: &Fixture, capabilities: &BTreeSet<String>) -> SnippetInclusion {
    let missing_requirements: Vec<_> = fixture
        .requirements
        .iter()
        .filter(|requirement| !capabilities.contains(*requirement))
        .cloned()
        .collect();
    if missing_requirements.is_empty() {
        SnippetInclusion::Include
    } else {
        SnippetInclusion::Exclude { missing_requirements }
    }
}

fn capabilities(language: &str, snippets: &SnippetConfig, crate_config: &ResolvedCrateConfig) -> BTreeSet<String> {
    let mut values = snippets.capabilities.for_language(language);
    values.extend(crate_config.features.iter().map(|feature| format!("feature:{feature}")));
    values
}

fn snippet_path(
    output: &str,
    docs: &FixtureDocs,
    fixture_id: &str,
    target_language: &str,
    language: DocumentationLanguage,
) -> Result<PathBuf> {
    if let Some(relative) = docs.paths.get(target_language) {
        let relative = Path::new(relative);
        validate_relative_path(relative, "fixture docs target path")?;
        if relative.extension().and_then(|value| value.to_str()) != Some("md") {
            bail!("fixture docs target path must end in .md: {}", relative.display());
        }
        return Ok(Path::new(output)
            .join(snippet_output_slug(target_language, language))
            .join(relative));
    }
    validate_component(&docs.topic, "snippet topic")?;
    let stem = docs.stem.as_deref().unwrap_or(fixture_id);
    validate_component(stem, "snippet stem")?;
    Ok(Path::new(output)
        .join(snippet_output_slug(target_language, language))
        .join(&docs.topic)
        .join(format!("{stem}.md")))
}

fn snippet_output_slug(target_language: &str, language: DocumentationLanguage) -> &'static str {
    match target_language {
        "node" => "typescript",
        "wasm" => "wasm",
        "kotlin_android" => "kotlin-android",
        "brew" => "brew",
        "homebrew" => "homebrew",
        _ => language.canonical_name(),
    }
}

fn validate_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || Path::new(value).components().count() != 1 || matches!(value, "." | "..") {
        bail!("unsafe {label} `{value}`");
    }
    Ok(())
}

fn validate_relative_path(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("{label} must be a safe relative path: {}", path.display());
    }
    Ok(())
}

fn validate_docs_paths(fixture: &Fixture, languages: &[String]) -> Result<()> {
    let Some(docs) = &fixture.docs else {
        return Ok(());
    };
    for target in docs.paths.keys() {
        if !languages.iter().any(|language| language == target) {
            bail!(
                "fixture `{}` docs path targets unconfigured language `{target}`",
                fixture.id
            );
        }
    }
    Ok(())
}

/// Aliases resolve through [`crate::e2e::fixture::canonical_language`] before matching here,
/// so `"c_ffi"`/`"ffi"` and `"core"`/`"rust_core"` are not spelled out a second time in this
/// match arm list.
fn parse_language(value: &str) -> Option<DocumentationLanguage> {
    let language = match crate::e2e::fixture::canonical_language(value) {
        "python" => Language::Python,
        "node" => Language::Node,
        "wasm" => Language::Wasm,
        "ruby" => Language::Ruby,
        "php" | "php_ext" => Language::Php,
        "elixir" => Language::Elixir,
        "go" => Language::Go,
        "java" => Language::Java,
        "csharp" => Language::Csharp,
        "r" => Language::R,
        "rust" => Language::Rust,
        "kotlin" => Language::Kotlin,
        "kotlin_android" => Language::KotlinAndroid,
        "swift" => Language::Swift,
        "dart" => Language::Dart,
        "gleam" => Language::Gleam,
        "zig" => Language::Zig,
        "c" => Language::C,
        "brew" | "homebrew" => return Some(DocumentationLanguage::Shell),
        _ => return None,
    };
    Some(DocumentationLanguage::Binding(language))
}

#[cfg(test)]
mod tests;
