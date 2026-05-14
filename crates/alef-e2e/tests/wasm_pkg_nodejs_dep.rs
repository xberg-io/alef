//! Regression: WASM e2e `package.json` must point its local `file:` dependency at
//! `<wasm-crate>/pkg/nodejs`, not the parent `pkg/` directory.
//!
//! `wasm-pack build --target nodejs --out-dir pkg/nodejs` writes the actual
//! npm-consumable package (with its own `package.json` declaring `main`/`types`)
//! to `pkg/nodejs/`. The parent `pkg/` has no `package.json`, so pointing a pnpm
//! `file:` dependency at `pkg/` fails resolution and leaves the e2e suite unable
//! to import the binding.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::wasm::WasmCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

const CONFIG_TOML: &str = r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "kreuzcrawl"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "kreuzcrawl"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "url", field = "url", type = "string" },
]
"#;

fn group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "wasm_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "wasm smoke".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "url": "https://example.com" }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::Value::String("<html></html>".to_string())),
                stream_chunks: None,
                headers: std::collections::HashMap::new(),
            }),
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "smoke.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn wasm_package_json_dep_points_at_pkg_nodejs() {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");

    let files = WasmCodegen
        .generate(&[group()], &e2e, &resolved, &[])
        .expect("generation succeeds");

    let package_json = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "package.json"))
        .expect("package.json generated")
        .content
        .clone();

    // Local-mode file: dep must end in `/pkg/nodejs`, never bare `/pkg`.
    assert!(
        package_json.contains("/pkg/nodejs\""),
        "package.json local dep must end in /pkg/nodejs (wasm-pack --target nodejs --out-dir pkg/nodejs):\n{package_json}"
    );
    assert!(
        !package_json.contains("/pkg\""),
        "package.json must not point at bare /pkg/ — wasm-pack writes the consumable package to pkg/nodejs/:\n{package_json}"
    );
}
