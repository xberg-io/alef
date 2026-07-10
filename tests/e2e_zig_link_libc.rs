//! Regression: Zig e2e `build.zig` must link libc on every module that imports
//! the binding (which references C stdlib symbols via the FFI header) and on the
//! shared binding module itself. Zig 0.16 surfaces missing libc as:
//!
//! ```text
//! error: dependency on libc must be explicitly specified in the build command
//! pub extern "c" fn getenv(name: [*:0]const u8) ?[*:0]u8;
//! ```
//!
//! Each `b.createModule(.{...})` block in the generated `build.zig` must include
//! `.link_libc = true,`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

const CONFIG_TOML: &str = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo_crawler"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "demo_crawler"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "url", field = "url", type = "string" },
]
"#;

fn group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "scrape_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "basic scrape".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "url": "https://example.com" }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::Value::String("<html></html>".to_string())),
                stream_chunks: None,
                headers: std::collections::BTreeMap::new(),
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
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

fn render_build_zig() -> String {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let files = ZigE2eCodegen
        .generate(&[group()], &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "build.zig"))
        .expect("build.zig generated")
        .content
        .clone()
}

#[test]
fn build_zig_links_libc_on_every_test_module() {
    let content = render_build_zig();

    let create_module_count = content.matches("b.createModule(.{").count();
    let link_libc_count = content.matches(".link_libc = true,").count();

    assert!(
        create_module_count >= 1,
        "expected at least one createModule block in:\n{content}"
    );
    assert!(
        link_libc_count > create_module_count,
        "expected `.link_libc = true,` on each createModule block plus the shared addModule, \
         found {link_libc_count} link_libc lines vs {create_module_count} createModule blocks:\n{content}"
    );
}

#[test]
fn build_zig_shared_binding_module_links_libc() {
    let content = render_build_zig();
    let add_module_idx = content
        .find("b.addModule(")
        .expect("shared binding addModule call missing");
    let block_end = content[add_module_idx..]
        .find("    });")
        .expect("addModule block close missing");
    let block = &content[add_module_idx..add_module_idx + block_end];
    assert!(
        block.contains(".link_libc = true,"),
        "shared binding addModule block must enable libc:\n{block}"
    );
}
