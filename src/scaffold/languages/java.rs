use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::core::ir::ApiSurface;
use crate::core::template_versions::toolchain;
use crate::scaffold::dependency_versions::{ManagedVersions, override_notice_lines};
use crate::{scaffold::parse_author, scaffold::scaffold_meta, scaffold::xml_escape};
use anyhow::Context as _;
use minijinja::context;
use std::path::{Path, PathBuf};

/// Build `<dependency>` template values for host-native capsule (Language) passthrough.
/// Each `package` is a Maven `groupId:artifactId` coordinate (e.g.
/// `io.github.tree-sitter:jtreesitter`); `package_version` is the `<version>`.
fn java_capsule_dependencies(config: &ResolvedCrateConfig) -> Vec<minijinja::Value> {
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
            context! {
                group_id => group_id,
                artifact_id => artifact_id,
                version => ver,
            }
        })
        .collect()
}

pub(crate) fn scaffold_java(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let versions = ManagedVersions::new(config, Language::Java);
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

    let authors: Vec<minijinja::Value> = meta
        .authors
        .iter()
        .map(|a| {
            let (name, email) = parse_author(a);
            context! {
                name => xml_escape(name),
                email => xml_escape(email),
            }
        })
        .collect();

    let license_url = match license {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
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
            license_url => license_url,
            authors => authors,
            scm_connection => scm.connection,
            scm_developer_connection => scm.developer_connection,
            capsule_deps => java_capsule_dependencies(config),
            source_root => source_root,
            version_notice_lines => override_notice_lines(Language::Java),
            java_release => toolchain::JAVA_JVM_TARGET,
            junit_version => versions.get("maven:junit"),
            maven_core_version => versions.get("maven:maven-core"),
            maven_compiler_plugin_version => versions.get("maven:maven-compiler-plugin"),
            maven_surefire_plugin_version => versions.get("maven:maven-surefire-plugin"),
            maven_checkstyle_plugin_version => versions.get("maven:maven-checkstyle-plugin"),
            maven_source_plugin_version => versions.get("maven:maven-source-plugin"),
            maven_javadoc_plugin_version => versions.get("maven:maven-javadoc-plugin"),
            maven_gpg_plugin_version => versions.get("maven:maven-gpg-plugin"),
            maven_clean_plugin_version => versions.get("maven:maven-clean-plugin"),
            maven_resources_plugin_version => versions.get("maven:maven-resources-plugin"),
            maven_jar_plugin_version => versions.get("maven:maven-jar-plugin"),
            maven_install_plugin_version => versions.get("maven:maven-install-plugin"),
            maven_deploy_plugin_version => versions.get("maven:maven-deploy-plugin"),
            maven_site_plugin_version => versions.get("maven:maven-site-plugin"),
            central_publishing_plugin_version => versions.get("maven:central-publishing-maven-plugin"),
            versions_maven_plugin_version => versions.get("maven:versions-maven-plugin"),
            maven_enforcer_plugin_version => versions.get("maven:maven-enforcer-plugin"),
            jacoco_maven_plugin_version => versions.get("maven:jacoco-maven-plugin"),
            checkstyle_version => versions.get("maven:checkstyle"),
            jspecify_version => versions.get("maven:jspecify"),
            jackson_version => versions.get("maven:jackson"),
            assertj_version => versions.get("maven:assertj"),
        },
    );

    let checkstyle_xml = r#"<?xml version="1.0"?>
<!DOCTYPE module PUBLIC
    "-//Checkstyle//DTD Checkstyle Configuration 1.3//EN"
    "https://checkstyle.org/dtds/configuration_1_3.dtd">

<!-- Checkstyle handles correctness checks only. Formatting is handled by poly. -->
<module name="Checker">
  <property name="charset" value="UTF-8" />
  <property name="severity" value="error" />
  <property name="fileExtensions" value="java" />

  <module name="SuppressionFilter">
    <property name="file" value="checkstyle-suppressions.xml" />
    <property name="optional" value="true" />
  </module>

  <module name="LineLength">
    <!-- 200 accommodates the alef-emitted DefaultClient.java FFM call shims:
             the codegen chains arena allocation, MemorySegment marshalling, and
             error-result handling onto single lines that don't reflow cleanly.
             Tests and hand-written code stay well below this; the limit only
             gives the generator headroom. -->
    <property name="max" value="200" />
    <property name="ignorePattern" value="^package.*|^import.*|a]href|href|http://|https://|ftp://" />
  </module>

  <module name="TreeWalker">
    <!-- Naming Conventions (relaxed for FFI snake_case from Rust) -->
    <module name="ConstantName">
      <property name="format" value="^([A-Z][A-Z0-9]*(_[A-Z0-9]+)*|[a-z][a-zA-Z0-9_]*)$" />
    </module>
    <module name="PackageName" />
    <module name="TypeName" />

    <!-- Modifier Checks -->
    <module name="ModifierOrder" />
    <module name="RedundantModifier" />

    <!-- Imports -->
    <module name="UnusedImports" />

    <!-- Coding -->
    <module name="EmptyStatement" />
    <module name="EqualsHashCode" />
    <module name="SimplifyBooleanExpression" />
    <module name="SimplifyBooleanReturn" />

    <!-- Size Violations -->
    <module name="MethodLength">
      <property name="max" value="150" />
    </module>

    <!-- Misc -->
    <module name="ArrayTypeStyle" />
    <module name="UpperEll" />
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
  <suppress checks="ConstantName" files=".*FFI\.java" />
  <suppress checks="MagicNumber" files=".*FFI\.java" />

  <!-- Allow star imports and magic numbers in test files -->
  <suppress checks="AvoidStarImport" files=".*Test\.java" />
  <suppress checks="MagicNumber" files=".*Test\.java" />
  <suppress checks="MethodLength" files=".*Test\.java" />
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
<ruleset
  xmlns="http://mojo.codehaus.org/versions-maven-plugin/rules/2.0.0"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  xsi:schemaLocation="http://mojo.codehaus.org/versions-maven-plugin/rules/2.0.0
                             https://www.mojohaus.org/versions/versions-maven-plugin/xsd/rule-2.0.0.xsd"
  comparisonMethod="maven"
>
  <ignoreVersions>
    <ignoreVersion type="regex">(?i).*[.-](alpha|beta|rc|cr|milestone|preview|ea|eap|snapshot).*</ignoreVersion>
    <ignoreVersion type="regex">(?i).*[.-]m\d+.*</ignoreVersion>
  </ignoreVersions>
</ruleset>
"#
            .to_string(),
            generated_header: false,
        },
    ])
}

/// The two known-stale `LineLength` max values `scaffold_java` emitted before the fix that
/// raised the ceiling to 200 for alef-emitted FFM call shims (`a95defbf5` raised 120 -> 140,
/// `6382afdf6` raised 140 -> 200). Each is paired with the immediately following
/// `ignorePattern` property line -- unique to this one `LineLength` module in the whole
/// template -- so the anchor can never coincidentally match `MethodLength`'s own unrelated
/// `max` property (150) or anything else in the file.
const STALE_LINE_LENGTH_120: &str = "<property name=\"max\" value=\"120\"/>\n        <property name=\"ignorePattern\" value=\"^package.*|^import.*|a]href|href|http://|https://|ftp://\"/>";
const STALE_LINE_LENGTH_140: &str = "<property name=\"max\" value=\"140\"/>\n        <property name=\"ignorePattern\" value=\"^package.*|^import.*|a]href|href|http://|https://|ftp://\"/>";
const FIXED_LINE_LENGTH: &str = "<property name=\"max\" value=\"200\"/>\n        <property name=\"ignorePattern\" value=\"^package.*|^import.*|a]href|href|http://|https://|ftp://\"/>";

/// Repair a pre-existing `packages/java/checkstyle.xml` whose `LineLength` ceiling is still
/// 120 or 140 -- the exact defect fixed across two commits (`a95defbf5`, `6382afdf6`) that
/// raised the limit to 200 so `mvn verify` stops failing on alef-emitted FFM call shims in
/// `DefaultClient.java` that cannot reflow within a shorter line.
///
/// `packages/java/checkstyle.xml` is `generated_header: false` (create-only), so a repo
/// scaffolded before either fix landed keeps checkstyle's default "error" severity on every
/// regenerated facade file that legitimately needs more than 120 or 140 columns -- a build
/// failure, not a warning. 120 and 140 are the *only* two values alef itself has ever emitted
/// as this property's default (see the fix history), so an exact match against either is a
/// safe positive-identification signature: a user who happened to configure `max` at precisely
/// one of alef's own two historical defaults either never touched the line, or chose the same
/// value alef would have -- either way, advancing it to alef's current correct value is the
/// right outcome, and any other value (a deliberate customisation) is left untouched. ~keep
pub(crate) fn migrate_java_checkstyle_line_length(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };

    let stale = [STALE_LINE_LENGTH_120, STALE_LINE_LENGTH_140]
        .into_iter()
        .find(|candidate| existing.matches(candidate).count() == 1);
    let Some(stale) = stale else {
        return Ok(false);
    };

    let migrated = existing.replacen(stale, FIXED_LINE_LENGTH, 1);

    let parent = path.parent().context("checkstyle.xml path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing packages/java/checkstyle.xml: raised LineLength max to 200 \
         to accommodate alef-emitted FFM call shims"
    );
    Ok(true)
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
mod migrate_tests {
    use super::*;

    fn checkstyle_with_line_length(max_property: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<module name=\"Checker\">\n    <module name=\"LineLength\">\n        {max_property}\n        <property name=\"ignorePattern\" value=\"^package.*|^import.*|a]href|href|http://|https://|ftp://\"/>\n    </module>\n    <module name=\"TreeWalker\">\n        <module name=\"MethodLength\">\n            <property name=\"max\" value=\"150\"/>\n        </module>\n    </module>\n</module>\n"
        )
    }

    #[test]
    fn should_raise_line_length_to_200_when_stale_at_120() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/java");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/java");
        let stale = checkstyle_with_line_length("<property name=\"max\" value=\"120\"/>");
        std::fs::write(pkg_dir.join("checkstyle.xml"), &stale).expect("write stale checkstyle.xml");

        let relative_path = Path::new("packages/java/checkstyle.xml");
        let changed = migrate_java_checkstyle_line_length(dir.path(), relative_path).expect("must not error");
        assert!(
            changed,
            "a checkstyle.xml stuck at the old LineLength ceiling must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("checkstyle.xml")).expect("read migrated file");
        assert!(on_disk.contains("<property name=\"max\" value=\"200\"/>"));
        assert!(!on_disk.contains("value=\"120\""));
        assert!(
            on_disk.contains("<property name=\"max\" value=\"150\"/>"),
            "MethodLength's unrelated max property must survive untouched"
        );

        let changed_again = migrate_java_checkstyle_line_length(dir.path(), relative_path).expect("must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_raise_line_length_to_200_when_stale_at_140() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/java");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/java");
        let stale = checkstyle_with_line_length("<property name=\"max\" value=\"140\"/>");
        std::fs::write(pkg_dir.join("checkstyle.xml"), &stale).expect("write stale checkstyle.xml");

        let relative_path = Path::new("packages/java/checkstyle.xml");
        let changed = migrate_java_checkstyle_line_length(dir.path(), relative_path).expect("must not error");
        assert!(changed);

        let on_disk = std::fs::read_to_string(pkg_dir.join("checkstyle.xml")).expect("read migrated file");
        assert!(on_disk.contains("<property name=\"max\" value=\"200\"/>"));
        assert!(!on_disk.contains("value=\"140\""));
    }

    #[test]
    fn should_not_touch_a_checkstyle_with_a_customised_line_length() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/java");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/java");
        let customised = checkstyle_with_line_length("<property name=\"max\" value=\"100\"/>");
        std::fs::write(pkg_dir.join("checkstyle.xml"), &customised).expect("write customised checkstyle.xml");

        let relative_path = Path::new("packages/java/checkstyle.xml");
        let changed = migrate_java_checkstyle_line_length(dir.path(), relative_path).expect("must not error");
        assert!(
            !changed,
            "a LineLength max outside alef's two known historical defaults must never be touched"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("checkstyle.xml")).expect("read file");
        assert_eq!(
            on_disk, customised,
            "customised checkstyle.xml must survive byte-for-byte"
        );
    }

    #[test]
    fn should_not_touch_a_checkstyle_already_at_200() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/java");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/java");
        let current = checkstyle_with_line_length("<property name=\"max\" value=\"200\"/>");
        std::fs::write(pkg_dir.join("checkstyle.xml"), &current).expect("write current checkstyle.xml");

        let relative_path = Path::new("packages/java/checkstyle.xml");
        let changed = migrate_java_checkstyle_line_length(dir.path(), relative_path).expect("must not error");
        assert!(!changed);
    }

    #[test]
    fn migrate_java_checkstyle_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = Path::new("packages/java/checkstyle.xml");
        let changed = migrate_java_checkstyle_line_length(dir.path(), relative_path).expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }
}
