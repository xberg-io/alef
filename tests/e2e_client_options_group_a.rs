use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::all_generators;
use alef::e2e::fixture::{Assertion, Fixture, FixtureEnv, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

const LANGUAGES: &[&str] = &[
    "node",
    "wasm",
    "go",
    "java",
    "kotlin",
    "kotlin_android",
    "csharp",
    "dart",
];

fn capture_generated(language: &str, files: &[alef::core::backend::GeneratedFile]) {
    let Ok(root) = std::env::var("ALEF_CLIENT_OPTIONS_CAPTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(root.is_absolute(), "capture root must be absolute");
    assert!(!files.is_empty(), "capture must contain generated files");
    for file in files {
        assert!(!file.path.is_absolute());
        assert!(
            !file
                .path
                .components()
                .any(|part| part == std::path::Component::ParentDir)
        );
        let path = root.join(language).join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).expect("capture directory");
        std::fs::write(path, &file.content).expect("capture generated file");
    }
}

fn configuration(language: &str) -> NewAlefConfig {
    toml::from_str(&format!(
        r#"
[workspace]
languages = ["{language}"]
[[crates]]
name = "demo_client"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "ping"
module = "demo_client"
result_is_simple = true
returns_result = true
[crates.e2e.call.overrides.{language}]
client_factory = "create_client"
client_factory_from_json = "create_client_from_json"
class = "DemoClient"
"#
    ))
    .expect("valid config")
}

fn group(options: bool, live: bool) -> FixtureGroup {
    let fixture = Fixture {
        id: "options_probe".into(),
        description: "Client options probe".into(),
        category: Some("options".into()),
        input: serde_json::json!({}),
        assertions: vec![Assertion {
            assertion_type: "not_error".into(),
            ..Assertion::default()
        }],
        client_options: options.then(|| {
            BTreeMap::from([
                (
                    "blocked_networks".into(),
                    serde_json::json!(["10.0.0.0/8", "quote\"slash\\"]),
                ),
                ("thresholds".into(), serde_json::json!([-2, 1.5])),
                ("empty".into(), serde_json::json!([])),
                ("large".into(), serde_json::json!(18446744073709551615_u64)),
            ])
        }),
        env: live.then(|| FixtureEnv {
            api_key_var: Some("OPTIONS_API_KEY".into()),
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::json!({"ok": true})),
            stream_chunks: None,
            headers: BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        ..Fixture::default()
    };
    FixtureGroup {
        category: "options".into(),
        fixtures: vec![fixture],
    }
}

fn generated(language: &str, options: bool, live: bool) -> String {
    let config = configuration(language);
    let e2e = config.crates[0].e2e.as_ref().unwrap();
    let resolved = config.clone().resolve().unwrap().remove(0);
    let generators = all_generators();
    let generator = generators
        .iter()
        .find(|g| g.language_name() == language)
        .expect("registered language");
    let files = generator
        .generate(&[group(options, live)], e2e, &resolved, &[], &[], &[], &[])
        .expect("generation");
    if options {
        let capture_name = if live {
            format!("{language}/env")
        } else {
            language.into()
        };
        capture_generated(&capture_name, &files);
    }
    files
        .into_iter()
        .filter(|file| file.path.to_string_lossy().to_ascii_lowercase().contains("options"))
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn group_a_client_options_reach_json_factories_in_all_backend_variants() {
    for language in LANGUAGES {
        let output = generated(language, true, false);
        assert!(
            output.contains("options_probe") || output.contains("OptionsProbe"),
            "{language}: fixture excluded\n{output}"
        );
        assert!(
            output.to_ascii_lowercase().contains("createclientfromjson") || output.contains("create_client_from_json"),
            "{language}: JSON factory absent\n{output}"
        );
        for value in [
            "blocked_networks",
            "10.0.0.0/8",
            "thresholds",
            "18446744073709551615",
            "empty",
        ] {
            assert!(output.contains(value), "{language}: missing {value}\n{output}");
        }
    }
}

#[test]
fn group_a_options_preserve_live_credentials_and_mock_routing() {
    for language in LANGUAGES {
        let output = generated(language, true, true);
        assert!(
            output.contains("OPTIONS_API_KEY"),
            "{language}: credential lookup missing\n{output}"
        );
        assert!(
            output.contains("MOCK_SERVER_URL") || output.contains("_fixtureUrl"),
            "{language}: routing missing\n{output}"
        );
    }
}

#[test]
fn group_a_fixtures_without_options_keep_plain_client_construction() {
    for language in LANGUAGES {
        let output = generated(language, false, false);
        assert!(
            !output.to_ascii_lowercase().contains("createclientfromjson"),
            "{language}: changed plain factory\n{output}"
        );
        assert!(
            !output.contains("blocked_networks"),
            "{language}: invented options\n{output}"
        );
    }
}
