use super::render_test_file;
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn call(function: &str, traits: &[&str]) -> CallConfig {
    let mut call = CallConfig {
        function: function.to_string(),
        r#async: false,
        ..Default::default()
    };
    call.overrides.insert(
        "rust".to_string(),
        CallOverride {
            trait_imports: traits.iter().map(|name| (*name).to_string()).collect(),
            ..Default::default()
        },
    );
    call
}

fn config() -> E2eConfig {
    let mut default = call("chat", &["ChatClient"]);
    default.overrides.get_mut("rust").unwrap().client_factory = Some("create_client".to_string());
    E2eConfig {
        call: default,
        calls: [
            ("file".to_string(), call("file", &["FileClient"])),
            ("batch".to_string(), call("batch", &["BatchClient"])),
            ("fallback".to_string(), call("chat", &[])),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    }
}

fn fixture(name: &str, call: Option<&str>) -> Fixture {
    Fixture {
        id: name.to_string(),
        description: "client method dispatch".to_string(),
        call: call.map(str::to_string),
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn render(fixtures: &[Fixture], config: &E2eConfig) -> String {
    let config_source =
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"sample_lib\"\nsources = [\"src/lib.rs\"]\n";
    let crate_config: crate::core::config::NewAlefConfig = toml::from_str(config_source).unwrap();
    render_test_file(
        "client_traits",
        &fixtures.iter().collect::<Vec<_>>(),
        config,
        &crate_config.resolve().unwrap().remove(0),
        &[],
        &[],
        &[],
        "sample_lib",
        false,
        None,
        false,
    )
}

fn imports(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| line.starts_with("use sample_lib::"))
        .collect()
}

#[test]
fn selected_call_traits_replace_unrelated_global_traits() {
    let source = render(&[fixture("file_request", Some("file"))], &config());
    assert_eq!(imports(&source), ["use sample_lib::FileClient;"]);
    assert!(source.contains("client.file()"), "{source}");
}

#[test]
fn empty_named_trait_override_preserves_global_fallback() {
    let mut config = config();
    config
        .calls
        .get_mut("fallback")
        .unwrap()
        .overrides
        .get_mut("rust")
        .unwrap()
        .result_is_simple = true;
    let source = render(&[fixture("fallback_request", Some("fallback"))], &config);
    assert_eq!(imports(&source), ["use sample_lib::ChatClient;"]);
}

#[test]
fn mixed_fixtures_union_and_deduplicate_only_contributing_traits() {
    let source = render(
        &[
            fixture("first_file", Some("file")),
            fixture("second_file", Some("file")),
            fixture("batch_request", Some("batch")),
        ],
        &config(),
    );
    assert_eq!(
        imports(&source),
        ["use sample_lib::BatchClient;", "use sample_lib::FileClient;"]
    );
}

#[test]
fn automatic_call_selection_uses_the_selected_trait_override() {
    let mut config = config();
    config.calls.get_mut("file").unwrap().select_when =
        Some(serde_json::from_value(serde_json::json!({"id_prefix": "file_"})).unwrap());
    let source = render(&[fixture("file_request", None)], &config);
    assert_eq!(imports(&source), ["use sample_lib::FileClient;"]);
}

#[test]
fn free_functions_import_no_client_traits() {
    let mut config = config();
    let free = config.calls.get_mut("file").unwrap().overrides.get_mut("rust").unwrap();
    free.client_factory = Some(String::new());
    let source = render(&[fixture("free_request", Some("file"))], &config);
    assert_eq!(imports(&source), ["use sample_lib::file;"]);
}

#[test]
fn stub_fixtures_do_not_contribute_configured_traits_to_a_client_suite() {
    let mut config = config();
    config.calls.insert("stub".to_string(), call("", &["UnusedClient"]));
    let source = render(
        &[
            fixture("file_request", Some("file")),
            fixture("schema_stub", Some("stub")),
        ],
        &config,
    );
    assert_eq!(imports(&source), ["use sample_lib::FileClient;"]);
}

#[test]
fn http_fixtures_do_not_contribute_configured_client_traits() {
    let mut http = fixture("http_request", Some("batch"));
    http.http = Some(
        serde_json::from_value(serde_json::json!({
            "handler": {"route": "/", "method": "GET"},
            "request": {"path": "/", "method": "GET"},
            "expected_response": {"status_code": 200}
        }))
        .unwrap(),
    );
    let source = render(&[fixture("file_request", Some("file")), http], &config());
    assert_eq!(imports(&source), ["use sample_lib::FileClient;"]);
}

const SAMPLE_LIBRARY: &str = r#"
pub struct Client;
pub fn create_client(_: String, _: Option<String>, _: Option<()>, _: Option<()>, _: Option<()>)
    -> Result<Client, String> { Ok(Client) }
pub trait ChatClient { fn chat(&self) -> Result<String, String>; }
pub trait FileClient { fn file(&self) -> Result<String, String>; }
pub trait BatchClient { fn batch(&self) -> Result<String, String>; }
impl ChatClient for Client { fn chat(&self) -> Result<String, String> { Ok("chat".into()) } }
impl FileClient for Client { fn file(&self) -> Result<String, String> { Ok("file".into()) } }
impl BatchClient for Client { fn batch(&self) -> Result<String, String> { Ok("batch".into()) } }
"#;

fn rustc() -> std::process::Command {
    let _path = crate::test_support::PATH_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let executable = which::which("rustc").expect("Cargo tests require rustc");
    let mut command = std::process::Command::new(executable);
    command.env("PATH", std::env::var_os("PATH").expect("Rust compiler requires PATH"));
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command.args(["--edition=2024", "-Dwarnings"]);
    command
}

fn compile_suite(directory: &std::path::Path, name: &str, source: &str) -> std::process::Output {
    let source_path = directory.join(format!("{name}.rs"));
    std::fs::write(&source_path, source).unwrap();
    rustc()
        .arg("--test")
        .arg(&source_path)
        .arg("--extern")
        .arg(format!("sample_lib={}", directory.join("libsample_lib.rlib").display()))
        .arg("-o")
        .arg(directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
        .output()
        .unwrap()
}

fn compile_library(directory: &std::path::Path) {
    let library = directory.join("sample_lib.rs");
    std::fs::write(&library, SAMPLE_LIBRARY).unwrap();
    let built = rustc()
        .args(["--crate-type=lib", "--crate-name=sample_lib"])
        .arg(&library)
        .arg("-o")
        .arg(directory.join("libsample_lib.rlib"))
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
}

fn execute_suite(directory: &std::path::Path, name: &str, fixtures: &[Fixture]) {
    let source = render(fixtures, &config());
    let built = compile_suite(directory, name, &source);
    assert!(
        built.status.success(),
        "{name}: {}\n{source}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = std::process::Command::new(directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
        .output()
        .unwrap();
    assert!(run.status.success(), "{name}: {run:?}");
    assert!(!fixtures.is_empty());
    assert!(
        String::from_utf8_lossy(&run.stdout)
            .contains(&format!("test result: ok. {} passed; 0 failed;", fixtures.len()))
    );
}

#[test]
fn generated_client_suites_compile_and_execute_with_warnings_denied() {
    let directory = tempfile::tempdir().unwrap();
    compile_library(directory.path());
    let cases = [
        ("default_call", vec![fixture("chat_request", None)]),
        ("file_call", vec![fixture("file_request", Some("file"))]),
        ("batch_call", vec![fixture("batch_request", Some("batch"))]),
        ("fallback_call", vec![fixture("fallback_request", Some("fallback"))]),
        (
            "mixed_calls",
            vec![
                fixture("file_request", Some("file")),
                fixture("batch_request", Some("batch")),
            ],
        ),
    ];
    assert_eq!(cases.len(), 5);
    for (name, fixtures) in cases {
        execute_suite(directory.path(), name, &fixtures);
    }
    let source = render(&[fixture("file_request", Some("file"))], &config());
    let missing = compile_suite(
        directory.path(),
        "missing_trait",
        &source.replace("use sample_lib::FileClient;", ""),
    );
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("E0599"));
    let extra = compile_suite(
        directory.path(),
        "unused_trait",
        &format!("{source}\nuse sample_lib::ChatClient;\n"),
    );
    assert!(!extra.status.success());
    assert!(String::from_utf8_lossy(&extra.stderr).contains("unused import"));
}
