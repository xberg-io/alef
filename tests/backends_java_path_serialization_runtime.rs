use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef,
};
use regex::Regex;
use std::fs;
use std::path::Path;
use std::process::Command;

fn java_config(crate_name: &str, package: &str, extra: &str) -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "{package}"

{extra}
"#,
    ))
    .expect("valid Java config");
    config.resolve().expect("resolved Java config").remove(0)
}

fn path_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Path,
        ..Default::default()
    }
}

fn record(crate_name: &str, name: &str) -> TypeDef {
    TypeDef {
        name: name.into(),
        rust_path: format!("{crate_name}::{name}"),
        fields: vec![path_field("path")],
        has_serde: true,
        ..Default::default()
    }
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture directory");
    fs::write(path, contents).expect("write fixture file");
}

fn write_generated_package(root: &Path, package: &str, api: &ApiSurface, config: &ResolvedCrateConfig) {
    let package_dir = package.replace('.', "/");
    for file in JavaBackend
        .generate_bindings(api, config)
        .expect("generate Java package")
    {
        if file.path.extension().and_then(|extension| extension.to_str()) != Some("java") {
            continue;
        }
        let file_name = file
            .path
            .file_name()
            .expect("generated Java filename")
            .to_string_lossy();
        write_file(root, &format!("src/main/java/{package_dir}/{file_name}"), &file.content);
    }
}

fn normal_api() -> ApiSurface {
    ApiSurface {
        crate_name: "normal_lib".into(),
        version: "0.1.0".into(),
        types: vec![record("normal_lib", "Config")],
        functions: vec![FunctionDef {
            name: "create".into(),
            rust_path: "normal_lib::create".into(),
            params: vec![ParamDef {
                name: "config".into(),
                ty: TypeRef::Named("Config".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn streaming_api() -> ApiSurface {
    ApiSurface {
        crate_name: "stream_lib".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "EventSource".into(),
                rust_path: "stream_lib::EventSource".into(),
                is_opaque: true,
                ..Default::default()
            },
            record("stream_lib", "EventRequest"),
            record("stream_lib", "Event"),
        ],
        ..Default::default()
    }
}

fn callback_api() -> ApiSurface {
    ApiSurface {
        crate_name: "callback_lib".into(),
        version: "0.1.0".into(),
        types: vec![
            record("callback_lib", "Config"),
            TypeDef {
                name: "Renderer".into(),
                rust_path: "callback_lib::Renderer".into(),
                methods: vec![MethodDef {
                    name: "render".into(),
                    receiver: Some(ReceiverKind::Ref),
                    return_type: TypeRef::Named("Config".into()),
                    error_type: Some("RenderError".into()),
                    ..Default::default()
                }],
                is_opaque: true,
                is_trait: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn newtype_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "0".into(),
        ty,
        ..Default::default()
    }
}

fn union_api() -> ApiSurface {
    ApiSurface {
        crate_name: "union_lib".into(),
        version: "0.1.0".into(),
        enums: vec![
            EnumDef {
                name: "Choice".into(),
                rust_path: "union_lib::Choice".into(),
                has_serde: true,
                serde_tag: Some("type".into()),
                variants: vec![EnumVariant {
                    name: "Local".into(),
                    fields: vec![path_field("path")],
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumDef {
                name: "AssistantContent".into(),
                rust_path: "union_lib::AssistantContent".into(),
                has_serde: true,
                serde_untagged: true,
                variants: vec![
                    EnumVariant {
                        name: "Text".into(),
                        fields: vec![newtype_field(TypeRef::String)],
                        is_tuple: true,
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Items".into(),
                        fields: vec![newtype_field(TypeRef::Vec(Box::new(TypeRef::String)))],
                        is_tuple: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn generated_user_value_mappers_serialize_native_paths() {
    let directory = tempfile::tempdir().expect("temporary Maven project");
    write_generated_package(
        directory.path(),
        "com.test.normal",
        &normal_api(),
        &java_config("normal_lib", "com.test.normal", ""),
    );
    write_generated_package(
        directory.path(),
        "com.test.stream",
        &streaming_api(),
        &java_config(
            "stream_lib",
            "com.test.stream",
            r#"
[[crates.adapters]]
name = "events"
pattern = "streaming"
core_path = "events"
owner_type = "EventSource"
item_type = "Event"
error_type = "StreamError"
request_type = "stream_lib::EventRequest"

[[crates.adapters.params]]
name = "request"
type = "EventRequest"
"#,
        ),
    );
    write_generated_package(
        directory.path(),
        "com.test.callback",
        &callback_api(),
        &java_config(
            "callback_lib",
            "com.test.callback",
            r#"
[[crates.trait_bridges]]
trait_name = "Renderer"
register_fn = "register_renderer"
"#,
        ),
    );
    write_generated_package(
        directory.path(),
        "com.test.union",
        &union_api(),
        &java_config("union_lib", "com.test.union", ""),
    );
    write_file(directory.path(), "pom.xml", POM);
    write_file(
        directory.path(),
        "src/test/java/com/test/probe/PathSerializationTest.java",
        JAVA_TEST,
    );

    let output = Command::new("mvn")
        .args(["-q", "test"])
        .current_dir(directory.path())
        .output()
        .expect("run generated Java path serialization regression");
    assert!(
        output.status.success(),
        "generated Java packages must compile and pass their path probes:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report = fs::read_to_string(
        directory
            .path()
            .join("target/surefire-reports/TEST-com.test.probe.PathSerializationTest.xml"),
    )
    .expect("Surefire XML report");
    let suite = Regex::new(r#"<testsuite\b[^>]*\btests="6"[^>]*>"#)
        .expect("testsuite regex")
        .find(&report)
        .expect("exactly six Java tests executed")
        .as_str();
    for attribute in [r#"errors="0""#, r#"failures="0""#, r#"skipped="0""#] {
        assert!(
            suite.contains(attribute),
            "Surefire suite must contain {attribute}: {suite}"
        );
    }
}

const POM: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.test</groupId>
  <artifactId>path-serialization-test</artifactId>
  <version>1.0.0</version>
  <properties>
    <maven.compiler.release>25</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>
  <dependencies>
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId>
      <artifactId>jackson-databind</artifactId>
      <version>2.22.3</version>
    </dependency>
    <dependency>
      <groupId>com.fasterxml.jackson.datatype</groupId>
      <artifactId>jackson-datatype-jdk8</artifactId>
      <version>2.22.3</version>
    </dependency>
    <dependency>
      <groupId>org.jspecify</groupId>
      <artifactId>jspecify</artifactId>
      <version>1.0.1</version>
    </dependency>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter</artifactId>
      <version>6.1.3</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-compiler-plugin</artifactId>
        <version>3.16.0</version>
        <configuration>
          <release>25</release>
          <compilerArgs><arg>--enable-preview</arg></compilerArgs>
        </configuration>
      </plugin>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.6.0</version>
        <configuration>
          <failIfNoTests>true</failIfNoTests>
          <argLine>--enable-preview --enable-native-access=ALL-UNNAMED</argLine>
        </configuration>
      </plugin>
    </plugins>
  </build>
</project>
"#;

const JAVA_TEST: &str = r#"package com.test.probe;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.datatype.jdk8.Jdk8Module;
import java.lang.reflect.Field;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

final class PathSerializationTest {
  private static final Path PATH = Path.of("fixtures", "a path");

  @Test
  void normalFfiMapperUsesNativePath() throws Exception {
    assertNativePath(mapper("com.test.normal.NormalLibRs", "MAPPER"));
  }

  @Test
  void streamingMapperUsesNativePath() throws Exception {
    assertNativePath(mapper("com.test.stream.EventSource", "STREAM_MAPPER"));
  }

  @Test
  void callbackMapperUsesNativePath() throws Exception {
    assertNativePath(mapper("com.test.callback.RendererBridge", "JSON"));
  }

  @Test
  void sealedUnionSerializerUsesNativePath() throws Exception {
    var choice = new com.test.union.Choice.Local(PATH);
    var json = new ObjectMapper().writeValueAsString(choice);
    assertFalse(json.contains("file:"), json);
    assertEquals(PATH.toString(), new ObjectMapper().readTree(json).path("path").textValue());
  }

  @Test
  void untaggedUnionMapperUsesNativePath() {
    var content = com.test.union.AssistantContent.ofObject(PATH);
    assertEquals(PATH.toString(), content.value().textValue());
  }

  @Test
  void negativeControlProvesDefaultJacksonWritesFileUri() throws Exception {
    var mapper = new ObjectMapper().registerModule(new Jdk8Module()).findAndRegisterModules();
    assertEquals("\"" + PATH.toAbsolutePath().toUri() + "\"", mapper.writeValueAsString(PATH));
  }

  private static ObjectMapper mapper(String className, String fieldName) throws Exception {
    Field field = Class.forName(className).getDeclaredField(fieldName);
    field.setAccessible(true);
    return (ObjectMapper) field.get(null);
  }

  private static void assertNativePath(ObjectMapper mapper) throws Exception {
    assertEquals("\"" + PATH + "\"", mapper.writeValueAsString(PATH));
  }
}
"#;
