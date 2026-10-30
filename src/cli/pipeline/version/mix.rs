use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use tracing::info;

use super::replace_version_pattern;

pub(super) fn sync_version(version: &str, updated: &mut Vec<String>) -> anyhow::Result<()> {
    // Elixir: mix.exs — handle both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Ok(content) = std::fs::read_to_string("packages/elixir/mix.exs") {
        if let Some(new_content) = replace_version_pattern(&content, r#"version: "[^"]*""#, version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
        } else if let Some(new_content) = replace_version_pattern(&content, r#"@version "[^"]*""#, version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
        }
    }
    Ok(())
}

pub(super) fn refresh_changed_projects(updated: &[String]) {
    let directories: BTreeSet<_> = updated
        .iter()
        .map(Path::new)
        .filter(|path| path.file_name().is_some_and(|name| name == "mix.exs"))
        .filter_map(Path::parent)
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .collect();
    for directory in directories {
        info!(project = %directory.display(), "Running (optional): mix deps.get");
        match Command::new("mix").arg("deps.get").current_dir(directory).status() {
            Ok(status) if !status.success() => {
                info!(
                    project = %directory.display(),
                    "Optional command failed with exit code {:?}: mix deps.get",
                    status.code()
                );
            }
            Err(error) => {
                info!(
                    project = %directory.display(),
                    "Optional command not found or failed to execute: mix deps.get ({error})"
                );
            }
            Ok(_) => {}
        }
    }
}
