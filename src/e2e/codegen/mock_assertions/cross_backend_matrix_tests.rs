//! Cross-backend table test over every wired `mock.*` generator (alef issue #443, capstone).
//!
//! `mock_assertions::tests` proves the shared *pre-generation gate*
//! (`ensure_capture_declared`) cannot silently green an unwired backend. It never drives a real
//! backend's rendering. Every wired backend's OWN correctness instead rests on a
//! `mock_capture_gate_tests.rs` written by whoever wired it -- and that per-backend audit has
//! already missed dispatch sites more than once this cycle: csharp had two extra call sites
//! (`emit_chat_stream_assertion`, `emit_non_chat_stream_assertion`) that never routed through
//! `render_assertion`; dart had a second (`render_streaming_assertion_dart`) a name-based grep for
//! `emit_*stream*assertion` missed entirely; zig had a THIRD, an unnamed inline
//! `match assertion.assertion_type.as_str()` block inside `test_file::render_test_fn`'s
//! bytes-result branch, found only by reading the file end to end.
//!
//! This module is the missing cross-backend gate: for every language
//! [`snippets::mock_capture`] has an entry for, it drives that language's real, public
//! [`crate::e2e::codegen::E2eCodegen::generate`] entry point -- never a backend's private
//! renderer, which a peer module cannot reach across a `mod` boundary -- across a small matrix
//! of fixture *shapes* chosen because each one is where a real dispatch gap was found this cycle
//! (see [`Shape`]'s doc). It belongs here rather than in `tests/` because every type and function
//! it drives (`all_generators`, `mock_assertions::snippets`, `field_skip::FieldSkip`,
//! `E2eCodegen::generate`) is `pub(crate)`/crate-private; a `tests/` integration test compiles as
//! a separate crate and cannot see any of it.

// ~keep This test reports to stderr deliberately: the executed/skipped cell counts and each
// named exemption must be visible in CI output, because a matrix that silently skips most of
// its cells is the exact vacuous gate this file exists to prevent. `logging-tracing` denies
// `print_stderr` in production Rust; the rule's documented carve-out is a top-of-file allow on
// a test file, which this is (`#[cfg(test)] mod` only).
#![allow(clippy::print_stderr)]

use crate::core::config::e2e::{CallConfig, CallOverride};
use crate::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::{E2eCodegen, all_generators};
use crate::e2e::config::{E2eConfig, StreamingConfig, StreamingRecipe};
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// Language-neutral function name every shape's `CallConfig` uses. Kept as a constant so the
/// swift-only streaming adapter below (keyed by name) cannot drift from the call config it
/// describes.
const MATRIX_FUNCTION_NAME: &str = "mock_matrix_call";

/// Stream item type name every streaming shape declares. Never resolved against a real IR type
/// (this matrix passes empty `type_defs`/`enums` to `generate()` throughout) -- only spelled out
/// verbatim in rendered, uncompiled source, exactly like `zig::mock_capture_gate_tests`'s
/// `"Client.create"` client-factory value.
const MATRIX_STREAM_ITEM_TYPE: &str = "MockMatrixStreamItem";

use super::snippets;

/// A fixture shape a wired backend must still render its mock capture call under.
///
/// Each variant is not arbitrary -- it is the exact shape that let a real dispatch gap slip past
/// a per-backend author this cycle:
///
/// - [`Shape::Plain`] -- the common path: a mock assertion alongside an ordinary one.
/// - [`Shape::Streaming`] -- csharp/dart/ruby/python each dispatch a STREAMING fixture's
///   assertions through a separate call site from their non-streaming `render_assertion`; this
///   is exactly the shape csharp's and dart's missed sites above only reached.
/// - [`Shape::Bytes`] -- the bytes-result call path (`result_is_bytes`, and for zig specifically
///   `client_factory` too). This is zig's third, unnamed site: a fixture whose result is raw
///   bytes rather than a JSON struct, on a backend that inline-matches the assertion type on that
///   path instead of delegating to its shared renderer.
/// - [`Shape::MockOnly`] -- the fixture's ONLY assertion is `mock.*`. Several backends gate
///   whether their assertion-rendering loop runs at all on "does any assertion emit code against
///   the *typed* result" (zig's `assertion_emits_code`/`has_mock_assertions` split is the
///   documented case); `mock.*` is never `is_valid_for_result`, so a mock-only fixture is the one
///   shape that finds a backend which forgot to OR its "any mock assertions" flag into that gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Plain,
    Streaming,
    Bytes,
    MockOnly,
}

impl Shape {
    const ALL: [Shape; 4] = [Shape::Plain, Shape::Streaming, Shape::Bytes, Shape::MockOnly];

    fn label(self) -> &'static str {
        match self {
            Shape::Plain => "plain_typed_result",
            Shape::Streaming => "streaming",
            Shape::Bytes => "bytes_result",
            Shape::MockOnly => "mock_only",
        }
    }

    fn mock_assertion() -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some("mock.requests.total".to_string()),
            value: Some(serde_json::json!(1)),
            ..Default::default()
        }
    }

    /// `not_error` is supported by every backend in [`crate::e2e::codegen::all_generators`]
    /// (see `assertion_types::BACKEND_UNSUPPORTED_ASSERTION_TYPES`, which excludes only
    /// `not_equals`/`matches_regex`/`contains_any`/`starts_with` per language, never
    /// `not_error`), so it is a safe language-neutral "ordinary assertion" for
    /// [`Shape::Plain`] that carries no field-availability requirement on any result type.
    fn assertions(self) -> Vec<Assertion> {
        match self {
            Shape::Plain => vec![
                Assertion {
                    assertion_type: "not_error".to_string(),
                    ..Default::default()
                },
                Self::mock_assertion(),
            ],
            Shape::Streaming | Shape::Bytes | Shape::MockOnly => vec![Self::mock_assertion()],
        }
    }

    fn fixture(self, id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            description: format!("cross-backend mock-capture matrix fixture ({})", self.label()),
            assertions: self.assertions(),
            ..Fixture::default()
        }
    }

    /// Build the `E2eConfig` this shape needs for `language`, so a driver can call
    /// [`crate::e2e::codegen::E2eCodegen::generate`] directly.
    fn e2e_config(self, language: &str) -> E2eConfig {
        let mut call = CallConfig {
            function: MATRIX_FUNCTION_NAME.to_string(),
            // Mirrors `streaming_result_binding_tests::streaming_call_config`'s own note: the
            // streaming/bytes dispatch paths in every backend below only activate on the
            // `Result`-returning branch.
            returns_result: true,
            ..CallConfig::default()
        };
        match self {
            Shape::Plain | Shape::MockOnly => {}
            Shape::Streaming => {
                // The `Recipe` form (rather than the legacy `Enabled(true)` boolean) is
                // required so `CallConfig::streaming_item_type` -- read by
                // `recipe::streaming_item_type`, which every non-swift streaming backend below
                // consults before falling back to a crate adapter -- resolves without needing a
                // `ResolvedCrateConfig::adapters` entry per language. csharp's
                // `resolve_csharp_streaming_item_type` hard-requires this: with no item type it
                // renders "streaming fixture requires adapter item_type for C# e2e codegen" and
                // drops the assertion (and therefore the mock capture call) entirely -- exactly
                // the false-green shape this matrix exists to catch, except here it is a missing
                // fixture/config, not a backend bug.
                call.streaming = Some(StreamingConfig::Recipe(StreamingRecipe {
                    enabled: Some(true),
                    item_type: Some(MATRIX_STREAM_ITEM_TYPE.to_string()),
                }));
                if language == "zig" {
                    // zig treats a streaming fixture as emittable at all only when a
                    // `client_factory` resolves for it (`zig::zig_client_factory_configured`) --
                    // without one the entire category is excluded as "not zig-emittable", never
                    // reaching the mock capture call site. See this module's own doc.
                    call.overrides.insert(
                        "zig".to_string(),
                        CallOverride {
                            client_factory: Some("Client.create".to_string()),
                            ..CallOverride::default()
                        },
                    );
                }
            }
            Shape::Bytes => {
                // A call-level property, not per-language (`CallConfig::effective_result_is_bytes`
                // ORs a per-language override on top of this), so every backend sees it uniformly.
                call.result_is_bytes = true;
                if language == "zig" {
                    // zig's bytes-result dispatch (`test_file::render_test_fn`) additionally
                    // requires `client_factory.is_some()` -- see this module's own doc and
                    // `zig::mock_capture_gate_tests::a_mock_only_assertion_on_the_bytes_result_path_still_renders`,
                    // whose `CallOverride` this mirrors. The value is never compiled, only
                    // rendered, so an arbitrary but realistic-looking name is fine.
                    call.overrides.insert(
                        "zig".to_string(),
                        CallOverride {
                            result_is_bytes: true,
                            client_factory: Some("Client.create".to_string()),
                            ..CallOverride::default()
                        },
                    );
                }
            }
        }
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    /// Build the `ResolvedCrateConfig` this shape needs.
    ///
    /// Only [`Shape::Streaming`] needs one: swift's `resolve_streaming_adapter` (unlike every
    /// other streaming backend) resolves its stream item type ONLY from
    /// `ResolvedCrateConfig::adapters`, never from `CallConfig::streaming_item_type` -- see
    /// `swift::values::resolve_streaming_adapter`. Every other shape uses
    /// `ResolvedCrateConfig::default()`, matching `working_directory_guard_tests.rs` and
    /// `streaming_result_binding_tests.rs`'s own pattern of driving `generate()` with an
    /// otherwise-empty crate config.
    fn resolved_crate_config(self) -> ResolvedCrateConfig {
        match self {
            Shape::Plain | Shape::MockOnly | Shape::Bytes => ResolvedCrateConfig::default(),
            Shape::Streaming => ResolvedCrateConfig {
                adapters: vec![AdapterConfig {
                    name: MATRIX_FUNCTION_NAME.to_string(),
                    pattern: AdapterPattern::Streaming,
                    core_path: format!("mock_matrix_core::{MATRIX_FUNCTION_NAME}"),
                    params: Vec::new(),
                    returns: None,
                    error_type: None,
                    owner_type: None,
                    item_type: Some(MATRIX_STREAM_ITEM_TYPE.to_string()),
                    gil_release: false,
                    trait_name: None,
                    trait_method: None,
                    detect_async: false,
                    request_type: None,
                    skip_languages: Vec::new(),
                }],
                ..ResolvedCrateConfig::default()
            },
        }
    }
}

/// A named, justified exemption for one `(language, shape)` cell -- never a silent skip.
struct Exemption {
    language: &'static str,
    shape: Shape,
    reason: &'static str,
}

/// Cells that a wired backend structurally cannot render, each with a load-bearing reason.
///
/// ~keep Per the `prove-the-check-fired` rule, an exemption here must be loud and reviewable, not
/// a silent `continue`. Keep this list short: every entry is a language/shape pair this matrix
/// does NOT protect, so adding one should be rare and deliberate.
const EXEMPTIONS: &[Exemption] = &[];

/// Backends with no [`snippets::mock_capture`] entry at all -- structurally unwired, not merely
/// "not tested yet". Mirrors `mock_assertions::tests::UNWIRED_LANGUAGE`'s own doc: `php_ext` and
/// `homebrew` render no field-access assertions at all, and `brew` has no capture-table row
/// either (see `snippets::MOCK_CAPTURE_TABLE`). All three are skipped for every shape below,
/// loudly -- [`unwired_languages_are_exactly_the_expected_three`] pins the set so this cannot
/// silently grow.
const UNWIRED_LANGUAGES: &[&str] = &["brew", "homebrew", "php_ext"];

/// One emitted file's path (relative, for diagnostics only) and content.
struct RenderedFile {
    path: String,
    content: String,
}

/// A line naming a dropped `mock.*` assertion through the shared [`FieldSkip`] funnel -- the
/// general form of the fallthrough this matrix exists to catch, derived from the SAME registered
/// wordings `field_skip` uses everywhere else rather than a hardcoded string, so a new backend
/// wording is covered automatically.
fn field_skip_names_a_mock_field(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let (field, _variant) = FieldSkip::extract_classified(line)?;
        super::is_mock_virtual_field(field).then(|| line.to_string())
    })
}

/// Literal fallthrough artifacts named in the alef #443 capstone brief: a hand-rolled `mock.*`
/// resolution that never went through the registered capture/skip machinery at all, so
/// [`field_skip_names_a_mock_field`] would not catch it either.
const LITERAL_FALLTHROUGH_ARTIFACTS: &[&str] = &["result.mock", ".mock().requests()"];

/// Drive `generator` for `shape` and check every emitted file for the mock-capture call and
/// helper, rather than indexing into `files` positionally -- backends differ in how many files
/// they emit and in what order (csharp's `.csproj` manifest sorts before its test source), so a
/// fixed index or a single assumed "the test file" path would silently examine the wrong
/// artifact for some backends while looking correct for others. See this module's own doc.
fn assert_renders_mock_capture(language: &str, shape: Shape, files: &[RenderedFile]) {
    assert!(
        !files.is_empty(),
        "{language}/{}: generate() returned no files at all -- nothing to examine",
        shape.label()
    );
    let capture = snippets::mock_capture(language)
        .unwrap_or_else(|_| panic!("{language} is in the matrix's wired set but snippets::mock_capture rejects it"));

    // The helper name must appear at least twice SOMEWHERE across the emitted files: once for
    // its own definition, at least once more for a call site invoking it. A backend that emits
    // the helper's definition unconditionally (e.g. as boilerplate) but never actually calls it
    // for this fixture would pass a bare `contains` check while asserting nothing -- exactly the
    // vacuous-gate shape this matrix exists to rule out. Occurrences can span multiple files (a
    // backend may define the helper in one file and call it from another), so this counts across
    // the whole emitted set, not per file.
    let total_occurrences: usize = files
        .iter()
        .map(|file| file.content.matches(capture.helper_name).count())
        .sum();
    let per_file_hits: Vec<&str> = files
        .iter()
        .filter(|file| file.content.contains(capture.helper_name))
        .map(|file| file.path.as_str())
        .collect();
    assert!(
        total_occurrences >= 2,
        "{language}/{}: expected the mock-capture helper name '{}' to appear at least twice \
         across the emitted files (a definition and at least one call site), found {} \
         occurrence(s) in file(s) {:?}. Emitted files: {:?}",
        shape.label(),
        capture.helper_name,
        total_occurrences,
        per_file_hits,
        files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
    );

    for file in files {
        if let Some(line) = field_skip_names_a_mock_field(&file.content) {
            panic!(
                "{language}/{}: file '{}' fell through to a generic field-skip marker instead of \
                 being captured -- got line: {line}",
                shape.label(),
                file.path,
            );
        }
        for artifact in LITERAL_FALLTHROUGH_ARTIFACTS {
            assert!(
                !file.content.contains(artifact),
                "{language}/{}: file '{}' contains the literal fallthrough artifact '{artifact}'",
                shape.label(),
                file.path,
            );
        }
    }
}

fn render(generator: &dyn E2eCodegen, shape: Shape) -> anyhow::Result<Vec<RenderedFile>> {
    let language = generator.language_name();
    let e2e_config = shape.e2e_config(language);
    let fixture = shape.fixture(&format!("mock_matrix_{}_{}", language, shape.label()));
    let groups = [FixtureGroup {
        category: "mock_matrix".to_string(),
        fixtures: vec![fixture],
    }];
    let crate_config = shape.resolved_crate_config();
    let files = generator.generate(&groups, &e2e_config, &crate_config, &[], &[], &[], &[])?;
    Ok(files
        .into_iter()
        .map(|file| RenderedFile {
            path: file.path.display().to_string(),
            content: file.content,
        })
        .collect())
}

#[test]
fn unwired_languages_are_exactly_the_expected_three() {
    let mut actually_unwired: Vec<&str> = all_generators()
        .iter()
        .map(|generator| generator.language_name())
        .filter(|language| snippets::mock_capture(language).is_err())
        .collect();
    actually_unwired.sort_unstable();
    let mut expected = UNWIRED_LANGUAGES.to_vec();
    expected.sort_unstable();
    assert_eq!(
        actually_unwired, expected,
        "the unwired-language allowlist has drifted from snippets::MOCK_CAPTURE_TABLE; update \
         UNWIRED_LANGUAGES (and this matrix's understanding of which cells are structurally \
         exempt) to match"
    );
}

/// The capstone matrix test: every wired backend, every fixture shape, driven through the real
/// `generate()` entry point. Reports the executed/skipped cell counts as part of the assertion
/// message on the way out, so a future reader does not have to re-derive them from a passing run.
#[test]
fn every_wired_backend_emits_its_mock_capture_call_across_every_shape() {
    let generators = all_generators();
    let mut executed = 0usize;
    let mut skipped_unwired = 0usize;
    let mut skipped_exempt = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for generator in &generators {
        let language = generator.language_name();
        if UNWIRED_LANGUAGES.contains(&language) {
            assert!(
                snippets::mock_capture(language).is_err(),
                "'{language}' is listed in UNWIRED_LANGUAGES but snippets::mock_capture now \
                 resolves it -- move it into the executed set instead of skipping it"
            );
            skipped_unwired += Shape::ALL.len();
            continue;
        }
        for shape in Shape::ALL {
            if let Some(exemption) = EXEMPTIONS
                .iter()
                .find(|entry| entry.language == language && entry.shape == shape)
            {
                skipped_exempt += 1;
                eprintln!(
                    "mock-capture matrix: skipping {language}/{} -- {}",
                    shape.label(),
                    exemption.reason
                );
                continue;
            }
            executed += 1;
            let outcome = render(generator.as_ref(), shape);
            match outcome {
                Ok(output) => {
                    if let Err(panic_message) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        assert_renders_mock_capture(language, shape, &output);
                    })) {
                        let message = panic_message
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| panic_message.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_else(|| "<non-string panic payload>".to_string());
                        failures.push(message);
                    }
                }
                Err(error) => {
                    failures.push(format!("{language}/{}: generation failed: {error:#}", shape.label()));
                }
            }
        }
    }

    let total_cells = generators.len() * Shape::ALL.len();
    let summary = format!(
        "mock-capture matrix: {executed} executed, {skipped_unwired} skipped (unwired language), \
         {skipped_exempt} skipped (named exemption), {total_cells} total cells"
    );
    eprintln!("{summary}");

    assert!(
        failures.is_empty(),
        "{summary}\n{} failure(s):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
