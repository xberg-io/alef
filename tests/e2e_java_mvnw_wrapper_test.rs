//! Verifies that Java e2e test_app codegen emits mvnw wrapper scripts
//! (mvnw, mvnw.cmd, .mvn/wrapper/maven-wrapper.properties).

use std::path::Path;

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

[crates.e2e.call]
function = "noop"
result_var = "result"
"#;

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
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
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
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
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generate failed");

    assert!(
        generated.iter().any(|file| file.path.ends_with("mvnw")),
        "mvnw not found in generated files"
    );
    assert!(
        generated.iter().any(|file| file.path.ends_with("mvnw.cmd")),
        "mvnw.cmd not found in generated files"
    );
    let maven_wrapper_properties = Path::new(".mvn").join("wrapper").join("maven-wrapper.properties");
    assert!(
        generated
            .iter()
            .any(|file| file.path.ends_with(&maven_wrapper_properties)),
        "maven-wrapper.properties not found in generated files"
    );

    for file in &generated {
        if file.path.ends_with("mvnw") {
            assert!(
                file.content.contains("Apache Maven Wrapper"),
                "mvnw should contain Apache Maven Wrapper text"
            );
            assert!(
                file.content.contains("distributionUrl"),
                "mvnw should contain distributionUrl reference"
            );
        }
        if file.path.ends_with("mvnw.cmd") {
            assert!(
                file.content.contains("Apache Maven Wrapper"),
                "mvnw.cmd should contain Apache Maven Wrapper text"
            );
        }
        if file.path.ends_with("maven-wrapper.properties") {
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
