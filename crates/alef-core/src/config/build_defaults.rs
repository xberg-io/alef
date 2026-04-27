use super::extras::Language;
use super::output::{BuildCommandConfig, StringOrVec};
use super::tools::{LangContext, require_tool, wrap_command as wrap};

/// Return the default build configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). The `crate_name` is the name of the core crate
/// (e.g. `my-lib`). Both are substituted into command templates. `ctx`
/// provides tool selection and run_wrapper.
pub(crate) fn default_build_config(
    lang: Language,
    output_dir: &str,
    crate_name: &str,
    ctx: &LangContext,
) -> BuildCommandConfig {
    match lang {
        Language::Rust => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            build: Some(StringOrVec::Single("cargo build --workspace".to_string())),
            build_release: Some(StringOrVec::Single("cargo build --release --workspace".to_string())),
        },
        Language::Python => BuildCommandConfig {
            precondition: Some(require_tool("maturin")),
            before: None,
            build: Some(StringOrVec::Single(format!(
                "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml --release"
            ))),
        },
        Language::Node => BuildCommandConfig {
            precondition: Some(require_tool("napi")),
            before: None,
            build: Some(StringOrVec::Single(format!(
                "napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node --release"
            ))),
        },
        Language::Wasm => BuildCommandConfig {
            precondition: Some(require_tool("wasm-pack")),
            before: None,
            build: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --dev"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --release"
            ))),
        },
        Language::Go => {
            let cmd = format!("cd {output_dir} && go build ./...");
            BuildCommandConfig {
                precondition: Some(require_tool("go")),
                before: None,
                build: Some(StringOrVec::Single(wrap(cmd.clone(), ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(cmd, ctx.run_wrapper))),
            }
        }
        Language::Ruby => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-rb"))),
            build_release: Some(StringOrVec::Single(format!("cargo build --release -p {crate_name}-rb"))),
        },
        Language::Php => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-php"))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-php"
            ))),
        },
        Language::Ffi => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-ffi"))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-ffi"
            ))),
        },
        Language::Java => {
            let (build_path, release_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("mvn -f {proj} package -DskipTests -q"),
                    format!("mvn -f {proj} package -DskipTests -q"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml package -DskipTests -q"),
                    format!("mvn -f {output_dir}/pom.xml package -DskipTests -q"),
                )
            };
            BuildCommandConfig {
                precondition: Some(require_tool("mvn")),
                before: None,
                build: Some(StringOrVec::Single(wrap(build_path, ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(release_path, ctx.run_wrapper))),
            }
        }
        Language::Csharp => {
            let (build_path, release_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("dotnet build {proj} --configuration Debug -q"),
                    format!("dotnet build {proj} --configuration Release -q"),
                )
            } else {
                (
                    format!("dotnet build {output_dir} --configuration Debug -q"),
                    format!("dotnet build {output_dir} --configuration Release -q"),
                )
            };
            BuildCommandConfig {
                precondition: Some(require_tool("dotnet")),
                before: None,
                build: Some(StringOrVec::Single(wrap(build_path, ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(release_path, ctx.run_wrapper))),
            }
        }
        Language::Elixir => BuildCommandConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            build: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
            build_release: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
        },
        Language::R => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-r"))),
            build_release: Some(StringOrVec::Single(format!("cargo build --release -p {crate_name}-r"))),
        },
        Language::Kotlin => BuildCommandConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle build -Prelease"),
                ctx.run_wrapper,
            ))),
        },
        Language::Swift => BuildCommandConfig {
            precondition: Some(require_tool("swift")),
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("swift build --package-path {output_dir}"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("swift build --package-path {output_dir} --configuration release"),
                ctx.run_wrapper,
            ))),
        },
        Language::Dart => BuildCommandConfig {
            precondition: Some(require_tool("dart")),
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && dart pub get"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && dart pub get"),
                ctx.run_wrapper,
            ))),
        },
        Language::Gleam => BuildCommandConfig {
            precondition: Some(require_tool("gleam")),
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gleam build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gleam build"),
                ctx.run_wrapper,
            ))),
        },
        Language::Zig => BuildCommandConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && zig build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && zig build --release=fast"),
                ctx.run_wrapper,
            ))),
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

    fn cfg(lang: Language, dir: &str, crate_name: &str) -> BuildCommandConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_build_config(lang, dir, crate_name, &ctx)
    }

    #[test]
    fn every_language_has_build_and_build_release() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            assert!(c.build.is_some(), "{lang} should have a default build command");
            assert!(
                c.build_release.is_some(),
                "{lang} should have a default build_release command"
            );
        }
    }

    #[test]
    fn every_language_has_default_precondition() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn rust_uses_cargo_build_workspace() {
        let c = cfg(Language::Rust, "packages/rust", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build --workspace"));
        assert!(release.contains("cargo build --release --workspace"));
    }

    #[test]
    fn python_uses_maturin_develop() {
        let c = cfg(Language::Python, "packages/python", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("maturin develop"));
        assert!(build.contains("my-lib-py"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn node_uses_napi_build() {
        let c = cfg(Language::Node, "packages/node", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("napi build"));
        assert!(build.contains("my-lib-node"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn wasm_uses_wasm_pack() {
        let c = cfg(Language::Wasm, "packages/wasm", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("wasm-pack build"));
        assert!(build.contains("my-lib-wasm"));
        assert!(build.contains("--dev"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn ffi_uses_cargo_build_p() {
        let c = cfg(Language::Ffi, "packages/ffi", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-ffi"));
        assert!(release.contains("cargo build --release -p my-lib-ffi"));
    }

    #[test]
    fn ruby_uses_cargo_build_rb() {
        let c = cfg(Language::Ruby, "packages/ruby", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-rb"));
    }

    #[test]
    fn php_uses_cargo_build_php() {
        let c = cfg(Language::Php, "packages/php", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-php"));
    }

    #[test]
    fn r_uses_cargo_build_r() {
        let c = cfg(Language::R, "packages/r", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-r"));
    }

    #[test]
    fn java_uses_maven_package() {
        let c = cfg(Language::Java, "packages/java", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("mvn"));
        assert!(build.contains("package"));
        assert!(build.contains("-DskipTests"));
    }

    #[test]
    fn csharp_uses_dotnet_build_configurations() {
        let c = cfg(Language::Csharp, "packages/csharp", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("dotnet build"));
        assert!(build.contains("--configuration Debug"));
        assert!(release.contains("--configuration Release"));
    }

    #[test]
    fn elixir_uses_mix_compile() {
        let c = cfg(Language::Elixir, "packages/elixir", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("mix compile"));
    }

    #[test]
    fn crate_name_substituted_in_commands() {
        let c = cfg(Language::Python, "packages/python", "custom-crate");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("custom-crate-py"));
    }

    #[test]
    fn output_dir_substituted_in_go_commands() {
        let c = cfg(Language::Go, "my/custom/path", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("my/custom/path"));
    }

    #[test]
    fn kotlin_uses_gradle_build() {
        let c = cfg(Language::Kotlin, "packages/kotlin", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("gradle build"), "Kotlin build should use gradle build, got: {build}");
        assert!(release.contains("gradle build"), "Kotlin release should use gradle build, got: {release}");
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_build_with_package_path() {
        let c = cfg(Language::Swift, "packages/swift", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("swift build"), "Swift build should use swift build, got: {build}");
        assert!(build.contains("--package-path packages/swift"), "Swift build should include package path, got: {build}");
        assert!(release.contains("--configuration release"), "Swift release should use --configuration release, got: {release}");
    }

    #[test]
    fn dart_uses_dart_pub_get() {
        let c = cfg(Language::Dart, "packages/dart", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("dart pub get"), "Dart build should use dart pub get, got: {build}");
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
    }

    #[test]
    fn gleam_uses_gleam_build() {
        let c = cfg(Language::Gleam, "packages/gleam", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("gleam build"), "Gleam build should use gleam build, got: {build}");
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_build() {
        let c = cfg(Language::Zig, "packages/zig", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("zig build"), "Zig build should use zig build, got: {build}");
        assert!(release.contains("--release=fast"), "Zig release should use --release=fast, got: {release}");
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
