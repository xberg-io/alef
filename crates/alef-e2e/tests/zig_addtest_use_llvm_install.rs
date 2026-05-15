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
//! Zig 0.16+ also stopped copying test binaries into `zig-out/bin/` even when
//! `addInstallArtifact` is invoked, so the earlier "install + run.dependOn
//! (install)" workaround that materialised an absolute spawn path is no longer
//! viable — every test now fails with `FileNotFound` against the would-be
//! `zig-out/bin/<name>` path. The current fix is to:
//!
//!   1. Pin every `addTest` to `.use_llvm = true` so the LLVM backend is used
//!      regardless of host arch (LLVM is already the default on x86_64).
//!   2. Run the binary directly via `addRunArtifact` against the cache path,
//!      WITHOUT `addInstallArtifact` and WITHOUT `setCwd`. The generated
//!      Zig tests reach the mock server purely through `MOCK_SERVER_*` env
//!      vars and never read anything cwd-relative, so leaving cwd alone keeps
//!      `convertPathArg` from re-resolving the cache-relative spawn path
//!      against a different directory.

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
fn every_test_artifact_runs_via_addrunartifact_directly() {
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

    // Zig 0.16+ no longer installs test binaries to `zig-out/bin/`, so the
    // emitted build.zig must NOT use `addInstallArtifact` for test artifacts.
    assert!(
        !content.contains("addInstallArtifact"),
        "build.zig must not call addInstallArtifact for test artifacts \
         (Zig 0.16+ does not copy test binaries to zig-out/bin/):\n{content}"
    );
    assert!(
        !content.contains("getInstallStep"),
        "build.zig must not register test artifacts under getInstallStep:\n{content}"
    );

    // Tests reach the mock server via MOCK_SERVER_* env vars and never read
    // workspace-relative files, so `setCwd` is unnecessary and re-introduces
    // the `convertPathArg` re-resolution that triggers FileNotFound on
    // Zig 0.16+ Linux backends.
    assert!(
        !content.contains("setCwd"),
        "build.zig must not call setCwd on test run steps:\n{content}"
    );

    // Each test must still be wired up: addTest -> addRunArtifact ->
    // test_step.dependOn(<run>.step).
    assert!(
        content.contains("const redirect_run = b.addRunArtifact(redirect_tests);"),
        "redirect run step must be created via addRunArtifact:\n{content}"
    );
    assert!(
        content.contains("test_step.dependOn(&redirect_run.step);"),
        "redirect run step must be wired into the test_step:\n{content}"
    );

    // Order: addTest -> addRunArtifact -> test_step.dependOn(run).
    let add_test_pos = content
        .find("const redirect_tests = b.addTest(")
        .expect("redirect addTest present");
    let run_pos = content
        .find("const redirect_run = b.addRunArtifact(redirect_tests);")
        .expect("redirect run present");
    let depend_pos = content
        .find("test_step.dependOn(&redirect_run.step);")
        .expect("redirect test_step dependency present");
    assert!(
        add_test_pos < run_pos && run_pos < depend_pos,
        "expected addTest -> addRunArtifact -> test_step.dependOn(run) order:\n{content}"
    );
}
