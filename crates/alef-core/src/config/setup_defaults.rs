use super::extras::Language;
use super::output::{SetupConfig, StringOrVec};

/// Return the default setup configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
pub(crate) fn default_setup_config(lang: Language, output_dir: &str) -> SetupConfig {
    match lang {
        Language::Rust => SetupConfig {
            install: Some(StringOrVec::Single(
                "rustup update && cargo install cargo-llvm-cov".to_string(),
            )),
        },
        Language::Python => SetupConfig {
            install: Some(StringOrVec::Single(format!("cd {output_dir} && uv sync"))),
        },
        Language::Node | Language::Wasm => SetupConfig {
            install: Some(StringOrVec::Single("pnpm install".to_string())),
        },
        Language::Go => SetupConfig {
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && go mod download"
            ))),
        },
        Language::Ruby => SetupConfig {
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && bundle install"
            ))),
        },
        Language::Php => SetupConfig {
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer install"
            ))),
        },
        Language::Java => SetupConfig {
            install: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml dependency:resolve -q"
            ))),
        },
        Language::Csharp => SetupConfig {
            install: Some(StringOrVec::Single(format!("dotnet restore {output_dir}"))),
        },
        Language::Elixir => SetupConfig {
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix deps.get"
            ))),
        },
        Language::R => SetupConfig {
            install: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::install_deps()\""
            ))),
        },
        Language::Ffi => SetupConfig { install: None },
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
    fn ffi_has_no_install_command() {
        let cfg = default_setup_config(Language::Ffi, "packages/ffi");
        assert!(cfg.install.is_none());
    }

    #[test]
    fn non_ffi_languages_have_install_command() {
        for lang in all_languages() {
            if lang == Language::Ffi {
                continue;
            }
            let cfg = default_setup_config(lang, "packages/test");
            assert!(
                cfg.install.is_some(),
                "{lang} should have a default install command"
            );
        }
    }

    #[test]
    fn rust_uses_rustup_and_cargo_install() {
        let cfg = default_setup_config(Language::Rust, "packages/rust");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("rustup update"));
        assert!(install.contains("cargo install cargo-llvm-cov"));
    }

    #[test]
    fn python_uses_uv_sync() {
        let cfg = default_setup_config(Language::Python, "packages/python");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("uv sync"));
        assert!(install.contains("packages/python"));
    }

    #[test]
    fn node_uses_pnpm_install() {
        let cfg = default_setup_config(Language::Node, "packages/node");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("pnpm install"));
    }

    #[test]
    fn wasm_matches_node() {
        let node = default_setup_config(Language::Node, "packages/node");
        let wasm = default_setup_config(Language::Wasm, "packages/wasm");
        let node_install = node.install.unwrap().commands().join(" ");
        let wasm_install = wasm.install.unwrap().commands().join(" ");
        assert_eq!(node_install, wasm_install, "WASM and Node should share install command");
    }

    #[test]
    fn go_uses_go_mod_download() {
        let cfg = default_setup_config(Language::Go, "packages/go");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("go mod download"));
    }

    #[test]
    fn ruby_uses_bundle_install() {
        let cfg = default_setup_config(Language::Ruby, "packages/ruby");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("bundle install"));
    }

    #[test]
    fn java_uses_maven_dependency_resolve() {
        let cfg = default_setup_config(Language::Java, "packages/java");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("mvn"));
        assert!(install.contains("dependency:resolve"));
    }

    #[test]
    fn csharp_uses_dotnet_restore() {
        let cfg = default_setup_config(Language::Csharp, "packages/csharp");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("dotnet restore"));
    }

    #[test]
    fn elixir_uses_mix_deps_get() {
        let cfg = default_setup_config(Language::Elixir, "packages/elixir");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("mix deps.get"));
    }

    #[test]
    fn r_uses_remotes_install_deps() {
        let cfg = default_setup_config(Language::R, "packages/r");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(install.contains("remotes::install_deps()"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let cfg = default_setup_config(Language::Go, "my/custom/path");
        let install = cfg.install.unwrap().commands().join(" ");
        assert!(
            install.contains("my/custom/path"),
            "Go install should contain output dir, got: {install}"
        );
    }
}
