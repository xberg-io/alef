use crate::core::config::manifest_extras::ManifestExtras;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

/// Render `<dependency>` XML blocks for harness extras dependencies and dev dependencies.
///
/// Maven dependency keys are in `groupId:artifactId` format (e.g., `io.github.tree-sitter:jtreesitter`).
/// Development dependencies are marked with `<scope>test</scope>`. The output is idempotent
/// and deterministic (keys sorted alphabetically via BTreeMap).
///
/// Returns a multi-line string of `<dependency>` blocks (or empty if extras is empty).
fn inject_pom_xml_extras(extras: &ManifestExtras) -> String {
    let mut out = String::new();

    // Process runtime dependencies
    for (key, spec) in &extras.dependencies {
        if let Some(version) = spec.version() {
            let (group_id, artifact_id) = split_maven_key(key);
            out.push_str(&format!(
                "        <dependency>\n\
                 \x20\x20\x20\x20<groupId>{}</groupId>\n\
                 \x20\x20\x20\x20<artifactId>{}</artifactId>\n\
                 \x20\x20\x20\x20<version>{}</version>\n\
                 \x20\x20\x20\x20</dependency>\n",
                group_id, artifact_id, version
            ));
        }
    }

    // Process dev/test dependencies with <scope>test</scope>
    for (key, spec) in &extras.dev_dependencies {
        if let Some(version) = spec.version() {
            let (group_id, artifact_id) = split_maven_key(key);
            out.push_str(&format!(
                "        <dependency>\n\
                 \x20\x20\x20\x20<groupId>{}</groupId>\n\
                 \x20\x20\x20\x20<artifactId>{}</artifactId>\n\
                 \x20\x20\x20\x20<version>{}</version>\n\
                 \x20\x20\x20\x20<scope>test</scope>\n\
                 \x20\x20\x20\x20</dependency>\n",
                group_id, artifact_id, version
            ));
        }
    }

    out
}

/// Split a Maven dependency key `groupId:artifactId` on the LAST colon.
/// If no colon is found, treat the entire key as artifactId with an empty groupId
/// (caller should handle this case, typically not expected in valid alef.toml input).
fn split_maven_key(key: &str) -> (&str, &str) {
    match key.rfind(':') {
        Some(idx) => (&key[..idx], &key[idx + 1..]),
        None => ("", key),
    }
}

pub(super) fn render_pom_xml(
    pkg_name: &str,
    java_group_id: &str,
    pkg_version: &str,
    e2e_config: &E2eConfig,
    ffi_lib_name: &str,
    env_vars: &[(String, String)],
) -> String {
    // pkg_name may be in "groupId:artifactId" Maven format; split accordingly.
    let (dep_group_id, dep_artifact_id) = if let Some((g, a)) = pkg_name.split_once(':') {
        (g, a)
    } else {
        (java_group_id, pkg_name)
    };
    let artifact_id = format!("{dep_artifact_id}-e2e-java");
    let dep_block = match e2e_config.dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            format!(
                r#"        <dependency>
            <groupId>{dep_group_id}</groupId>
            <artifactId>{dep_artifact_id}</artifactId>
            <version>{pkg_version}</version>
        </dependency>"#
            )
        }
        crate::e2e::config::DependencyMode::Local => {
            format!(
                r#"        <dependency>
            <groupId>{dep_group_id}</groupId>
            <artifactId>{dep_artifact_id}</artifactId>
            <version>{pkg_version}</version>
            <scope>system</scope>
            <systemPath>${{project.basedir}}/../../packages/java/target/{dep_artifact_id}-{pkg_version}.jar</systemPath>
        </dependency>"#
            )
        }
    };
    // E2E tests always depend on locally-built native libraries, regardless of
    // dep_mode. This ensures the shared library is copied from the workspace
    // cargo build output into the JAR at package time, so NativeLib can
    // extract and load it at startup.
    let include_native_lib_path = true;

    // Build extras_block from harness_extras if present (gated on Local dep_mode).
    // Registry mode should not inject harness-specific dev deps which may have
    // incompatible native build requirements.
    let extras_block = match e2e_config.dep_mode {
        crate::e2e::config::DependencyMode::Local => {
            if let Some(extras) = e2e_config.harness_extras.get("java") {
                if !extras.is_empty() {
                    inject_pom_xml_extras(extras)
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        }
        crate::e2e::config::DependencyMode::Registry => String::new(),
    };

    crate::e2e::template_env::render(
        "java/pom.xml.jinja",
        minijinja::context! {
            artifact_id => artifact_id,
            java_group_id => java_group_id,
            dep_block => dep_block,
            extras_block => extras_block,
            junit_version => tv::maven::JUNIT,
            jackson_version => tv::maven::JACKSON_E2E,
            build_helper_version => tv::maven::BUILD_HELPER_MAVEN_PLUGIN,
            maven_surefire_version => tv::maven::MAVEN_SUREFIRE_PLUGIN_E2E,
            jetbrains_annotations_version => tv::maven::JETBRAINS_ANNOTATIONS,
            maven_antrun_version => tv::maven::MAVEN_ANTRUN_PLUGIN,
            test_documents_path => e2e_config.test_documents_relative_from(0),
            include_native_lib_path => include_native_lib_path,
            ffi_lib_name => ffi_lib_name,
            env_entries => env_vars,
        },
    )
}

/// Render HarnessMain.java for server-pattern e2e tests.
///
/// This harness loads fixtures from classpath resources, registers handlers via
/// the app binding, and serves on a port read from SUT_URL env var or the
/// configured default. Tests hit the real SUT at /fixtures/<fixture_id>{path}.
// The server-pattern `HarnessMain.java` is now emitted by a consumer extension via
// `Extension::emit_e2e`; alef no longer emits it. Retained for tests pending the
// dead-code sweep.
#[allow(dead_code)]
pub(super) fn render_harness_main(
    e2e_config: &E2eConfig,
    groups: &[FixtureGroup],
    java_group_id: &str,
    binding_pkg: &str,
) -> String {
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;
    let app_class_owned = e2e_config.harness.app_class_for_lang("java");
    let app_class = app_class_owned.as_deref().unwrap_or("App");
    let run_method_owned = e2e_config.harness.run_method_for_lang("java");
    let run_method = run_method_owned.as_deref().unwrap_or("run");
    // Java methods are camelCase by convention. `register_method_idiomatic`
    // honors `[crates.e2e.harness.overrides.java]` first, then converts the
    // canonical name to camelCase (e.g. `register_route` → `registerRoute`).
    // The actual Java facade method is `registerAppRoute`, so expand bare `route` to it.
    let register_method = e2e_config
        .harness
        .register_method_idiomatic("java")
        .unwrap_or_else(|| "registerAppRoute".to_string());
    let register_method = if register_method == "route" {
        "registerAppRoute".to_string()
    } else {
        register_method
    };
    let body_field = &e2e_config.harness.response_body_field;

    // Collect all HTTP fixtures for this harness to register.
    let mut fixture_ids: Vec<String> = Vec::new();
    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_some() {
                fixture_ids.push(fixture.id.clone());
            }
        }
    }

    let default_harness_port = E2eConfig::default().harness.port;

    let ctx = minijinja::context! {
        java_group_id => java_group_id,
        binding_pkg => binding_pkg,
        app_class => app_class,
        run_method => run_method,
        register_method => register_method.as_str(),
        response_body_field => body_field.as_str(),
        host => host,
        port => port,
        default_port => default_harness_port,
        fixture_ids => fixture_ids,
    };

    crate::e2e::template_env::render("java/harness_main.jinja", ctx)
}

/// Render FixtureLoader.java helper that loads fixture JSON files from classpath.
///
/// This avoids inlining all fixtures as Java string literals, which would exceed
/// Java's 65535-byte limit for large fixture sets. Fixtures are stored as individual
/// JSON files in src/test/resources/fixtures/ and loaded at test runtime.
// The server-pattern `FixtureLoader.java` is now emitted by a consumer extension
// via `Extension::emit_e2e`; alef no longer emits it. Retained pending the
// dead-code sweep so the migration diff stays minimal.
#[allow(dead_code)]
pub(super) fn render_fixture_loader(java_group_id: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = header;
    out.push_str(&format!("package {java_group_id}.e2e;\n\n"));
    out.push_str("import com.fasterxml.jackson.databind.JsonNode;\n");
    out.push_str("import com.fasterxml.jackson.databind.ObjectMapper;\n");
    out.push_str("import java.io.IOException;\n");
    out.push_str("import java.io.InputStream;\n");
    out.push_str("import java.util.HashMap;\n");
    out.push_str("import java.util.Map;\n");
    out.push('\n');
    out.push_str("/**\n");
    out.push_str(" * Helper class for loading fixture JSON files from classpath.\n");
    out.push_str(" *\n");
    out.push_str(" * Fixtures are stored as individual JSON files in src/test/resources/fixtures/\n");
    out.push_str(" * to avoid exceeding Java's 65KB string literal limit.\n");
    out.push_str(" */\n");
    out.push_str("public class FixtureLoader {\n");
    out.push_str("    private static final ObjectMapper MAPPER = new ObjectMapper();\n");
    out.push('\n');
    out.push_str("    /**\n");
    out.push_str("     * Load a single fixture by ID from classpath resources.\n");
    out.push_str("     *\n");
    out.push_str("     * @param fixtureId the fixture identifier (e.g., \"smoke_basic\")\n");
    out.push_str("     * @return the parsed fixture as a JsonNode, or null if not found\n");
    out.push_str("     */\n");
    out.push_str("    public static JsonNode loadFixture(String fixtureId) {\n");
    out.push_str("        String resourcePath = \"/fixtures/\" + fixtureId + \".json\";\n");
    out.push_str("        try (InputStream is = FixtureLoader.class.getResourceAsStream(resourcePath)) {\n");
    out.push_str("            if (is == null) {\n");
    out.push_str("                System.err.println(\"Fixture not found: \" + fixtureId);\n");
    out.push_str("                return null;\n");
    out.push_str("            }\n");
    out.push_str("            return MAPPER.readTree(is);\n");
    out.push_str("        } catch (IOException e) {\n");
    out.push_str(
        "            System.err.println(\"Failed to load fixture \" + fixtureId + \": \" + e.getMessage());\n",
    );
    out.push_str("            e.printStackTrace();\n");
    out.push_str("            return null;\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    /**\n");
    out.push_str("     * Load all fixtures from the classpath resources directory.\n");
    out.push_str("     *\n");
    out.push_str("     * @return a map of fixture IDs to parsed fixture JsonNodes\n");
    out.push_str("     */\n");
    out.push_str("    public static Map<String, JsonNode> loadAllFixtures() {\n");
    out.push_str("        Map<String, JsonNode> fixtures = new HashMap<>();\n");
    out.push_str("        // Note: Loading all fixtures requires iterating the classpath.\n");
    out.push_str("        // For typical e2e test suites, only the fixtures needed by the\n");
    out.push_str("        // specific test class should be loaded via loadFixture(id).\n");
    out.push_str("        return fixtures;\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

/// Render MockServerListener.java from jinja template.
pub(super) fn render_mock_server_listener(java_group_id: &str) -> String {
    let header_comment = hash::header(CommentStyle::DoubleSlash);
    let ctx = minijinja::context! {
        java_group_id => java_group_id,
        header_comment => header_comment,
    };
    crate::e2e::template_env::render("java/MockServerListener.java.jinja", ctx)
}

/// Generate a `{TypeName}Display.java` helper that pattern-matches on every
/// variant of a sealed interface and returns a display string for e2e assertions.
pub(super) fn render_sealed_display(
    type_name: &str,
    enum_def: &crate::core::ir::EnumDef,
    type_defs: &[crate::core::ir::TypeDef],
    java_group_id: &str,
) -> String {
    let helper_class = format!("{type_name}Display");
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = header;
    out.push_str(&format!("package {java_group_id}.e2e;\n\n"));
    out.push_str(&format!("import {java_group_id}.{type_name};\n"));
    out.push('\n');
    out.push_str(&format!(
        "/**\n * Helper class for extracting display strings from {type_name} sealed interface.\n */\n"
    ));
    out.push_str(&format!("class {helper_class} {{\n"));
    out.push_str(&format!("    static String toDisplayString({type_name} value) {{\n"));
    out.push_str("        if (value == null) return \"\";\n");
    out.push_str("        return switch (value) {\n");

    for variant in &enum_def.variants {
        let variant_name = &variant.name;
        let has_format_field = variant.is_tuple && variant.fields.len() == 1 && {
            let field_type_name = match &variant.fields[0].ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            };
            field_type_name.is_some_and(|tn| {
                type_defs
                    .iter()
                    .find(|td| td.name == tn)
                    .is_some_and(|td| td.fields.iter().any(|f| f.name == "format"))
            })
        };

        let display = if has_format_field {
            "i.value().format()".to_string()
        } else {
            let serde_name = variant
                .serde_rename
                .as_deref()
                .unwrap_or(variant_name.as_str())
                .to_lowercase();
            format!("\"{serde_name}\"")
        };

        let binding = if has_format_field {
            format!("{type_name}.{variant_name} i")
        } else {
            format!("{type_name}.{variant_name} _")
        };

        out.push_str(&format!("            case {binding} -> {display};\n"));
    }

    out.push_str("            default -> \"unknown\";\n");
    out.push_str("        };\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::manifest_extras::ExtraDepSpec;
    use crate::e2e::config::{DependencyMode, E2eConfig};

    fn make_e2e_config(dep_mode: DependencyMode, harness_extras: Option<ManifestExtras>) -> E2eConfig {
        let mut cfg = E2eConfig {
            dep_mode,
            ..E2eConfig::default()
        };
        if let Some(extras) = harness_extras {
            cfg.harness_extras.insert("java".to_string(), extras);
        }
        cfg
    }

    #[test]
    fn render_pom_xml_local_without_extras_has_single_dep() {
        let e2e_cfg = make_e2e_config(DependencyMode::Local, None);
        let out = render_pom_xml("tree-sitter", "com.example", "1.0.0", &e2e_cfg, "tree_sitter", &[]);
        assert!(out.contains("<groupId>com.example</groupId>"), "got: {out}");
        assert!(out.contains("<artifactId>tree-sitter</artifactId>"), "got: {out}");
        assert!(
            !out.contains("io.github.tree-sitter"),
            "extras should not appear without None"
        );
    }

    #[test]
    fn render_pom_xml_with_harness_extras_includes_dependencies() {
        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "io.github.tree-sitter:jtreesitter".to_string(),
            ExtraDepSpec::Simple("0.26.0".to_string()),
        );
        let e2e_cfg = make_e2e_config(DependencyMode::Local, Some(extras));
        let out = render_pom_xml("my-lib", "com.example", "1.0.0", &e2e_cfg, "my_lib", &[]);
        assert!(
            out.contains("<groupId>io.github.tree-sitter</groupId>"),
            "groupId should be injected"
        );
        assert!(
            out.contains("<artifactId>jtreesitter</artifactId>"),
            "artifactId should be injected"
        );
        assert!(out.contains("<version>0.26.0</version>"), "version should be injected");
        // Runtime deps should not have <scope>test</scope>
        let jtreesitter_idx = out.find("jtreesitter").expect("jtreesitter found");
        let after_jtreesitter = &out[jtreesitter_idx..];
        let next_closing = after_jtreesitter.find("</dependency>").expect("closing tag found");
        let dep_block = &after_jtreesitter[..next_closing];
        assert!(
            !dep_block.contains("<scope>test</scope>"),
            "runtime deps should not have test scope"
        );
    }

    #[test]
    fn render_pom_xml_with_harness_extras_dev_dependencies_include_test_scope() {
        let mut extras = ManifestExtras::default();
        extras.dev_dependencies.insert(
            "com.custom.org:custom-testing-lib".to_string(),
            ExtraDepSpec::Simple("4.13.2".to_string()),
        );
        let e2e_cfg = make_e2e_config(DependencyMode::Local, Some(extras));
        let out = render_pom_xml("my-lib", "com.example", "1.0.0", &e2e_cfg, "my_lib", &[]);
        assert!(
            out.contains("com.custom.org"),
            "custom-testing-lib groupId should be present"
        );
        assert!(
            out.contains("custom-testing-lib"),
            "custom-testing-lib artifactId should be present"
        );
        assert!(
            out.contains("<version>4.13.2</version>"),
            "custom-testing-lib version should be present"
        );
        // Dev deps must include <scope>test</scope> — find it right after the custom artifact.
        let custom_idx = out.find("custom-testing-lib").expect("custom-testing-lib found");
        let after_custom = &out[custom_idx..];
        let next_closing = after_custom.find("</dependency>").expect("closing tag found");
        let dep_block = &after_custom[..next_closing];
        assert!(
            dep_block.contains("<scope>test</scope>"),
            "dev deps must have test scope; got:\n{dep_block}"
        );
    }

    #[test]
    fn render_pom_xml_registry_mode_excludes_harness_extras() {
        let mut extras = ManifestExtras::default();
        extras.dev_dependencies.insert(
            "io.github.tree-sitter:jtreesitter".to_string(),
            ExtraDepSpec::Simple("0.26.0".to_string()),
        );
        let e2e_cfg = make_e2e_config(DependencyMode::Registry, Some(extras));
        let out = render_pom_xml("my-lib", "com.example", "1.0.0", &e2e_cfg, "my_lib", &[]);
        assert!(
            !out.contains("jtreesitter"),
            "registry mode should not inject harness extras"
        );
    }

    #[test]
    fn render_pom_xml_empty_extras_matches_no_extras() {
        let empty_extras = ManifestExtras::default();
        let with_empty = make_e2e_config(DependencyMode::Local, Some(empty_extras));
        let without = make_e2e_config(DependencyMode::Local, None);
        let with_empty_out = render_pom_xml("my-lib", "com.example", "1.0.0", &with_empty, "my_lib", &[]);
        let without_out = render_pom_xml("my-lib", "com.example", "1.0.0", &without, "my_lib", &[]);
        assert_eq!(
            with_empty_out, without_out,
            "empty extras should produce identical output"
        );
    }

    #[test]
    fn split_maven_key_splits_on_last_colon() {
        let (group, artifact) = split_maven_key("io.github.tree-sitter:jtreesitter");
        assert_eq!(group, "io.github.tree-sitter");
        assert_eq!(artifact, "jtreesitter");
    }

    #[test]
    fn split_maven_key_with_nested_groups() {
        let (group, artifact) = split_maven_key("com.example.org.subgroup:my-artifact");
        assert_eq!(group, "com.example.org.subgroup");
        assert_eq!(artifact, "my-artifact");
    }

    #[test]
    fn split_maven_key_without_colon() {
        let (group, artifact) = split_maven_key("bare-artifact-name");
        assert_eq!(group, "");
        assert_eq!(artifact, "bare-artifact-name");
    }

    #[test]
    fn inject_pom_xml_extras_preserves_order_from_btreemap() {
        let mut extras = ManifestExtras::default();
        // Insert in non-alphabetical order to verify BTreeMap sorts them.
        extras
            .dependencies
            .insert("z.org:zulu".to_string(), ExtraDepSpec::Simple("1.0".to_string()));
        extras
            .dependencies
            .insert("a.org:alpha".to_string(), ExtraDepSpec::Simple("2.0".to_string()));
        let block = inject_pom_xml_extras(&extras);

        // Both should be present.
        assert!(block.contains("a.org") && block.contains("z.org"));
        // alpha should come before zulu (alphabetical by key).
        let alpha_idx = block.find("alpha").expect("alpha found");
        let zulu_idx = block.find("zulu").expect("zulu found");
        assert!(alpha_idx < zulu_idx, "keys should be alphabetically sorted");
    }

    #[test]
    fn inject_pom_xml_extras_skips_entries_without_version() {
        let mut extras = ManifestExtras::default();
        // ExtraDepSpec::Detailed without a "version" key
        let bad_spec = toml::Table::new(); // Empty table, no "version" key
        extras
            .dependencies
            .insert("bad.org:badlib".to_string(), ExtraDepSpec::Detailed(bad_spec));
        extras
            .dev_dependencies
            .insert("good.org:goodlib".to_string(), ExtraDepSpec::Simple("1.0".to_string()));

        let block = inject_pom_xml_extras(&extras);

        // goodlib should be present, badlib should be absent.
        assert!(block.contains("goodlib"), "entries with version should be included");
        assert!(!block.contains("badlib"), "entries without version should be skipped");
    }
}
