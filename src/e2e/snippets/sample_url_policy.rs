//! Which public address one fixture's documentation snippet publishes, and whether the
//! reserved documentation domain reaching that snippet is a defect or the project's own
//! declared truth.
//!
//! The three pre-existing levers -- `sample_base_url`, `sample_url_template` +
//! `docs.sample_url_vars`, and `sample_url_manifest` -- all answer the same question: *what*
//! is a fixture's public address. Every one of them assumes such an address exists. A corpus
//! whose sample inputs are served by the e2e mock harness, or fetched from a private bucket,
//! has no answer to give: the URLs genuinely do not exist, so "configure a sample host" is not
//! a fix and per-fixture acknowledgement of every occurrence is bookkeeping rather than a
//! resolution.
//!
//! `[crates.e2e.snippets].mock_only` answers the prior question instead -- whether the corpus
//! has public addresses at all -- and `docs.sample_url` lets one fixture answer it differently
//! from its corpus, in both directions.
//!
//! # The two warning classes, and why they must stay apart
//!
//! One address can reach a snippet as the reserved documentation domain for two unrelated
//! reasons, and only the first is what `mock_only` describes:
//!
//! * [`PlaceholderClass::Unconfigured`] -- *no public address exists for this fixture*. Nobody
//!   claimed one: the corpus configured no `sample_base_url` and the fixture declares no
//!   `docs.sample_url`. This is the class `mock_only` suppresses, because under `mock_only`
//!   there is nothing left for a consumer to configure.
//! * [`PlaceholderClass::Unresolved`] -- *a public address was claimed and did not resolve*.
//!   The fixture declared `docs.sample_url`, and the reserved domain still reached its
//!   published body. `mock_only` must never suppress this: the corpus-level default says
//!   "this corpus has no hosted inputs", which is not, and cannot become, a statement that a
//!   fixture's own declared address is fine when it is broken.
//!
//! The separation is only trustworthy because it is exhaustive, which is what
//! [`reject_conflicting_public_address`] buys. Under `mock_only` the ONLY route by which a
//! fixture can claim a public address is `docs.sample_url`, and that route is never
//! suppressed -- so no fixture whose address is merely missing-but-expected can fall into the
//! suppressed class. Were `sample_base_url` or a template allowed alongside `mock_only`, a
//! fixture that failed to resolve against them would land in `Unconfigured` and be muted, and
//! the flag would have become the blanket mute it must not be. ~keep

use super::{Fixture, SnippetConfig};
use crate::core::config::e2e::{
    DEFAULT_DOCS_SAMPLE_BASE_URL, DOCS_SAMPLE_URL_FIXTURE_KEY, DocsSampleBaseUrl, SAMPLE_BASE_URL_CONFIG_KEY,
    SAMPLE_URL_MANIFEST_CONFIG_KEY, SAMPLE_URL_MOCK_ONLY_CONFIG_KEY, SAMPLE_URL_TEMPLATE_CONFIG_KEY, SampleUrlManifest,
    SampleUrlTemplate,
};
use anyhow::Result;
use std::path::Path;

/// Which of the two distinct reserved-domain defects one rendered snippet exhibits. See the
/// module doc comment for why conflating them would make `mock_only` a blanket mute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlaceholderClass {
    /// No public address exists for this fixture, and nothing claimed one.
    Unconfigured,
    /// This fixture claimed a public address through `docs.sample_url` and the reserved
    /// documentation domain reached its published body anyway.
    Unresolved,
}

/// Every project-level input to sample URL resolution, resolved once per run.
///
/// Owns the template and manifest rather than borrowing them so a caller has one value to
/// thread through the render seam instead of four, and so the config-conflict check below has
/// somewhere to run exactly once.
#[derive(Debug)]
pub(super) struct SampleUrlPolicy<'a> {
    base: DocsSampleBaseUrl<'a>,
    template: Option<SampleUrlTemplate>,
    manifest: Option<SampleUrlManifest>,
    mock_only: bool,
}

impl<'a> SampleUrlPolicy<'a> {
    /// Resolve every sample-URL key before anything renders: an unusable base, template or
    /// manifest must fail the run rather than reach published documentation as a broken
    /// address, and a corpus that contradicts itself about whether public addresses exist must
    /// fail before the contradiction can decide a warning. `project_root` is the directory a
    /// manifest's project-root-relative `path` resolves against.
    pub(super) fn resolve(snippets: &'a SnippetConfig, project_root: &Path) -> Result<Self> {
        // Before validating any of the three addresses: if the corpus contradicts itself about
        // whether public addresses exist at all, saying so is more useful than reporting that
        // one of the addresses it should not have configured is also malformed. ~keep
        if snippets.mock_only {
            reject_conflicting_public_address(snippets)?;
        }
        let base = snippets
            .docs_sample_base_url()
            .map_err(|error| anyhow::anyhow!("invalid documentation sample base URL: {error}"))?;
        let template = snippets
            .sample_url_template()
            .map_err(|error| anyhow::anyhow!("invalid documentation sample URL template: {error}"))?;
        let manifest = snippets
            .sample_url_manifest(project_root)
            .map_err(|error| anyhow::anyhow!("invalid documentation sample URL manifest: {error}"))?;
        Ok(Self {
            base,
            template,
            manifest,
            mock_only: snippets.mock_only,
        })
    }

    /// The corpus-level base, for the run-level report that names it.
    pub(super) fn base(&self) -> DocsSampleBaseUrl<'a> {
        self.base
    }

    /// This fixture's effective address and warning disposition.
    ///
    /// A fixture that declares `docs.sample_url` is resolved through the same validator the
    /// corpus-level base uses, so the two cannot disagree about what a usable address is, and
    /// an unusable declaration fails the run for the same reason an unusable corpus base does.
    pub(super) fn for_fixture<'f>(&'f self, fixture: &'f Fixture) -> Result<FixtureSampleUrl<'f>> {
        let declared = fixture.docs.as_ref().and_then(|docs| docs.sample_url.as_deref());
        let (base, source) = match declared {
            Some(value) => {
                let base =
                    DocsSampleBaseUrl::resolve_at(Some(value), DOCS_SAMPLE_URL_FIXTURE_KEY).map_err(|error| {
                        anyhow::anyhow!("fixture `{}` declares an unusable sample URL: {error}", fixture.id)
                    })?;
                (base, SampleUrlSource::Declared)
            }
            None if self.mock_only => (self.base, SampleUrlSource::InheritedMockOnly),
            None => (self.base, SampleUrlSource::InheritedCorpus),
        };
        Ok(FixtureSampleUrl {
            base,
            template: self.template.as_ref(),
            manifest: self.manifest.as_ref(),
            source,
        })
    }
}

/// `mock_only` and the three public-address keys make contradictory claims about one corpus:
/// the first says no fixture's sample input is hosted anywhere, the others say every fixture's
/// is, at a computable address. Letting either win silently would make the suppression
/// decision depend on which key the resolution path happened to consult first -- and would
/// reopen the exact hole the module doc comment closes, since a fixture that failed to resolve
/// against a coexisting base would be classified `Unconfigured` and then muted. Fail instead,
/// naming the pair. ~keep
fn reject_conflicting_public_address(snippets: &SnippetConfig) -> Result<()> {
    let configured = [
        (SAMPLE_BASE_URL_CONFIG_KEY, snippets.sample_base_url.is_some()),
        (SAMPLE_URL_TEMPLATE_CONFIG_KEY, snippets.sample_url_template.is_some()),
        (SAMPLE_URL_MANIFEST_CONFIG_KEY, snippets.sample_url_manifest.is_some()),
    ];
    let conflicting: Vec<&str> = configured.iter().filter_map(|(key, set)| set.then_some(*key)).collect();
    if conflicting.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "`{SAMPLE_URL_MOCK_ONLY_CONFIG_KEY}` is set alongside `{}`; they state contradictory facts \
         about the same corpus. Remove one, or drop the key(s) above and give each hosted fixture \
         its own `{DOCS_SAMPLE_URL_FIXTURE_KEY}`.",
        conflicting.join("`, `")
    )
}

/// Where one fixture's effective sample address came from -- the fact that decides which, if
/// either, reserved-domain warning its snippet is eligible for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleUrlSource {
    /// The fixture declared `docs.sample_url`. It has opted in to a real public address, so
    /// the corpus default has no say over it in either direction.
    Declared,
    /// No fixture-level declaration, and the corpus declares itself mock-only.
    InheritedMockOnly,
    /// No fixture-level declaration and no mock-only default: the corpus base applies,
    /// configured or placeholder, exactly as it did before either key existed.
    InheritedCorpus,
}

/// One fixture's resolved sample address, ready to render against and to classify afterwards.
#[derive(Debug)]
pub(super) struct FixtureSampleUrl<'a> {
    base: DocsSampleBaseUrl<'a>,
    template: Option<&'a SampleUrlTemplate>,
    manifest: Option<&'a SampleUrlManifest>,
    source: SampleUrlSource,
}

impl<'a> FixtureSampleUrl<'a> {
    pub(super) fn base(&self) -> DocsSampleBaseUrl<'a> {
        self.base
    }

    pub(super) fn template(&self) -> Option<&'a SampleUrlTemplate> {
        self.template
    }

    pub(super) fn manifest(&self) -> Option<&'a SampleUrlManifest> {
        self.manifest
    }

    /// Which reserved-domain defect, if any, the finished `body` exhibits.
    ///
    /// Measured on the published text rather than on what resolution intended, for the reason
    /// `super::render_body::rendered` documents: a fixture can route an address into a snippet
    /// by several paths, and only the text is common to all of them.
    pub(super) fn classify(&self, body: &str) -> Option<PlaceholderClass> {
        match self.source {
            // This fixture claimed an address of its own. The corpus default -- mock-only or
            // not -- is out of the picture, so the only question left is whether the claim
            // actually reached the published body. It did not if the reserved documentation
            // domain is still in there: either the declaration names that domain itself, or
            // something behind it fell back to the corpus placeholder. Tested against the
            // reserved domain rather than `self.base` because `self.base` IS the declared
            // address here, and asking whether a body contains its own configured host would
            // answer the wrong question entirely. ~keep
            SampleUrlSource::Declared => body
                .contains(DEFAULT_DOCS_SAMPLE_BASE_URL)
                .then_some(PlaceholderClass::Unresolved),
            // The corpus declared it has no public addresses and this fixture did not dissent.
            // The illustrative reserved domain in its snippet is what the project said would
            // be there, so there is nothing to report.
            SampleUrlSource::InheritedMockOnly => None,
            SampleUrlSource::InheritedCorpus => (self.base.is_placeholder() && body.contains(self.base.base()))
                .then_some(PlaceholderClass::Unconfigured),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture whose `docs.sample_url` is whatever `sample_url` says, declared through the
    /// real deserializer so these tests also cover the serde wiring rather than assuming it.
    fn fixture_declaring(sample_url: Option<&str>) -> Fixture {
        let mut docs = serde_json::json!({"topic": "contract"});
        if let Some(value) = sample_url {
            docs["sample_url"] = serde_json::Value::String(value.to_string());
        }
        serde_json::from_value(serde_json::json!({
            "id": "extract_uri",
            "description": "Extract a document from a URI",
            "input": {"url": "/pdf/report.pdf"},
            "assertions": [{"type": "not_error"}],
            "docs": docs,
        }))
        .expect("fixture must parse")
    }

    fn config(mock_only: bool) -> SnippetConfig {
        SnippetConfig {
            output: "docs/snippets".into(),
            mock_only,
            ..SnippetConfig::default()
        }
    }

    /// A body that carries the reserved documentation domain -- what every unresolved address
    /// produces, whatever route it took to get there.
    const PLACEHOLDER_BODY: &str = "client.extract(\"https://example.com/pdf/report.pdf\")";

    #[test]
    fn an_inherited_fixture_under_a_mock_only_corpus_has_no_defect_to_report() {
        let config = config(true);
        let policy = SampleUrlPolicy::resolve(&config, Path::new(".")).expect("policy resolves");
        let fixture = fixture_declaring(None);

        let sample_url = policy.for_fixture(&fixture).expect("fixture resolves");

        assert_eq!(sample_url.classify(PLACEHOLDER_BODY), None);
    }

    /// The control at the unit level: without `mock_only`, the same fixture and the same body
    /// still classify as a defect. If this ever returns `None`, the suppression has stopped
    /// depending on the flag and become unconditional.
    #[test]
    fn an_inherited_fixture_without_mock_only_is_classified_unconfigured() {
        let config = config(false);
        let policy = SampleUrlPolicy::resolve(&config, Path::new(".")).expect("policy resolves");
        let fixture = fixture_declaring(None);

        let sample_url = policy.for_fixture(&fixture).expect("fixture resolves");

        assert_eq!(
            sample_url.classify(PLACEHOLDER_BODY),
            Some(PlaceholderClass::Unconfigured)
        );
    }

    #[test]
    fn a_declared_fixture_publishing_its_own_address_has_no_defect_to_report() {
        let config = config(true);
        let policy = SampleUrlPolicy::resolve(&config, Path::new(".")).expect("policy resolves");

        let fixture = fixture_declaring(Some("https://samples.example.org"));

        let sample_url = policy.for_fixture(&fixture).expect("fixture resolves");

        assert_eq!(sample_url.base().base(), "https://samples.example.org");
        assert_eq!(
            sample_url.classify("client.extract(\"https://samples.example.org/pdf/report.pdf\")"),
            None
        );
    }

    /// The distinction the whole feature rests on: under the very same mock-only corpus that
    /// silences the fixture above, a fixture that CLAIMED an address and still published the
    /// reserved domain is classified as a defect -- and as the other class, so the two can
    /// never be reported as the same thing. ~keep
    #[test]
    fn a_declared_fixture_still_publishing_the_reserved_domain_is_unresolved_even_under_mock_only() {
        let config = config(true);
        let policy = SampleUrlPolicy::resolve(&config, Path::new(".")).expect("policy resolves");

        let fixture = fixture_declaring(Some("https://example.com/hosted"));

        let sample_url = policy.for_fixture(&fixture).expect("fixture resolves");

        assert_eq!(
            sample_url.classify(PLACEHOLDER_BODY),
            Some(PlaceholderClass::Unresolved)
        );
        assert_ne!(
            PlaceholderClass::Unresolved,
            PlaceholderClass::Unconfigured,
            "a broken declared URL and a missing one must never collapse into one class"
        );
    }

    #[test]
    fn an_unusable_fixture_declaration_fails_naming_the_fixture_and_the_fixture_key() {
        let config = config(true);
        let policy = SampleUrlPolicy::resolve(&config, Path::new(".")).expect("policy resolves");

        let fixture = fixture_declaring(Some("samples.example.org"));

        let error = policy
            .for_fixture(&fixture)
            .expect_err("a scheme-less fixture address cannot form a public URL");

        let message = format!("{error:#}");
        assert!(
            message.contains("extract_uri") && message.contains(DOCS_SAMPLE_URL_FIXTURE_KEY),
            "the failure must name the fixture and the key its author wrote: {message}"
        );
    }

    #[test]
    fn mock_only_alongside_a_template_is_rejected_naming_both_keys() {
        let config = SnippetConfig {
            output: "docs/snippets".into(),
            mock_only: true,
            sample_url_template: Some("https://cdn.example.org/objects/{digest}".to_string()),
            ..SnippetConfig::default()
        };

        let error = SampleUrlPolicy::resolve(&config, Path::new("."))
            .expect_err("a mock-only corpus cannot also compute every fixture's public address");

        let message = format!("{error:#}");
        assert!(
            message.contains("mock_only") && message.contains("sample_url_template"),
            "the failure must name both halves of the contradiction: {message}"
        );
    }
}
