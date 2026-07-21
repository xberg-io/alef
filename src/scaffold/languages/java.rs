use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::{scaffold::parse_author, scaffold::scaffold_meta, scaffold::xml_escape};
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

    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <groupId>{group_id}</groupId>
    <artifactId>{name}</artifactId>
    <version>{version}</version>
    <packaging>jar</packaging>

    <name>{name}</name>
    <description>{description}</description>
    <url>{repository}</url>

    <licenses>
        <license>
            <name>{license}</name>{license_url}
        </license>
    </licenses>
{developers}
    <scm>
        <connection>{scm_connection}</connection>
        <developerConnection>{scm_developer_connection}</developerConnection>
        <url>{repository}</url>
    </scm>

    <properties>
        <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
        <maven.compiler.release>25</maven.compiler.release>
        <junit.version>5.11.4</junit.version>
        <maven.version>3.9.11</maven.version>
        <maven-compiler-plugin.version>3.15.0</maven-compiler-plugin.version>
        <maven-surefire-plugin.version>3.5.5</maven-surefire-plugin.version>
        <maven-checkstyle-plugin.version>3.6.0</maven-checkstyle-plugin.version>
        <maven-pmd-plugin.version>3.28.0</maven-pmd-plugin.version>
        <maven-source-plugin.version>3.4.0</maven-source-plugin.version>
        <maven-javadoc-plugin.version>3.12.0</maven-javadoc-plugin.version>
        <maven-gpg-plugin.version>3.2.8</maven-gpg-plugin.version>
        <maven-clean-plugin.version>3.4.1</maven-clean-plugin.version>
        <maven-resources-plugin.version>3.3.1</maven-resources-plugin.version>
        <maven-jar-plugin.version>3.4.2</maven-jar-plugin.version>
        <maven-install-plugin.version>3.1.3</maven-install-plugin.version>
        <maven-deploy-plugin.version>3.1.3</maven-deploy-plugin.version>
        <maven-site-plugin.version>4.0.0-M16</maven-site-plugin.version>
        <central-publishing-plugin.version>0.10.0</central-publishing-plugin.version>
        <versions-maven-plugin.version>2.21.0</versions-maven-plugin.version>
        <maven-enforcer-plugin.version>3.6.2</maven-enforcer-plugin.version>
        <jacoco-maven-plugin.version>0.8.14</jacoco-maven-plugin.version>
        <checkstyle.version>13.4.0</checkstyle.version>
        <pmd.version>7.17.0</pmd.version>
        <gpg.skip>true</gpg.skip>
    </properties>

    <dependencies>
        <dependency>
            <groupId>org.jspecify</groupId>
            <artifactId>jspecify</artifactId>
            <version>1.0.0</version>
        </dependency>
        <dependency>
            <groupId>com.fasterxml.jackson.core</groupId>
            <artifactId>jackson-databind</artifactId>
            <version>2.21.2</version>
        </dependency>
        <dependency>
            <groupId>com.fasterxml.jackson.datatype</groupId>
            <artifactId>jackson-datatype-jdk8</artifactId>
            <version>2.21.2</version>
        </dependency>
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>${{junit.version}}</version>
            <scope>test</scope>
        </dependency>
        <dependency>
            <groupId>org.assertj</groupId>
            <artifactId>assertj-core</artifactId>
            <version>4.0.0-M1</version>
            <scope>test</scope>
        </dependency>{capsule_deps}
    </dependencies>

    <build>
        <!-- The alef Java backend emits source files at the package root
             (e.g. packages/java/{source_root}/<group>/<artifact>/Foo.java), not
             under the Maven-default `src/main/java/` layout. Point sourceDirectory
             at the package root so `mvn package` finds them. -->
        <sourceDirectory>${{project.basedir}}</sourceDirectory>
        <resources>
            <resource>
                <directory>src/main/resources</directory>
            </resource>
        </resources>
        <pluginManagement>
            <plugins>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-clean-plugin</artifactId>
                    <version>${{maven-clean-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-resources-plugin</artifactId>
                    <version>${{maven-resources-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-jar-plugin</artifactId>
                    <version>${{maven-jar-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-install-plugin</artifactId>
                    <version>${{maven-install-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-deploy-plugin</artifactId>
                    <version>${{maven-deploy-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-site-plugin</artifactId>
                    <version>${{maven-site-plugin.version}}</version>
                </plugin>
            </plugins>
        </pluginManagement>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-compiler-plugin</artifactId>
                <version>${{maven-compiler-plugin.version}}</version>
                <configuration>
                    <release>25</release>
                    <compilerArgs>
                        <arg>--enable-preview</arg>
                    </compilerArgs>
                </configuration>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-surefire-plugin</artifactId>
                <version>${{maven-surefire-plugin.version}}</version>
                <configuration>
                    <argLine>@{{argLine}} -XX:-ClassUnloading -XX:-ClassUnloadingWithConcurrentMark --enable-native-access=ALL-UNNAMED --enable-preview -Djava.library.path=${{project.basedir}}/../../target/release</argLine>
                    <forkedProcessExitTimeoutInSeconds>600</forkedProcessExitTimeoutInSeconds>
                    <parallel>classes</parallel>
                    <threadCount>4</threadCount>
                    <redirectTestOutputToFile>true</redirectTestOutputToFile>
                </configuration>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-jar-plugin</artifactId>
                <version>${{maven-jar-plugin.version}}</version>
                <configuration>
                    <!-- Bind the ${{classifier}} property so native JARs are emitted
                         with the correct classifier (e.g. osx-aarch64, linux-x64, …).
                         When CI runs `mvn package -Dclassifier=osx-aarch64`, Maven
                         passes the classifier through to this configuration, allowing
                         the JAR to be published as example-language-pack-java-1.9.0-osx-aarch64.jar
                         instead of example-language-pack-java-1.9.0.jar. -->
                    <classifier>${{classifier}}</classifier>
                </configuration>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-source-plugin</artifactId>
                <version>${{maven-source-plugin.version}}</version>
                <configuration>
                    <!-- sourceDirectory is the project basedir, so the default
                         source-archive include of everything under basedir
                         pulls in target/ as well (which contains the archive
                         being assembled — "A zip file cannot include itself").
                         Restrict to the alef-emitted {source_root}/ subtree and
                         any conventional `src/main/java/` overlay. -->
                    <includes>
                        <include>{source_root}/**/*.java</include>
                        <include>src/main/java/**/*.java</include>
                    </includes>
                </configuration>
                <executions>
                    <execution>
                        <id>attach-sources</id>
                        <goals>
                            <goal>jar-no-fork</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-javadoc-plugin</artifactId>
                <version>${{maven-javadoc-plugin.version}}</version>
                <configuration>
                    <doclint>all,-missing</doclint>
                    <failOnWarning>true</failOnWarning>
                    <show>protected</show>
                    <additionalOptions>--enable-preview</additionalOptions>
                    <!-- sourcepath MUST match <sourceDirectory> above (which is
                         ${{project.basedir}} for the flat layout alef emits) — the
                         Maven-default `src/main/java/` does not exist in our tree,
                         so attach-javadocs found no sources and skipped jar
                         creation, which Sonatype Central rejected as
                         "Javadocs must be provided but not found in entries". -->
                    <sourcepath>${{project.basedir}}</sourcepath>
                </configuration>
                <executions>
                    <execution>
                        <id>attach-javadocs</id>
                        <goals>
                            <goal>jar</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-enforcer-plugin</artifactId>
                <version>${{maven-enforcer-plugin.version}}</version>
                <executions>
                    <execution>
                        <id>enforce-maven</id>
                        <goals>
                            <goal>enforce</goal>
                        </goals>
                        <configuration>
                            <rules>
                                <requireMavenVersion>
                                    <version>${{maven.version}}</version>
                                </requireMavenVersion>
                            </rules>
                        </configuration>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-checkstyle-plugin</artifactId>
                <version>${{maven-checkstyle-plugin.version}}</version>
                <dependencies>
                    <dependency>
                        <groupId>com.puppycrawl.tools</groupId>
                        <artifactId>checkstyle</artifactId>
                        <version>${{checkstyle.version}}</version>
                    </dependency>
                </dependencies>
                <configuration>
                    <configLocation>${{project.basedir}}/checkstyle.xml</configLocation>
                    <propertiesLocation>${{project.basedir}}/checkstyle.properties</propertiesLocation>
                    <consoleOutput>true</consoleOutput>
                    <failsOnError>true</failsOnError>
                    <violationSeverity>warning</violationSeverity>
                    <propertyExpansion>config_loc=${{project.basedir}}</propertyExpansion>
                </configuration>
                <executions>
                    <execution>
                        <id>validate</id>
                        <phase>validate</phase>
                        <goals>
                            <goal>check</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-pmd-plugin</artifactId>
                <version>${{maven-pmd-plugin.version}}</version>
                <dependencies>
                    <dependency>
                        <groupId>net.sourceforge.pmd</groupId>
                        <artifactId>pmd-java</artifactId>
                        <version>${{pmd.version}}</version>
                    </dependency>
                </dependencies>
                <configuration>
                    <targetJdk>${{maven.compiler.release}}</targetJdk>
                    <typeResolution>false</typeResolution>
                    <rulesets>
                        <ruleset>/rulesets/java/quickstart.xml</ruleset>
                    </rulesets>
                    <!--
                        CPD threshold raised above the default 100 tokens because alef-generated
                        streaming method bodies (`streamItems`, `batchStreamItems`, etc.) share
                        an identical iterator-driving loop by design (per-stream-handle JNI
                        externs differ, the surrounding plumbing is the same). The shared block
                        is ~106 tokens — well within the default. 200 is the smallest threshold
                        that lets two streaming methods coexist in the same handle class without
                        a false-positive while still catching genuine large-scale duplication.
                    -->
                    <minimumTokens>200</minimumTokens>
                </configuration>
                <executions>
                    <execution>
                        <goals>
                            <goal>pmd</goal>
                            <goal>cpd-check</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.codehaus.mojo</groupId>
                <artifactId>versions-maven-plugin</artifactId>
                <version>${{versions-maven-plugin.version}}</version>
                <configuration>
                    <generateBackupPoms>false</generateBackupPoms>
                    <rulesUri>file://${{project.basedir}}/versions-rules.xml</rulesUri>
                </configuration>
            </plugin>
            <plugin>
                <groupId>org.jacoco</groupId>
                <artifactId>jacoco-maven-plugin</artifactId>
                <version>${{jacoco-maven-plugin.version}}</version>
                <configuration>
                    <excludes>
                        <exclude>java/**/*</exclude>
                        <exclude>sun/**/*</exclude>
                        <exclude>jdk/**/*</exclude>
                    </excludes>
                </configuration>
                <executions>
                    <execution>
                        <goals>
                            <goal>prepare-agent</goal>
                        </goals>
                    </execution>
                    <execution>
                        <id>report</id>
                        <phase>test</phase>
                        <goals>
                            <goal>report</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-gpg-plugin</artifactId>
                <version>${{maven-gpg-plugin.version}}</version>
                <executions>
                    <execution>
                        <id>sign-artifacts</id>
                        <phase>verify</phase>
                        <goals>
                            <goal>sign</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
        </plugins>
    </build>

    <profiles>
        <profile>
            <id>publish</id>
            <properties>
                <gpg.skip>false</gpg.skip>
                <!-- alef-emitted stream methods can exceed 200 tokens and trigger CPD/PMD
                     duplicate-code violations; skip those checks in the publish profile so
                     they do not block Maven Central deployment. -->
                <cpd.skip>true</cpd.skip>
                <pmd.skip>true</pmd.skip>
            </properties>
            <build>
                <plugins>
                    <plugin>
                        <groupId>org.apache.maven.plugins</groupId>
                        <artifactId>maven-deploy-plugin</artifactId>
                        <configuration>
                            <skip>true</skip>
                        </configuration>
                    </plugin>
                    <plugin>
                        <groupId>org.apache.maven.plugins</groupId>
                        <artifactId>maven-gpg-plugin</artifactId>
                        <version>${{maven-gpg-plugin.version}}</version>
                        <executions>
                            <execution>
                                <id>sign-artifacts</id>
                                <phase>verify</phase>
                                <goals>
                                    <goal>sign</goal>
                                </goals>
                                <configuration>
                                    <passphraseEnvName>MAVEN_GPG_PASSPHRASE</passphraseEnvName>
                                    <gpgArguments>
                                        <arg>--batch</arg>
                                        <arg>--yes</arg>
                                        <arg>--pinentry-mode=loopback</arg>
                                    </gpgArguments>
                                </configuration>
                            </execution>
                        </executions>
                    </plugin>
                    <plugin>
                        <groupId>org.sonatype.central</groupId>
                        <artifactId>central-publishing-maven-plugin</artifactId>
                        <version>${{central-publishing-plugin.version}}</version>
                        <extensions>true</extensions>
                        <configuration>
                            <publishingServerId>ossrh</publishingServerId>
                            <autoPublish>true</autoPublish>
                            <waitUntil>validated</waitUntil>
                            <waitMaxTime>7200</waitMaxTime>
                        </configuration>
                    </plugin>
                </plugins>
            </build>
        </profile>
    </profiles>
</project>
"#,
        group_id = group_id,
        name = name,
        version = version,
        description = meta.description,
        repository = repo_url,
        license = license,
        license_url = license_url_xml,
        developers = developers_xml,
        scm_connection = scm.connection,
        scm_developer_connection = scm.developer_connection,
        capsule_deps = java_capsule_dependencies(config),
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
