use super::*;

const JAVA_E2E_POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <groupId>dev.sample_crate.sample_crawler</groupId>
    <artifactId>sample_crawler-e2e-java</artifactId>
    <version>0.1.0</version>

    <dependencies>
        <dependency>
            <groupId>dev.sample_crate.sample_crawler</groupId>
            <artifactId>sample_crawler</artifactId>
            <version>0.3.0-rc.27</version>
            <scope>system</scope>
            <systemPath>${project.basedir}/../../packages/java/target/sample_crawler-0.3.0-rc.27.jar</systemPath>
        </dependency>
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>${junit.version}</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>
"#;

#[test]
fn sync_e2e_java_pom_updates_dependency_version_and_system_path() {
    let result = sync_e2e_java_pom(JAVA_E2E_POM, "0.3.0-rc.28");
    assert!(result.is_some(), "expected Some when version changes");
    let new = result.unwrap();
    assert!(
        new.contains("<version>0.3.0-rc.28</version>"),
        "dependency version must be updated:\n{new}"
    );
    assert!(
        new.contains("sample_crawler-0.3.0-rc.28.jar"),
        "systemPath must be updated:\n{new}"
    );
    assert!(
        new.contains("<version>0.1.0</version>"),
        "project version must be unchanged:\n{new}"
    );
    assert!(
        new.contains("<version>${junit.version}</version>"),
        "junit version placeholder must be unchanged:\n{new}"
    );
    assert!(!new.contains("0.3.0-rc.27"), "old version must be removed:\n{new}");
}

#[test]
fn sync_e2e_java_pom_is_idempotent() {
    let first = sync_e2e_java_pom(JAVA_E2E_POM, "0.3.0-rc.28").unwrap();
    let second = sync_e2e_java_pom(&first, "0.3.0-rc.28");
    assert!(second.is_none(), "second call with same version must be a no-op");
}

#[test]
fn sync_e2e_java_pom_no_system_scope_returns_none() {
    let content = "<?xml version=\"1.0\"?>\n<project><version>0.1.0</version></project>\n";
    assert!(
        sync_e2e_java_pom(content, "1.0.0").is_none(),
        "no system-scope dep means nothing to update"
    );
}

const GO_MOD_E2E: &str = "\
module e2e_go

go 1.26

require (
\tgithub.com/sample_crate-dev/sample_crawler/packages/go v0.3.0-rc.27
\tgithub.com/stretchr/testify v1.11.1
)
";

const GO_MOD_E2E_LOCAL_REPLACE: &str = "\
module e2e_go

go 1.26

require (
\tgithub.com/sample_crate-dev/sample_crawler/packages/go v0.3.0-rc.27
\tgithub.com/stretchr/testify v1.11.1
)

replace github.com/sample_crate-dev/sample_crawler/packages/go => ../../packages/go
";

#[test]
fn sync_e2e_go_mod_updates_library_require_line() {
    let fragment = "github.com/sample_crate-dev/sample_crawler/packages/go";
    let result = sync_e2e_go_mod(GO_MOD_E2E, fragment, "0.3.0-rc.28");
    assert!(result.is_some(), "expected Some when version changes");
    let new = result.unwrap();
    assert!(
        new.contains("github.com/sample_crate-dev/sample_crawler/packages/go v0.3.0-rc.28"),
        "library require line must be updated:\n{new}"
    );
    assert!(
        new.contains("github.com/stretchr/testify v1.11.1"),
        "testify version must be unchanged:\n{new}"
    );
    assert!(!new.contains("v0.3.0-rc.27"), "old version must be gone:\n{new}");
}

#[test]
fn sync_e2e_go_mod_is_idempotent() {
    let fragment = "github.com/sample_crate-dev/sample_crawler/packages/go";
    let first = sync_e2e_go_mod(GO_MOD_E2E, fragment, "0.3.0-rc.28").unwrap();
    let second = sync_e2e_go_mod(&first, fragment, "0.3.0-rc.28");
    assert!(second.is_none(), "second call with same version must be a no-op");
}

#[test]
fn sync_e2e_go_mod_skips_local_replace_placeholder_version() {
    let fragment = "github.com/sample_crate-dev/sample_crawler/packages/go";
    let result = sync_e2e_go_mod(GO_MOD_E2E_LOCAL_REPLACE, fragment, "0.3.0-rc.28");
    assert!(
        result.is_none(),
        "local replace entries keep generated placeholder versions"
    );
}

const SWIFT_PACKAGE_FIRST_PARTY_FIRST: &str = "\
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: \"E2eSwift\",
    dependencies: [
        .package(url: \"https://github.com/example-org/example-swift-package\", from: \"1.10.1\"),
        .package(url: \"https://github.com/example-org/external-swift-package\", from: \"0.25.0\"),
    ]
)
";

const SWIFT_PACKAGE_EXTERNAL_FIRST: &str = "\
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: \"E2eSwift\",
    dependencies: [
        .package(url: \"https://github.com/example-org/external-swift-package\", from: \"0.25.0\"),
        .package(url: \"https://github.com/example-org/example-swift-package.git\", from: \"1.10.1\"),
    ]
)
";

const SWIFT_PACKAGE_ONLY_EXTERNAL: &str = "\
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: \"E2eSwift\",
    dependencies: [
        .package(path: \"../../packages/swift\"),
        .package(url: \"https://github.com/example-org/external-swift-package\", from: \"0.25.0\"),
    ]
)
";

const FIRST_PARTY_REPO: &str = "https://github.com/example-org/example-swift-package";

#[test]
fn sync_swift_first_party_from_bumps_first_party_only() {
    let result = sync_swift_first_party_from(SWIFT_PACKAGE_FIRST_PARTY_FIRST, FIRST_PARTY_REPO, "1.10.2");
    let new = result.expect("first-party version changed");
    assert!(
        new.contains("example-swift-package\", from: \"1.10.2\""),
        "first-party from: must bump:\n{new}"
    );
    assert!(
        new.contains("external-swift-package\", from: \"0.25.0\""),
        "external from: must be preserved:\n{new}"
    );
}

#[test]
fn sync_swift_first_party_from_preserves_external_when_listed_first() {
    let result = sync_swift_first_party_from(SWIFT_PACKAGE_EXTERNAL_FIRST, FIRST_PARTY_REPO, "1.10.2");
    let new = result.expect("first-party version changed");
    assert!(
        new.contains("external-swift-package\", from: \"0.25.0\""),
        "external dep listed first must not be clobbered:\n{new}"
    );
    assert!(
        new.contains("example-swift-package.git\", from: \"1.10.2\""),
        "first-party (.git URL) from: must bump:\n{new}"
    );
}

#[test]
fn sync_swift_first_party_from_no_op_without_first_party() {
    let result = sync_swift_first_party_from(SWIFT_PACKAGE_ONLY_EXTERNAL, FIRST_PARTY_REPO, "1.10.2");
    assert!(result.is_none(), "file with only external deps must be left untouched");
}

#[test]
fn sync_swift_first_party_from_is_idempotent() {
    let first = sync_swift_first_party_from(SWIFT_PACKAGE_FIRST_PARTY_FIRST, FIRST_PARTY_REPO, "1.10.2").unwrap();
    let second = sync_swift_first_party_from(&first, FIRST_PARTY_REPO, "1.10.2");
    assert!(second.is_none(), "second call with same version must be a no-op");
}

const DART_PUBSPEC_LOCK: &str = "\
# Generated by pub
packages:
  async:
    dependency: transitive
    description:
      name: async
      sha256: abc123
      url: \"https://pub.dev\"
    source: hosted
    version: \"1.19.1\"
  sample_crawler:
    dependency: \"direct main\"
    description:
      path: \"../../packages/dart\"
      relative: true
    source: path
    version: \"0.3.0-rc.23\"
  logging:
    dependency: transitive
    description:
      name: logging
      sha256: def456
      url: \"https://pub.dev\"
    source: hosted
    version: \"1.2.0\"
";

#[test]
fn sync_e2e_dart_pubspec_lock_updates_path_source_version() {
    let result = sync_e2e_dart_pubspec_lock(DART_PUBSPEC_LOCK, "0.3.0-rc.28");
    assert!(result.is_some(), "expected Some when version changes");
    let new = result.unwrap();
    assert!(
        new.contains("version: \"0.3.0-rc.28\""),
        "path-source version must be updated:\n{new}"
    );
    assert!(
        new.contains("version: \"1.19.1\""),
        "hosted async version must be unchanged:\n{new}"
    );
    assert!(
        new.contains("version: \"1.2.0\""),
        "hosted logging version must be unchanged:\n{new}"
    );
    assert!(!new.contains("0.3.0-rc.23"), "old version must be gone:\n{new}");
}

#[test]
fn sync_e2e_dart_pubspec_lock_is_idempotent() {
    let first = sync_e2e_dart_pubspec_lock(DART_PUBSPEC_LOCK, "0.3.0-rc.28").unwrap();
    let second = sync_e2e_dart_pubspec_lock(&first, "0.3.0-rc.28");
    assert!(second.is_none(), "second call with same version must be a no-op");
}

#[test]
fn sync_e2e_dart_pubspec_lock_no_path_source_returns_none() {
    let content = "packages:\n  async:\n    dependency: transitive\n    description:\n      name: async\n      url: \"https://pub.dev\"\n    source: hosted\n    version: \"1.19.1\"\n";
    assert!(
        sync_e2e_dart_pubspec_lock(content, "0.3.0-rc.28").is_none(),
        "no path-source means nothing to update"
    );
}

/// Regression test for the rc.13 incident: after `sync-versions` updates
/// `[crates.e2e.registry.packages.python].version` in alef.toml, the
/// generated `test_apps/python/pyproject.toml` must contain the new version
/// string rather than the stale prior version.
///
/// This test exercises `sync_versions` with `no_regen=false` (the default
/// for direct CLI invocations). The alef.toml has a minimal `[e2e]` block
/// with an empty fixtures directory so `generate_e2e` runs scaffold-only
/// (no IR extraction needed for pyproject.toml generation).
#[test]
fn sync_versions_regenerates_test_apps_pins() {
    use crate::core::config::NewAlefConfig;

    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.2.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    std::fs::create_dir_all(root.join("fixtures")).expect("mkdir fixtures");

    // default to empty string via #[serde(default)].
    let alef_toml = format!(
        concat!(
            "[workspace]\n",
            "languages = [\"python\"]\n\n",
            "[[crates]]\n",
            "name = \"mylib\"\n",
            "sources = []\n",
            "version_from = \"{cargo_toml}\"\n\n",
            "[crates.e2e]\n",
            "fixtures = \"fixtures\"\n",
            "languages = [\"python\"]\n\n",
            "[crates.e2e.call]\n",
            "module = \"mylib\"\n",
            "function = \"parse\"\n\n",
            "[crates.e2e.registry.packages.python]\n",
            "name = \"mylib\"\n",
            "version = \"0.0.0\"\n",
        ),
        cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, false, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let updated_toml = std::fs::read_to_string(&alef_toml_path).expect("read alef.toml");
    assert!(
        updated_toml.contains("version = \"1.2.3\""),
        "alef.toml registry package version must be updated to 1.2.3:\n{updated_toml}"
    );
    assert!(
        !updated_toml.contains("version = \"0.0.0\""),
        "stale 0.0.0 must be gone from alef.toml:\n{updated_toml}"
    );

    let pyproject_path = root.join("test_apps/python/pyproject.toml");
    assert!(
        pyproject_path.exists(),
        "test_apps/python/pyproject.toml must be generated by auto-regen"
    );
    let pyproject = std::fs::read_to_string(&pyproject_path).expect("read pyproject.toml");
    assert!(
        pyproject.contains("mylib==1.2.3"),
        "test_apps/python/pyproject.toml must pin the new registry version 1.2.3:\n{pyproject}"
    );
    assert!(
        !pyproject.contains("mylib==0.0.0"),
        "stale registry pin mylib==0.0.0 must be gone from test_apps/python/pyproject.toml:\n{pyproject}"
    );
}

/// Regression test for the rc.13/rc.14 incident: `sync-versions` must rewrite
/// `moduleVersion = "..."` in `packages/go/cmd/download_ffi/main.go` so that
/// Go module consumers of a freshly released version pull the correct FFI binary.
///
/// Without this fix, the developer's `alef all` (run before the version bump)
/// bakes the old version into main.go; the subsequent `sync-versions` bump
/// updated Cargo.toml and other manifests but left main.go stale, causing
/// Go consumers to pull the previous release's FFI binary.
#[test]
fn sync_versions_updates_go_module_version_in_download_ffi() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.9.0-rc.14\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let download_ffi_dir = root.join("packages/go/cmd/download_ffi");
    std::fs::create_dir_all(&download_ffi_dir).expect("mkdir download_ffi");
    let stale_main_go = concat!(
        "// Tool to download platform-specific FFI libraries from GitHub releases.\n",
        "package main\n\nconst (\n",
        "\tmoduleVersion = \"1.9.0-rc.13\"\n",
        "\trepoURL       = \"https://github.com/example/mylib\"\n",
        ")\n",
    );
    std::fs::write(download_ffi_dir.join("main.go"), stale_main_go).expect("write main.go");

    let alef_toml = format!(
        concat!(
            "[workspace]\nlanguages = [\"go\"]\n\n",
            "[[crates]]\nname = \"mylib\"\nsources = []\n",
            "version_from = \"{cargo_toml}\"\n",
        ),
        cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let updated_main = std::fs::read_to_string(download_ffi_dir.join("main.go")).expect("read main.go");
    assert!(
        updated_main.contains("moduleVersion = \"1.9.0-rc.14\""),
        "moduleVersion must be updated to 1.9.0-rc.14:\n{updated_main}"
    );
    assert!(
        !updated_main.contains("1.9.0-rc.13"),
        "stale rc.13 moduleVersion must be gone from main.go:\n{updated_main}"
    );
    assert!(
        updated_main.contains("repoURL"),
        "other constants must be preserved:\n{updated_main}"
    );
}

/// Regression test: after `sync-versions` bumps the workspace version, the
/// scaffold generator must be re-run so that scaffold files embedding the
/// version (R DESCRIPTION, Dart pubspec.yaml, Ruby gemspec, etc.) reflect
/// the new version atomically with the workspace bump.
///
/// This test uses the R backend because `packages/r/DESCRIPTION` embeds
/// `Version: X.Y.Z` at scaffold time and is therefore the canonical
/// scaffold-side version surface not covered by the existing text-replacement
/// pass in `sync_versions`.
#[test]
fn sync_versions_regenerates_scaffold_version_fields() {
    use crate::core::config::NewAlefConfig;

    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.2.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    std::fs::create_dir_all(root.join("packages/r")).expect("mkdir packages/r");
    let stale_description = concat!(
        "Package: mylib\nTitle: My Library\nVersion: 0.0.0\nDescription: A library.\n",
        "License: MIT\nEncoding: UTF-8\nRoxygenNote: 7.3.1\n",
    );
    std::fs::write(root.join("packages/r/DESCRIPTION"), stale_description).expect("write DESCRIPTION");

    let alef_toml = format!(
        concat!(
            "[workspace]\n",
            "languages = [\"r\"]\n\n",
            "[[crates]]\n",
            "name = \"mylib\"\n",
            "sources = []\n",
            "version_from = \"{cargo_toml}\"\n",
        ),
        cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, false, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let description_path = root.join("packages/r/DESCRIPTION");
    assert!(
        description_path.exists(),
        "packages/r/DESCRIPTION must exist after scaffold regen"
    );
    let description = std::fs::read_to_string(&description_path).expect("read DESCRIPTION");
    assert!(
        description.contains("Version: 1.2"),
        "DESCRIPTION must contain the new version 1.2.x after scaffold regen:\n{description}"
    );
    assert!(
        !description.contains("Version: 0.0.0"),
        "stale Version: 0.0.0 must be gone from DESCRIPTION:\n{description}"
    );
}

/// `sync_versions` must bump `version = "..."` inside `packages/kotlin-android/build.gradle.kts`
/// (the `coordinates()` block version) and remove stale AGP 8-era Kotlin Android plugin lines.
#[test]
fn sync_versions_bumps_kotlin_android_gradle_coordinates_version() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let gradle_content = concat!(
        "plugins {\n",
        "    id(\"com.android.library\") version \"9.2.0\"\n",
        "    kotlin(\"android\") version \"2.4.0\"\n",
        "}\n",
        "\n",
        "mavenPublishing {\n",
        "    coordinates(\n",
        "        groupId = \"dev.example\",\n",
        "        artifactId = \"mylib-android\",\n",
        "        version = \"1.9.0-rc.16\",\n",
        "    )\n",
        "}\n",
    );
    std::fs::create_dir_all(root.join("packages/kotlin-android")).expect("mkdir");
    std::fs::write(root.join("packages/kotlin-android/build.gradle.kts"), gradle_content)
        .expect("write build.gradle.kts");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"kotlin_android\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let gradle =
        std::fs::read_to_string(root.join("packages/kotlin-android/build.gradle.kts")).expect("read build.gradle.kts");
    assert!(
        gradle.contains("version = \"1.9.0-rc.17\""),
        "kotlin-android coordinates version must be bumped:\n{gradle}"
    );
    assert!(
        !gradle.contains(r#"kotlin("android")"#),
        "AGP 9.0+ ships built-in Kotlin; the explicit kotlin(\"android\") plugin line must be absent:\n{gradle}"
    );
    assert!(
        gradle.contains(r#"id("com.android.library") version "9.2.0""#),
        "android plugin version must not change:\n{gradle}"
    );
    assert!(
        !gradle.contains("1.9.0-rc.16"),
        "stale rc.16 version must be gone:\n{gradle}"
    );
}

/// `sync_versions` must find `__version__ = "..."` in a nested module `__init__.py`
/// under `packages/python/<module>/` (src layout), not just a flat `packages/python/__init__.py`.
#[test]
fn sync_versions_bumps_nested_python_init_version() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let py_module_dir = root.join("packages/python/mylib");
    std::fs::create_dir_all(&py_module_dir).expect("mkdir");
    std::fs::write(
        py_module_dir.join("__init__.py"),
        "\"\"\"mylib public API.\"\"\"\n\n__version__ = \"1.9.0-rc.16\"\n",
    )
    .expect("write __init__.py");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"python\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let content = std::fs::read_to_string(py_module_dir.join("__init__.py")).expect("read __init__.py");
    assert!(
        content.contains("__version__ = \"1.9.0-rc.17\""),
        "nested __version__ must be bumped:\n{content}"
    );
    assert!(
        !content.contains("1.9.0-rc.16"),
        "stale rc.16 __version__ must be gone:\n{content}"
    );
}

/// `sync_versions` must bump the `from: "X.Y.Z"` version pin in
/// `test_apps/swift/Package.swift` without touching the rest of the file.
#[test]
fn sync_versions_bumps_swift_package_from_version() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let swift_pkg_content = concat!(
        "// swift-tools-version: 6.0\n",
        "import PackageDescription\n",
        "\n",
        "let package = Package(\n",
        "    name: \"TestApp\",\n",
        "    dependencies: [\n",
        "        .package(url: \"https://example.com/alef-sample/mylib.git\", from: \"1.9.0-rc.16\"),\n",
        "    ],\n",
        "    targets: []\n",
        ")\n",
    );
    let swift_dir = root.join("test_apps/swift");
    std::fs::create_dir_all(&swift_dir).expect("mkdir");
    std::fs::write(swift_dir.join("Package.swift"), swift_pkg_content).expect("write Package.swift");

    let alef_toml = format!(
        concat!(
            "[workspace]\n",
            "languages = [\"swift\"]\n",
            "[workspace.scaffold]\n",
            "repository = \"https://example.com/alef-sample\"\n",
            "[[crates]]\n",
            "name = \"mylib\"\n",
            "sources = []\n",
            "version_from = \"{}\"\n",
        ),
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let swift_pkg = std::fs::read_to_string(swift_dir.join("Package.swift")).expect("read Package.swift");
    assert!(
        swift_pkg.contains("from: \"1.9.0-rc.17\""),
        "swift from: version must be bumped:\n{swift_pkg}"
    );
    assert!(
        !swift_pkg.contains("from: \"1.9.0-rc.16\""),
        "stale rc.16 from: version must be gone:\n{swift_pkg}"
    );
    assert!(
        swift_pkg.contains("https://example.com/alef-sample/mylib.git"),
        "repo URL must be preserved:\n{swift_pkg}"
    );
}

/// `sync_versions` must substitute `v__ALEF_SWIFT_VERSION__` in the root
/// `Package.swift` and the substitution must SURVIVE the in-band scaffold
/// regen pass that runs immediately after the text-replacement loop.
///
/// Regression: the binaryTarget root manifest emitter writes the file with
/// `v__ALEF_SWIFT_VERSION__` as a placeholder so the in-VCS file stays
/// stable across version bumps. `regenerate_scaffold_after_sync` then
/// overwrites the substituted manifest with the placeholder form. Without
/// a second-pass substitution at the end of `sync_versions`, the on-disk
/// `Package.swift` permanently points at the literal
/// `…/releases/download/v__ALEF_SWIFT_VERSION__/…` URL and SwiftPM
/// resolution fails for downstream consumers.
#[test]
fn sync_versions_root_package_swift_placeholder_survives_scaffold_regen() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let root_pkg_content = concat!(
        "// swift-tools-version: 6.0\n",
        "import PackageDescription\n",
        "let package = Package(name: \"MyLib\", targets: [\n",
        "  .binaryTarget(\n",
        "    name: \"RustBridge\",\n",
        "    url: \"https://example.com/alef-sample/mylib/releases/download/v__ALEF_SWIFT_VERSION__/MyLib-rs.artifactbundle.zip\",\n",
        "    checksum: \"__ALEF_SWIFT_CHECKSUM__\"\n",
        "  ),\n",
        "])\n",
    );
    std::fs::write(root.join("Package.swift"), root_pkg_content).expect("write root Package.swift");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, false, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let root_pkg = std::fs::read_to_string(root.join("Package.swift")).expect("read root Package.swift");
    assert!(
        !root_pkg.contains("v__ALEF_SWIFT_VERSION__"),
        "root Package.swift must not retain the version placeholder after sync_versions, got:\n{root_pkg}"
    );
    assert!(
        root_pkg.contains("/releases/download/v1.9.0-rc.17/"),
        "root Package.swift URL must point at substituted version v1.9.0-rc.17, got:\n{root_pkg}"
    );
    assert!(
        root_pkg.contains("__ALEF_SWIFT_CHECKSUM__"),
        "root Package.swift must retain the checksum placeholder when skip_swift_checksum=true, got:\n{root_pkg}"
    );
}

/// `sync_versions` must bump `VERSION="X.Y.Z"` (no spaces around `=`) in
/// both `e2e/c/download_ffi.sh` and `test_apps/c/download_ffi.sh`.
#[test]
fn sync_versions_bumps_c_download_ffi_sh_version() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let sh_content = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail\n",
        "\n",
        "REPO_URL=\"https://example.com/alef-sample/mylib\"\n",
        "VERSION=\"1.9.0-rc.16\"\n",
        "FFI_PKG_NAME=\"mylib-ffi\"\n",
    );

    let e2e_c_dir = root.join("e2e/c");
    std::fs::create_dir_all(&e2e_c_dir).expect("mkdir e2e/c");
    std::fs::write(e2e_c_dir.join("download_ffi.sh"), sh_content).expect("write e2e download_ffi.sh");

    let test_apps_c_dir = root.join("test_apps/c");
    std::fs::create_dir_all(&test_apps_c_dir).expect("mkdir test_apps/c");
    std::fs::write(test_apps_c_dir.join("download_ffi.sh"), sh_content).expect("write test_apps download_ffi.sh");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"c\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    for (label, dir) in [("e2e", &e2e_c_dir), ("test_apps", &test_apps_c_dir)] {
        let content = std::fs::read_to_string(dir.join("download_ffi.sh"))
            .unwrap_or_else(|_| panic!("read {label}/c/download_ffi.sh"));
        assert!(
            content.contains("VERSION=\"1.9.0-rc.17\""),
            "{label}/c/download_ffi.sh VERSION must be bumped:\n{content}"
        );
        assert!(
            !content.contains("VERSION=\"1.9.0-rc.16\""),
            "{label}/c/download_ffi.sh stale rc.16 must be gone:\n{content}"
        );
        assert!(
            content.contains("REPO_URL="),
            "{label}/c/download_ffi.sh REPO_URL must be preserved:\n{content}"
        );
    }
}
