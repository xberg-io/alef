use crate::{parse_author, scaffold_meta, xml_escape};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

pub(crate) fn scaffold_java(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let name = &config.crate_config.name;
    let version = &api.version;

    // Derive SCM URLs from repository URL
    let repo_url = &meta.repository;
    let repo_path = repo_url
        .strip_prefix("https://github.com/")
        .or_else(|| repo_url.strip_prefix("http://github.com/"))
        .unwrap_or(repo_url.trim_start_matches("https://"));

    let group_id = config.java_group_id();

    // Build developers XML from authors
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

    // License URL mapping
    let license_url = match meta.license.as_str() {
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
        <connection>scm:git:git://github.com/{repo_path}.git</connection>
        <developerConnection>scm:git:ssh://github.com:{repo_path}.git</developerConnection>
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
        <spotless-maven-plugin.version>3.4.0</spotless-maven-plugin.version>
        <versions-maven-plugin.version>2.21.0</versions-maven-plugin.version>
        <maven-enforcer-plugin.version>3.6.2</maven-enforcer-plugin.version>
        <jacoco-maven-plugin.version>0.8.14</jacoco-maven-plugin.version>
        <checkstyle.version>13.4.0</checkstyle.version>
        <pmd.version>7.19.0</pmd.version>
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
        </dependency>
    </dependencies>

    <build>
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
                <groupId>com.diffplug.spotless</groupId>
                <artifactId>spotless-maven-plugin</artifactId>
                <version>${{spotless-maven-plugin.version}}</version>
                <configuration>
                    <java>
                        <eclipse>
                            <version>4.31</version>
                            <file>${{project.basedir}}/eclipse-formatter.xml</file>
                        </eclipse>
                    </java>
                </configuration>
                <executions>
                    <execution>
                        <goals>
                            <goal>apply</goal>
                        </goals>
                        <phase>process-sources</phase>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-source-plugin</artifactId>
                <version>${{maven-source-plugin.version}}</version>
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
                    <show>protected</show>
                    <additionalOptions>--enable-preview</additionalOptions>
                    <sourcepath>${{project.basedir}}/src/main/java</sourcepath>
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
                            <waitUntil>published</waitUntil>
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
        license = meta.license,
        license_url = license_url_xml,
        developers = developers_xml,
        repo_path = repo_path,
    );

    let checkstyle_xml = r#"<?xml version="1.0"?>
<!DOCTYPE module PUBLIC
    "-//Checkstyle//DTD Checkstyle Configuration 1.3//EN"
    "https://checkstyle.org/dtds/configuration_1_3.dtd">

<!-- Checkstyle handles correctness checks only. Spotless handles all formatting. -->
<module name="Checker">
    <property name="charset" value="UTF-8"/>
    <property name="severity" value="error"/>
    <property name="fileExtensions" value="java"/>

    <module name="SuppressionFilter">
        <property name="file" value="${config_loc}/checkstyle-suppressions.xml"/>
        <property name="optional" value="false"/>
    </module>

    <module name="LineLength">
        <property name="max" value="120"/>
        <property name="ignorePattern" value="^package.*|^import.*|a]href|href|http://|https://|ftp://"/>
    </module>

    <module name="TreeWalker">
        <!-- Naming Conventions -->
        <module name="ConstantName">
            <property name="format" value="^([A-Z][A-Z0-9]*(_[A-Z0-9]+)*|[a-z_]+)$"/>
        </module>
        <module name="LocalFinalVariableName"/>
        <module name="LocalVariableName"/>
        <module name="MemberName"/>
        <module name="MethodName"/>
        <module name="PackageName"/>
        <module name="ParameterName"/>
        <module name="TypeName"/>

        <!-- Imports -->
        <module name="AvoidStarImport">
            <property name="allowStaticMemberImports" value="true"/>
        </module>
        <module name="RedundantImport"/>
        <module name="UnusedImports"/>

        <!-- Size Violations -->
        <module name="MethodLength">
            <property name="max" value="150"/>
        </module>

        <!-- Modifier Checks -->
        <module name="ModifierOrder"/>
        <module name="RedundantModifier"/>

        <!-- Coding -->
        <module name="EmptyStatement"/>
        <module name="EqualsHashCode"/>
        <module name="SimplifyBooleanExpression"/>
        <module name="SimplifyBooleanReturn"/>

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

    let eclipse_formatter_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<profiles version="21">
    <profile kind="CodeFormatterProfile" name="Kreuzberg" version="21">
        <setting id="org.eclipse.jdt.core.formatter.lineSplit" value="120"/>
        <setting id="org.eclipse.jdt.core.formatter.tabulation.char" value="space"/>
        <setting id="org.eclipse.jdt.core.formatter.tabulation.size" value="4"/>
        <setting id="org.eclipse.jdt.core.formatter.indentation.size" value="4"/>
        <setting id="org.eclipse.jdt.core.formatter.comment.line_length" value="120"/>
    </profile>
</profiles>
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
            path: PathBuf::from("packages/java/eclipse-formatter.xml"),
            content: eclipse_formatter_xml.to_string(),
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
    </rule>
    <rule ref="category/java/codestyle.xml">
        <exclude name="AtLeastOneConstructor"/>
        <exclude name="CommentDefaultAccessModifier"/>
        <exclude name="OnlyOneReturn"/>
    </rule>
    <rule ref="category/java/design.xml">
        <exclude name="LawOfDemeter"/>
        <exclude name="DataClass"/>
    </rule>
    <rule ref="category/java/documentation.xml">
        <exclude name="CommentSize"/>
    </rule>
    <rule ref="category/java/errorprone.xml">
        <exclude name="EmptyCatchBlock"/>
    </rule>
    <rule ref="category/java/multithreading.xml"/>
    <rule ref="category/java/performance.xml"/>
    <rule ref="category/java/security.xml"/>
</ruleset>
"#
            .to_string(),
            generated_header: false,
        },
    ])
}
