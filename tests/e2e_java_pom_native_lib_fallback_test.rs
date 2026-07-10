//! Verifies that Java e2e test_app pom.xml uses fallback logic for native library resolution:
//! tries ffi/lib/ (pre-built distribution) first, then falls back to workspace/target/release/.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

const TOML: &str = r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate"
ffi_style = "panama"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
java_group_id = "dev.sample_crate"

[crates.e2e.call]
function = "noop"
result_var = "result"
"#;

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({"request": {}}),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: Vec::new(),
        source: "smoke.json".to_string(),
        http: None,
    }
}

#[test]
fn test_java_pom_native_lib_fallback_logic() {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture("test_basic")],
    }];

    let generated = JavaCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generate failed");

    let pom_file = generated
        .iter()
        .find(|f| f.path.to_str().unwrap().ends_with("pom.xml"))
        .expect("pom.xml should be generated");

    let pom_content = &pom_file.content;

    assert!(
        pom_content.contains("ffi/lib/"),
        "pom.xml should reference ffi/lib/ for pre-built FFI distribution"
    );

    assert!(
        pom_content.contains("target/release"),
        "pom.xml should reference target/release as fallback for local builds"
    );

    assert!(
        pom_content.contains("ffi.lib.path"),
        "pom.xml should use ffi.lib.path property to detect pre-built FFI"
    );

    assert!(
        pom_content.contains("native.source.dir"),
        "pom.xml should use native.source.dir property to conditionally select the lib directory"
    );

    assert!(
        pom_content.contains("failonerror=\"false\""),
        "pom.xml copy task should use failonerror=\"false\" to allow graceful handling of missing FFI libraries"
    );
}
