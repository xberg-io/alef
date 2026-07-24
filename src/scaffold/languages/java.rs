use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::template_versions::{maven, toolchain};
use crate::{scaffold::parse_author, scaffold::scaffold_meta, scaffold::xml_escape};
use minijinja::context;
use std::path::PathBuf;

/// Render `<dependency>` blocks for host-native capsule (Language) passthrough.
/// Each `package` is a Maven `groupId:artifactId` coordinate (e.g.
/// `io.github.tree-sitter:jtreesitter`); `package_version` is the `<version>`.
fn java_capsule_dependencies(config: &ResolvedCrateConfig) -> String {
    let mut deps: Vec<(String, String)> = config
        .java
        .as_ref()
        .map(|c| {
            c.capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .map(|cap| (cap.package.clone(), cap.package_version.clone()))
                .collect()
        })
        .unwrap_or_default();
    deps.sort();
    deps.dedup();
    deps.iter()
        .map(|(coord, ver)| {
            let (group_id, artifact_id) = coord.split_once(':').unwrap_or((coord.as_str(), ""));
            format!(
                "\n        <dependency>\n            <groupId>{group_id}</groupId>\n            \
                 <artifactId>{artifact_id}</artifactId>\n            <version>{ver}</version>\n        </dependency>"
            )
        })
        .collect()
}

pub(crate) fn scaffold_java(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let name = config.java_artifact_id();
    let name = name.as_str();
    let version = &api.version;

    let repo_url = meta.configured_repository.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Java scaffold requires package metadata repository; set package_metadata.repository or scaffold.repository"
        )
    })?;
    if meta.authors.is_empty() {
        anyhow::bail!(
            "Java scaffold requires package metadata authors; set package_metadata.authors or scaffold.authors"
        );
    }
    let license = meta.license.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Java scaffold requires package metadata license; set package_metadata.license or scaffold.license"
        )
    })?;

    let scm = scm_urls(repo_url);

    let group_id = config.java_group_id();
    let source_root = group_id.split('.').next().unwrap_or("dev");

    let developers_xml = if meta.authors.is_empty() {
        String::new()
    } else {
        let devs: Vec<String> = meta
            .authors
            .iter()
            .map(|a| {
                let (name, email) = parse_author(a);
                let name_escaped = xml_escape(name);
                let email_line = if email.is_empty() {
                    String::new()
                } else {
                    format!("\n            <email>{}</email>", xml_escape(email))
                };
                format!(
                    "        <developer>\n            <name>{name_escaped}</name>{email_line}\n        </developer>"
                )
            })
            .collect();
        format!("\n    <developers>\n{}\n    </developers>\n", devs.join("\n"))
    };

    let license_url = match license {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
    };
    let license_url_xml = if license_url.is_empty() {
        String::new()
    } else {
        format!("\n            <url>{license_url}</url>")
    };

    let content = crate::scaffold::template_env::render(
        "java_pom.xml.jinja",
        context! {
            group_id => group_id,
            name => name,
            version => version,
            description => meta.description,
            repository => repo_url,
            license => license,
            license_url => license_url_xml,
            developers => developers_xml,
            scm_connection => scm.connection,
            scm_developer_connection => scm.developer_connection,
            capsule_deps => java_capsule_dependencies(config),
            source_root => source_root,
            java_release => toolchain::JAVA_JVM_TARGET,
            junit_version => maven::JUNIT,
            maven_core_version => maven::MAVEN_CORE,
            maven_compiler_plugin_version => maven::MAVEN_COMPILER_PLUGIN,
            maven_surefire_plugin_version => maven::MAVEN_SUREFIRE_PLUGIN,
            maven_checkstyle_plugin_version => maven::MAVEN_CHECKSTYLE_PLUGIN,
            maven_pmd_plugin_version => maven::MAVEN_PMD_PLUGIN,
            maven_source_plugin_version => maven::MAVEN_SOURCE_PLUGIN,
            maven_javadoc_plugin_version => maven::MAVEN_JAVADOC_PLUGIN,
            maven_gpg_plugin_version => maven::MAVEN_GPG_PLUGIN,
            maven_clean_plugin_version => maven::MAVEN_CLEAN_PLUGIN,
            maven_resources_plugin_version => maven::MAVEN_RESOURCES_PLUGIN,
            maven_jar_plugin_version => maven::MAVEN_JAR_PLUGIN,
            maven_install_plugin_version => maven::MAVEN_INSTALL_PLUGIN,
            maven_deploy_plugin_version => maven::MAVEN_DEPLOY_PLUGIN,
            maven_site_plugin_version => maven::MAVEN_SITE_PLUGIN,
            central_publishing_plugin_version => maven::CENTRAL_PUBLISHING_PLUGIN,
            versions_maven_plugin_version => maven::VERSIONS_MAVEN_PLUGIN,
            maven_enforcer_plugin_version => maven::MAVEN_ENFORCER_PLUGIN,
            jacoco_maven_plugin_version => maven::JACOCO_MAVEN_PLUGIN,
            checkstyle_version => maven::CHECKSTYLE,
            pmd_version => maven::PMD,
            jspecify_version => maven::JSPECIFY,
            jackson_version => maven::JACKSON,
            assertj_version => maven::ASSERTJ,
        },
    );

    let checkstyle_xml = r#"<?xml version="1.0"?>
<!DOCTYPE module PUBLIC
    "-//Checkstyle//DTD Checkstyle Configuration 1.3//EN"
    "https://checkstyle.org/dtds/configuration_1_3.dtd">

<!-- Checkstyle handles correctness checks only. Formatting is handled by poly. -->
<module name="Checker">
    <property name="charset" value="UTF-8"/>
    <property name="severity" value="error"/>
    <property name="fileExtensions" value="java"/>

    <module name="SuppressionFilter">
        <property name="file" value="checkstyle-suppressions.xml"/>
        <property name="optional" value="true"/>
    </module>

    <module name="LineLength">
        <!-- 200 accommodates the alef-emitted DefaultClient.java FFM call shims:
             the codegen chains arena allocation, MemorySegment marshalling, and
             error-result handling onto single lines that don't reflow cleanly.
             Tests and hand-written code stay well below this; the limit only
             gives the generator headroom. -->
        <property name="max" value="200"/>
        <property name="ignorePattern" value="^package.*|^import.*|a]href|href|http://|https://|ftp://"/>
    </module>

    <module name="TreeWalker">
        <!-- Naming Conventions (relaxed for FFI snake_case from Rust) -->
        <module name="ConstantName">
            <property name="format" value="^([A-Z][A-Z0-9]*(_[A-Z0-9]+)*|[a-z][a-zA-Z0-9_]*)$"/>
        </module>
        <module name="PackageName"/>
        <module name="TypeName"/>

        <!-- Modifier Checks -->
        <module name="ModifierOrder"/>
        <module name="RedundantModifier"/>

        <!-- Imports -->
        <module name="UnusedImports"/>

        <!-- Coding -->
        <module name="EmptyStatement"/>
        <module name="EqualsHashCode"/>
        <module name="SimplifyBooleanExpression"/>
        <module name="SimplifyBooleanReturn"/>

        <!-- Size Violations -->
        <module name="MethodLength">
            <property name="max" value="150"/>
        </module>

        <!-- Misc -->
        <module name="ArrayTypeStyle"/>
        <module name="UpperEll"/>
    </module>
</module>
"#;

    let checkstyle_properties = "";

    let checkstyle_suppressions_xml = r#"<?xml version="1.0"?>
<!DOCTYPE suppressions PUBLIC
    "-//Checkstyle//DTD SuppressionFilter Configuration 1.2//EN"
    "https://checkstyle.org/dtds/suppressions_1_2.dtd">

<suppressions>
    <!-- FFI constants -->
    <suppress checks="ConstantName" files=".*FFI\.java"/>
    <suppress checks="MagicNumber" files=".*FFI\.java"/>


    <!-- Allow star imports and magic numbers in test files -->
    <suppress checks="AvoidStarImport" files=".*Test\.java"/>
    <suppress checks="MagicNumber" files=".*Test\.java"/>
    <suppress checks="MethodLength" files=".*Test\.java"/>
</suppressions>
"#;

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/java/pom.xml"),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/java/checkstyle.xml"),
            content: checkstyle_xml.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/java/checkstyle.properties"),
            content: checkstyle_properties.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/java/checkstyle-suppressions.xml"),
            content: checkstyle_suppressions_xml.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/java/versions-rules.xml"),
            content: r#"<?xml version="1.0" encoding="UTF-8"?>
<ruleset xmlns="http://mojo.codehaus.org/versions-maven-plugin/rules/2.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://mojo.codehaus.org/versions-maven-plugin/rules/2.0.0
                             https://www.mojohaus.org/versions/versions-maven-plugin/xsd/rule-2.0.0.xsd"
         comparisonMethod="maven">
    <ignoreVersions>
        <ignoreVersion type="regex">(?i).*[.-](alpha|beta|rc|cr|milestone|preview|ea|eap|snapshot).*</ignoreVersion>
        <ignoreVersion type="regex">(?i).*[.-]m\d+.*</ignoreVersion>
    </ignoreVersions>
</ruleset>
"#
            .to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/java/pmd-ruleset.xml"),
            content: r#"<?xml version="1.0"?>
<ruleset name="Custom PMD Ruleset"
         xmlns="http://pmd.sourceforge.net/ruleset/2.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://pmd.sourceforge.net/ruleset/2.0.0
                             https://pmd.sourceforge.io/ruleset_2_0_0.xsd">
    <description>PMD ruleset for Java bindings</description>

    <rule ref="category/java/bestpractices.xml">
        <exclude name="LooseCoupling"/>
        <!-- Codegen emits defensive @SuppressWarnings; some are situationally redundant. -->
        <exclude name="UnnecessaryWarningSuppression"/>
        <!--
            Generated config DTOs carry loopback defaults copied verbatim from the
            Rust source (e.g. `host = "127.0.0.1"`). The literal is the source of
            truth, not a deployment address, so AvoidUsingHardCodedIP does not apply.
        -->
        <exclude name="AvoidUsingHardCodedIP"/>
        <!--
            Generated DTOs mirror Rust array/byte fields and expose them verbatim;
            defensive copies are the consumer's concern, not codegen's. Suppress the
            array-aliasing pair project-wide.
        -->
        <exclude name="ArrayIsStoredDirectly"/>
        <exclude name="MethodReturnsInternalArray"/>
    </rule>
    <rule ref="category/java/codestyle.xml">
        <exclude name="AtLeastOneConstructor"/>
        <exclude name="CommentDefaultAccessModifier"/>
        <exclude name="OnlyOneReturn"/>
        <!--
            These records mirror the Rust source field-for-field; field, parameter and
            accessor names are inherited verbatim and cannot be renamed by codegen.
            ShortVariable (`id`, `n`) and LongVariable (`selfHarmInstructions`) flag those
            inherited names; UseUnderscoresInNumericLiterals flags default literals copied
            from Rust (`300000L`); the *CouldBeFinal rules are uniform codegen style.
            Codegen is the source of truth, so these are suppressed project-wide.
        -->
        <exclude name="ShortVariable"/>
        <exclude name="LongVariable"/>
        <exclude name="UseUnderscoresInNumericLiterals"/>
        <exclude name="LocalVariableCouldBeFinal"/>
        <exclude name="MethodArgumentCouldBeFinal"/>
    </rule>
    <rule ref="category/java/design.xml">
        <exclude name="LawOfDemeter"/>
        <exclude name="DataClass"/>
        <!--
            DTOs mirror their Rust struct: a wide struct (e.g. ModerationCategoryScores)
            yields many fields and many accessor methods. TooManyFields/TooManyMethods are
            inherent to the source shape, not a design smell codegen can refactor away.
        -->
        <exclude name="TooManyFields"/>
        <exclude name="TooManyMethods"/>
        <!--
            Generated Builder-pattern classes carry per-instance final fields
            with their type-default initializer (e.g. `private final boolean
            introspectionEnabled = true;`). PMD interprets these as constants
            and suggests static promotion, but each Builder instance is a
            mutable assembly point — promoting to static would shadow the
            field across all builders and break the pattern. Suppress the
            rule project-wide; codegen is the source of truth.
        -->
        <exclude name="FinalFieldCouldBeStatic"/>
    </rule>
    <rule ref="category/java/documentation.xml">
        <exclude name="CommentSize"/>
        <!--
            CommentRequired demands hand-written Javadoc on every field/constructor.
            These records are generated DTOs that mirror the Rust source field-for-field;
            codegen is the source of truth, so per-field comments are neither written nor
            meaningful here. Type-level Javadoc is still emitted and still required.
        -->
        <exclude name="CommentRequired"/>
    </rule>
    <rule ref="category/java/errorprone.xml">
        <exclude name="EmptyCatchBlock"/>
    </rule>
    <rule ref="category/java/multithreading.xml">
        <!-- Immutable DTOs use plain HashMap for their map fields; concurrency is the caller's concern. -->
        <exclude name="UseConcurrentHashMap"/>
    </rule>
    <rule ref="category/java/performance.xml"/>
    <rule ref="category/java/security.xml"/>
</ruleset>
"#
            .to_string(),
            generated_header: false,
        },
    ])
}

struct ScmUrls {
    connection: String,
    developer_connection: String,
}

fn scm_urls(repository: &str) -> ScmUrls {
    let normalized = repository.trim_end_matches(".git");
    let without_scheme = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
        .unwrap_or(normalized);
    let (host, path) = without_scheme.split_once('/').unwrap_or((without_scheme, ""));
    let suffix = if path.is_empty() {
        String::new()
    } else {
        format!("/{path}.git")
    };

    ScmUrls {
        connection: format!("scm:git:git://{host}{suffix}"),
        developer_connection: format!("scm:git:ssh://git@{host}{suffix}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::ApiSurface;
    use std::path::Path;

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    #[test]
    fn pom_publish_profile_contains_cpd_and_pmd_skip() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["java"]

[[crates]]
name = "testlib"
sources = []

[crates.package_metadata]
repository = "https://github.com/example/testlib"
authors = ["Test Author <test@example.com>"]
license = "MIT"
description = "A test library"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_java(&api, &config).expect("scaffold_java succeeds");
        let pom = files
            .iter()
            .find(|f| f.path == Path::new("packages/java/pom.xml"))
            .expect("pom.xml present");
        assert!(
            pom.content.contains("<cpd.skip>true</cpd.skip>"),
            "pom.xml publish profile must contain <cpd.skip>true</cpd.skip>"
        );
        assert!(
            pom.content.contains("<pmd.skip>true</pmd.skip>"),
            "pom.xml publish profile must contain <pmd.skip>true</pmd.skip>"
        );
    }
}
