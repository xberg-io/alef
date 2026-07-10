use super::resolve_workspace_root;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Validate that all package manifests are ready for publishing.
///
/// Checks:
/// - All required package directories exist
/// - Key manifest files are present (pyproject.toml, package.json, gemspec, etc.)
/// - Cargo.toml version can be read
pub fn validate(config: &ResolvedCrateConfig, languages: &[Language]) -> Result<Vec<String>> {
    let mut issues = Vec::new();
    let workspace_root = resolve_workspace_root(config);
    let workspace_path = Path::new(&workspace_root);

    if config.resolved_version().is_none() {
        issues.push(format!("cannot read version from {}", config.version_from));
    }

    for &lang in languages {
        let pkg_dir = config.package_dir(lang);
        let pkg_path = workspace_path.join(&pkg_dir);

        if matches!(lang, Language::Rust | Language::Ffi | Language::Jni) {
            continue;
        }

        if !pkg_path.exists() {
            issues.push(format!("{lang}: package directory {pkg_dir} does not exist"));
            continue;
        }

        let expected_files: Vec<&str> = match lang {
            Language::Python => vec!["pyproject.toml"],
            Language::Node => vec!["package.json"],
            Language::Ruby => vec![],
            Language::Php => vec!["composer.json"],
            Language::Elixir => vec!["mix.exs"],
            Language::Go => vec!["go.mod"],
            Language::Java => vec!["pom.xml"],
            Language::Csharp => vec![],
            Language::Wasm => vec![],
            Language::R => vec!["DESCRIPTION"],
            Language::Kotlin => vec!["build.gradle.kts"],
            Language::Gleam => vec!["gleam.toml"],
            Language::Zig => vec!["build.zig"],
            Language::Dart => vec!["pubspec.yaml"],
            Language::Swift => vec!["Package.swift"],
            _ => vec![],
        };

        for file in expected_files {
            if !pkg_path.join(file).exists() {
                issues.push(format!("{lang}: missing {pkg_dir}/{file}"));
            }
        }

        if lang == Language::Ruby {
            validate_ruby_gemspecs(&pkg_path, &pkg_dir, &mut issues);
        }
        validate_language_manifest(config, lang, workspace_path, &pkg_dir, &pkg_path, &mut issues);
    }

    Ok(issues)
}

fn validate_language_manifest(
    config: &ResolvedCrateConfig,
    lang: Language,
    workspace_root: &Path,
    pkg_dir: &str,
    pkg_path: &Path,
    issues: &mut Vec<String>,
) {
    match lang {
        Language::Elixir => validate_elixir_manifest(config, pkg_dir, pkg_path, issues),
        Language::Php => validate_php_manifests(pkg_dir, pkg_path, workspace_root, issues),
        Language::Csharp => validate_csharp_project(config, workspace_root, pkg_dir, issues),
        Language::Go => validate_go_module(config, pkg_dir, pkg_path, issues),
        Language::Java => validate_java_manifest(config, pkg_dir, pkg_path, issues),
        Language::Dart => validate_dart_manifest(config, pkg_dir, pkg_path, issues),
        Language::Swift => validate_swift_manifest(pkg_dir, pkg_path, issues),
        Language::Zig => validate_zig_manifest(config, pkg_dir, pkg_path, issues),
        _ => {}
    }
}

fn validate_elixir_manifest(config: &ResolvedCrateConfig, pkg_dir: &str, pkg_path: &Path, issues: &mut Vec<String>) {
    let mix_path = pkg_path.join("mix.exs");
    let Ok(content) = std::fs::read_to_string(&mix_path) else {
        return;
    };
    let targets = elixir_nif_targets(config).join(" ");
    if !content.contains(&format!("targets: ~w({targets})")) {
        issues.push(format!(
            "elixir: {pkg_dir}/mix.exs rustler_crates targets must match configured nif_targets: {targets}"
        ));
    }
}

fn validate_php_manifests(pkg_dir: &str, pkg_path: &Path, workspace_root: &Path, issues: &mut Vec<String>) {
    let package_manifest = pkg_path.join("composer.json");
    let root_manifest = workspace_root.join("composer.json");
    let Ok(package_json) = read_json(&package_manifest) else {
        return;
    };
    let Ok(root_json) = read_json(&root_manifest) else {
        issues.push("php: missing root composer.json".to_string());
        return;
    };

    if psr4_path(&package_json) != Some("src/") {
        issues.push(format!("php: {pkg_dir}/composer.json PSR-4 path must be src/"));
    }
    if psr4_path(&root_json) != Some("packages/php/src/") {
        issues.push("php: root composer.json PSR-4 path must be packages/php/src/".to_string());
    }

    let mut package_without_autoload = package_json.clone();
    let mut root_without_autoload = root_json.clone();
    if let Some(obj) = package_without_autoload.as_object_mut() {
        obj.remove("autoload");
    }
    if let Some(obj) = root_without_autoload.as_object_mut() {
        obj.remove("autoload");
    }
    if package_without_autoload != root_without_autoload {
        issues.push("php: root composer.json metadata must stay in sync with packages/php/composer.json".to_string());
    }
}

fn validate_csharp_project(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    pkg_dir: &str,
    issues: &mut Vec<String>,
) {
    let namespace = config.csharp_namespace();
    let configured_project_file = config
        .project_file_for_language(Language::Csharp)
        .map(PathBuf::from)
        .filter(|path| path.extension().is_some_and(|ext| ext == "csproj"));
    let nested = PathBuf::from(pkg_dir)
        .join(&namespace)
        .join(format!("{namespace}.csproj"));
    let root = PathBuf::from(pkg_dir).join(format!("{namespace}.csproj"));
    let nested_path = workspace_root.join(&nested);
    let root_path = workspace_root.join(&root);
    let project_file = configured_project_file.unwrap_or_else(|| {
        if nested_path.exists() {
            nested.clone()
        } else {
            root.clone()
        }
    });
    let project_path = if project_file.is_absolute() {
        project_file.clone()
    } else {
        workspace_root.join(&project_file)
    };

    if root_path.exists() && nested_path.exists() {
        issues.push(format!(
            "csharp: stale root project {pkg_dir}/{namespace}.csproj exists; keep only {pkg_dir}/{namespace}/{namespace}.csproj"
        ));
    }
    let Ok(content) = std::fs::read_to_string(&project_path) else {
        issues.push(format!("csharp: missing {}", project_file.display()));
        return;
    };
    for required in [
        r#"<None Include="../../../LICENSE" Pack="true" PackagePath="/" />"#,
        r#"<None Include="runtimes/**" Pack="true" PackagePath="runtimes/" CopyToOutputDirectory="PreserveNewest" />"#,
        r#"<Compile Include="../src/**/*.cs" />"#,
    ] {
        if !content.contains(required) {
            issues.push(format!("csharp: {namespace}.csproj missing expected item: {required}"));
        }
    }
}

fn validate_go_module(config: &ResolvedCrateConfig, pkg_dir: &str, pkg_path: &Path, issues: &mut Vec<String>) {
    let go_mod = pkg_path.join("go.mod");
    let Ok(content) = std::fs::read_to_string(&go_mod) else {
        return;
    };
    let module = content
        .lines()
        .find_map(|line| line.strip_prefix("module ").map(str::trim));
    let expected = config.go_module();
    if module != Some(expected.as_str()) {
        issues.push(format!("go: {pkg_dir}/go.mod module must be {expected}"));
        return;
    }
    if let Some(major) = go_major_suffix(&expected) {
        let expected_dir = format!("packages/go/{major}");
        if pkg_dir != expected_dir {
            issues.push(format!(
                "go: module path {expected} requires package directory {expected_dir}; set go scaffold output or use a non-/vN module path"
            ));
        }
    }
}

fn validate_java_manifest(config: &ResolvedCrateConfig, pkg_dir: &str, pkg_path: &Path, issues: &mut Vec<String>) {
    let pom = pkg_path.join("pom.xml");
    let Ok(content) = std::fs::read_to_string(&pom) else {
        return;
    };
    let group_id = config.java_group_id();
    let artifact_id = config.java_artifact_id();
    if !content.contains(&format!("<groupId>{group_id}</groupId>")) {
        issues.push(format!("java: {pkg_dir}/pom.xml groupId must be {group_id}"));
    }
    if !content.contains(&format!("<artifactId>{artifact_id}</artifactId>")) {
        issues.push(format!("java: {pkg_dir}/pom.xml artifactId must be {artifact_id}"));
    }
}

fn validate_dart_manifest(config: &ResolvedCrateConfig, pkg_dir: &str, pkg_path: &Path, issues: &mut Vec<String>) {
    let pubspec = pkg_path.join("pubspec.yaml");
    let Ok(content) = std::fs::read_to_string(&pubspec) else {
        return;
    };
    let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
        issues.push(format!("dart: {pkg_dir}/pubspec.yaml is not valid YAML"));
        return;
    };
    let name = yaml.get("name").and_then(|v| v.as_str());
    let expected = config.dart_pubspec_name();
    if name != Some(expected.as_str()) {
        issues.push(format!("dart: {pkg_dir}/pubspec.yaml name must be {expected}"));
    }
    for required in ["version", "description", "repository"] {
        if yaml.get(required).is_none() {
            issues.push(format!("dart: {pkg_dir}/pubspec.yaml missing {required}"));
        }
    }
}

fn validate_swift_manifest(pkg_dir: &str, pkg_path: &Path, issues: &mut Vec<String>) {
    let pkg_manifest = pkg_path.join("Package.swift");
    if let Ok(content) = std::fs::read_to_string(&pkg_manifest)
        && !content.contains("Sources/RustBridge")
    {
        issues.push(format!(
            "swift: {pkg_dir}/Package.swift must include RustBridge source targets"
        ));
    }
}

fn validate_zig_manifest(config: &ResolvedCrateConfig, pkg_dir: &str, pkg_path: &Path, issues: &mut Vec<String>) {
    let zon = pkg_path.join("build.zig.zon");
    let Ok(content) = std::fs::read_to_string(&zon) else {
        issues.push(format!("zig: missing {pkg_dir}/build.zig.zon"));
        return;
    };
    let expected_name = format!(".name = .{}", config.zig_module_name());
    if !content.contains(&expected_name) {
        issues.push(format!(
            "zig: {pkg_dir}/build.zig.zon name must be {}",
            config.zig_module_name()
        ));
    }
    for path in ["\"build.zig\"", "\"build.zig.zon\"", "\"src\""] {
        if !content.contains(path) {
            issues.push(format!("zig: {pkg_dir}/build.zig.zon paths must include {path}"));
        }
    }
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let content = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parsing {}", path.display()))
}

fn psr4_path(json: &serde_json::Value) -> Option<&str> {
    json.get("autoload")?
        .get("psr-4")?
        .as_object()?
        .values()
        .next()?
        .as_str()
}

fn go_major_suffix(module: &str) -> Option<String> {
    let suffix = module.rsplit('/').next()?;
    let major = suffix.strip_prefix('v')?;
    if !major.is_empty() && major.chars().all(|c| c.is_ascii_digit()) && major.parse::<u32>().ok()? >= 2 {
        Some(suffix.to_string())
    } else {
        None
    }
}

fn elixir_nif_targets(config: &ResolvedCrateConfig) -> Vec<String> {
    config
        .elixir
        .as_ref()
        .filter(|elixir| !elixir.nif_targets.is_empty())
        .map(|elixir| elixir.nif_targets.clone())
        .unwrap_or_else(|| {
            [
                "aarch64-apple-darwin",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
                "x86_64-pc-windows-gnu",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        })
}

fn validate_ruby_gemspecs(pkg_path: &Path, pkg_dir: &str, issues: &mut Vec<String>) {
    let mut root_gemspecs = Vec::new();
    let mut nested_gemspecs = Vec::new();
    collect_gemspecs(pkg_path, pkg_path, &mut root_gemspecs, &mut nested_gemspecs);

    if root_gemspecs.is_empty() {
        issues.push(format!("ruby: missing {pkg_dir}/*.gemspec"));
    }
    for nested in nested_gemspecs {
        issues.push(format!(
            "ruby: stale nested gemspec {} (only {pkg_dir}/*.gemspec should remain)",
            nested.display()
        ));
    }
}

fn collect_gemspecs(root: &Path, dir: &Path, root_gemspecs: &mut Vec<PathBuf>, nested_gemspecs: &mut Vec<PathBuf>) {
    if dir
        .strip_prefix(root)
        .ok()
        .is_some_and(|rel| rel.components().any(|component| component.as_os_str() == "vendor"))
    {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_gemspecs(root, &path, root_gemspecs, nested_gemspecs);
            continue;
        }
        if !file_type.is_file() || path.extension().is_none_or(|ext| ext != "gemspec") {
            continue;
        }
        if path.parent() == Some(root) {
            root_gemspecs.push(path);
        } else {
            nested_gemspecs.push(path);
        }
    }
}
