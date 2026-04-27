use super::extras::Language;
use super::output::{StringOrVec, UpdateConfig};
use super::tools::{LangContext, require_tool};

/// Return the default update configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides the package manager selection.
pub fn default_update_config(lang: Language, output_dir: &str, ctx: &LangContext) -> UpdateConfig {
    match lang {
        Language::Rust => UpdateConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            update: Some(StringOrVec::Single("cargo update".to_string())),
            upgrade: Some(StringOrVec::Multiple(vec![
                "cargo upgrade --incompatible".to_string(),
                "cargo update".to_string(),
            ])),
        },
        Language::Python => {
            let pm = ctx.tools.python_pm();
            let (update_cmd, upgrade_cmd) = match pm {
                "pip" => (
                    format!("cd {output_dir} && pip install -U -e ."),
                    format!("cd {output_dir} && pip install -U -e ."),
                ),
                "poetry" => (
                    format!("cd {output_dir} && poetry update"),
                    format!("cd {output_dir} && poetry update --with dev"),
                ),
                _ => (
                    format!("cd {output_dir} && uv sync --upgrade"),
                    format!("cd {output_dir} && uv sync --all-packages --all-extras --upgrade"),
                ),
            };
            UpdateConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                update: Some(StringOrVec::Single(update_cmd)),
                upgrade: Some(StringOrVec::Single(upgrade_cmd)),
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let (update_cmds, upgrade_cmds) = match pm {
                "npm" => (
                    vec![format!("cd {output_dir} && npm update")],
                    vec![format!(
                        "cd {output_dir} && npm install -g npm-check-updates && ncu -u && npm install"
                    )],
                ),
                "yarn" => (
                    vec![format!("cd {output_dir} && yarn upgrade")],
                    vec![format!("cd {output_dir} && yarn upgrade --latest")],
                ),
                _ => (
                    vec!["corepack up".to_string(), "pnpm up -r".to_string()],
                    vec![
                        "corepack use pnpm@latest".to_string(),
                        "pnpm up --latest -r -w".to_string(),
                    ],
                ),
            };
            UpdateConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                update: Some(StringOrVec::Multiple(update_cmds)),
                upgrade: Some(StringOrVec::Multiple(upgrade_cmds)),
            }
        }
        Language::Ruby => UpdateConfig {
            precondition: Some(require_tool("bundle")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && bundle update --all"))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && bundle update --all --conservative=false"
            ))),
        },
        Language::Php => UpdateConfig {
            precondition: Some(require_tool("composer")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && composer update"))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer update --with-all-dependencies"
            ))),
        },
        Language::Go => UpdateConfig {
            precondition: Some(require_tool("go")),
            before: None,
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
            precondition: Some(require_tool("mvn")),
            before: None,
            // The `-Dmaven.version.rules=file://...` flag is appended only when the rules file
            // exists, since `mvn versions:use-latest-releases` aborts on missing rule files.
            update: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml versions:use-latest-releases $([ -f {output_dir}/versions-rules.xml ] && echo \"-Dmaven.version.rules=file://${{PWD}}/{output_dir}/versions-rules.xml\") -q"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml versions:use-latest-releases -DallowMajorUpdates=true $([ -f {output_dir}/versions-rules.xml ] && echo \"-Dmaven.version.rules=file://${{PWD}}/{output_dir}/versions-rules.xml\") -q"
            ))),
        },
        Language::Csharp => UpdateConfig {
            precondition: Some(require_tool("dotnet")),
            before: None,
            // `dotnet outdated` requires either a .sln/.csproj path or a directory containing
            // one. Consumers typically nest csproj files under packages/csharp/<project>/, so
            // shell out to find a top-level project file or fall back to the first one found.
            update: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade $(ls {output_dir}/*.sln {output_dir}/*.csproj 2>/dev/null | head -1 || find {output_dir} -maxdepth 3 -name '*.sln' -o -name '*.csproj' 2>/dev/null | head -1 || echo {output_dir})"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade --version-lock major $(ls {output_dir}/*.sln {output_dir}/*.csproj 2>/dev/null | head -1 || find {output_dir} -maxdepth 3 -name '*.sln' -o -name '*.csproj' 2>/dev/null | head -1 || echo {output_dir})"
            ))),
        },
        Language::Elixir => UpdateConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.update --all"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.update --all"))),
        },
        Language::R => UpdateConfig {
            precondition: Some(require_tool("Rscript")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages(ask = FALSE)\""
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages(ask = FALSE)\""
            ))),
        },
        Language::Ffi => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools::ToolsConfig;
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

    fn cfg(lang: Language, dir: &str) -> UpdateConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_update_config(lang, dir, &ctx)
    }

    #[test]
    fn ffi_has_no_update_commands() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.update.is_none());
        assert!(c.upgrade.is_none());
    }

    #[test]
    fn non_ffi_languages_have_update_commands() {
        for lang in all_languages() {
            if lang == Language::Ffi {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.update.is_some(), "{lang} should have a default update command");
            assert!(c.upgrade.is_some(), "{lang} should have a default upgrade command");
        }
    }

    #[test]
    fn non_ffi_languages_have_default_precondition() {
        for lang in all_languages() {
            if lang == Language::Ffi {
                continue;
            }
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn rust_update_uses_cargo() {
        let c = cfg(Language::Rust, "packages/rust");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("cargo update"));
        assert!(upgrade.contains("cargo upgrade --incompatible"));
        assert!(upgrade.contains("cargo update"));
    }

    #[test]
    fn rust_upgrade_is_multi_command() {
        let c = cfg(Language::Rust, "packages/rust");
        let upgrade = c.upgrade.unwrap();
        let cmds = upgrade.commands();
        assert!(cmds.len() >= 2);
    }

    #[test]
    fn python_update_uses_uv_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("uv sync"));
        assert!(upgrade.contains("--all-packages"));
    }

    #[test]
    fn python_update_dispatches_on_package_manager() {
        for (pm, expected) in [("pip", "pip install -U"), ("poetry", "poetry update")] {
            let tools = ToolsConfig {
                python_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_update_config(Language::Python, "packages/python", &ctx);
            assert!(
                c.update.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    #[test]
    fn node_update_uses_pnpm_by_default() {
        let c = cfg(Language::Node, "packages/node");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("pnpm up"));
        assert!(upgrade.contains("pnpm up --latest"));
    }

    #[test]
    fn node_update_dispatches_on_package_manager() {
        for (pm, expected) in [("npm", "npm update"), ("yarn", "yarn upgrade")] {
            let tools = ToolsConfig {
                node_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_update_config(Language::Node, "packages/node", &ctx);
            assert!(
                c.update.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    #[test]
    fn java_update_uses_maven_versions() {
        let c = cfg(Language::Java, "packages/java");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("versions:use-latest-releases"));
        assert!(upgrade.contains("allowMajorUpdates=true"));
        // Rules-file flag must be guarded so missing versions-rules.xml doesn't fail mvn.
        assert!(
            update.contains("[ -f packages/java/versions-rules.xml ]"),
            "java update should make versions-rules.xml optional"
        );
    }

    #[test]
    fn csharp_update_resolves_csproj_in_subdir() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        // Both commands must search for a .sln/.csproj rather than passing the directory raw,
        // since `dotnet outdated` errors when the dir contains no top-level project file.
        assert!(update.contains("find packages/csharp"), "update should locate csproj");
        assert!(upgrade.contains("find packages/csharp"), "upgrade should locate csproj");
    }

    #[test]
    fn output_dir_substituted_in_update_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let update = c.update.unwrap().commands().join(" ");
        assert!(update.contains("my/custom/path"));
    }

    #[test]
    fn r_update_is_non_interactive() {
        let c = cfg(Language::R, "packages/r");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("ask = FALSE"), "R update must be non-interactive");
        assert!(upgrade.contains("ask = FALSE"), "R upgrade must be non-interactive");
    }

    #[test]
    fn wasm_defaults_match_node() {
        let node = cfg(Language::Node, "packages/node");
        let wasm = cfg(Language::Wasm, "packages/wasm");
        let node_update = node.update.unwrap().commands().join(" ");
        let wasm_update = wasm.update.unwrap().commands().join(" ");
        assert_eq!(node_update, wasm_update);
    }
}
