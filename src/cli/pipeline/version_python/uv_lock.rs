use std::path::Path;

use anyhow::Context as _;

use crate::cli::git::IgnoreFilter;

pub(in crate::cli::pipeline) fn project_version(manifest: &toml::Value, directory: &Path) -> Option<String> {
    let project = manifest.get("project")?;
    if let Some(version) = project.get("version").and_then(toml::Value::as_str) {
        return Some(version.to_string());
    }
    let dynamic = project.get("dynamic")?.as_array()?;
    if !dynamic.iter().any(|item| item.as_str() == Some("version")) {
        return None;
    }
    let backend = manifest.get("build-system")?.get("build-backend")?.as_str()?;
    if backend != "maturin" {
        return None;
    }
    let cargo = manifest
        .get("tool")
        .and_then(|tool| tool.get("maturin"))
        .and_then(|maturin| maturin.get("manifest-path"))
        .and_then(toml::Value::as_str)
        .unwrap_or("Cargo.toml");
    super::super::version_core::read_version(&directory.join(cargo).to_string_lossy())
        .ok()
        .map(|version| crate::core::version::to_pep440(&version))
}

fn local_package_version(package: &toml_edit::Table, directory: &Path, root: &Path) -> Option<String> {
    let source = package.get("source")?.as_inline_table()?;
    let relative = ["editable", "virtual", "directory"]
        .into_iter()
        .find_map(|key| source.get(key).and_then(toml_edit::Value::as_str))?;
    let path = directory.join(relative).canonicalize().ok()?;
    if !path.starts_with(root) {
        return None;
    }
    let text = std::fs::read_to_string(path.join("pyproject.toml")).ok()?;
    let manifest = toml::from_str::<toml::Value>(&text).ok()?;
    let name = manifest.get("project")?.get("name")?.as_str()?;
    let locked_name = package.get("name")?.as_str()?;
    if super::super::lock_freshness::normalize_pep503_name(name)
        != super::super::lock_freshness::normalize_pep503_name(locked_name)
    {
        return None;
    }
    project_version(&manifest, &path)
}

pub(super) fn sync(writable: &IgnoreFilter, updated: &mut Vec<String>) -> anyhow::Result<()> {
    let root = std::env::current_dir()
        .context("resolving workspace for uv.lock version sync")?
        .canonicalize()
        .context("canonicalizing workspace for uv.lock version sync")?;
    for path in writable.glob("**/uv.lock") {
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut lock: toml_edit::DocumentMut = text.parse().with_context(|| format!("parsing {}", path.display()))?;
        let Some(packages) = lock
            .get_mut("package")
            .and_then(toml_edit::Item::as_array_of_tables_mut)
        else {
            continue;
        };
        let directory = path.parent().unwrap_or(Path::new("."));
        let mut changed = false;
        for package in packages.iter_mut() {
            if let Some(version) = local_package_version(package, directory, &root)
                && package.get("version").and_then(toml_edit::Item::as_str) != Some(&version)
            {
                package["version"] = toml_edit::value(version);
                changed = true;
            }
        }
        let new_text = lock.to_string();
        if changed && new_text != text {
            std::fs::write(&path, new_text).with_context(|| format!("writing {}", path.display()))?;
            updated.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "uv_lock_tests.rs"]
mod tests;
