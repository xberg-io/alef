//! `--lang` scoping of the documentation-snippet stage.
//!
//! Split out of [`super`] (`src/e2e/tests.rs`) rather than appended to it: that file already
//! sits within reach of the modularization cap, and these tests carry their own fixture harness.
//!
//! The defect these protect: `--lang X` scoped the e2e generator stage but not the snippet
//! stage, so a single-language run rewrote every language's snippet tree. The fix must not be
//! "generate fewer snippets" -- the coverage ledger at the snippet output root is a whole-tree
//! record, and narrowing it would drop every unnamed language's ownership entry. Hence the
//! pairing below: one test proves the per-language files are scoped, its control proves an
//! unfiltered run still writes all of them, and a third proves the shared ledger stayed whole.
//!
//! # Why the fixture renders through the built-in recipe, not an extension
//!
//! An earlier version of this harness supplied snippet bodies from a local `Extension` and
//! rendered nothing at all, in every mode, so all three tests compared empty sets against empty
//! sets. The cause is worth writing down, because the obvious reading of `render_snippet_body`
//! is wrong: extensions ARE consulted before the built-in recipe there, but the snippet stage
//! inside `generate_e2e_with_extensions` does not call it with the extensions it was handed --
//! it calls [`snippets::generate_snippet_report`], which re-enters `crate::with_extensions` and
//! reads the process-global `EXTENSIONS` `OnceLock` instead. That global is settable exactly
//! once per process, so no test may populate it. A locally-passed extension therefore reaches
//! the generator and `emit_e2e` stages and never the snippet stage.
//!
//! So the fixture earns its snippets the way a real consumer's does: `[e2e.call].function` gives
//! the built-in recipe a function identity, which is the one thing it needs (proven by
//! `snippets::tests::coverage::rust_snippet_report` for rust and
//! `snippets::tests::adapter_handled` for python, both of which render with an empty IR).
//!
//! The IR stays empty deliberately. `validate_call_arg_signatures` and its siblings skip
//! entirely when `functions` and `type_defs` are both empty ("absent IR licenses no claim"), so
//! an empty surface disables the validators that would otherwise have to be satisfied; handing
//! them a partial one would manufacture failures unrelated to what these tests measure.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::SnippetConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::diagnostic_log::DiagnosticLog;
use crate::e2e::fixture_api::FixtureApi;
use crate::e2e::{generate_e2e_with_extensions, snippets};
use std::path::{Path, PathBuf};

/// The two targets every test here scopes between. Both are real registered e2e generators, so
/// `codegen::generators_for` and `snippets::snippet_generators` both resolve them.
const FIRST_LANGUAGE: &str = "rust";
const SECOND_LANGUAGE: &str = "python";

/// The fixture's documentation topic and id, which together fix its snippet path under each
/// language slug (`snippets::snippet_path`).
const TOPIC: &str = "api";
const FIXTURE_ID: &str = "list_records";

fn write_documented_fixture(directory: &Path) {
    std::fs::write(
        directory.join("list_records.json"),
        serde_json::json!({
            "id": FIXTURE_ID,
            "description": "lists records",
            "docs": { "topic": TOPIC },
        })
        .to_string(),
    )
    .expect("write documented fixture");
}

fn e2e_config(fixtures: &Path, snippet_output: &str) -> E2eConfig {
    let mut config = E2eConfig {
        fixtures: fixtures.display().to_string(),
        output: "e2e".to_string(),
        languages: vec![FIRST_LANGUAGE.to_string(), SECOND_LANGUAGE.to_string()],
        // Set explicitly, mirroring the consumer configuration the defect was reported against:
        // a non-empty `[crates.e2e.snippets].languages` is what made `languages_or` ignore the
        // `--lang`-narrowed set entirely. ~keep
        snippets: Some(SnippetConfig {
            output: snippet_output.to_string(),
            languages: vec![FIRST_LANGUAGE.to_string(), SECOND_LANGUAGE.to_string()],
            ..SnippetConfig::default()
        }),
        ..E2eConfig::default()
    };
    // The whole reason the fixture renders at all -- see this module's header. Without it every
    // cell lands in `coverage.missing` as "has no function identity" and the run publishes
    // nothing for either language, leaving every assertion below comparing empty sets. ~keep
    config.call.function = "convert".to_string();
    config
}

/// One e2e run's observable output: what it would write, what it recorded, and what it deferred.
///
/// The deferred error is carried rather than asserted on, so an anti-vacuity failure can quote
/// it. It is deliberately not the anti-vacuity check itself: a per-backend codegen failure from
/// `run_generators` claims that slot first (`Option::get_or_insert`) and would mask the snippet
/// diagnostic that actually matters here.
struct Rendered {
    files: Vec<GeneratedFile>,
    ledger: snippets::SnippetCoverageLedger,
    deferred: Option<String>,
    snippet_output: String,
}

/// Render one e2e run against a fresh fixture directory.
///
/// Extensions are passed as `&[]`: they cannot influence the snippet stage anyway (module
/// header), and an empty slice keeps the run independent of whatever any sibling test may have
/// installed in the process-global registry.
fn render(snippet_output: &str, languages: Option<&[String]>) -> Rendered {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    write_documented_fixture(directory.path());

    let (files, deferred_error) = generate_e2e_with_extensions(
        &ResolvedCrateConfig::default(),
        &e2e_config(directory.path(), snippet_output),
        languages,
        FixtureApi::new(&[], &[], &[], &[]),
        &[],
        &DiagnosticLog::new(),
    )
    .expect("the e2e run must render, deferring any per-backend failure rather than aborting");

    let deferred = deferred_error.map(|error| format!("{error:#}"));
    let manifest_path = Path::new(snippet_output).join(snippets::COVERAGE_MANIFEST);
    let manifest = files.iter().find(|file| file.path == manifest_path).unwrap_or_else(|| {
        panic!(
            "the snippet stage produced no coverage manifest at {}, so it did not run at all. \
                 Deferred failure: {deferred:?}",
            manifest_path.display()
        )
    });
    let ledger = serde_json::from_str(&manifest.content).expect("the coverage manifest must parse");

    Rendered {
        files,
        ledger,
        deferred,
        snippet_output: snippet_output.to_string(),
    }
}

impl Rendered {
    fn snippet_path(&self, language: &str) -> PathBuf {
        Path::new(&self.snippet_output)
            .join(language)
            .join(TOPIC)
            .join(format!("{FIXTURE_ID}.md"))
    }

    fn wrote(&self, language: &str) -> bool {
        let path = self.snippet_path(language);
        self.files.iter().any(|file| file.path == path)
    }

    /// Every snippet-tree path this run would write, for failure messages -- so an assertion
    /// names what the run actually produced and not only what it expected.
    fn snippet_paths(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|file| file.path.starts_with(&self.snippet_output))
            .map(|file| file.path.display().to_string())
            .collect()
    }

    /// Anti-vacuity gate. Every test calls this BEFORE its discriminating assertions.
    ///
    /// Without it, a fixture that renders nothing satisfies "no other language's files were
    /// written" and "the ledger was not narrowed" for the same reason an empty set satisfies
    /// anything, and three tests fail with three confusing messages about ownership entries
    /// instead of one saying the fixture is broken. That is not hypothetical -- it is exactly
    /// how the first version of this harness failed.
    ///
    /// Asserted against the ledger rather than the written files, because the ledger is computed
    /// over the full configured language set in every mode: it measures whether the FIXTURE
    /// renders, independently of what the `--lang` filter under test then does with the files.
    fn assert_the_fixture_rendered_for_every_language(&self) {
        assert!(
            self.ledger.missing.is_empty(),
            "the test fixture rendered no snippet for {} cell(s), so nothing below is measuring \
             the language filter -- fix the fixture, not the assertions. Gaps: {}. Deferred \
             failure: {:?}",
            self.ledger.missing.len(),
            self.ledger
                .missing
                .iter()
                .map(|missing| format!(
                    "`{}`/`{}`: {}",
                    missing.key.fixture_id, missing.key.language, missing.reason
                ))
                .collect::<Vec<_>>()
                .join("; "),
            self.deferred
        );
        for language in [FIRST_LANGUAGE, SECOND_LANGUAGE] {
            assert!(
                self.ledger.generated.iter().any(|key| key.language == language),
                "the test fixture must render for `{language}` in every mode; the ledger recorded \
                 {:?}. Deferred failure: {:?}",
                self.ledger.generated,
                self.deferred
            );
        }
        assert!(
            !self.ledger.generated_metadata.is_empty(),
            "a ledger with no generated-path entries is empty, not whole; nothing downstream can \
             assert it survived anything. Deferred failure: {:?}",
            self.deferred
        );
    }
}

/// The defect, stated as a test: a `--lang rust` run must not rewrite python's snippet files.
///
/// Asserts on the file set the run hands its writer -- the actual writes -- rather than on the
/// `[rust] generated N file(s)` log line, which reported the correct scope even while the
/// snippet stage went on rewriting every language's tree behind it.
#[test]
fn a_language_filtered_run_writes_no_other_languages_snippet_files() {
    let rendered = render("docs/lang-scope-filtered", Some(&[FIRST_LANGUAGE.to_string()]));
    rendered.assert_the_fixture_rendered_for_every_language();

    assert!(
        rendered.wrote(FIRST_LANGUAGE),
        "the language that WAS named must still be regenerated: {:?}",
        rendered.snippet_paths()
    );
    assert!(
        !rendered.wrote(SECOND_LANGUAGE),
        "a `--lang {FIRST_LANGUAGE}` run must not rewrite {SECOND_LANGUAGE}'s snippets: {:?}",
        rendered.snippet_paths()
    );
}

/// The control, and the reason the test above cannot be satisfied by writing nothing: with no
/// `--lang` filter, every configured language's snippet file must still be written.
#[test]
fn an_unfiltered_run_still_writes_every_languages_snippet_files() {
    let rendered = render("docs/lang-scope-unfiltered", None);
    rendered.assert_the_fixture_rendered_for_every_language();

    for language in [FIRST_LANGUAGE, SECOND_LANGUAGE] {
        assert!(
            rendered.wrote(language),
            "an unfiltered run must still write `{language}`'s snippets: {:?}",
            rendered.snippet_paths()
        );
    }
}

/// The shared artifact must survive the narrowing.
///
/// `.alef-snippet-coverage.json` is the only record of which path alef personally generated for
/// which cell; `coverage::orphaned_paths` and `ownership::is_ledger_owned_snippet_path` both
/// read it back. A filtered run that rewrote it from its own narrowed language set would erase
/// every other language's ownership entry, leaving those files permanently unpruneable and
/// unrecognised -- silently dropping the languages it merely declined to regenerate.
///
/// The anti-vacuity gate runs first here for a specific reason: whole-and-empty must not read as
/// whole. Every "language X is still present" assertion below passes trivially against a ledger
/// with no entries at all.
#[test]
fn a_language_filtered_run_leaves_the_shared_coverage_ledger_whole() {
    let rendered = render("docs/lang-scope-ledger", Some(&[FIRST_LANGUAGE.to_string()]));
    rendered.assert_the_fixture_rendered_for_every_language();

    for language in [FIRST_LANGUAGE, SECOND_LANGUAGE] {
        assert!(
            rendered.ledger.expected.iter().any(|key| key.language == language),
            "`{language}` must stay in the ledger's expected set: {:?}",
            rendered.ledger.expected
        );
        assert!(
            rendered
                .ledger
                .generated_metadata
                .iter()
                .any(|entry| entry.key.language == language),
            "`{language}`'s generated-path ownership entry must survive a filtered run: {:?}",
            rendered.ledger.generated_metadata
        );
    }
}
