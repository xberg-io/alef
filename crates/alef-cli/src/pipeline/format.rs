use alef_core::config::{AlefConfig, Language};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, warn};

/// One formatter invocation (command + args).
struct FormatterCommand {
    command: String,
    args: Vec<String>,
}

/// Post-generation formatter configuration.
/// Each language may run a sequence of formatter commands in a working directory.
struct FormatterSpec {
    /// Commands to run in sequence; on first failure the rest are skipped (warning logged).
    commands: Vec<FormatterCommand>,
    /// Working directory relative to project root; empty string = project root.
    work_dir: String,
}

/// Get the default formatter spec for a language.
///
/// Notes on Rust formatting: backends that emit Rust code (FFI, PyO3, NAPI, Magnus,
/// ext-php-rs, Rustler, wasm-bindgen) all live inside the consumer workspace, so a
/// single `cargo fmt --all` from the project root covers every generated `.rs` file.
/// We attach this to `Language::Ffi` because FFI is always present when any of the
/// C-FFI-bridged languages (Go/Java/C#) are enabled, and harmless when only WASM
/// or pure-Rust bindings are used.
fn get_default_formatter(config: &AlefConfig, lang: Language) -> Option<FormatterSpec> {
    match lang {
        // ruff check --fix runs lint autofixes (unused imports, missing TypeAlias
        // annotations, import sorting); ruff format applies whitespace formatting.
        // Both must run — `format` alone leaves I001/F401/TC008 issues that fail CI.
        Language::Python => Some(FormatterSpec {
            commands: vec![
                FormatterCommand {
                    command: "ruff".to_owned(),
                    args: vec!["check".to_owned(), "--fix".to_owned(), ".".to_owned()],
                },
                FormatterCommand {
                    command: "ruff".to_owned(),
                    args: vec!["format".to_owned(), ".".to_owned()],
                },
            ],
            work_dir: "packages/python/".to_owned(),
        }),
        Language::Node => Some(FormatterSpec {
            // Run the Oxc formatter and linter from the repo root so package,
            // e2e, and registry-mode test app output are normalized consistently.
            commands: vec![
                FormatterCommand {
                    command: "npx".to_owned(),
                    args: vec!["oxfmt".to_owned(), ".".to_owned()],
                },
                FormatterCommand {
                    command: "npx".to_owned(),
                    args: vec!["oxlint".to_owned(), "--fix".to_owned(), ".".to_owned()],
                },
            ],
            work_dir: ".".to_owned(),
        }),
        Language::Ruby => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "rubocop".to_owned(),
                args: vec!["-A".to_owned(), "--no-server".to_owned()],
            }],
            work_dir: "packages/ruby/".to_owned(),
        }),
        Language::Php => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "php-cs-fixer".to_owned(),
                args: vec!["fix".to_owned()],
            }],
            work_dir: "packages/php/".to_owned(),
        }),
        Language::Elixir => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "mix".to_owned(),
                args: vec!["format".to_owned()],
            }],
            work_dir: "packages/elixir/".to_owned(),
        }),
        Language::Go => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "gofmt".to_owned(),
                args: vec!["-w".to_owned(), ".".to_owned()],
            }],
            work_dir: "packages/go/".to_owned(),
        }),
        // google-java-format requires explicit file paths — no recursive flag.
        // We collect *.java files from the work_dir at runtime and pass them as args.
        // The command is built dynamically in `format_generated`; this spec carries
        // only the base args (the file list is appended before invocation).
        Language::Java => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "google-java-format".to_owned(),
                args: vec!["-i".to_owned()],
            }],
            work_dir: "packages/java/src/".to_owned(),
        }),
        // Bug fix: when both a .csproj and a .slnx exist in packages/csharp/, `dotnet
        // format` without a workspace argument aborts. Use the project_file from config
        // when available so the correct project is targeted unambiguously.
        Language::Csharp => {
            let mut args = vec!["format".to_owned()];
            let work_dir = "packages/csharp/".to_owned();
            if let Some(project_file) = config.project_file_for_language(Language::Csharp) {
                // project_file is a path relative to the project root (e.g.
                // "packages/csharp/LiterLlm.csproj"). Strip the work_dir prefix so the
                // argument is relative to work_dir where the command runs.
                let relative = Path::new(project_file)
                    .strip_prefix(&work_dir)
                    .unwrap_or(Path::new(project_file));
                args.push(relative.to_string_lossy().into_owned());
            }
            Some(FormatterSpec {
                commands: vec![FormatterCommand {
                    command: "dotnet".to_owned(),
                    args,
                }],
                work_dir,
            })
        }
        // Bug fix: derive wasm crate name from config instead of hardcoding "wasm".
        // Runs at workspace root so `cargo fmt -p <pkg>` can resolve the workspace member.
        Language::Wasm => {
            let wasm_crate = format!("{}-wasm", config.core_crate_dir());
            Some(FormatterSpec {
                commands: vec![FormatterCommand {
                    command: "cargo".to_owned(),
                    args: vec!["fmt".to_owned(), "-p".to_owned(), wasm_crate],
                }],
                work_dir: String::new(),
            })
        }
        // FFI runs `cargo fmt --all` from project root: this formats every generated
        // Rust crate in the consumer workspace (FFI, PyO3, NAPI-RS, Magnus, ext-php-rs,
        // Rustler, wasm-bindgen). Was previously `cargo fmt` in `packages/ffi/` —
        // which has no Cargo.toml, so it silently no-op'd and CI's `cargo fmt --check`
        // would fail on the unformatted FFI lib.rs.
        Language::Ffi => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "cargo".to_owned(),
                args: vec!["fmt".to_owned(), "--all".to_owned()],
            }],
            work_dir: String::new(),
        }),
        Language::R => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "Rscript".to_owned(),
                args: vec!["-e".to_owned(), "styler::style_pkg('packages/r')".to_owned()],
            }],
            work_dir: String::new(),
        }),
        Language::Kotlin => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "ktlint".to_owned(),
                args: vec!["--format".to_owned()],
            }],
            work_dir: "packages/kotlin/src/".to_owned(),
        }),
        Language::Swift => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "swift".to_owned(),
                args: vec![
                    "format".to_owned(),
                    "--in-place".to_owned(),
                    "--recursive".to_owned(),
                    "Sources".to_owned(),
                ],
            }],
            work_dir: "packages/swift/".to_owned(),
        }),
        Language::Dart => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "dart".to_owned(),
                args: vec!["format".to_owned(), ".".to_owned()],
            }],
            work_dir: "packages/dart/".to_owned(),
        }),
        Language::Gleam => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "gleam".to_owned(),
                args: vec!["format".to_owned()],
            }],
            work_dir: "packages/gleam/".to_owned(),
        }),
        Language::Zig => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "zig".to_owned(),
                args: vec!["fmt".to_owned(), "src".to_owned()],
            }],
            work_dir: "packages/zig/".to_owned(),
        }),
        Language::Rust => None,
    }
}

/// Detect whether the consumer's Java package configures Spotless via
/// `spotless-maven-plugin`. The walked pom is `<base>/<work_dir>/../pom.xml`
/// (one level up from `packages/java/src/`, which is `packages/java/pom.xml`).
///
/// Returns the path to the pom when Spotless is configured, otherwise `None`.
/// The check is conservative — a literal substring match on the plugin
/// artifactId — because parsing a full pom is wildly out of scope for a
/// formatter-selection heuristic.
fn detect_spotless_pom(base_dir: &Path, java_work_dir: &str) -> Option<PathBuf> {
    let pom = base_dir.join(java_work_dir).parent()?.join("pom.xml");
    if !pom.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&pom).ok()?;
    if content.contains("spotless-maven-plugin") {
        Some(pom)
    } else {
        None
    }
}

/// Collect all `.java` files under `dir` recursively (up to `limit` paths).
/// Returns an empty vec if the directory does not exist or cannot be read.
fn collect_java_files(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let pattern = format!("{}/**/*.java", dir.display());
    let Ok(entries) = glob::glob(&pattern) else {
        return vec![];
    };
    entries.flatten().filter(|p| p.is_file()).take(limit).collect()
}

/// Run language-native formatters on emitted packages after generation.
/// For each language in the output, if formatting is enabled and the formatter binary
/// is available, run the formatter in the package directory.
/// Formatter errors are logged as warnings and do not fail the generate command.
pub fn format_generated(
    files: &[(Language, Vec<alef_core::backend::GeneratedFile>)],
    config: &AlefConfig,
    base_dir: &Path,
    only_languages: Option<&std::collections::HashSet<Language>>,
) {
    let mut formatted_langs = std::collections::HashSet::new();

    for (lang, _) in files {
        // Skip if already formatted in this batch
        if formatted_langs.contains(lang) {
            continue;
        }
        // Skip languages outside the explicit filter (if provided).
        if let Some(filter) = only_languages {
            if !filter.contains(lang) {
                continue;
            }
        }

        // Resolve format config for this language (check overrides first)
        let lang_str = lang.to_string().to_lowercase();
        let format_cfg = config
            .format_overrides
            .get(&lang_str)
            .cloned()
            .unwrap_or_else(|| config.format.clone());

        if !format_cfg.enabled {
            debug!("  [{lang_str}] formatting disabled, skipping");
            continue;
        }

        // Get the formatter command (custom or default)
        let formatter_cmd = if let Some(custom) = format_cfg.command {
            // Custom command: run as-is in the package directory
            if !run_custom_formatter(&custom, base_dir) {
                warn!("[{lang_str}] custom formatter failed");
            }
            formatted_langs.insert(*lang);
            continue;
        } else if let Some(spec) = get_default_formatter(config, *lang) {
            // For Java, prefer the project's Spotless pipeline when configured in
            // packages/java/pom.xml — running the same tool the project's prek
            // hook would run keeps `alef-verify` stable. Falling back to
            // google-java-format would produce different bytes than Spotless,
            // and the embedded `alef:hash:` value would invalidate as soon as
            // prek's `mvn spotless:apply` ran. See issue tslp/v1.8.0-rc.14.
            if *lang == Language::Java
                && let Some(pom) = detect_spotless_pom(base_dir, &spec.work_dir)
            {
                debug!(
                    "  [java] spotless detected at {}, using mvn spotless:apply",
                    pom.display()
                );
                FormatterSpec {
                    commands: vec![FormatterCommand {
                        command: "mvn".to_owned(),
                        args: vec![
                            "-f".to_owned(),
                            pom.to_string_lossy().into_owned(),
                            "spotless:apply".to_owned(),
                            "-q".to_owned(),
                        ],
                    }],
                    work_dir: spec.work_dir,
                }
            } else {
                spec
            }
        } else {
            // No formatter for this language (e.g., Rust is formatted in-memory before writing)
            debug!("  [{lang_str}] no default formatter configured");
            continue;
        };

        // Resolve work dir (empty string = project root)
        let work_dir = if formatter_cmd.work_dir.is_empty() {
            base_dir.to_path_buf()
        } else {
            base_dir.join(&formatter_cmd.work_dir)
        };
        if !work_dir.exists() {
            debug!(
                "  [{lang_str}] package directory does not exist: {}, skipping",
                work_dir.display()
            );
            continue;
        }

        // Run each command in sequence; stop on first failure (warning logged)
        for step in &formatter_cmd.commands {
            if !is_tool_available(&step.command) {
                warn!("[{lang_str}] formatter not found: {} (skipping format)", step.command);
                break;
            }

            // For Java, google-java-format requires explicit file paths: collect them now.
            // Spotless and other Maven-driven formatters operate on the pom and don't
            // take per-file arguments, so the collection is skipped for them.
            let extra_args: Vec<String> = if *lang == Language::Java && step.command == "google-java-format" {
                const JAVA_FILE_BATCH_LIMIT: usize = 200;
                let java_files = collect_java_files(&work_dir, JAVA_FILE_BATCH_LIMIT);
                if java_files.is_empty() {
                    debug!(
                        "  [{lang_str}] no .java files found in {}, skipping",
                        work_dir.display()
                    );
                    break;
                }
                java_files
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            } else {
                vec![]
            };

            let mut all_args: Vec<&str> = step.args.iter().map(String::as_str).collect();
            let extra_refs: Vec<&str> = extra_args.iter().map(String::as_str).collect();
            all_args.extend_from_slice(&extra_refs);

            match run_formatter(&step.command, &all_args, &work_dir) {
                Ok(()) => {
                    debug!("  [{lang_str}] {} {:?} ok", step.command, all_args);
                }
                Err(e) => {
                    warn!("[{lang_str}] {} {:?} failed: {}", step.command, all_args, e);
                    break;
                }
            }
        }

        formatted_langs.insert(*lang);
    }
}

/// Check if a tool is available on PATH.
fn is_tool_available(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run a formatter command with arguments in a specific directory.
fn run_formatter(command: &str, args: &[&str], work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new(command).args(args).current_dir(work_dir).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "formatter exited with code {:?}: {}",
            output.status.code(),
            stderr.trim()
        ));
    }

    Ok(())
}

/// Run a custom formatter command (shell-style string) in a directory.
fn run_custom_formatter(cmd: &str, work_dir: &Path) -> bool {
    let output = Command::new("sh").arg("-c").arg(cmd).current_dir(work_dir).output();

    match output {
        Ok(out) => out.status.success(),
        Err(e) => {
            debug!("custom formatter error: {}", e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::{AlefConfig, Language};

    fn make_config(crate_name: &str) -> AlefConfig {
        toml::from_str(&format!(
            r#"
languages = ["rust"]

[crate]
name = "{crate_name}"
sources = ["src/lib.rs"]
"#
        ))
        .expect("valid config")
    }

    fn make_config_with_csharp_project(crate_name: &str, project_file: &str) -> AlefConfig {
        toml::from_str(&format!(
            r#"
languages = ["csharp"]

[crate]
name = "{crate_name}"
sources = ["src/lib.rs"]

[csharp]
project_file = "{project_file}"
"#
        ))
        .expect("valid config")
    }

    fn make_config_with_source(crate_name: &str, source: &str) -> AlefConfig {
        toml::from_str(&format!(
            r#"
languages = ["wasm"]

[crate]
name = "{crate_name}"
sources = ["{source}"]
"#
        ))
        .expect("valid config")
    }

    // Bug 1: WASM crate name must be derived from the core crate dir, not the
    // public package name.
    #[test]
    fn test_wasm_formatter_uses_config_crate_name() {
        let config = make_config("liter-llm");
        let spec = get_default_formatter(&config, Language::Wasm).expect("should have formatter");
        assert_eq!(spec.commands.len(), 1);
        let cmd = &spec.commands[0];
        assert_eq!(cmd.command, "cargo");
        assert_eq!(cmd.args, vec!["fmt", "-p", "liter-llm-wasm"]);
        assert!(spec.work_dir.is_empty(), "WASM formatter must run at workspace root");
    }

    #[test]
    fn test_wasm_formatter_different_crate_name() {
        let config = make_config_with_source("kreuzberg", "crates/kreuzberg-core/src/lib.rs");
        let spec = get_default_formatter(&config, Language::Wasm).expect("should have formatter");
        let cmd = &spec.commands[0];
        assert_eq!(cmd.args, vec!["fmt", "-p", "kreuzberg-core-wasm"]);
    }

    // Bug 2: C# formatter must include project_file when configured to avoid workspace ambiguity.
    #[test]
    fn test_csharp_formatter_with_project_file() {
        let config = make_config_with_csharp_project("liter-llm", "packages/csharp/LiterLlm.csproj");
        let spec = get_default_formatter(&config, Language::Csharp).expect("should have formatter");
        assert_eq!(spec.commands.len(), 1);
        let cmd = &spec.commands[0];
        assert_eq!(cmd.command, "dotnet");
        assert!(cmd.args.contains(&"format".to_owned()), "args must contain 'format'");
        assert!(
            cmd.args.contains(&"LiterLlm.csproj".to_owned()),
            "args must contain the relative project file, got: {:?}",
            cmd.args
        );
        assert_eq!(spec.work_dir, "packages/csharp/");
    }

    #[test]
    fn test_csharp_formatter_without_project_file() {
        let config = make_config("liter-llm");
        let spec = get_default_formatter(&config, Language::Csharp).expect("should have formatter");
        let cmd = &spec.commands[0];
        assert_eq!(cmd.command, "dotnet");
        assert_eq!(
            cmd.args,
            vec!["format"],
            "without project_file, args must be just ['format']"
        );
    }

    // Bug 3: Java file collection — only .java files are returned, non-.java files are excluded.
    #[test]
    fn test_collect_java_files_returns_only_java_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();

        // Create a nested structure with .java and other files
        std::fs::create_dir_all(root.join("com/example")).unwrap();
        std::fs::write(root.join("com/example/Foo.java"), "class Foo {}").unwrap();
        std::fs::write(root.join("com/example/Bar.java"), "class Bar {}").unwrap();
        std::fs::write(root.join("com/example/readme.txt"), "ignore me").unwrap();
        std::fs::write(root.join("com/example/Baz.class"), "ignore me").unwrap();

        let files = collect_java_files(root, 200);
        assert_eq!(files.len(), 2, "expected 2 .java files, got: {:?}", files);
        assert!(files.iter().all(|p| p.extension().is_some_and(|e| e == "java")));
    }

    #[test]
    fn test_collect_java_files_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = collect_java_files(dir.path(), 200);
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_java_files_nonexistent_dir() {
        let files = collect_java_files(Path::new("/nonexistent/path/to/src"), 200);
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_java_files_respects_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        for i in 0..10 {
            std::fs::write(root.join(format!("File{i}.java")), "class Foo {}").unwrap();
        }
        let files = collect_java_files(root, 5);
        assert_eq!(files.len(), 5);
    }
}
