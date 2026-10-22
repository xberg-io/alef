//! Verifies the Elixir e2e codegen emits a `.formatter.exs` alongside `mix.exs`.
//!
//! `.ex`/`.exs` are excluded from poly's pass, so `mix format` is their only
//! formatter — and a bare `mix format` refuses to run without an `inputs:` key,
//! which lives in `.formatter.exs`. Without this file the generated Elixir suite
//! is never formatted at all and ships with the emitter's unwrapped long lines.

use std::path::Path;

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "MyLib"
result_var = "result"
returns_result = true
args = [
  { name = "input", field = "input", type = "string" },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn smoke_group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "smoke_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "basic call".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "input": "test" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![],
            source: "smoke/smoke_basic.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
        }],
    }
}

#[test]
fn elixir_e2e_emits_formatter_exs_next_to_mix_exs() {
    let (e2e, resolved) = build_config();
    let files = ElixirCodegen
        .generate(&[smoke_group()], &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");

    let formatter = files
        .iter()
        .find(|f| f.path.ends_with(".formatter.exs"))
        .expect("Elixir e2e must emit a .formatter.exs");

    // ~keep Compared as a `Path`, not a string: `PathBuf::join` uses the native
    // separator, so `to_string_lossy()` yields `e2e\elixir\.formatter.exs` on Windows
    // and never matches a `/`-literal. `Path`'s `PartialEq` compares components, so
    // this holds on every platform.
    assert_eq!(
        formatter.path,
        Path::new("e2e/elixir/.formatter.exs"),
        "the .formatter.exs must sit at the mix project root so `mix format` reads it"
    );

    let body = &formatter.content;
    assert!(
        body.contains("inputs:"),
        "without an `inputs:` key a bare `mix format` refuses to run, got:\n{body}"
    );
    assert!(
        body.contains("{config,lib,test}/**/*.{ex,exs}"),
        "`inputs:` must cover the generated lib/ and test/ trees, got:\n{body}"
    );
    assert!(
        body.contains("line_length: 140"),
        "line_length must match the binding package's .formatter.exs so every \
         generated Elixir tree wraps identically, got:\n{body}"
    );
    assert!(
        !body.contains("import_deps"),
        "import_deps would make formatting fail whenever deps/ is unfetched, got:\n{body}"
    );
}
