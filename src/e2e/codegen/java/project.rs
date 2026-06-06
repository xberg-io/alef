use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

pub(super) fn render_pom_xml(
    pkg_name: &str,
    java_group_id: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    test_documents_path: &str,
    ffi_lib_name: &str,
) -> String {
    // pkg_name may be in "groupId:artifactId" Maven format; split accordingly.
    let (dep_group_id, dep_artifact_id) = if let Some((g, a)) = pkg_name.split_once(':') {
        (g, a)
    } else {
        (java_group_id, pkg_name)
    };
    let artifact_id = format!("{dep_artifact_id}-e2e-java");
    let dep_block = match dep_mode {
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
    // Registry-mode test_apps consume the published Maven Central JAR, which
    // bundles natives under `/natives/{rid}/`; NativeLib extracts and loads
    // them at startup without needing java.library.path. Local-mode e2e tests
    // depend on a locally-built JAR that does NOT bundle natives, and must
    // resolve the shared library from a separate cargo build output.
    let include_native_lib_path = matches!(dep_mode, crate::e2e::config::DependencyMode::Local);
    crate::e2e::template_env::render(
        "java/pom.xml.jinja",
        minijinja::context! {
            artifact_id => artifact_id,
            java_group_id => java_group_id,
            dep_block => dep_block,
            junit_version => tv::maven::JUNIT,
            jackson_version => tv::maven::JACKSON_E2E,
            build_helper_version => tv::maven::BUILD_HELPER_MAVEN_PLUGIN,
            maven_surefire_version => tv::maven::MAVEN_SUREFIRE_PLUGIN_E2E,
            test_documents_path => test_documents_path,
            include_native_lib_path => include_native_lib_path,
            ffi_lib_name => ffi_lib_name,
        },
    )
}

/// Render HarnessMain.java for server-pattern e2e tests.
///
/// This harness loads fixtures from classpath resources, registers handlers via
/// the app binding, and serves on a port read from SUT_URL env var or the
/// configured default. Tests hit the real SUT at /fixtures/<fixture_id>{path}.
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

    let ctx = minijinja::context! {
        java_group_id => java_group_id,
        binding_pkg => binding_pkg,
        app_class => app_class,
        run_method => run_method,
        register_method => register_method.as_str(),
        response_body_field => body_field.as_str(),
        host => host,
        port => port,
        fixture_ids => fixture_ids,
    };

    crate::e2e::template_env::render("java/harness_main.jinja", ctx)
}

/// Render FixtureLoader.java helper that loads fixture JSON files from classpath.
///
/// This avoids inlining all fixtures as Java string literals, which would exceed
/// Java's 65535-byte limit for large fixture sets. Fixtures are stored as individual
/// JSON files in src/test/resources/fixtures/ and loaded at test runtime.
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
