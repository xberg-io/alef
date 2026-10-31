use super::metadata_cache_tests::FactCounts;
use super::resolver_reuse_tests::{config, enums, functions, rich_fixture, rich_types};
use super::*;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::SnippetConfig;
use crate::core::ir::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::field_access::{IrResultFieldMap, PhpGetterMap, SwiftFirstClassMap};
use crate::e2e::fixture::FixtureGroup;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) const PRESENTATION_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "node",
    "wasm",
    "ruby",
    "php",
    "elixir",
    "java",
    "kotlin",
    "kotlin_android",
    "go",
    "swift",
    "dart",
    "csharp",
    "zig",
    "r",
];

#[test]
fn constructor_observer_exposes_unscoped_prep_and_separately_counts_call_body_work() {
    let counts = FactCounts {
        presentation_only: true,
        ..FactCounts::default()
    };
    counts.observe(|| {
        let types = rich_types();
        let enums = enums();
        let prep = FieldResolver::ir_enum_fields(&types, &enums);
        assert_eq!(prep.enum_field_types["Result"]["kind"], "Kind");
        let _span = tracing::trace_span!("snippet_call_body").entered();
        let call = FieldResolver::ir_enum_fields(&types, &enums);
        assert_eq!(call.enum_field_types["Result"]["kind"], "Kind");
    });
    assert_eq!(
        *counts.outside_presentation.lock().unwrap(),
        HashMap::from([("enum".into(), 1)])
    );
    assert_eq!(*counts.call_body.lock().unwrap(), HashMap::from([("enum".into(), 1)]));
    assert_eq!(*counts.counts.lock().unwrap(), HashMap::new());
}

#[test]
fn session_operation_bytes_match_one_shot_resolution_for_every_presentation_backend() {
    let types = rich_types();
    let enums = enums();
    let functions = functions();
    let config = config();
    let fixture = rich_fixture();
    let session = SnippetRenderSession::new(&types, &enums, &functions, &[]);
    assert_eq!(PRESENTATION_LANGUAGES.len(), 16);
    for &language in PRESENTATION_LANGUAGES {
        let previous = resolve(&fixture, &config, language, &types, &enums, &functions);
        let current = session.resolve(&fixture, &config, language);
        assert_eq!(
            current.len(),
            3,
            "{language} must exercise all rich presentation operations"
        );
        if language == "rust" {
            let iterate = current
                .iter()
                .find(|operation| operation.kind == "iterate")
                .expect("actual iterate operation");
            assert_eq!(iterate.field_optionals, [true], "retain legacy item.title optionality");
        }
        assert_eq!(
            serde_json::to_vec(&current).unwrap(),
            serde_json::to_vec(&previous).unwrap(),
            "{language}"
        );
    }
    let caller = caller_resolver();
    for language in ["php", "swift"] {
        let previous = resolve_with(&fixture, &config, language, &caller, &types, &enums, &functions);
        let current = session.resolve_with(&fixture, &config, language, &caller);
        assert_eq!(current.len(), 3, "{language} caller must retain all three operations");
        assert_eq!(
            serde_json::to_vec(&current).unwrap(),
            serde_json::to_vec(&previous).unwrap(),
            "{language}"
        );
    }
}

fn two_fixtures() -> Vec<Fixture> {
    ["first-cache-cell", "second-cache-cell"]
        .into_iter()
        .map(|id| {
            let mut fixture = rich_fixture();
            fixture.id = id.into();
            fixture
        })
        .collect()
}

fn actual_report() -> (crate::e2e::snippets::SnippetGenerationReport, FactCounts) {
    let types = rich_types();
    let enums = enums();
    let functions = functions();
    let config = config();
    let project = ResolvedCrateConfig {
        name: "cache-probe".into(),
        ..ResolvedCrateConfig::default()
    };
    let snippets = SnippetConfig {
        output: "docs/snippet-cache-test".into(),
        ..SnippetConfig::default()
    };
    let counts = FactCounts {
        presentation_only: true,
        ..FactCounts::default()
    };
    let report = counts
        .observe(|| {
            crate::e2e::snippets::generate_snippet_report(
                &two_fixtures(),
                &["rust".into(), "python".into()],
                &config,
                &snippets,
                &project,
                &types,
                &enums,
                &functions,
                &[],
            )
        })
        .expect("render actual documentation cells");
    (report, counts)
}

fn assert_four_real_cells(report: &crate::e2e::snippets::SnippetGenerationReport) {
    assert_eq!(report.coverage.expected.len(), 4);
    assert_eq!(report.coverage.generated.len(), 4, "{:?}", report.coverage.missing);
    assert_eq!(report.snippets.len(), 4);
    let cells: BTreeSet<_> = report
        .snippets
        .iter()
        .map(|snippet| {
            assert_ne!(snippet.file.content.trim(), "");
            (snippet.fixture_id.as_str(), snippet.language.as_str())
        })
        .collect();
    assert_eq!(
        cells,
        BTreeSet::from([
            ("first-cache-cell", "python"),
            ("first-cache-cell", "rust"),
            ("second-cache-cell", "python"),
            ("second-cache-cell", "rust"),
        ])
    );
}

#[test]
fn real_report_renders_four_cells_with_one_presentation_metadata_build() {
    let (report, counts) = actual_report();
    assert_four_real_cells(&report);
    assert_eq!(
        *counts.outside_presentation.lock().unwrap(),
        HashMap::new(),
        "presentation preparation must not hide metadata construction outside its resolution span"
    );
    let actual = counts.counts.lock().unwrap();
    assert_eq!(actual.len(), 4, "all actual presentation constructors must be observed");
    assert_eq!(
        *actual,
        HashMap::from([
            ("fields".into(), 1),
            ("enum".into(), 1),
            ("collection".into(), 1),
            ("result".into(), 1),
        ])
    );
}

struct LegacyOnly<'a> {
    functions: &'a [FunctionDef],
    calls: AtomicUsize,
}

impl E2eCodegen for LegacyOnly<'_> {
    fn generate(
        &self,
        _: &[FixtureGroup],
        _: &E2eConfig,
        _: &ResolvedCrateConfig,
        _: &[TypeDef],
        _: &[EnumDef],
        _: &[FunctionDef],
        _: &[ErrorDef],
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(Vec::new())
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        _: &E2eConfig,
        _: &ResolvedCrateConfig,
        types: &[TypeDef],
        enums: &[EnumDef],
        functions: &[FunctionDef],
        errors: &[ErrorDef],
    ) -> anyhow::Result<String> {
        assert!(std::ptr::eq(functions, self.functions));
        assert_eq!((types.len(), enums.len(), functions.len(), errors.len()), (2, 1, 1, 1));
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(format!("custom:{}", fixture.id))
    }

    fn language_name(&self) -> &'static str {
        "python"
    }
}

#[test]
fn borrowed_custom_implementation_keeps_its_functions_aware_override_by_default() {
    let types = rich_types();
    let enums = enums();
    let functions = functions();
    let errors: Vec<ErrorDef> = serde_json::from_value(serde_json::json!([
        {"name": "ProbeError", "rust_path": "probe::ProbeError", "variants": [], "doc": ""}
    ]))
    .expect("nonempty error registry");
    let session = SnippetRenderSession::new(&types, &enums, &functions, &errors);
    let legacy = LegacyOnly {
        functions: &functions,
        calls: AtomicUsize::new(0),
    };
    let generator: &dyn E2eCodegen = &legacy;
    let body = generator
        .render_snippet_body_in_session(&rich_fixture(), &config(), &ResolvedCrateConfig::default(), &session)
        .expect("custom implementation retains its existing method");
    assert_eq!(body, "custom:rich");
    assert_eq!(legacy.calls.load(Ordering::Relaxed), 1);
}

fn caller_resolver() -> FieldResolver {
    let php = PhpGetterMap {
        getters: [("Result".into(), ["title".into()].into())].into(),
        field_types: [("Result".into(), HashMap::new())].into(),
        all_fields: [("Result".into(), ["title".into()].into())].into(),
        root_type: Some("Result".into()),
    };
    let swift = SwiftFirstClassMap {
        first_class_types: ["Result".into()].into(),
        field_types: [("Result".into(), HashMap::new())].into(),
        root_type: Some("Result".into()),
        ..SwiftFirstClassMap::default()
    };
    FieldResolver::new(
        &HashMap::new(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_php_getter_map(php)
    .with_swift_first_class_map(swift)
    .with_ir_result_fields(
        IrResultFieldMap {
            optional_fields: [("Result".into(), ["title".into()].into())].into(),
            ..IrResultFieldMap::default()
        },
        Some("Result".into()),
    )
}

#[test]
fn caller_getter_metadata_stays_shared_when_optional_paths_are_enriched() {
    let config = config();
    let resolver = caller_resolver();
    let enriched = resolver.clone().with_anchored_optional_paths(["title"]);
    assert_eq!(
        enriched.accessor("title", "php", config.call.effective_result_var()),
        "result->getTitle()"
    );
    assert_eq!(
        enriched.accessor("title", "swift", config.call.effective_result_var()),
        "result.title"
    );
    crate::e2e::field_access::metadata_storage_tests::assert_shared_custom_maps(&resolver, &enriched);
}
