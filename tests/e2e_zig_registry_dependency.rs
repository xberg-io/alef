//! Regression: in registry mode the generated Zig `build.zig` must consume the
//! published package via the dependency declared in `build.zig.zon`
//! (`b.dependency(pkg).module(module)`) rather than referencing the in-tree
//! `../../packages/zig/src/<module>.zig` source and `../../target/release`
//! library path. The published package's own `build.zig` wires the bundled
//! FFI library (lib/) + header (include/), so a registry consumer links the
//! prebuilt native library shipped in the release tarball.
//!
//! Without this, a standalone `test_apps/zig` build fails at link time with
//! `unable to find dynamic system library '<lib>_ffi'` because the in-tree
//! paths do not exist outside the monorepo.

use alef::core::config::NewAlefConfig;
use alef::core::config::e2e::DependencyMode;
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

[crates.e2e.registry]
github_repo = "https://github.com/example/demo_crawler"

[crates.e2e.registry.packages.zig]
name = "demo_crawler"
version = "1.2.3"
hash = "demo_crawler-1.2.3-AAAAfakehashfortestonly000000000000000000000"
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

fn render_registry_build_zig() -> String {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("resolves").remove(0);
    let mut e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    // `dep_mode` is `#[serde(skip)]` — set at runtime by the `--registry` flag.
    e2e.dep_mode = DependencyMode::Registry;
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
fn registry_build_zig_consumes_published_dependency() {
    let content = render_registry_build_zig();

    assert!(
        content.contains("b.dependency(\"demo_crawler\", .{"),
        "registry build.zig must consume the published package via b.dependency(\"demo_crawler\", ...):\n{content}"
    );
    assert!(
        content.contains(".module(\"demo_crawler\")"),
        "registry build.zig must import the dependency's exported module:\n{content}"
    );
}

#[test]
fn registry_build_zig_does_not_reference_in_tree_paths() {
    let content = render_registry_build_zig();

    assert!(
        !content.contains("../../packages/zig/src/"),
        "registry build.zig must not reference the in-tree binding source:\n{content}"
    );
    assert!(
        !content.contains(".cwd_relative = ffi_path"),
        "registry build.zig must not link against the in-tree ../../target/release path:\n{content}"
    );
}

#[test]
fn placeholder_hash_is_stripped() {
    let cfg_with_placeholder = r#"
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

[crates.e2e.registry]
github_repo = "https://github.com/example/demo_crawler"

[crates.e2e.registry.packages.zig]
name = "demo_crawler"
version = "1.2.3"
hash = "demo_crawler-1.2.3-STALE_HASH_REGENERATE"
"#;

    let cfg: NewAlefConfig = toml::from_str(cfg_with_placeholder).expect("config with placeholder parses");
    let resolved = cfg.clone().resolve().expect("resolves").remove(0);
    let mut e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    e2e.dep_mode = DependencyMode::Registry;

    let result = ZigE2eCodegen.generate(&[group()], &e2e, &resolved, &[], &[]);

    assert!(
        result.is_ok(),
        "zig e2e codegen must not bail on STALE_HASH_REGENERATE placeholder: {:?}",
        result.err()
    );

    let files = result.expect("generation succeeds");
    let zon_file = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "build.zig.zon"))
        .expect("build.zig.zon generated");

    assert!(
        !zon_file.content.contains("STALE_HASH_REGENERATE"),
        "generated build.zig.zon must not contain STALE_HASH_REGENERATE placeholder:\n{}",
        zon_file.content
    );
}
