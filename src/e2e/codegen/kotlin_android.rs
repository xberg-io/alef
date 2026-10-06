//! Kotlin Android e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates host-JVM tests that validate the AAR-bundled Java facade and Kotlin wrapper
//! via JNA against the generated FFI library. Tests are emitted to `e2e/kotlin_android/src/test/kotlin/`
//! without requiring an Android emulator — the tests run directly on the host JVM against
//! the shared library.

#[cfg(test)]
mod assertion_guard_tests;
mod enum_fixtures;
mod gradle;
#[cfg(test)]
mod gradle_tests;
mod gradle_wrapper;
mod project;
mod stubs;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;

use super::E2eCodegen;

pub use stubs::emit_test_backend;

/// Kotlin Android e2e code generator.
/// Emits a host-JVM test project that depends on the AAR-bundled Java facade
/// and Kotlin wrapper via sourceSets and JNA, without requiring an Android emulator.
pub struct KotlinAndroidE2eCodegen;

impl E2eCodegen for KotlinAndroidE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        project::generate(groups, e2e_config, config, type_defs, enums, functions)
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        crate::e2e::codegen::kotlin::snippet::render_snippet_body(fixture, e2e_config, config, type_defs, enums, true)
    }

    /// Without this override, the trait default forwards to [`Self::render_snippet_body`] and
    /// silently drops `functions`, so the docs-snippet field-access oracle can only anchor a
    /// call's result type via a same-named IR *method* rather than the free function `functions`
    /// carries -- misresolving the root type (and every field-availability verdict downstream of
    /// it) whenever an unrelated method happens to share the call's name. The e2e TEST generator
    /// (`project::generate`) already receives `functions` correctly; this keeps the documentation
    /// snippet path anchored at the exact same declared result type. Mirrors
    /// `kotlin.rs::render_snippet_body_with_functions`. ~keep
    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        crate::e2e::codegen::kotlin::snippet::render_snippet_body_with_ir(
            fixture, e2e_config, config, type_defs, enums, true, functions,
        )
    }

    fn renders_client_config(&self) -> bool {
        true
    }

    fn language_name(&self) -> &'static str {
        "kotlin_android"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, FunctionDef, MethodDef, TypeDef, TypeRef};
    use crate::e2e::codegen::kotlin::KotlinE2eCodegen;
    use crate::e2e::config::{CallConfig, CallOverride};
    use crate::e2e::fixture::Fixture;
    use std::collections::HashSet;

    #[test]
    fn snippet_uses_android_coroutine_call_without_junit_harness() {
        let fixture = Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            r#async: true,
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                class: Some("DocumentApi".into()),
                ..CallOverride::default()
            },
        );
        let body = KotlinAndroidE2eCodegen
            .render_snippet_body(
                &fixture,
                &E2eConfig {
                    call,
                    ..E2eConfig::default()
                },
                &ResolvedCrateConfig::default(),
                &[],
                &[],
            )
            .expect("snippet renders");

        assert!(body.contains("kotlinx.coroutines.runBlocking"));
        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
    }

    /// Pins that a `client_factory` docs snippet reached through the Kotlin Android
    /// entry point reads the credential from the environment and never points the
    /// reader at the e2e mock server: no `MOCK_SERVER` env var, no `mockServer`
    /// system property, no `/fixtures/<id>` route, and no inlined `"test-key"`
    /// credential. Delegates to `kotlin::snippet::render_snippet_body(..., true)`, so
    /// this must go through `KotlinAndroidE2eCodegen`, not the plain `kotlin` renderer.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let body = KotlinAndroidE2eCodegen
            .render_snippet_body(
                &fixture,
                &E2eConfig {
                    call,
                    ..E2eConfig::default()
                },
                &ResolvedCrateConfig::default(),
                &[],
                &[],
            )
            .expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(!body.contains("mockServer"), "mock-server property leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("System.getenv(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
        assert!(
            body.contains("createClient(apiKey = apiKey)"),
            "an unconfigured project must construct the client without a mock base URL:\n{body}"
        );
    }

    /// Confirms the `use`-block client release (`kotlin::snippet::render_snippet_body`'s
    /// `client_factory` branch) reaches the Kotlin Android entry point too, since
    /// `KotlinAndroidE2eCodegen::render_snippet_body` delegates to the same shared renderer
    /// rather than a separate `kotlin_android/snippet_body.jinja`. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_in_a_use_block() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = KotlinAndroidE2eCodegen
            .render_snippet_body(
                &fixture,
                &E2eConfig {
                    call,
                    ..E2eConfig::default()
                },
                &config,
                &[],
                &[],
            )
            .expect("snippet renders");

        assert!(
            body.contains("Sample.createClient(apiKey = apiKey).use { client -> client.chat() }"),
            "the client must be released via a `use` block around the call:\n{body}"
        );
        assert!(
            !body.contains("client.close()"),
            "no bare close() call must remain:\n{body}"
        );
    }

    #[test]
    fn excluded_binding_fixture_uses_native_disabled_test() {
        let rendered = crate::e2e::template_env::render(
            "kotlin_android/excluded_fixtures.kt.jinja",
            minijinja::context! {
                package_name => "dev.sample",
                entries => vec![minijinja::context! {
                    name => "visitor_round_trip",
                    reason => "visitor is excluded by crates.kotlin_android.exclude_functions",
                }],
            },
        );

        assert!(rendered.contains("@Disabled(\"visitor is excluded by crates.kotlin_android.exclude_functions\")"));
        assert!(rendered.contains("fun visitor_round_trip() {}"));
    }

    /// The IR shape backing every test below: a free function `extract_batch` returning
    /// `BatchResult { results: Vec<ExtractionResult> }`, where `ExtractionResult` nests
    /// `Metadata { output_format: String }`. Mirrors the consumer report exactly: a
    /// `results[0].metadata.output_format` path the binding genuinely exposes.
    ///
    /// `ExtractionResult` ALSO declares an unrelated inherent method also named
    /// `extract_batch` (a builder-style method a real client type could plausibly carry).
    /// `CallIr::signature` prefers a free-function match over a method match, so this
    /// collision is harmless whenever `functions` is passed through -- but if `functions` is
    /// ever dropped, the method search is all that is left, and it resolves the call's return
    /// type to the WRONG struct (`ExtractionResult`, which has no `results` field). ~keep
    fn batch_result_ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
        let type_defs = vec![
            TypeDef {
                name: "BatchResult".into(),
                fields: vec![FieldDef {
                    name: "results".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("ExtractionResult".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ExtractionResult".into(),
                fields: vec![FieldDef {
                    name: "metadata".into(),
                    ty: TypeRef::Named("Metadata".into()),
                    ..FieldDef::default()
                }],
                methods: vec![MethodDef {
                    name: "extract_batch".into(),
                    return_type: TypeRef::Named("ExtractionResult".into()),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Metadata".into(),
                fields: vec![FieldDef {
                    name: "output_format".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ];
        let functions = vec![FunctionDef {
            name: "extract_batch".into(),
            return_type: TypeRef::Named("BatchResult".into()),
            ..FunctionDef::default()
        }];
        (type_defs, functions)
    }

    fn batch_result_fixture(field: &str) -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "batch_extract",
            "description": "Batch extract",
            "input": {},
            "assertions": [{"type": "equals", "field": field, "value": "html"}],
            "docs": {"topic": "smoke", "stem": "batch_extract"}
        }))
        .expect("fixture must parse")
    }

    fn batch_result_e2e_config() -> E2eConfig {
        E2eConfig {
            call: CallConfig {
                function: "extract_batch".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            result_fields: HashSet::from(["results".to_string()]),
            ..E2eConfig::default()
        }
    }

    /// OBSERVED root cause: `KotlinAndroidE2eCodegen` has no `render_snippet_body_with_functions`
    /// override, so `render_body.rs`'s call into it hits the trait's default, which discards the
    /// `functions` registry entirely and forwards to `render_snippet_body` -- the same method the
    /// plain `kotlin` backend overrides to keep `functions` wired through
    /// (`kotlin.rs::render_snippet_body_with_functions`). The e2e TEST generator
    /// (`kotlin_android/project.rs::generate`) receives `functions` correctly, which is why the
    /// generated JUnit suite calls `result.results.first().metadata.outputFormat` successfully
    /// while the documentation snippet for the exact same call omits it.
    ///
    /// This test reproduces the field-access oracle's `Some(false)` misclassification the missing
    /// `functions` registry produces, through the ACTUAL public entry point `render_body.rs`
    /// calls, not merely at `FieldResolver::result_field_oracle_knows`'s level -- an anchored
    /// oracle correctly rejecting a name the WRONG root type doesn't declare is by design, not a
    /// bug; the bug is that `functions` being dropped is what anchors kotlin_android on the wrong
    /// root in the first place.
    #[test]
    fn kotlin_android_docs_snippet_shows_the_same_declared_field_kotlin_does() {
        let (type_defs, functions) = batch_result_ir();
        let fixture = batch_result_fixture("results[0].metadata.output_format");
        let e2e_config = batch_result_e2e_config();
        let config = ResolvedCrateConfig::default();

        let kotlin_body = KotlinE2eCodegen
            .render_snippet_body_with_functions(&fixture, &e2e_config, &config, &type_defs, &[], &functions, &[])
            .expect("kotlin snippet renders");
        assert!(
            kotlin_body.contains(".results"),
            "control: the plain kotlin backend must show the `results` field `extract_batch` \
             genuinely declares:\n{kotlin_body}"
        );

        let android_body = KotlinAndroidE2eCodegen
            .render_snippet_body_with_functions(&fixture, &e2e_config, &config, &type_defs, &[], &functions, &[])
            .expect("kotlin_android snippet renders");
        assert!(
            android_body.contains(".results"),
            "kotlin_android must show the same `results` field the plain kotlin backend shows -- \
             both back the identical Rust `extract_batch` result type:\n{android_body}"
        );
    }

    /// The companion direction: a field genuinely absent from every type in the IR must stay
    /// omitted, both before and after the `functions`-wiring fix. Without this, "make
    /// `kotlin_android` show everything `kotlin` shows" could be satisfied by gutting the
    /// oracle instead of fixing the wiring gap, and a config-drift field would silently stop
    /// being reported.
    #[test]
    fn kotlin_android_docs_snippet_still_omits_a_field_the_result_type_never_declares() {
        let (type_defs, functions) = batch_result_ir();
        let fixture = batch_result_fixture("results[0].metadata.nonexistent_field");
        let mut e2e_config = batch_result_e2e_config();
        e2e_config.result_fields.insert("nonexistent_field".to_string());
        let config = ResolvedCrateConfig::default();

        let kotlin_body = KotlinE2eCodegen
            .render_snippet_body_with_functions(&fixture, &e2e_config, &config, &type_defs, &[], &functions, &[])
            .expect("kotlin snippet renders");
        assert!(
            !kotlin_body.contains("nonexistent_field"),
            "control: plain kotlin must never derive an accessor for a field no type declares:\n{kotlin_body}"
        );

        let android_body = KotlinAndroidE2eCodegen
            .render_snippet_body_with_functions(&fixture, &e2e_config, &config, &type_defs, &[], &functions, &[])
            .expect("kotlin_android snippet renders");
        assert!(
            !android_body.contains("nonexistent_field"),
            "a genuinely undeclared field must stay rejected after the functions-wiring fix, not \
             just the false positive on `results`:\n{android_body}"
        );
    }
}
