use super::extras::Language;
use super::output::{BuildCommandConfig, StringOrVec};

/// Return the default build configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). The `crate_name` is the name of the core crate
/// (e.g. `my-lib`). Both are substituted into command templates.
pub(crate) fn default_build_config(
    lang: Language,
    output_dir: &str,
    crate_name: &str,
) -> BuildCommandConfig {
    match lang {
        Language::Rust => BuildCommandConfig {
            build: Some(StringOrVec::Single("cargo build --workspace".to_string())),
            build_release: Some(StringOrVec::Single(
                "cargo build --release --workspace".to_string(),
            )),
        },
        Language::Python => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml --release"
            ))),
        },
        Language::Node => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node --release"
            ))),
        },
        Language::Wasm => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --dev"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --release"
            ))),
        },
        Language::Go => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "cd {output_dir} && go build ./..."
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "cd {output_dir} && go build ./..."
            ))),
        },
        Language::Ruby => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "cargo build -p {crate_name}-rb"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-rb"
            ))),
        },
        Language::Php => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "cargo build -p {crate_name}-php"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-php"
            ))),
        },
        Language::Ffi => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "cargo build -p {crate_name}-ffi"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-ffi"
            ))),
        },
        Language::Java => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml package -DskipTests -q"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml package -DskipTests -q"
            ))),
        },
        Language::Csharp => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "dotnet build {output_dir} --configuration Debug -q"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "dotnet build {output_dir} --configuration Release -q"
            ))),
        },
        Language::Elixir => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
            build_release: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix compile"
            ))),
        },
        Language::R => BuildCommandConfig {
            build: Some(StringOrVec::Single(format!(
                "cargo build -p {crate_name}-r"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-r"
            ))),
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
    fn every_language_has_build_and_build_release() {
        for lang in all_languages() {
            let cfg = default_build_config(lang, "packages/test", "my-lib");
            assert!(
                cfg.build.is_some(),
                "{lang} should have a default build command"
            );
            assert!(
                cfg.build_release.is_some(),
                "{lang} should have a default build_release command"
            );
        }
    }

    #[test]
    fn rust_uses_cargo_build_workspace() {
        let cfg = default_build_config(Language::Rust, "packages/rust", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        let release = cfg.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build --workspace"));
        assert!(release.contains("cargo build --release --workspace"));
    }

    #[test]
    fn python_uses_maturin_develop() {
        let cfg = default_build_config(Language::Python, "packages/python", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        let release = cfg.build_release.unwrap().commands().join(" ");
        assert!(build.contains("maturin develop"));
        assert!(build.contains("my-lib-py"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn node_uses_napi_build() {
        let cfg = default_build_config(Language::Node, "packages/node", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        let release = cfg.build_release.unwrap().commands().join(" ");
        assert!(build.contains("napi build"));
        assert!(build.contains("my-lib-node"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn wasm_uses_wasm_pack() {
        let cfg = default_build_config(Language::Wasm, "packages/wasm", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        let release = cfg.build_release.unwrap().commands().join(" ");
        assert!(build.contains("wasm-pack build"));
        assert!(build.contains("my-lib-wasm"));
        assert!(build.contains("--dev"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn ffi_uses_cargo_build_p() {
        let cfg = default_build_config(Language::Ffi, "packages/ffi", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        let release = cfg.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-ffi"));
        assert!(release.contains("cargo build --release -p my-lib-ffi"));
    }

    #[test]
    fn ruby_uses_cargo_build_rb() {
        let cfg = default_build_config(Language::Ruby, "packages/ruby", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-rb"));
    }

    #[test]
    fn php_uses_cargo_build_php() {
        let cfg = default_build_config(Language::Php, "packages/php", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-php"));
    }

    #[test]
    fn r_uses_cargo_build_r() {
        let cfg = default_build_config(Language::R, "packages/r", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-r"));
    }

    #[test]
    fn java_uses_maven_package() {
        let cfg = default_build_config(Language::Java, "packages/java", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(build.contains("mvn"));
        assert!(build.contains("package"));
        assert!(build.contains("-DskipTests"));
    }

    #[test]
    fn csharp_uses_dotnet_build_configurations() {
        let cfg = default_build_config(Language::Csharp, "packages/csharp", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        let release = cfg.build_release.unwrap().commands().join(" ");
        assert!(build.contains("dotnet build"));
        assert!(build.contains("--configuration Debug"));
        assert!(release.contains("--configuration Release"));
    }

    #[test]
    fn elixir_uses_mix_compile() {
        let cfg = default_build_config(Language::Elixir, "packages/elixir", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(build.contains("mix compile"));
    }

    #[test]
    fn crate_name_substituted_in_commands() {
        let cfg = default_build_config(Language::Python, "packages/python", "custom-crate");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(
            build.contains("custom-crate-py"),
            "Python build should contain crate name, got: {build}"
        );
    }

    #[test]
    fn output_dir_substituted_in_go_commands() {
        let cfg = default_build_config(Language::Go, "my/custom/path", "my-lib");
        let build = cfg.build.unwrap().commands().join(" ");
        assert!(
            build.contains("my/custom/path"),
            "Go build should contain output dir, got: {build}"
        );
    }
}
