//! Rendering one fixture's snippet body, and reporting the addresses that reached it.
//!
//! Split out of [`super`], which owns the driver loop, the coverage ledger and the write
//! guards. This module owns the narrower question of how a single fixture/language cell
//! becomes target-language source: which recipe renders it (an extension's, or the
//! backend's built-in), and which public addresses its URL arguments are bound to.

use super::sample_url_policy::{FixtureSampleUrl, PlaceholderClass};
use super::*;
use crate::core::config::e2e::{DocsSampleBaseUrl, SAMPLE_URL_MOCK_ONLY_CONFIG_KEY};

/// A rendered snippet body, plus which reserved-domain defect -- if either -- the addresses in
/// it exhibit (see [`crate::e2e::snippets::sample_url_policy::PlaceholderClass`]).
///
/// The classification travels with the body instead of being logged where it is discovered so
/// the run can report once per class, naming every affected fixture, rather than emitting one
/// warning per fixture per language.
pub(super) struct RenderedSnippetBody {
    pub body: String,
    pub placeholder_class: Option<PlaceholderClass>,
}

pub(super) struct RenderContext<'a> {
    pub inputs: &'a SnippetRenderContext<'a>,
    pub session: &'a crate::e2e::codegen::SnippetRenderSession<'a>,
}

pub(super) fn render_snippet_body(
    extensions: &[Box<dyn crate::Extension>],
    generator: &dyn E2eCodegen,
    fixture: &Fixture,
    language: &str,
    run: &RenderContext<'_>,
    sample_url: &FixtureSampleUrl<'_>,
) -> Result<RenderedSnippetBody> {
    let context = run.inputs;
    let session = run.session;
    let docs_fixture = fixture.docs_call_fixture_with_sample_url(
        sample_url.base().base(),
        sample_url.template(),
        sample_url.manifest(),
    );
    for extension in extensions {
        if let Some(body) = extension
            .render_e2e_snippet(
                &docs_fixture,
                context.e2e,
                context.crate_config,
                language,
                context.type_defs,
                context.enums,
            )
            .map_err(|error| anyhow::anyhow!("extension `{}` could not render snippet: {error:#}", extension.name()))?
        {
            if body.trim().is_empty() {
                bail!("extension `{}` returned an empty snippet body", extension.name());
            }
            mock_harness_guard::reject_mock_harness_scaffolding(&body, &docs_fixture, language)?;
            return Ok(rendered(body, sample_url));
        }
    }
    let call = context.e2e.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let docs_fixture = mock_url_defaults::with_default_mock_url_literals(
        docs_fixture,
        call,
        sample_url.base(),
        sample_url.template(),
        sample_url.manifest(),
    );
    let fixture = &docs_fixture;
    if let Some(kind) = recipe_policy::extension_owned_recipe_kind(fixture, fixture.resolved_args(call)) {
        bail!("{kind} fixture requires an extension-owned documentation recipe");
    }
    let effective_function = call
        .effective_function(language)
        .or_else(|| {
            // The naive identity fallback below derives a symbol name from the raw
            // `fixture.call` config text (`register_fn`/`unregister_fn`/`clear_fn`), which
            // can diverge from what the FFI backend actually generates (see
            // `trait_bridge_function_identity`'s doc comment). A fixture author who set
            // `skip.languages` for this language has already declared that the harness
            // (and by extension this naive fallback) cannot speak for it here; only trust
            // the fallback when the fixture is not skipped for this language. Fixtures with
            // a real, extension-owned recipe never reach this branch — the extension loop
            // above already returned their body, and `recipe_policy::extension_owned_recipe_kind`
            // already bailed for fixtures that require one but lack it.
            let skipped_for_language = fixture.skip.as_ref().is_some_and(|skip| skip.should_skip(language));
            (!skipped_for_language && crate::e2e::fixture::canonical_language(language) == "c")
                .then(|| crate::e2e::codegen::recipe::trait_bridge_function_identity(context.crate_config, fixture))
                .flatten()
        })
        .unwrap_or_default();
    if effective_function.trim().is_empty() {
        bail!(
            "built-in `{language}` snippet recipe has no function identity; configure a call function or provide an extension-owned documentation recipe"
        );
    }
    let rendered_body = generator.render_snippet_body_in_session(fixture, context.e2e, context.crate_config, session);
    // Drained here, unconditionally, for the same reason `E2eCodegen::generate_gated` drains it:
    // a builder that recognised an undeclared fixture key many frames down cannot return a
    // `Result`, and a refusal left on the ledger would be reported against whichever backend
    // drains next. Reported ahead of a render error because it names the fixture, the call and
    // the config lever, where a render error at this boundary names only the language. ~keep
    if let Some(refusal) = crate::e2e::codegen::fixture_refusal::take_error(language) {
        return Err(refusal);
    }
    let body = rendered_body
        .map_err(|error| anyhow::anyhow!("built-in `{language}` snippet recipe is incompatible: {error:#}"))?;
    if body.trim().is_empty() {
        bail!("built-in `{language}` snippet recipe returned an empty body");
    }
    mock_harness_guard::reject_mock_harness_scaffolding(&body, fixture, language)?;
    Ok(rendered(body, sample_url))
}

/// Pair a rendered body with what a run can honestly say about its addresses: whether the
/// published text carries the reserved-domain placeholder, and if so which of
/// [`PlaceholderClass`]'s two very different causes put it there.
///
/// Deliberately measured on the finished body rather than on what
/// [`mock_url_defaults::with_default_mock_url_literals`] injected. A fixture that writes
/// `"$mock_url"` in its own `docs.input` never reaches that injection branch -- the
/// placeholder is already substituted by [`Fixture::docs_call_fixture_with_sample_url`] before
/// the module runs, and the resulting absolute URL reads to it as an address the author chose.
/// Reporting the injection would therefore stay silent about a snippet that does publish an
/// unreachable address. The published text cannot.
///
/// This is also what makes per-fixture template resolution safe to add without touching this
/// function at all: a templated address that resolved successfully never contains
/// `sample_base_url`'s text, so it never trips this check, and an unresolved occurrence falls
/// back to `sample_base_url` -- which does -- so the placeholder warning still fires for
/// exactly the fixtures a template cannot actually resolve. ~keep
fn rendered(body: String, sample_url: &FixtureSampleUrl<'_>) -> RenderedSnippetBody {
    let placeholder_class = sample_url.classify(&body);
    RenderedSnippetBody {
        body,
        placeholder_class,
    }
}

/// The number of fixture/language occurrences a placeholder-URL warning names before it stops
/// enumerating; a consumer with hundreds of URL fixtures should get a usable message, not a log
/// dump.
const PLACEHOLDER_SAMPLE_URL_FIXTURES_NAMED: usize = 10;

/// The unacknowledged reserved-domain occurrences of one run, kept apart by class.
///
/// The two classes are accumulated separately rather than in one list with a discriminant so
/// that reporting cannot accidentally merge them back into a single message: they have
/// different causes, different fixes, and only one of them is suppressible by
/// `[crates.e2e.snippets].mock_only`. See
/// [`crate::e2e::snippets::sample_url_policy`] for why that separation is load-bearing. ~keep
#[derive(Debug, Default)]
pub(super) struct PlaceholderSampleUrlLedger {
    fixtures: BTreeSet<String>,
    unconfigured: Vec<(String, String)>,
    unresolved: Vec<(String, String)>,
}

impl PlaceholderSampleUrlLedger {
    /// Record one rendered cell's classification, consulting `acknowledgements` first.
    ///
    /// Both classes go through the same `doc_snippet_reserved_domain` acknowledgement ledger:
    /// a consumer that has already acknowledged a fixture/language pair must not be handed a
    /// second, differently-worded warning about the same pair just because the run learned to
    /// name its cause more precisely.
    pub(super) fn record(
        &mut self,
        acknowledgements: &mut AcknowledgementLedger,
        class: Option<PlaceholderClass>,
        fixture_id: &str,
        language: &str,
    ) {
        let Some(class) = class else {
            return;
        };
        // Keyed by BOTH the fixture id (warning identity) and this target language (source
        // target) -- an acknowledgement for one target must never silence the same fixture
        // publishing the placeholder for a different one. ~keep
        let category = AcknowledgeableWarningCategory::DocSnippetReservedDomain;
        if matches!(
            acknowledgements.check(category, fixture_id, language),
            AckOutcome::Acknowledged { .. }
        ) {
            return;
        }
        self.fixtures.insert(fixture_id.to_string());
        let occurrence = (fixture_id.to_string(), language.to_string());
        match class {
            PlaceholderClass::Unconfigured => self.unconfigured.push(occurrence),
            PlaceholderClass::Unresolved => self.unresolved.push(occurrence),
        }
    }

    /// Every fixture named by either class, for `SnippetGenerationReport`.
    pub(super) fn fixtures(&self) -> Vec<String> {
        self.fixtures.iter().cloned().collect()
    }

    pub(super) fn report(&self, sample_base_url: DocsSampleBaseUrl<'_>) {
        report_unconfigured_sample_urls(&self.unconfigured, sample_base_url);
        report_unresolved_fixture_sample_urls(&self.unresolved);
    }
}

/// Render `occurrences` as a human-readable list, each carrying the exact `alef.toml` entry
/// that would acknowledge it (task #540's provenance requirement), so a consumer can act
/// without guessing the config shape. Returns the list and how many occurrences it omitted.
fn named_occurrences(occurrences: &[(String, String)]) -> (String, usize) {
    let named = occurrences
        .iter()
        .take(PLACEHOLDER_SAMPLE_URL_FIXTURES_NAMED)
        .map(|(fixture_id, language)| {
            format!(
                "{fixture_id} ({language}, acknowledge with {})",
                WarningAcknowledgement::config_entry_for(
                    AcknowledgeableWarningCategory::DocSnippetReservedDomain,
                    fixture_id,
                    language,
                )
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    (
        named,
        occurrences.len().saturating_sub(PLACEHOLDER_SAMPLE_URL_FIXTURES_NAMED),
    )
}

fn more_suffix(remaining: usize) -> String {
    if remaining > 0 {
        format!(" (+{remaining} more)")
    } else {
        String::new()
    }
}

/// Say, once per run, that snippets were published carrying an address nobody serves *because
/// nobody claimed one*: no `sample_base_url` is configured and these fixtures declare no
/// `docs.sample_url` either.
///
/// `occurrences` carries only fixture/language pairs that were NOT acknowledged via
/// `[crates.e2e.snippets].acknowledged_warnings` -- a suppressed occurrence must not be named
/// here (it is already visible in the acknowledged-warnings report instead; see
/// `report_acknowledged_warnings`). Not an error by itself, because a project may legitimately
/// have no public sample host, and failing here would break every consumer that already ships
/// these snippets.
///
/// This is the class `[crates.e2e.snippets].mock_only` retires, and the message says so: a
/// consumer whose sample URLs genuinely do not exist has a third option beyond configuring a
/// host it does not have and acknowledging every pair by hand.
fn report_unconfigured_sample_urls(occurrences: &[(String, String)], sample_base_url: DocsSampleBaseUrl<'_>) {
    if occurrences.is_empty() {
        return;
    }
    let (named, remaining) = named_occurrences(occurrences);
    tracing::warn!(
        target: "alef::e2e::snippets",
        fixtures = occurrences.len(),
        base_url = sample_base_url.base(),
        config_key = crate::core::config::e2e::SAMPLE_BASE_URL_CONFIG_KEY,
        "{} documentation snippet fixture/language occurrence(s) publish the reserved placeholder \
         address `{}`, which serves nothing: a reader who copies them gets a request that cannot \
         succeed. Set `{}` to a host that really serves your sample inputs; set `{}` if these \
         sample inputs are mock-only and no such host exists; or acknowledge a specific \
         fixture/language pair below -- a stale acknowledgement (one that matches nothing) fails \
         the run. Affected: {named}{}",
        occurrences.len(),
        sample_base_url.base(),
        crate::core::config::e2e::SAMPLE_BASE_URL_CONFIG_KEY,
        SAMPLE_URL_MOCK_ONLY_CONFIG_KEY,
        more_suffix(remaining)
    );
}

/// Say, once per run, that a fixture which DID claim a public address still published the
/// reserved documentation domain.
///
/// Deliberately a separate message from `report_unconfigured_sample_urls` rather than a variant
/// of it: this is a broken declaration, not a missing one, and `mock_only` never suppresses it.
/// Merging the two would let a corpus-level "we host nothing" statement stand in for "this
/// fixture's own URL is fine", which it is not. ~keep
fn report_unresolved_fixture_sample_urls(occurrences: &[(String, String)]) {
    if occurrences.is_empty() {
        return;
    }
    let (named, remaining) = named_occurrences(occurrences);
    tracing::warn!(
        target: "alef::e2e::snippets",
        fixtures = occurrences.len(),
        base_url = crate::core::config::e2e::DEFAULT_DOCS_SAMPLE_BASE_URL,
        fixture_key = crate::core::config::e2e::DOCS_SAMPLE_URL_FIXTURE_KEY,
        "{} documentation snippet fixture/language occurrence(s) declare `{}` but still publish \
         the reserved placeholder address `{}`, so the declared address never reached the \
         snippet. This is not the missing-sample-host warning and `{}` does not suppress it: a \
         fixture that claims a public address is reporting a broken one, not the absence of one. \
         Fix the declared value, or remove it to let the corpus default stand. Affected: \
         {named}{}",
        occurrences.len(),
        crate::core::config::e2e::DOCS_SAMPLE_URL_FIXTURE_KEY,
        crate::core::config::e2e::DEFAULT_DOCS_SAMPLE_BASE_URL,
        SAMPLE_URL_MOCK_ONLY_CONFIG_KEY,
        more_suffix(remaining)
    );
}

/// Say, once per run, how many warning occurrences a configured acknowledgement suppressed.
///
/// Task #540's third requirement: the suppressed set must be visible, not invisible. Silent on
/// a run with nothing acknowledged, matching `report_unconfigured_sample_urls`'s silence on a run
/// with nothing to report.
pub(super) fn report_acknowledged_warnings(report: &crate::core::warning_ack::AcknowledgementReport) {
    if report.matched_count == 0 {
        return;
    }
    tracing::info!(
        target: "alef::e2e::snippets",
        matched = report.matched_count,
        "{} documentation snippet warning occurrence(s) acknowledged via \
         [crates.e2e.snippets].acknowledged_warnings: {}",
        report.matched_count,
        report.matched_entries.join(", ")
    );
}
