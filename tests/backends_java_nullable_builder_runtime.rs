use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::fs;

fn generated_nullable_record() -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[crates.java]
package = "com.test"
[crates.java.dto]
builder = "always"
"#,
    )
    .unwrap();
    let api = ApiSurface {
        crate_name: "sample".into(),
        types: vec![TypeDef {
            name: "SampleDetails".into(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "status".into(),
                ty: TypeRef::Primitive(PrimitiveType::U16),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = JavaBackend
        .generate_bindings(&api, &config.resolve().unwrap()[0])
        .unwrap();
    let dto = files
        .iter()
        .find(|file| file.path.ends_with("SampleDetails.java"))
        .unwrap();
    dto.content.clone()
}

#[test]
fn generated_nullable_builder_preserves_json_null_and_primitive_overloads() {
    let dto = generated_nullable_record();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("src/main/java/com/test");
    let tests = directory.path().join("src/test/java/com/test");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&tests).unwrap();
    fs::write(source.join("SampleDetails.java"), dto).unwrap();
    fs::write(tests.join("NullableBuilderTest.java"), JAVA_TEST).unwrap();
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
            .join("target/surefire-reports/TEST-com.test.NullableBuilderTest.xml"),
    )
    .unwrap();
    for attribute in ["tests=\"3\"", "errors=\"0\"", "failures=\"0\"", "skipped=\"0\""] {
        assert!(report.contains(attribute), "missing {attribute} in {report}");
    }
}

const JAVA_TEST: &str = r#"package com.test;
import static org.junit.jupiter.api.Assertions.*;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

final class NullableBuilderTest {
    @Test void explicitNullRemainsNull() throws Exception {
        var mapper = new ObjectMapper();
        var value = mapper.readValue("{\"status\":null}", SampleDetails.class);
        assertNull(value.status());
        assertNull(mapper.readValue(mapper.writeValueAsString(value), SampleDetails.class).status());
    }
    @Test void omittedValueRemainsNullAndNumberSurvives() throws Exception {
        var mapper = new ObjectMapper();
        assertNull(mapper.readValue("{}", SampleDetails.class).status());
        assertEquals((short)403, mapper.readValue("{\"status\":403}", SampleDetails.class).status());
    }
    @Test void releasedPrimitiveSetterRemainsCallable() throws Exception {
        var method = SampleDetails.Builder.class.getMethod("withStatus", short.class);
        var builder = new SampleDetails.Builder();
        method.invoke(builder, (short)403);
        assertEquals((short)403, builder.build().status());
        assertNull(new SampleDetails.Builder().withStatus(null).build().status());
    }
}
"#;
