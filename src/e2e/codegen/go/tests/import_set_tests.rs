//! Regression coverage for the Go test-file import set.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow. The import list was
//! derived from a heuristic that never consulted `fixture.env.api_key_var`, so a fixture declaring
//! only an API-key variable emitted `os.Getenv` with no `os` import. ~keep

use super::super::go::render_test_file;
use crate::e2e::codegen::go::GoTestFileContext;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

/// Regression test: a fixture that declares `env.api_key_var` but has no
/// `mock_response`/`http` and no `client_factory` override emits `os.Getenv` in
/// its live-API-key skip branch (see `test_function.rs`'s `api_key_var` handling).
/// The old `needs_os` heuristic in `test_file.rs` only recognized http tests,
/// `client_factory` overrides, `mock_url`/`mock_url_list` args, and bytes-as-path
/// args -- it never checked `fixture.env.api_key_var`, so this exact shape
/// referenced `os.` in the rendered body without importing `"os"`.
#[test]
fn declared_api_key_var_without_mock_or_client_factory_still_imports_os() {
    use crate::core::config::e2e::ArgMapping;
    use crate::e2e::fixture::FixtureEnv;

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "detect_service".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            args: vec![ArgMapping {
                name: "query".to_string(),
                field: "input.query".to_string(),
                arg_type: "string".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "detect_service_live".to_string(),
        category: None,
        description: "Detect service using a live API key".to_string(),
        tags: vec![],
        skip: None,
        env: Some(FixtureEnv {
            api_key_var: Some("SAMPLE_SERVICE_TOKEN".to_string()),
        }),
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({"query": "hello"}),
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };

    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let out = render_test_file(
        "service_detection",
        &[&fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/mylib",
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
            crate_facts: None,
        },
    );

    assert!(
        out.contains("os.Getenv(\"SAMPLE_SERVICE_TOKEN\")"),
        "expected the body to reference os.Getenv for the declared api_key_var; got:\n{out}"
    );
    assert!(
        out.contains("\t\"os\""),
        "body references os.Getenv, so \"os\" must be imported; got:\n{out}"
    );
}
