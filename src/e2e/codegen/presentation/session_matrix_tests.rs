use super::metadata_cache_tests::FactCounts;
use super::resolver_reuse_tests::{config, enums, functions, rich_fixture, rich_types};
use super::session_render_tests::PRESENTATION_LANGUAGES;
use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::SnippetConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeRef};
use crate::e2e::field_access::VariantAccessorMap;
use crate::e2e::field_access::metadata_storage_tests::{
    assert_shared_maps, assert_shared_variant_maps, assert_variant_map_contents, clone_cached_maps_into_owned_storage,
};
use std::collections::{BTreeSet, HashMap};

fn project() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "cache-probe".into(),
        ..ResolvedCrateConfig::default()
    }
}

fn derived_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "derived-cache-cell", "description": "Derive a shown field from an assertion", "input": {},
        "docs": {"topic": "reuse"}, "assertions": [{"type": "not_empty", "field": "title"}]
    }))
    .unwrap()
}

fn item_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "item-cache-cell", "description": "Show the other call's required title", "input": {},
        "call": "item", "docs": {"topic": "reuse", "shows": ["title"]}
    }))
    .unwrap()
}

fn model_functions() -> Vec<FunctionDef> {
    let mut functions = functions();
    functions.push(FunctionDef {
        name: "convert_item".into(),
        return_type: TypeRef::Named("Item".into()),
        ..FunctionDef::default()
    });
    functions
}

fn model_config() -> E2eConfig {
    let mut config = config();
    config.registry.packages.entry("brew".into()).or_default().name = Some("cache-probe".into());
    let mut item = config.call.clone();
    item.function = "convert_item".into();
    item.result_var = "other_result".into();
    config.calls.insert("item".into(), item);
    config
}

fn model_enums() -> Vec<EnumDef> {
    let mut enums = enums();
    enums.push(EnumDef {
        name: "Format".into(),
        variants: vec![EnumVariant {
            name: "Html".into(),
            fields: vec![FieldDef {
                name: "_0".into(),
                ty: TypeRef::Named("Item".into()),
                ..FieldDef::default()
            }],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    });
    enums
}

fn real_report_counts(languages: &[&str], expected_cells: usize) -> FactCounts {
    let counts = FactCounts {
        presentation_only: true,
        ..FactCounts::default()
    };
    counts.observe(|| render_real_report(languages, expected_cells));
    counts
}

fn render_real_report(languages: &[&str], expected_cells: usize) {
    let types = rich_types();
    let enums = model_enums();
    let functions = model_functions();
    let config = model_config();
    let fixtures = [rich_fixture(), derived_fixture(), item_fixture()];
    assert!(!fixtures[1].has_docs_presentation());
    assert_eq!(fixtures[1].assertions[0].field.as_deref(), Some("title"));
    let languages: Vec<_> = languages.iter().map(|language| (*language).to_owned()).collect();
    let snippets = SnippetConfig {
        output: "docs/snippet-cache-test".into(),
        ..SnippetConfig::default()
    };
    let report = crate::e2e::snippets::generate_snippet_report(
        &fixtures,
        &languages,
        &config,
        &snippets,
        &project(),
        &types,
        &enums,
        &functions,
        &[],
    )
    .expect("all real presentation cells must render");
    assert_real_cells(&report, &fixtures, &languages, expected_cells);
}

fn assert_real_cells(
    report: &crate::e2e::snippets::SnippetGenerationReport,
    fixtures: &[Fixture],
    languages: &[String],
    expected_cells: usize,
) {
    let expected: BTreeSet<_> = fixtures
        .iter()
        .flat_map(|fixture| {
            languages
                .iter()
                .map(move |language| (fixture.id.as_str(), language.as_str()))
        })
        .collect();
    assert_eq!(expected.len(), expected_cells);
    assert_eq!(report.coverage.expected.len(), expected_cells);
    assert_eq!(
        report.coverage.generated.len(),
        expected_cells,
        "{:?}",
        report.coverage.missing
    );
    assert_eq!(report.snippets.len(), expected_cells);
    let actual: BTreeSet<_> = report
        .snippets
        .iter()
        .map(|snippet| {
            assert_ne!(snippet.file.content.trim(), "");
            (snippet.fixture_id.as_str(), snippet.language.as_str())
        })
        .collect();
    assert_eq!(actual, expected);
}

fn assert_shared_constructor_counts(counts: &FactCounts, result_rules: usize) {
    assert_eq!(
        *counts.entries.lock().unwrap(),
        HashMap::from([("csharp_variants".into(), 1), ("dart_variants".into(), 1),]),
        "both actual builders must emit a nonempty narrowing table"
    );
    assert_eq!(
        counts.related_counts(),
        HashMap::from([
            ("fields".into(), 1),
            ("enum".into(), 1),
            ("collection".into(), 1),
            ("result".into(), result_rules),
            ("csharp_variants".into(), 1),
            ("dart_variants".into(), 1),
        ]),
        "unscoped constructors: {:?}",
        *counts.outside_presentation.lock().unwrap()
    );
    assert_eq!(*counts.outside_presentation.lock().unwrap(), HashMap::new());
}

#[test]
fn all_presentation_backends_share_facts_across_rich_derived_and_distinct_call_cells() {
    let counts = real_report_counts(PRESENTATION_LANGUAGES, 48);
    assert_shared_constructor_counts(&counts, 2);
}

#[test]
fn variant_cache_builds_each_nonempty_backend_map_once_across_six_real_cells() {
    let counts = real_report_counts(&["csharp", "dart"], 6);
    assert_shared_constructor_counts(&counts, 1);
}

#[test]
fn constructor_observer_keeps_events_and_scopes_after_unobserved_report_priming() {
    const CHILD_TEST: &str = concat!(
        "e2e::codegen::presentation::session_matrix_tests::",
        "constructor_observer_keeps_events_and_scopes_after_unobserved_report_priming"
    );
    const CHILD_ENV: &str = "ALEF_METADATA_OBSERVER_CHILD";
    if std::env::var(CHILD_ENV).as_deref() == Ok(CHILD_TEST) {
        let counts = FactCounts {
            presentation_only: true,
            ..FactCounts::default()
        };
        counts.observe(|| {
            std::thread::spawn(|| {
                tracing::dispatcher::get_default(|dispatch| {
                    assert!(dispatch.downcast_ref::<tracing::subscriber::NoSubscriber>().is_some());
                });
                render_real_report(&["csharp", "dart"], 6);
            })
            .join()
            .expect("the unobserved real-report primer must complete");
            render_real_report(&["csharp", "dart"], 6);
        });
        assert_shared_constructor_counts(&counts, 1);
        return;
    }
    assert_fresh_observer_test(CHILD_TEST, CHILD_ENV);
}

fn assert_fresh_observer_test(test_name: &str, child_env: &str) {
    let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            test_name,
            "--nocapture",
            "--color",
            "never",
            "--test-threads=1",
        ])
        .env(child_env, test_name)
        .output()
        .expect("fresh-process observer regression");
    let stdout = String::from_utf8(child.stdout).expect("child UTF-8 stdout");
    let stderr = String::from_utf8_lossy(&child.stderr);
    assert!(child.status.success(), "{stdout}\n{stderr}");
    assert_eq!(stdout.lines().filter(|line| *line == "running 1 test").count(), 1);
    assert_eq!(
        stdout
            .lines()
            .filter(|line| *line == format!("test {test_name} ... ok"))
            .count(),
        1
    );
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"))
            .count(),
        1
    );
}

#[test]
fn variant_cache_shares_actual_session_storage_across_requests_and_separates_backends() {
    let types = rich_types();
    let enums = model_enums();
    let functions = model_functions();
    let session = SnippetRenderSession::new(&types, &enums, &functions, &[]);
    let resolver = || {
        FieldResolver::new(
            &HashMap::new(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )
    };
    for (first_backend, second_backend, narrowing, payload) in [
        (
            VariantAccessorBackend::CSharp,
            VariantAccessorBackend::CSharp,
            "AsHtml",
            None,
        ),
        (
            VariantAccessorBackend::Dart,
            VariantAccessorBackend::Dart,
            "Format_Html",
            Some("field0"),
        ),
    ] {
        let first = session.enrich_variant_fields(resolver(), first_backend, |model| {
            assert!(std::ptr::eq(model, enums.as_slice()));
            VariantAccessorMap {
                narrowing: [(("Format".into(), "Html".into()), narrowing.into())].into(),
                payload: payload
                    .map(|payload| [(("Format".into(), "Html".into()), payload.into())].into())
                    .unwrap_or_default(),
            }
        });
        let second = session.enrich_variant_fields(resolver(), second_backend, |_| {
            panic!("the same backend/model builder must not run twice")
        });
        assert_variant_map_contents(&first, narrowing, payload);
        assert_variant_map_contents(&second, narrowing, payload);
        assert_shared_variant_maps(&first, &second);
    }
}

#[test]
fn every_supported_builtin_template_matches_its_one_shot_body_for_all_three_call_shapes() {
    let types = rich_types();
    let enums = enums();
    let functions = model_functions();
    let config = model_config();
    let project = project();
    let session = SnippetRenderSession::new(&types, &enums, &functions, &[]);
    let generators = crate::e2e::codegen::all_generators();
    assert_eq!(generators.len(), 21);
    let unsupported = BTreeSet::from(["gleam", "homebrew", "php_ext"]);
    let mut cells = BTreeSet::new();
    let mut refusals = BTreeSet::new();
    for fixture in [rich_fixture(), derived_fixture(), item_fixture()] {
        for generator in &generators {
            let language = generator.language_name();
            let previous = generator.render_snippet_body_with_functions(
                &fixture,
                &config,
                &project,
                &types,
                &enums,
                &functions,
                &[],
            );
            let current = generator.render_snippet_body_in_session(&fixture, &config, &project, &session);
            if unsupported.contains(language) {
                assert_eq!(previous.unwrap_err().to_string(), current.unwrap_err().to_string());
                refusals.insert((fixture.id.clone(), language));
            } else {
                let previous = previous.unwrap_or_else(|error| panic!("{} {language}: {error:#}", fixture.id));
                let current = current.unwrap_or_else(|error| panic!("{} {language}: {error:#}", fixture.id));
                assert_ne!(previous.trim(), "");
                assert_eq!(current.as_bytes(), previous.as_bytes(), "{} {language}", fixture.id);
                cells.insert((fixture.id.clone(), language));
            }
        }
    }
    assert_eq!(cells.len(), 54);
    assert_eq!(refusals.len(), 9);
}

#[test]
fn sessions_with_identical_type_names_keep_model_optionality_independent() {
    let optional_types = rich_types();
    let mut required_types = rich_types();
    required_types[0].fields[0].ty = TypeRef::String;
    required_types[0].fields[0].optional = false;
    let enums = enums();
    let functions = functions();
    let config = config();
    let fixture = rich_fixture();
    let optional = SnippetRenderSession::new(&optional_types, &enums, &functions, &[]);
    let required = SnippetRenderSession::new(&required_types, &enums, &functions, &[]);
    let first = optional.resolve(&fixture, &config, "rust");
    let other = required.resolve(&fixture, &config, "rust");
    assert_eq!(first.len(), 3);
    assert_eq!(other.len(), 3);
    assert!(first[0].shown_optional);
    assert!(!other[0].shown_optional);
    assert_ne!(first, other);
    assert_eq!(optional.resolve(&fixture, &config, "rust"), first);
    assert_eq!(
        first,
        resolve(&fixture, &config, "rust", &optional_types, &enums, &functions)
    );
    assert_eq!(
        other,
        resolve(&fixture, &config, "rust", &required_types, &enums, &functions)
    );
}

#[test]
fn aliases_result_variables_and_call_roots_remain_local_to_each_resolution() {
    let types = rich_types();
    let enums = enums();
    let functions = model_functions();
    let mut first_config = model_config();
    first_config.fields.insert("heading".into(), "title".into());
    first_config.call.result_var = "first_result".into();
    let mut second_config = model_config();
    second_config.fields.insert("heading".into(), "items[0].title".into());
    second_config.call.result_var = "second_result".into();
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "alias-cache-cell", "description": "Show the configured alias", "input": {},
        "docs": {"topic": "reuse", "shows": ["heading"]}
    }))
    .unwrap();
    let session = SnippetRenderSession::new(&types, &enums, &functions, &[]);
    let first = session.resolve(&fixture, &first_config, "rust");
    let second = session.resolve(&fixture, &second_config, "rust");
    let item = session.resolve(&item_fixture(), &first_config, "rust");
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(item.len(), 1);
    assert_eq!(first[0].expression, "first_result.title");
    assert_eq!(second[0].expression, "second_result.items[0].title");
    assert_eq!(item[0].expression, "other_result.title");
    assert!(first[0].shown_optional);
    assert!(!item[0].shown_optional);
    assert_eq!(
        first,
        resolve(&fixture, &first_config, "rust", &types, &enums, &functions)
    );
    assert_eq!(
        second,
        resolve(&fixture, &second_config, "rust", &types, &enums, &functions)
    );
    assert_eq!(session.resolve(&fixture, &first_config, "rust"), first);
}

#[test]
fn sharing_oracle_rejects_cache_then_clone_without_extra_builds_or_changed_accessors() {
    let types = rich_types();
    let enums = enums();
    let functions = functions();
    let config = config();
    let factory = ResolverFactory::new(&types, &enums, &functions);
    let counts = FactCounts::default();
    let (original, detached, before) = counts.observe(|| {
        let original = factory.build(&config, &config.call, "rust");
        let before = counts.counts.lock().unwrap().clone();
        let detached = clone_cached_maps_into_owned_storage(&original);
        assert_eq!(*counts.counts.lock().unwrap(), before);
        (original, detached, before)
    });
    assert!(original.is_enum("kind"));
    assert!(original.is_array("items"));
    assert!(original.is_optional("title"));
    assert_eq!(
        before,
        HashMap::from([
            ("fields".into(), 1),
            ("enum".into(), 1),
            ("collection".into(), 1),
            ("result".into(), 1),
        ])
    );
    assert_eq!(
        detached.accessor("title", "rust", "result"),
        original.accessor("title", "rust", "result")
    );
    assert_eq!(detached.is_enum("kind"), original.is_enum("kind"));
    assert_eq!(detached.is_array("items"), original.is_array("items"));
    assert_eq!(detached.is_optional("title"), original.is_optional("title"));
    assert_eq!(*counts.counts.lock().unwrap(), before);
    assert!(std::panic::catch_unwind(|| assert_shared_maps(&original, &detached)).is_err());
}
