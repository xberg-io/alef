use super::extras::Language;
use super::output::{SetupConfig, StringOrVec};
use super::tools::{LangContext, require_tool};

/// Return the default setup configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides the package manager selection.
pub(crate) fn default_setup_config(lang: Language, output_dir: &str, ctx: &LangContext) -> SetupConfig {
    match lang {
        Language::Rust => {
            let mut commands: Vec<String> = vec!["rustup update stable".to_string()];
            commands.extend(
                ctx.tools
                    .rust_tools()
                    .iter()
                    .map(|t| format!("cargo install {t} --locked")),
            );
            commands.push("rustup component add rustfmt clippy".to_string());
            SetupConfig {
                precondition: Some(require_tool("cargo")),
                before: None,
                install: Some(StringOrVec::Multiple(commands)),
                timeout_seconds: 600,
            }
        }
        Language::Python => {
            let pm = ctx.tools.python_pm();
            let install_cmd = match pm {
                "pip" => format!("cd {output_dir} && pip install -e ."),
                "poetry" => format!("cd {output_dir} && poetry install"),
                _ => format!("cd {output_dir} && uv sync"),
            };
            SetupConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                install: Some(StringOrVec::Single(install_cmd)),
                timeout_seconds: 600,
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let install_cmd = match pm {
                "npm" => format!("cd {output_dir} && npm install"),
                "yarn" => format!("cd {output_dir} && yarn install"),
                _ => format!("cd {output_dir} && pnpm install"),
            };
            SetupConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                install: Some(StringOrVec::Single(install_cmd)),
                timeout_seconds: 600,
            }
        }
        Language::Go => SetupConfig {
            precondition: Some(require_tool("go")),
            before: None,
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && GOWORK=off go mod download"
            ))),
            timeout_seconds: 600,
        },
        Language::Ruby => SetupConfig {
            precondition: Some(require_tool("bundle")),
            before: None,
            install: Some(StringOrVec::Single(format!("cd {output_dir} && bundle install"))),
            timeout_seconds: 600,
        },
        Language::Php => SetupConfig {
            precondition: Some(require_tool("composer")),
            before: None,
            install: Some(StringOrVec::Single(format!("cd {output_dir} && composer install"))),
            timeout_seconds: 600,
        },
        Language::Java => SetupConfig {
            precondition: Some(require_tool("mvn")),
            before: None,
            install: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml dependency:resolve -q"
            ))),
            timeout_seconds: 600,
        },
        Language::Csharp => SetupConfig {
            // Both `dotnet` AND a discoverable .sln/.csproj must exist under output_dir, or
            // `dotnet restore` walks the entire repo (including target/ and node_modules/)
            // looking for a project file and times out. Skip cleanly when no project is present.
            precondition: Some(format!(
                "command -v dotnet >/dev/null 2>&1 && [ -n \"$(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)\" ]"
            )),
            before: None,
            // Resolve the first .sln/.csproj under output_dir (depth 3) — same approach as
            // the C# upgrade default. Avoids the unbounded directory walk that caused the
            // 600s timeout on CI.
            install: Some(StringOrVec::Single(format!(
                "dotnet restore $(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)"
            ))),
            timeout_seconds: 600,
        },
        Language::Elixir => SetupConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            install: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.get"))),
            timeout_seconds: 600,
        },
        Language::R => SetupConfig {
            precondition: Some(require_tool("Rscript")),
            before: None,
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::install_deps()\""
            ))),
            timeout_seconds: 600,
        },
        Language::Ffi => SetupConfig {
            // FFI shares cargo with the parent Rust crate; there is no
            // separate install step and therefore nothing to precondition.
            precondition: None,
            before: None,
            install: None,
            timeout_seconds: 600,
        },
        Language::Kotlin => SetupConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            install: Some(StringOrVec::Single(
                "gradle build --refresh-dependencies".to_string(),
            )),
            timeout_seconds: 600,
        },
        Language::Swift => SetupConfig {
            precondition: Some(require_tool("swift")),
            before: None,
            install: Some(StringOrVec::Single("swift package resolve".to_string())),
            timeout_seconds: 600,
        },
        Language::Dart => SetupConfig {
            precondition: Some(require_tool("dart")),
            before: None,
            install: Some(StringOrVec::Single("dart pub get".to_string())),
            timeout_seconds: 600,
        },
        Language::Gleam => SetupConfig {
            precondition: Some(require_tool("gleam")),
            before: None,
            install: Some(StringOrVec::Single("gleam deps download".to_string())),
            timeout_seconds: 600,
        },
        Language::Zig => SetupConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            install: Some(StringOrVec::Single("zig build --fetch".to_string())),
            timeout_seconds: 600,
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
            Language::Kotlin,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
        ]
    }

    fn cfg(lang: Language, dir: &str) -> SetupConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_setup_config(lang, dir, &ctx)
    }

    #[test]
    fn ffi_has_no_install_command() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.install.is_none());
    }

    #[test]
    fn non_ffi_languages_have_install_command() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.install.is_some(), "{lang} should have a default install command");
        }
    }

    #[test]
    fn non_ffi_languages_have_default_precondition() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
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
    fn rust_install_lists_full_tool_set() {
        let c = cfg(Language::Rust, "packages/rust");
        let install = c.install.unwrap();
        let cmds = install.commands();
        let joined = cmds.join(" || ");
        assert!(joined.contains("rustup update stable"));
        for tool in super::super::tools::DEFAULT_RUST_DEV_TOOLS {
            assert!(
                joined.contains(&format!("cargo install {tool} --locked")),
                "Rust setup should install {tool}, got: {joined}"
            );
        }
        assert!(joined.contains("rustup component add rustfmt clippy"));
    }

    #[test]
    fn rust_install_respects_user_tool_list() {
        let tools = ToolsConfig {
            rust_dev_tools: Some(vec!["cargo-edit".to_string(), "cargo-foo".to_string()]),
            ..Default::default()
        };
        let ctx = LangContext::default(&tools);
        let c = default_setup_config(Language::Rust, "packages/rust", &ctx);
        let cmds = c.install.unwrap().commands().join(" || ");
        assert!(cmds.contains("cargo install cargo-edit --locked"));
        assert!(cmds.contains("cargo install cargo-foo --locked"));
        // Default tools that aren't in the user override should be absent.
        assert!(!cmds.contains("cargo install cargo-deny"));
    }

    fn python_tools(pm: &str) -> ToolsConfig {
        ToolsConfig {
            python_package_manager: Some(pm.to_string()),
            ..Default::default()
        }
    }

    fn node_tools(pm: &str) -> ToolsConfig {
        ToolsConfig {
            node_package_manager: Some(pm.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn python_setup_dispatches_on_package_manager() {
        for (pm, expected_install, expected_pre) in [
            ("uv", "uv sync", "command -v uv >/dev/null 2>&1"),
            ("pip", "pip install -e", "command -v pip >/dev/null 2>&1"),
            ("poetry", "poetry install", "command -v poetry >/dev/null 2>&1"),
        ] {
            let tools = python_tools(pm);
            let ctx = LangContext::default(&tools);
            let c = default_setup_config(Language::Python, "packages/python", &ctx);
            assert!(c.install.unwrap().commands().join(" ").contains(expected_install));
            assert_eq!(c.precondition.as_deref(), Some(expected_pre));
        }
    }

    #[test]
    fn node_setup_dispatches_on_package_manager() {
        for (pm, expected_install) in [
            ("pnpm", "pnpm install"),
            ("npm", "npm install"),
            ("yarn", "yarn install"),
        ] {
            let tools = node_tools(pm);
            let ctx = LangContext::default(&tools);
            let c = default_setup_config(Language::Node, "packages/node", &ctx);
            assert!(c.install.unwrap().commands().join(" ").contains(expected_install));
        }
    }

    #[test]
    fn python_uses_uv_sync_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("uv sync"));
        assert!(install.contains("packages/python"));
    }

    #[test]
    fn node_uses_pnpm_install_by_default() {
        let c = cfg(Language::Node, "packages/node");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("pnpm install"));
    }

    #[test]
    fn wasm_matches_node() {
        // Same package manager invocation, only the output dir differs.
        let node = cfg(Language::Node, "packages/foo");
        let wasm = cfg(Language::Wasm, "packages/foo");
        assert_eq!(
            node.install.unwrap().commands().join(" "),
            wasm.install.unwrap().commands().join(" "),
            "WASM and Node should share install command"
        );
    }

    #[test]
    fn go_uses_go_mod_download() {
        let c = cfg(Language::Go, "packages/go");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("go mod download"));
    }

    #[test]
    fn ruby_uses_bundle_install() {
        let c = cfg(Language::Ruby, "packages/ruby");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("bundle install"));
    }

    #[test]
    fn java_uses_maven_dependency_resolve() {
        let c = cfg(Language::Java, "packages/java");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("mvn"));
        assert!(install.contains("dependency:resolve"));
    }

    #[test]
    fn csharp_uses_dotnet_restore() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("dotnet restore"));
    }

    #[test]
    fn elixir_uses_mix_deps_get() {
        let c = cfg(Language::Elixir, "packages/elixir");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("mix deps.get"));
    }

    #[test]
    fn r_uses_remotes_install_deps() {
        let c = cfg(Language::R, "packages/r");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("remotes::install_deps()"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("my/custom/path"));
    }
}
