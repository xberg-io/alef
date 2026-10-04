use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use std::fs;
use std::path::Path;
use std::process::Command;

fn java_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.test"
"#,
    )
    .expect("valid Java config");
    config.resolve().expect("resolved Java config").remove(0)
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture directory");
    fs::write(path, contents).expect("write fixture file");
}

fn assert_mapper_configuration_precedes_path_override(template: &str, configuration: &str) {
    let configuration_index = template.find(configuration).expect("mapper configuration");
    let override_index = template
        .find("return JsonMapperFactory.withNativePathSerialization(mapper);")
        .expect("native Path override");
    assert!(
        configuration_index < override_index,
        "native Path serialization must be registered after `{configuration}`"
    );
}

#[test]
fn every_user_value_mapper_serializes_paths_after_its_other_modules() {
    let api = ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    let generated = JavaBackend
        .generate_bindings(&api, &java_config())
        .expect("generate Java bindings");
    let factory = generated
        .iter()
        .find(|file| file.path.ends_with("JsonMapperFactory.java"))
        .expect("generated JsonMapperFactory.java");

    let normal_template = include_str!("../src/backends/java/templates/helper_object_mapper.jinja");
    let streaming_template = include_str!("../src/backends/java/templates/streaming_helpers.jinja");
    let callback_template = include_str!("../src/backends/java/templates/trait_bridge.jinja");
    let sealed_template = include_str!("../src/backends/java/templates/sealed_union_serializer.jinja");
    let untagged_template = include_str!("../src/backends/java/templates/untagged_union_wrapper.jinja");
    let templates = [
        normal_template,
        streaming_template,
        callback_template,
        sealed_template,
        untagged_template,
    ];
    for template in templates {
        assert!(
            template.contains("JsonMapperFactory.withNativePathSerialization"),
            "every mapper that writes user values must use the shared Path serializer"
        );
    }
    assert_mapper_configuration_precedes_path_override(normal_template, ".findAndRegisterModules()");
    assert_mapper_configuration_precedes_path_override(normal_template, "mapper.registerModule(module);");
    assert_mapper_configuration_precedes_path_override(streaming_template, ".findAndRegisterModules()");
    assert_mapper_configuration_precedes_path_override(
        sealed_template,
        ".registerModule(new com.fasterxml.jackson.datatype.jdk8.Jdk8Module())",
    );

    let directory = tempfile::tempdir().expect("temporary Maven project");
    write_file(
        directory.path(),
        "src/main/java/com/test/JsonMapperFactory.java",
        &factory.content,
    );
    write_file(directory.path(), "pom.xml", POM);
    write_file(
        directory.path(),
        "src/test/java/com/test/PathSerializationTest.java",
        JAVA_TEST,
    );

    let output = Command::new("mvn")
        .args(["-q", "test"])
        .current_dir(directory.path())
        .output()
        .expect("run Maven path serialization regression");
    assert!(
        output.status.success(),
        "generated mapper factory must serialize Path.toString() in every mapper context:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const POM: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.test</groupId>
  <artifactId>path-serialization-test</artifactId>
  <version>1.0.0</version>
  <properties>
    <maven.compiler.release>17</maven.compiler.release>
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
          <release>17</release>
        </configuration>
      </plugin>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.6.0</version>
      </plugin>
    </plugins>
  </build>
</project>
"#;

const JAVA_TEST: &str = r#"package com.test;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.databind.MapperFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;
import com.fasterxml.jackson.datatype.jdk8.Jdk8Module;
import java.nio.file.Path;
import java.util.stream.Stream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.MethodSource;

final class PathSerializationTest {
  private static final Path PATH = Path.of("fixtures", "a path");

  @ParameterizedTest
  @MethodSource("userValueMappers")
  void serializesNativePathAfterContextModules(ObjectMapper mapper) throws Exception {
    assertEquals("\"" + PATH + "\"", mapper.writeValueAsString(PATH));
  }

  static Stream<ObjectMapper> userValueMappers() {
    return Stream.of(normalMapper(), streamingMapper(), callbackMapper(), sealedMapper(), untaggedMapper());
  }

  @Test
  void negativeControlProvesJacksonWouldOtherwiseWriteAFileUri() throws Exception {
    var mapper = new ObjectMapper().registerModule(new Jdk8Module()).findAndRegisterModules();
    assertEquals("\"" + PATH.toAbsolutePath().toUri() + "\"", mapper.writeValueAsString(PATH));
  }

  private static ObjectMapper normalMapper() {
    var mapper = new ObjectMapper()
        .registerModule(new Jdk8Module())
        .findAndRegisterModules()
        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
        .setSerializationInclusion(JsonInclude.Include.ALWAYS)
        .configure(MapperFeature.ACCEPT_CASE_INSENSITIVE_ENUMS, true);
    mapper.registerModule(new com.fasterxml.jackson.databind.module.SimpleModule());
    return JsonMapperFactory.withNativePathSerialization(mapper);
  }

  private static ObjectMapper streamingMapper() {
    var mapper = new ObjectMapper()
        .registerModule(new Jdk8Module())
        .findAndRegisterModules()
        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
        .setSerializationInclusion(JsonInclude.Include.NON_NULL)
        .configure(MapperFeature.ACCEPT_CASE_INSENSITIVE_ENUMS, true);
    return JsonMapperFactory.withNativePathSerialization(mapper);
  }

  private static ObjectMapper callbackMapper() {
    return JsonMapperFactory.withNativePathSerialization(new ObjectMapper());
  }

  private static ObjectMapper sealedMapper() {
    var mapper = new ObjectMapper()
        .registerModule(new Jdk8Module())
        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
        .setSerializationInclusion(JsonInclude.Include.NON_NULL);
    return JsonMapperFactory.withNativePathSerialization(mapper);
  }

  private static ObjectMapper untaggedMapper() {
    return JsonMapperFactory.withNativePathSerialization(new ObjectMapper());
  }
}
"#;
