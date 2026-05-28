//! Verifies that Java e2e test_app codegen emits mvnw wrapper scripts
//! (mvnw, mvnw.cmd, .mvn/wrapper/maven-wrapper.properties).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

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
        mock_response: Some(MockResponse {
            status_code: 200,
            headers: BTreeMap::new(),
            body: "{}".to_string(),
        }),
        http: None,
        assertions: Vec::new(),
        visitor: None,
    }
}

#[test]
fn test_java_mvnw_wrapper_files_emitted() {
    let config = NewAlefConfig::fixture_config();
    let e2e_config = config.e2e.clone().unwrap();
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture("test_basic")],
    }];

    let java_gen = JavaCodegen;
    let generated = java_gen
        .generate(&groups, &e2e_config, &config, &[], &[])
        .expect("generate failed");

    // Check that wrapper files are present
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
        paths.iter().any(|p| p.ends_with(".mvn/wrapper/maven-wrapper.properties")),
        "maven-wrapper.properties not found in generated files"
    );

    // Verify content contains expected text
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
