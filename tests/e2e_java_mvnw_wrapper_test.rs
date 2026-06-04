//! Verifies that Java e2e test_app codegen emits mvnw wrapper scripts
//! (mvnw, mvnw.cmd, .mvn/wrapper/maven-wrapper.properties).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

// TODO(alef-generic-cleanup): replace sample_crate/dev.sample_crate Java fixture names with neutral ones.
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
fn test_java_mvnw_wrapper_files_emitted() {
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

    let paths: Vec<&str> = generated.iter().map(|f| f.path.to_str().unwrap()).collect();

    assert!(
        paths.iter().any(|p| p.ends_with("mvnw")),
        "mvnw not found in generated files"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("mvnw.cmd")),
        "mvnw.cmd not found in generated files"
    );
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with(".mvn/wrapper/maven-wrapper.properties")),
        "maven-wrapper.properties not found in generated files"
    );

    for file in &generated {
        let path_str = file.path.to_str().unwrap();
        if path_str.ends_with("mvnw") {
            assert!(
                file.content.contains("Apache Maven Wrapper"),
                "mvnw should contain Apache Maven Wrapper text"
            );
            assert!(
                file.content.contains("distributionUrl"),
                "mvnw should contain distributionUrl reference"
            );
        }
        if path_str.ends_with("mvnw.cmd") {
            assert!(
                file.content.contains("Apache Maven Wrapper"),
                "mvnw.cmd should contain Apache Maven Wrapper text"
            );
        }
        if path_str.ends_with("maven-wrapper.properties") {
            assert!(
                file.content.contains("wrapperVersion=3.3.4"),
                "maven-wrapper.properties should contain wrapperVersion"
            );
            assert!(
                file.content.contains("distributionUrl"),
                "maven-wrapper.properties should contain distributionUrl"
            );
        }
    }
}
