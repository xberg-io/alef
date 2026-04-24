use super::extras::Language;
use super::output::{StringOrVec, UpdateConfig};

/// Return the default update configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
pub fn default_update_config(lang: Language, output_dir: &str) -> UpdateConfig {
    match lang {
        Language::Rust => UpdateConfig {
            update: Some(StringOrVec::Single("cargo update".to_string())),
            upgrade: Some(StringOrVec::Multiple(vec![
                "cargo upgrade --incompatible".to_string(),
                "cargo update".to_string(),
            ])),
        },
        Language::Python => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && uv sync --upgrade"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && uv sync --all-packages --all-extras --upgrade"
            ))),
        },
        Language::Node | Language::Wasm => UpdateConfig {
            update: Some(StringOrVec::Single("pnpm up -r".to_string())),
            upgrade: Some(StringOrVec::Multiple(vec![
                "corepack up".to_string(),
                "pnpm up --latest -r -w".to_string(),
            ])),
        },
        Language::Ruby => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && bundle update --all"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && bundle update --all --conservative=false"
            ))),
        },
        Language::Php => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer update"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer update --with-all-dependencies"
            ))),
        },
        Language::Go => UpdateConfig {
            update: Some(StringOrVec::Multiple(vec![
                format!("cd {output_dir} && go get -u ./..."),
                format!("cd {output_dir} && go mod tidy"),
            ])),
            upgrade: Some(StringOrVec::Multiple(vec![
                format!("cd {output_dir} && go get -u ./..."),
                format!("cd {output_dir} && go mod tidy"),
            ])),
        },
        Language::Java => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml versions:use-latest-releases -q"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml versions:use-latest-releases -DallowMajorUpdates=true -q"
            ))),
        },
        Language::Csharp => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade {output_dir}"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade --version-lock major {output_dir}"
            ))),
        },
        Language::Elixir => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix deps.update --all"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix deps.update --all"
            ))),
        },
        Language::R => UpdateConfig {
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages()\""
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages()\""
            ))),
        },
        Language::Ffi => UpdateConfig {
            update: None,
            upgrade: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_languages() -> Vec<Language> {
        vec![
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
            Language::R,
            Language::Ffi,
            Language::Rust,
        ]
    }

    #[test]
    fn ffi_has_no_update_commands() {
        let cfg = default_update_config(Language::Ffi, "packages/ffi");
        assert!(cfg.update.is_none());
        assert!(cfg.upgrade.is_none());
    }

    #[test]
    fn non_ffi_languages_have_update_commands() {
        for lang in all_languages() {
            if lang == Language::Ffi {
                continue;
            }
            let cfg = default_update_config(lang, "packages/test");
            assert!(
                cfg.update.is_some(),
                "{lang} should have a default update command"
            );
            assert!(
                cfg.upgrade.is_some(),
                "{lang} should have a default upgrade command"
            );
        }
    }

    #[test]
    fn rust_update_uses_cargo() {
        let cfg = default_update_config(Language::Rust, "packages/rust");
        let update = cfg.update.unwrap().commands().join(" ");
        let upgrade = cfg.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("cargo update"));
        assert!(upgrade.contains("cargo upgrade --incompatible"));
        assert!(upgrade.contains("cargo update"));
    }

    #[test]
    fn rust_upgrade_is_multi_command() {
        let cfg = default_update_config(Language::Rust, "packages/rust");
        let upgrade = cfg.upgrade.unwrap();
        let cmds = upgrade.commands();
        assert!(cmds.len() >= 2, "Rust upgrade should have multiple commands");
    }

    #[test]
    fn python_update_uses_uv() {
        let cfg = default_update_config(Language::Python, "packages/python");
        let update = cfg.update.unwrap().commands().join(" ");
        let upgrade = cfg.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("uv sync"));
        assert!(upgrade.contains("uv sync"));
        assert!(upgrade.contains("--all-packages"));
    }

    #[test]
    fn node_update_uses_pnpm() {
        let cfg = default_update_config(Language::Node, "packages/node");
        let update = cfg.update.unwrap().commands().join(" ");
        let upgrade = cfg.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("pnpm up"));
        assert!(upgrade.contains("pnpm up --latest"));
    }

    #[test]
    fn node_upgrade_includes_corepack() {
        let cfg = default_update_config(Language::Node, "packages/node");
        let upgrade = cfg.upgrade.unwrap();
        let cmds = upgrade.commands();
        assert!(
            cmds.iter().any(|c| c.contains("corepack up")),
            "Node upgrade should include corepack up"
        );
    }

    #[test]
    fn java_update_uses_maven_versions() {
        let cfg = default_update_config(Language::Java, "packages/java");
        let update = cfg.update.unwrap().commands().join(" ");
        let upgrade = cfg.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("versions:use-latest-releases"));
        assert!(upgrade.contains("allowMajorUpdates=true"));
    }

    #[test]
    fn output_dir_substituted_in_update_commands() {
        let cfg = default_update_config(Language::Go, "my/custom/path");
        let update = cfg.update.unwrap().commands().join(" ");
        assert!(
            update.contains("my/custom/path"),
            "Go update should contain output dir, got: {update}"
        );
    }

    #[test]
    fn wasm_defaults_match_node() {
        let node = default_update_config(Language::Node, "packages/node");
        let wasm = default_update_config(Language::Wasm, "packages/wasm");
        let node_update = node.update.unwrap().commands().join(" ");
        let wasm_update = wasm.update.unwrap().commands().join(" ");
        assert_eq!(node_update, wasm_update, "WASM and Node should share update commands");
    }
}
