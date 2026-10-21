use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use std::fs;

fn generate_records(root: &std::path::Path) {
    let path = root.join("lib.rs");
    fs::write(&path, RUST_SOURCE).unwrap();
    let api = alef::extract::extractor::extract(&[&path], "sample", "1.0.0", None).unwrap();
    let config: NewAlefConfig = toml::from_str(
        r#"[workspace]
languages = ["java", "ffi"]
[[crates]]
name = "sample"
sources = ["lib.rs"]
[crates.java]
package = "com.test"
[crates.java.dto]
builder = "always"
"#,
    )
    .unwrap();
    let source = root.join("src/main/java/com/test");
    fs::create_dir_all(&source).unwrap();
    let mut count = 0;
    for file in JavaBackend
        .generate_bindings(&api, &config.resolve().unwrap()[0])
        .unwrap()
    {
        let Some(name) = file.path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if ["Response.java", "BlockDetails.java", "ExplicitDefaults.java"].contains(&name) {
            fs::write(source.join(name), file.content).unwrap();
            count += 1;
        }
    }
    assert_eq!(count, 3, "all fixture DTOs must be compiled");
}

#[test]
fn generated_optional_constructor_defaults_keep_none_distinct_from_false_and_zero() {
    let directory = tempfile::tempdir().unwrap();
    generate_records(directory.path());
    let tests = directory.path().join("src/test/java/com/test");
    fs::create_dir_all(&tests).unwrap();
    fs::write(tests.join("OptionalConstructorTest.java"), JAVA_TEST).unwrap();
    fs::write(
        directory.path().join("pom.xml"),
        include_str!("fixtures/java_nullable_builder_pom.xml"),
    )
    .unwrap();
    let output = alef::core::tool_command("mvn")
        .args(["-q", "test"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read_to_string(
        directory
            .path()
            .join("target/surefire-reports/TEST-com.test.OptionalConstructorTest.xml"),
    )
    .unwrap();
    for attribute in ["tests=\"3\"", "errors=\"0\"", "failures=\"0\"", "skipped=\"0\""] {
        assert!(report.contains(attribute), "missing {attribute}: {report}");
    }
}

const RUST_SOURCE: &str = r#"
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BlockDetails { pub vendor: String }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["url"])]
pub struct Response {
    pub url: String,
    #[serde(default)] pub block_details: Option<BlockDetails>,
    #[serde(default)] pub retry_enabled: Option<bool>,
    #[serde(default)] pub limit: Option<usize>,
    #[serde(default)] pub ratio: Option<f64>,
    #[serde(default)] pub label: Option<String>,
    #[serde(default)] pub items: Option<Vec<String>>,
    pub required_enabled: bool,
    pub required_count: usize,
    pub required_ratio: f64,
    pub required_label: String,
    pub required_items: Vec<String>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["url"])]
pub struct ExplicitDefaults { pub url: String, pub enabled: Option<bool>, pub count: Option<usize> }
impl Default for ExplicitDefaults {
    fn default() -> Self { Self { url: String::new(), enabled: Some(false), count: Some(0) } }
}
"#;

const JAVA_TEST: &str = r#"package com.test;
import static org.junit.jupiter.api.Assertions.*;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;
final class OptionalConstructorTest {
    @Test void legacyConstructorKeepsRustNone() {
        var response = new Response("https://example.com");
        assertNull(response.blockDetails());
        assertNull(response.retryEnabled());
        assertNull(response.limit());
        assertNull(response.ratio());
        assertNull(response.label());
        assertNull(response.items());
        assertFalse(response.requiredEnabled());
        assertEquals(0L, response.requiredCount());
        assertEquals(0.0, response.requiredRatio());
        assertEquals("", response.requiredLabel());
        assertEquals(java.util.List.of(), response.requiredItems());
    }
    @Test void jacksonDistinguishesMissingNullFalseAndZero() throws Exception {
        var mapper = new ObjectMapper();
        var missing = mapper.readValue("{}", Response.class);
        assertNull(missing.retryEnabled());
        assertNull(missing.limit());
        assertNull(missing.ratio());
        assertNull(missing.label());
        assertNull(missing.items());
        var empty = mapper.readValue(
            "{\"retry_enabled\":null,\"limit\":null,\"ratio\":null,"
                + "\"label\":null,\"items\":null}", Response.class);
        assertNull(empty.retryEnabled());
        assertNull(empty.limit());
        assertNull(empty.ratio());
        assertNull(empty.label());
        assertNull(empty.items());
        var supplied = mapper.readValue(
            "{\"retry_enabled\":false,\"limit\":0,\"ratio\":0.0,"
                + "\"label\":\"\",\"items\":[]}", Response.class);
        assertEquals(Boolean.FALSE, supplied.retryEnabled());
        assertEquals(0L, supplied.limit());
        assertEquals(0.0, supplied.ratio());
        assertEquals("", supplied.label());
        assertEquals(java.util.List.of(), supplied.items());
    }
    @Test void explicitSomeFalseAndZeroSurviveDelegation() {
        var response = new ExplicitDefaults("https://example.com");
        assertEquals(Boolean.FALSE, response.enabled());
        assertEquals(0L, response.count());
    }
}
"#;
