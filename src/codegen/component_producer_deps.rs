//! Location and Cargo dependency resolution for generated native component producers.

use crate::codegen::component::ResolvedComponent;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::component::{
    PRODUCER_BUILTIN_DEPENDENCIES, component_producer_dir, is_valid_cargo_package_name,
};
use crate::core::config::resolve_helpers::relative_slash_path;
use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

/// Directory the producer crate of `component` is written to.
pub(super) fn component_crate_dir(config: &ResolvedCrateConfig, component: &ResolvedComponent) -> PathBuf {
    component_producer_dir(
        &config.core_crate_dir(),
        &component.component_name,
        component.producer_dir.as_deref(),
    )
}

/// `path` joined onto `base` with `.` and `..` segments folded away, so paths written relative to
/// different directories (the working directory, the directory of `alef.toml`) compare lexically.
fn absolute_clean(base: &Path, path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in base.join(path).components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The `[package] name` of the Cargo manifest at `manifest`.
fn read_package_name(manifest: &Path, configured: &str) -> Result<String> {
    let contents = std::fs::read_to_string(manifest).with_context(|| {
        format!(
            "component source_manifest `{configured}` could not be read ({}); the sibling workspace \
             that holds it may not be checked out",
            manifest.display()
        )
    })?;
    let table: toml::Table = toml::from_str(&contents).with_context(|| {
        format!(
            "component source_manifest `{configured}` is not valid TOML ({})",
            manifest.display()
        )
    })?;
    let name = table
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .with_context(|| {
            format!(
                "component source_manifest `{configured}` has no literal `[package] name`: it is a virtual \
                 workspace manifest (no `[package]`) or uses `name.workspace = true`; point it at the \
                 source crate's own Cargo.toml and give that crate a literal name"
            )
        })?;
    if !is_valid_cargo_package_name(name) {
        bail!("component source_manifest `{configured}` has an invalid package name `{name}`");
    }
    Ok(name.to_string())
}

fn same_package(left: &str, right: &str) -> bool {
    left.replace('_', "-") == right.replace('_', "-")
}

/// Render the producer's core-crate and source-crate `[dependencies]` lines (newline-terminated).
///
/// Without `source_manifest`, or when it names the core crate, a single dependency on the core
/// crate carries the component's features, as it always has. With a distinct source crate the
/// producer keeps its path dependency on the core crate (the contract trait home, whose
/// `trait_path`s are emitted verbatim) and additionally depends on the source crate, named by its
/// `[package] name`. The component's `features` then go on the source crate, where the
/// implementation lives; `default_features` applies to both. Relative `source_manifest` and
/// `producer_dir` values and the core crate location resolve against `base`.
pub(super) fn producer_dependencies(
    config: &ResolvedCrateConfig,
    component: &ResolvedComponent,
    version: &str,
    base: &Path,
) -> Result<String> {
    let features = component
        .features
        .iter()
        .map(|feature| format!("\"{feature}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let default_features = component.default_features;
    let core_package = &config.name;
    let producer_dir = absolute_clean(base, &component_crate_dir(config, component));
    // The default producer location keeps the historical `../<core dir>` spelling so existing
    // generated manifests stay byte-identical, even for a root-flat core crate.
    let core_path = if component.producer_dir.is_none() {
        format!("../{}", config.core_crate_dir())
    } else {
        relative_slash_path(&producer_dir, &absolute_clean(base, &config.core_crate_root()))
    };
    let single = format!(
        "{core_package} = {{ path = \"{core_path}\", version = \"{version}\", default-features = {default_features}, features = [{features}] }}\n"
    );
    let Some(configured) = component.source_manifest.as_deref() else {
        return Ok(single);
    };

    let manifest = absolute_clean(base, Path::new(configured));
    let source_package = read_package_name(&manifest, configured)?;
    if same_package(&source_package, core_package) {
        return Ok(single);
    }
    if PRODUCER_BUILTIN_DEPENDENCIES
        .iter()
        .any(|builtin| same_package(builtin, &source_package))
        || component
            .extra_dependencies
            .keys()
            .any(|name| same_package(name, &source_package))
    {
        bail!(
            "component `{}` source crate `{source_package}` collides with another dependency of the producer crate",
            component.component_name
        );
    }
    let source_dir = manifest.parent().unwrap_or(&manifest);
    let source_path = relative_slash_path(&producer_dir, source_dir);
    Ok(format!(
        "{core_package} = {{ path = \"{core_path}\", version = \"{version}\", default-features = {default_features} }}\n\
         {source_package} = {{ path = \"{source_path}\", default-features = {default_features}, features = [{features}] }}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "x".into(),
            sources: vec![PathBuf::from("crates/x/src/lib.rs")],
            ..ResolvedCrateConfig::default()
        }
    }

    fn component(source_manifest: Option<&str>, producer_dir: Option<&str>) -> ResolvedComponent {
        ResolvedComponent {
            producer_language: Default::default(),
            component_name: "web".into(),
            features: vec!["web".into()],
            default_features: false,
            targets: Vec::new(),
            bundled_on: Vec::new(),
            extra_dependencies: BTreeMap::new(),
            source_manifest: source_manifest.map(str::to_string),
            producer_dir: producer_dir.map(str::to_string),
            provides: Vec::new(),
        }
    }

    fn write_manifest(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn default_location_and_dependency_are_unchanged() {
        let config = config();
        let component = component(None, None);
        assert_eq!(
            component_crate_dir(&config, &component),
            PathBuf::from("crates/x-web-component")
        );
        let dependencies = producer_dependencies(&config, &component, "1.2.3", Path::new("/work")).unwrap();
        assert_eq!(
            dependencies,
            "x = { path = \"../x\", version = \"1.2.3\", default-features = false, features = [\"web\"] }\n"
        );
    }

    #[test]
    fn source_crate_in_a_sibling_workspace_gets_a_second_relative_path_dependency() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            "components/crates/x-web/Cargo.toml",
            "[package]\nname = \"x-web\"\nversion.workspace = true\n",
        );
        let component = component(
            Some("components/crates/x-web/Cargo.toml"),
            Some("components/crates/x-web-component"),
        );
        let dependencies = producer_dependencies(&config(), &component, "1.2.3", root.path()).unwrap();
        assert_eq!(
            dependencies,
            "x = { path = \"../../../crates/x\", version = \"1.2.3\", default-features = false }\n\
             x-web = { path = \"../x-web\", default-features = false, features = [\"web\"] }\n"
        );
        let manifest = format!("[dependencies]\n{dependencies}");
        toml::from_str::<toml::Table>(&manifest).expect("dependencies stay valid TOML");
    }

    #[test]
    fn source_manifest_naming_the_core_crate_emits_a_single_dependency() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(root.path(), "crates/x/Cargo.toml", "[package]\nname = \"x\"\n");
        let component = component(Some("crates/x/Cargo.toml"), None);
        let dependencies = producer_dependencies(&config(), &component, "1.2.3", root.path()).unwrap();
        assert_eq!(
            dependencies,
            "x = { path = \"../x\", version = \"1.2.3\", default-features = false, features = [\"web\"] }\n"
        );
    }

    #[test]
    fn custom_producer_dir_alone_recomputes_the_core_path() {
        let component = component(None, Some("components/crates/x-web-component"));
        let dependencies = producer_dependencies(&config(), &component, "1.2.3", Path::new("/work")).unwrap();
        assert!(dependencies.contains("path = \"../../../crates/x\""), "{dependencies}");
        assert!(dependencies.contains("features = [\"web\"]"), "{dependencies}");
    }

    #[test]
    fn config_relative_prefixes_cancel_out() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            "components/crates/x-web/Cargo.toml",
            "[package]\nname = \"x-web\"\n",
        );
        let base = root.path().join("sub");
        let component = component(
            Some("../components/crates/x-web/Cargo.toml"),
            Some("../components/crates/x-web-component"),
        );
        let dependencies = producer_dependencies(&config(), &component, "1.2.3", &base).unwrap();
        assert_eq!(
            dependencies,
            "x = { path = \"../../../sub/crates/x\", version = \"1.2.3\", default-features = false }\n\
             x-web = { path = \"../x-web\", default-features = false, features = [\"web\"] }\n"
        );
    }

    #[test]
    fn default_location_keeps_the_historical_core_path_for_a_root_flat_core_crate() {
        let config = ResolvedCrateConfig {
            name: "x".into(),
            sources: vec![PathBuf::from("src/lib.rs")],
            ..ResolvedCrateConfig::default()
        };
        let dependencies = producer_dependencies(&config, &component(None, None), "1", Path::new("/work")).unwrap();
        assert_eq!(
            dependencies,
            "x = { path = \"../x\", version = \"1\", default-features = false, features = [\"web\"] }\n"
        );
    }

    #[test]
    fn missing_manifest_or_package_name_is_a_clear_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = component(Some("nope/Cargo.toml"), None);
        let error = producer_dependencies(&config(), &missing, "1", root.path())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("source_manifest `nope/Cargo.toml` could not be read"),
            "{error}"
        );
        assert!(error.contains("not be checked out"), "{error}");

        write_manifest(root.path(), "ws/Cargo.toml", "[workspace]\nmembers = []\n");
        let nameless = component(Some("ws/Cargo.toml"), None);
        let error = producer_dependencies(&config(), &nameless, "1", root.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("virtual workspace manifest"), "{error}");
        assert!(error.contains("name.workspace = true"), "{error}");
    }

    #[test]
    fn source_crate_colliding_with_a_builtin_or_extra_dependency_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(root.path(), "s/Cargo.toml", "[package]\nname = \"futures\"\n");
        let error = producer_dependencies(&config(), &component(Some("s/Cargo.toml"), None), "1", root.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("collides"), "{error}");
    }
}
