//! Regression: Zig 0.16's `b.addTest(.{...})` defaults `use_llvm` to `null`,
//! which lets the compiler pick the self-hosted backend on aarch64-linux for
//! Debug builds. The self-hosted backend emits the test binary at a different
//! cache path than the `addRunArtifact` step computes via `getEmittedBin()`,
//! so every run fails with:
//!
//! ```text
//! error: failed to spawn and capture stdio from
//!     ./../e2e/zig/.zig-cache/o/<hash>/<test>_test: FileNotFound
//! ```
//!
//! even though the compile step reports success. The previous fix that pinned
//! a unique `.name = "<test>_test"` per test (see `zig_addtest_name.rs`) was
//! necessary to avoid cache collisions but not sufficient on aarch64-linux.
//!
//! The emitted `build.zig` must additionally:
//!   1. Pin every `addTest` to `.use_llvm = true` so the LLVM backend is used
//!      regardless of host arch (LLVM is already the default on x86_64).
//!   2. Call `b.installArtifact(<test>)` for every test artifact so the build
//!      system has a hard install dependency forcing the binary to materialise
//!      on disk before the run step executes.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::zig::ZigE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

const CONFIG_TOML: &str = r#"
[workspace]
languages = ["zig"]

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
async = false
returns_result = true
args = [
  { name = "url", field = "url", type = "string" },
]
"#;

fn fixture_for(category: &str, id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some(category.to_string()),
        description: format!("{category} fixture {id}"),
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
        source: format!("{category}.json"),
        http: None,
    }
}

fn render_build_zig(groups: Vec<FixtureGroup>) -> String {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "build.zig"))
        .expect("build.zig generated")
        .content
        .clone()
}

#[test]
fn every_add_test_block_sets_use_llvm_true() {
    let groups = vec![
        FixtureGroup {
            category: "redirect".to_string(),
            fixtures: vec![fixture_for("redirect", "follows_301")],
        },
        FixtureGroup {
            category: "cookies".to_string(),
            fixtures: vec![fixture_for("cookies", "per_domain")],
        },
        FixtureGroup {
            category: "robots".to_string(),
            fixtures: vec![fixture_for("robots", "disallow_path")],
        },
    ];
    let content = render_build_zig(groups);

    let add_test_count = content.matches("b.addTest(.{").count();
    let use_llvm_count = content.matches(".use_llvm = true,").count();
    assert!(
        add_test_count >= 3,
        "expected at least three addTest blocks for three fixture groups:\n{content}"
    );
    assert_eq!(
        use_llvm_count, add_test_count,
        "every addTest block must set .use_llvm = true; \
         found {use_llvm_count} .use_llvm entries for {add_test_count} addTest blocks:\n{content}"
    );
}

#[test]
fn every_test_artifact_is_installed() {
    let groups = vec![
        FixtureGroup {
            category: "redirect".to_string(),
            fixtures: vec![fixture_for("redirect", "follows_301")],
        },
        FixtureGroup {
            category: "cookies".to_string(),
            fixtures: vec![fixture_for("cookies", "per_domain")],
        },
    ];
    let content = render_build_zig(groups);

    // Each test artifact must be installed by name so the build system
    // materialises the binary on disk before the run step executes.
    assert!(
        content.contains("b.installArtifact(redirect_tests);"),
        "redirect test artifact must be installed:\n{content}"
    );
    assert!(
        content.contains("b.installArtifact(cookies_tests);"),
        "cookies test artifact must be installed:\n{content}"
    );

    // installArtifact must be wired between addTest and addRunArtifact so the
    // install dependency is registered before the run step references the
    // emitted binary.
    let install_pos = content
        .find("b.installArtifact(redirect_tests);")
        .expect("redirect install present");
    let run_pos = content
        .find("b.addRunArtifact(redirect_tests)")
        .expect("redirect run present");
    let add_test_pos = content
        .find("const redirect_tests = b.addTest(")
        .expect("redirect addTest present");
    assert!(
        add_test_pos < install_pos && install_pos < run_pos,
        "expected addTest -> installArtifact -> addRunArtifact order in emitted build.zig:\n{content}"
    );
}
