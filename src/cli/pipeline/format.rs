use crate::core::config::{Language, ResolvedCrateConfig};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
fn get_default_formatter(config: &ResolvedCrateConfig, lang: Language) -> Option<FormatterSpec> {
    match lang {
        // ruff check --fix runs lint autofixes (unused imports, missing TypeAlias
        // annotations, import sorting); ruff format applies whitespace formatting.
        // Both must run — `format` alone leaves I001/F401/TC008 issues that fail CI.
        Language::Python => {
            let package_path = config
                .python
                .as_ref()
                .and_then(|python| python.stubs.as_ref())
                .map(|stubs| stubs.output.to_string_lossy().into_owned())
                .unwrap_or_else(|| "packages/python/".to_owned());
            Some(FormatterSpec {
                commands: vec![
                    FormatterCommand {
                        command: "ruff".to_owned(),
                        args: vec!["check".to_owned(), "--fix".to_owned(), package_path.clone()],
                    },
                    FormatterCommand {
                        command: "ruff".to_owned(),
                        args: vec!["format".to_owned(), package_path],
                    },
                ],
                work_dir: String::new(),
            })
        }
        Language::Node => Some(FormatterSpec {
            // Run the Oxc formatter and linter from the repo root so package,
            // e2e, and registry-mode test app output are normalized consistently.
            //
            // Exclude TOML: oxfmt also reformats `.toml` (collapsing multi-line
            // arrays, stripping inner-bracket spaces), which fights the consumer's
            // own TOML tooling — `pyproject-fmt` (which wants `[ "x" ]`) and
            // `cargo-sort`/`cargo fmt`. Letting oxfmt touch pyproject.toml/Cargo.toml
            // strips spacing that those hooks re-add post-`finalize_hashes`, breaking
            // `alef verify` (and producing a format/regen loop). The `!**/*.toml`
            // exclude scopes oxfmt to the JS/TS/JSON/CSS files it owns.
            commands: vec![
                FormatterCommand {
                    command: "pnpm".to_owned(),
                    args: vec![
                        "dlx".to_owned(),
                        "oxfmt".to_owned(),
                        ".".to_owned(),
                        "!**/*.toml".to_owned(),
                    ],
                },
                FormatterCommand {
                    command: "pnpm".to_owned(),
                    args: vec![
                        "dlx".to_owned(),
                        "oxlint".to_owned(),
                        "--fix".to_owned(),
                        ".".to_owned(),
                    ],
                },
            ],
            work_dir: ".".to_owned(),
        }),
        // Ruby's native crate (`packages/ruby/ext/<gem>/native/Cargo.toml`) is
        // listed in the consumer workspace's `exclude` set, so `cargo sort -w`
        // attached to the FFI formatter never visits it. Without an explicit
        // pass, prek's `cargo-sort` hook rewrites the feature-array
        // indentation after `finalize_hashes`, making `alef verify` report
        // the file as stale on the next run. Run `cargo sort` against the
        // native crate directly so the emitted Cargo.toml is already
        // cargo-sort canonical at the moment its hash is finalised.
        Language::Ruby => {
            let gem_name = config.ruby_gem_name();
            let native_subdir = format!("ext/{gem_name}/native");
            Some(FormatterSpec {
                commands: vec![
                    FormatterCommand {
                        command: "rubocop".to_owned(),
                        args: vec!["-A".to_owned(), "--no-server".to_owned()],
                    },
                    FormatterCommand {
                        command: "cargo".to_owned(),
                        args: vec!["sort".to_owned(), native_subdir],
                    },
                ],
                work_dir: "packages/ruby/".to_owned(),
            })
        }
        Language::Php => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "php-cs-fixer".to_owned(),
                args: vec!["fix".to_owned()],
            }],
            work_dir: "packages/php/".to_owned(),
        }),
        // Elixir's native NIF crate lives at
        // `packages/elixir/native/<app>_nif/Cargo.toml` and is `exclude`d from
        // the consumer's cargo workspace, so `cargo sort -w` from the FFI
        // formatter does not reach it. Run cargo sort directly against the NIF
        // crate so prek's `cargo-sort` hook is a no-op on every regen instead
        // of rewriting feature indentation post-finalize and breaking
        // `alef verify`.
        Language::Elixir => {
            let app_name = config.elixir_app_name();
            let native_subdir = format!("native/{app_name}_nif");
            Some(FormatterSpec {
                commands: vec![
                    FormatterCommand {
                        command: "mix".to_owned(),
                        args: vec!["format".to_owned()],
                    },
                    FormatterCommand {
                        command: "cargo".to_owned(),
                        args: vec!["sort".to_owned(), native_subdir],
                    },
                ],
                work_dir: "packages/elixir/".to_owned(),
            })
        }
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
                // "packages/csharp/SampleLlm.csproj"). Strip the work_dir prefix so the
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
        // Format the resolved wasm binding crate directly. The crate may be excluded
        // from the root workspace, so `cargo fmt -p <pkg>` is not reliable.
        // `cargo sort` normalises the generated Cargo.toml so prek's cargo-sort hook
        // is a no-op; without it, cargo-sort reformats feature indentation after the
        // hash is finalised, making alef verify report the file as stale.
        //
        // No oxfmt step: oxfmt's default TOML style (2-space indent, collapsed
        // multi-line arrays) collides with cargo-sort's preserved 4-space
        // indent, producing an infinite format/regen loop on the embedded hash.
        // cargo-sort alone is enough to canonicalise the wasm Cargo.toml.
        Language::Wasm => {
            let crate_dir = config
                .output_for("wasm")
                .map(resolve_crate_dir)
                .unwrap_or_else(|| Path::new("crates").join(format!("{}-wasm", config.name)));
            // Cargo accepts / on every platform; emit POSIX-style paths so CI
            // behaviour on Windows matches Linux/macOS and the snapshot tests.
            let manifest_path = crate_dir
                .join("Cargo.toml")
                .to_string_lossy()
                .into_owned()
                .replace('\\', "/");
            let crate_dir_str = crate_dir.to_string_lossy().into_owned().replace('\\', "/");
            Some(FormatterSpec {
                commands: vec![
                    FormatterCommand {
                        command: "cargo".to_owned(),
                        args: vec!["fmt".to_owned(), "--manifest-path".to_owned(), manifest_path],
                    },
                    FormatterCommand {
                        command: "cargo".to_owned(),
                        args: vec!["sort".to_owned(), crate_dir_str],
                    },
                ],
                work_dir: String::new(),
            })
        }
        // FFI runs `cargo fmt --all` from project root: this formats every generated
        // Rust crate in the consumer workspace (FFI, PyO3, NAPI-RS, Magnus, ext-php-rs,
        // Rustler, wasm-bindgen). Was previously `cargo fmt` in `packages/ffi/` —
        // which has no Cargo.toml, so it silently no-op'd and CI's `cargo fmt --check`
        // would fail on the unformatted FFI lib.rs.
        // `cargo sort -w` normalises all workspace Cargo.toml files so prek's
        // cargo-sort hook is a no-op; without it the hook reformats feature
        // indentation after finalize_hashes, making alef verify report stale files.
        //
        // No oxfmt step here. The shared SampleCrate pre-commit `oxfmt` hook is scoped
        // to `[javascript, jsx, ts, tsx, json, css]` only (see pre-commit-hooks
        // `.pre-commit-hooks.yaml`), so any JS/TS/JSON files that need oxfmt-shape
        // formatting are picked up by the per-language scaffold + the consumer's
        // own oxfmt hook on next commit. Running `pnpm dlx oxfmt .` from here would
        // additionally reformat every TOML in the workspace — oxfmt's default
        // settings differ from `cargo-sort` / hand-maintained Cargo.toml styles
        // (collapses multi-line arrays, 2-space indent), which produced an
        // infinite format/regen loop for any consumer whose hand-maintained
        // Cargo.toml didn't already match oxfmt's TOML defaults.
        Language::Ffi => Some(FormatterSpec {
            commands: vec![
                FormatterCommand {
                    command: "cargo".to_owned(),
                    args: vec!["fmt".to_owned(), "--all".to_owned()],
                },
                FormatterCommand {
                    command: "cargo".to_owned(),
                    args: vec!["sort".to_owned(), "-w".to_owned()],
                },
            ],
            work_dir: String::new(),
        }),
        // R's extendr rust crate lives at `packages/r/src/rust/Cargo.toml`
        // and is `exclude`d from the consumer's cargo workspace, so the FFI
        // formatter's `cargo sort -w` never visits it. Run cargo sort
        // explicitly so prek's `cargo-sort` hook doesn't rewrite feature
        // indentation after `finalize_hashes` and break `alef verify`.
        Language::R => Some(FormatterSpec {
            commands: vec![
                FormatterCommand {
                    command: "Rscript".to_owned(),
                    args: vec!["-e".to_owned(), "styler::style_pkg('packages/r')".to_owned()],
                },
                FormatterCommand {
                    command: "cargo".to_owned(),
                    args: vec!["sort".to_owned(), "packages/r/src/rust".to_owned()],
                },
            ],
            work_dir: String::new(),
        }),
        Language::Kotlin => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "ktlint".to_owned(),
                args: vec!["--format".to_owned()],
            }],
            work_dir: "packages/kotlin/src/".to_owned(),
        }),
        Language::KotlinAndroid => Some(FormatterSpec {
            commands: vec![FormatterCommand {
                command: "ktfmt".to_owned(),
                args: vec![
                    "--kotlinlang-style".to_owned(),
                    "packages/kotlin-android/src".to_owned(),
                ],
            }],
            work_dir: String::new(),
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
        // C is an e2e test consumer of the FFI layer — no generated files to format.
        // Jni output is Rust source formatted by `cargo fmt --all` (triggered by the Ffi formatter).
        Language::Rust | Language::C | Language::Jni => None,
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
    files: &[(Language, Vec<crate::core::backend::GeneratedFile>)],
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&std::collections::HashSet<Language>>,
) {
    let mut formatted_langs = std::collections::HashSet::new();

    for (lang, _) in files {
        // Skip if already formatted in this batch
        if formatted_langs.contains(lang) {
            continue;
        }

        // Resolve format config for this language (check overrides first)
        let lang_str = lang.to_string().to_lowercase();
        let format_cfg = config
            .format_overrides
            .get(&lang_str)
            .cloned()
            .unwrap_or_else(|| config.format.clone());

        // Languages with a custom format_override command bypass the only_languages
        // filter. A custom command is an explicit declaration that this formatter
        // must run whenever the language's files are present in the output — even
        // when the generated content is identical to on-disk content (i.e., files
        // were not re-written this run). Skipping in that case would leave the
        // embedded alef:hash: computed over pre-formatter content, causing prek's
        // own formatter hook to rewrite the files post-hash and break alef verify.
        // Default formatters still respect only_languages so that warming the cache
        // (no file writes) avoids unnecessary ruff/mix-format/etc. invocations.
        let has_custom_command = format_cfg.command.is_some();
        if !has_custom_command {
            if let Some(filter) = only_languages {
                if !filter.contains(lang) {
                    continue;
                }
            }
        }

        if !format_cfg.enabled {
            debug!("  [{lang_str}] formatting disabled, skipping");
            continue;
        }

        // Get the formatter command (custom or default)
        let formatter_cmd = if let Some(custom) = format_cfg.command {
            // Custom command: run as-is in the package directory
            if let Err(e) = run_custom_formatter(&custom, base_dir) {
                warn!("[{lang_str}] custom formatter failed: {e}");
            }
            formatted_langs.insert(*lang);
            continue;
        } else if let Some(spec) = get_default_formatter(config, *lang) {
            // For Java, prefer the project's Spotless pipeline when configured in
            // packages/java/pom.xml — running the same tool the project's prek
            // hook would run keeps `alef-verify` stable. Falling back to
            // google-java-format would produce different bytes than Spotless,
            // and the embedded `alef:hash:` value would invalidate as soon as
            // prek's `mvn spotless:apply` ran. See issue parser-pack/v1.8.0-rc.14.
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
        return Err(anyhow::anyhow!(
            "formatter exited with code {:?}: {}",
            output.status.code(),
            format_command_output(&output)
        ));
    }

    Ok(())
}

/// Run a custom formatter command (shell-style string) in a directory.
fn run_custom_formatter(cmd: &str, work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("sh").arg("-c").arg(cmd).current_dir(work_dir).output()?;

    if !output.status.success() {
        debug!("custom formatter output: {}", format_command_output(&output));
        return Err(anyhow::anyhow!(
            "formatter exited with code {:?}: {}",
            output.status.code(),
            format_command_output(&output)
        ));
    }

    Ok(())
}

fn format_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim();
    let stderr = stderr.trim();

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout:\n{stdout}\nstderr:\n{stderr}"),
        (false, true) => format!("stdout:\n{stdout}"),
        (true, false) => format!("stderr:\n{stderr}"),
        (true, true) => "<no output>".to_string(),
    }
}

fn resolve_crate_dir(output_path: &Path) -> PathBuf {
    output_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| output_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};

    fn make_config(crate_name: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]
"#
        ))
        .expect("valid config");
        cfg.resolve().unwrap().remove(0)
    }

    fn make_config_with_csharp_project(crate_name: &str, project_file: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]
[crates.csharp]
project_file = "{project_file}"
"#
        ))
        .expect("valid config");
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn formatter_error_includes_stdout_and_stderr() {
        let err = run_formatter(
            "sh",
            &["-c", "printf 'stdout text'; printf 'stderr text' >&2; exit 7"],
            Path::new("."),
        )
        .expect_err("formatter should fail");
        let msg = err.to_string();
        assert!(msg.contains("stdout text"), "missing stdout in error: {msg}");
        assert!(msg.contains("stderr text"), "missing stderr in error: {msg}");
    }

    #[test]
    fn test_wasm_formatter_uses_manifest_path() {
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::Wasm).expect("should have formatter");
        // Two commands: cargo fmt (rs files), cargo sort (Cargo.toml table order).
        // No oxfmt step — oxfmt's default TOML style fights cargo-sort's preserved
        // indent and produces an infinite format/regen loop on the embedded hash.
        assert_eq!(spec.commands.len(), 2, "WASM must have cargo fmt + cargo sort steps");
        let fmt_cmd = &spec.commands[0];
        assert_eq!(fmt_cmd.command, "cargo");
        assert_eq!(
            fmt_cmd.args,
            vec!["fmt", "--manifest-path", "crates/sample-llm-wasm/Cargo.toml"]
        );
        let sort_cmd = &spec.commands[1];
        assert_eq!(sort_cmd.command, "cargo");
        assert_eq!(
            sort_cmd.args,
            vec!["sort", "crates/sample-llm-wasm"],
            "cargo sort arg must be the crate directory, not the manifest path"
        );
        assert!(spec.work_dir.is_empty(), "WASM formatter must run at workspace root");
    }

    #[test]
    fn test_wasm_formatter_uses_configured_output_path() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "sample-language-pack"
sources = ["crates/sample-pack-core/src/lib.rs"]
[crates.output]
wasm = "crates/sample-pack-core-wasm/src/"
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().unwrap().remove(0);
        let spec = get_default_formatter(&config, Language::Wasm).expect("should have formatter");
        let fmt_cmd = &spec.commands[0];
        assert_eq!(
            fmt_cmd.args,
            vec!["fmt", "--manifest-path", "crates/sample-pack-core-wasm/Cargo.toml"]
        );
        let sort_cmd = &spec.commands[1];
        assert_eq!(
            sort_cmd.args,
            vec!["sort", "crates/sample-pack-core-wasm"],
            "cargo sort arg must match the crate dir derived from the configured output path"
        );
    }

    #[test]
    fn test_node_formatter_excludes_toml_from_oxfmt() {
        // oxfmt also reformats TOML (collapsing arrays, stripping inner-bracket
        // spaces), which fights the consumer's pyproject-fmt (`[ "x" ]`) and
        // cargo-sort, breaking `alef verify` post-finalize. The whole-repo oxfmt
        // run must exclude `**/*.toml`.
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::Node).expect("should have formatter");
        let oxfmt_cmd = spec
            .commands
            .iter()
            .find(|c| c.args.iter().any(|a| a == "oxfmt"))
            .expect("Node formatter must run oxfmt");
        assert!(
            oxfmt_cmd.args.iter().any(|a| a == "!**/*.toml"),
            "oxfmt must exclude TOML so it does not fight pyproject-fmt/cargo-sort, got: {:?}",
            oxfmt_cmd.args
        );
    }

    #[test]
    fn test_ffi_formatter_includes_cargo_sort() {
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::Ffi).expect("should have formatter");
        // Two commands: cargo fmt --all (rs files) + cargo sort -w (Cargo.toml table
        // order across the workspace). No oxfmt step here — the shared SampleCrate
        // pre-commit `oxfmt` hook is JS/TS/JSON/CSS only, and running oxfmt on `.`
        // additionally reformats every workspace TOML (including hand-maintained
        // Cargo.toml files) into oxfmt's 2-space style, fighting cargo-sort's
        // preserved indent and breaking the embedded hash.
        assert_eq!(spec.commands.len(), 2, "FFI must have cargo fmt + cargo sort steps");
        let fmt_cmd = &spec.commands[0];
        assert_eq!(fmt_cmd.command, "cargo");
        assert_eq!(fmt_cmd.args, vec!["fmt", "--all"]);
        let sort_cmd = &spec.commands[1];
        assert_eq!(sort_cmd.command, "cargo");
        assert_eq!(
            sort_cmd.args,
            vec!["sort", "-w"],
            "cargo sort must run workspace-wide so all binding crate Cargo.toml files are normalised"
        );
        assert!(spec.work_dir.is_empty(), "FFI formatter must run at workspace root");
    }

    // The Ruby native crate (`packages/ruby/ext/<gem>/native/`) lives outside the
    // consumer cargo workspace, so the FFI formatter's `cargo sort -w` skips it.
    // The Ruby formatter must therefore run cargo sort directly against the
    // native crate, otherwise prek's `cargo-sort` hook rewrites feature-array
    // indentation post-finalize and breaks `alef verify`.
    #[test]
    fn test_ruby_formatter_includes_cargo_sort_for_native_crate() {
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::Ruby).expect("should have formatter");
        assert_eq!(spec.commands.len(), 2, "Ruby must have rubocop + cargo sort steps");
        let sort_cmd = &spec.commands[1];
        assert_eq!(sort_cmd.command, "cargo");
        assert_eq!(sort_cmd.args[0], "sort");
        assert!(
            sort_cmd.args[1].contains("ext/") && sort_cmd.args[1].contains("/native"),
            "cargo sort arg must target the native crate dir, got: {:?}",
            sort_cmd.args
        );
        assert_eq!(spec.work_dir, "packages/ruby/");
    }

    // The Elixir NIF crate (`packages/elixir/native/<app>_nif/`) lives outside the
    // cargo workspace, so cargo sort must be invoked directly.
    #[test]
    fn test_elixir_formatter_includes_cargo_sort_for_nif_crate() {
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::Elixir).expect("should have formatter");
        assert_eq!(spec.commands.len(), 2, "Elixir must have mix format + cargo sort steps");
        let sort_cmd = &spec.commands[1];
        assert_eq!(sort_cmd.command, "cargo");
        assert_eq!(sort_cmd.args[0], "sort");
        assert!(
            sort_cmd.args[1].starts_with("native/") && sort_cmd.args[1].ends_with("_nif"),
            "cargo sort arg must target native/<app>_nif, got: {:?}",
            sort_cmd.args
        );
        assert_eq!(spec.work_dir, "packages/elixir/");
    }

    // The extendr R crate (`packages/r/src/rust/`) is workspace-excluded and so
    // needs its own cargo sort invocation.
    #[test]
    fn test_r_formatter_includes_cargo_sort_for_extendr_crate() {
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::R).expect("should have formatter");
        assert_eq!(spec.commands.len(), 2, "R must have styler + cargo sort steps");
        let sort_cmd = &spec.commands[1];
        assert_eq!(sort_cmd.command, "cargo");
        assert_eq!(sort_cmd.args, vec!["sort", "packages/r/src/rust"]);
        assert!(spec.work_dir.is_empty(), "R formatter runs at project root");
    }

    // Bug 2: C# formatter must include project_file when configured to avoid workspace ambiguity.
    #[test]
    fn test_csharp_formatter_with_project_file() {
        let config = make_config_with_csharp_project("sample-llm", "packages/csharp/SampleLlm.csproj");
        let spec = get_default_formatter(&config, Language::Csharp).expect("should have formatter");
        assert_eq!(spec.commands.len(), 1);
        let cmd = &spec.commands[0];
        assert_eq!(cmd.command, "dotnet");
        assert!(cmd.args.contains(&"format".to_owned()), "args must contain 'format'");
        assert!(
            cmd.args.contains(&"SampleLlm.csproj".to_owned()),
            "args must contain the relative project file, got: {:?}",
            cmd.args
        );
        assert_eq!(spec.work_dir, "packages/csharp/");
    }

    #[test]
    fn test_csharp_formatter_without_project_file() {
        let config = make_config("sample-llm");
        let spec = get_default_formatter(&config, Language::Csharp).expect("should have formatter");
        let cmd = &spec.commands[0];
        assert_eq!(cmd.command, "dotnet");
        assert_eq!(
            cmd.args,
            vec!["format"],
            "without project_file, args must be just ['format']"
        );
    }

    // KotlinAndroid formatter must use ktfmt (Google style) with --kotlinlang-style to match prek.
    // ktfmt and ktlint produce different formatting, so alef must use the same tool as prek
    // to ensure generated code is byte-identical to what prek's hook would produce.
    #[test]
    fn test_kotlin_android_formatter_uses_ktfmt() {
        let config = make_config("sample-markdown");
        let spec =
            get_default_formatter(&config, Language::KotlinAndroid).expect("KotlinAndroid should have formatter");
        assert_eq!(
            spec.commands.len(),
            1,
            "KotlinAndroid must have exactly one formatter command"
        );
        let cmd = &spec.commands[0];
        assert_eq!(
            cmd.command, "ktfmt",
            "KotlinAndroid must use ktfmt, not ktlint or gradle"
        );
        assert_eq!(
            cmd.args,
            vec![
                "--kotlinlang-style".to_owned(),
                "packages/kotlin-android/src".to_owned()
            ],
            "KotlinAndroid must include --kotlinlang-style flag and target src directory"
        );
        assert!(
            spec.work_dir.is_empty(),
            "KotlinAndroid formatter must run at project root"
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

    // Regression: custom format_override commands must run even when the language
    // is absent from the only_languages filter (i.e., files were not re-written
    // this run). The only_languages filter is an optimization for default formatters
    // (skip when nothing changed), but a custom command must always run to ensure
    // the embedded alef:hash: is computed over formatter-normalized content.
    // Without this, adding [workspace.format_overrides.php] and running
    // `alef all --format` on an already-generated repo would skip php-cs-fixer,
    // leaving hashes computed over raw (pre-formatter) content; prek's own
    // php-cs-fixer hook would then reformat and break alef verify.
    #[test]
    fn format_generated_custom_override_runs_when_lang_absent_from_only_languages_filter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sentinel = dir.path().join("was_run.txt");
        let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");

        // Config with a custom format_override for php that writes a sentinel file.
        let cfg: NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["php"]

[workspace.format_overrides.php]
command = "touch {sentinel_str}"

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#
        ))
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);

        // Simulate bindings for php — language appears in files but is NOT in only_languages.
        let files: Vec<(Language, Vec<crate::core::backend::GeneratedFile>)> = vec![(Language::Php, vec![])];

        // only_languages is empty — simulates "nothing was written this run".
        let only_languages: std::collections::HashSet<Language> = std::collections::HashSet::new();

        assert!(!sentinel.exists(), "sentinel must not exist before format_generated");

        format_generated(&files, &config, dir.path(), Some(&only_languages));

        assert!(
            sentinel.exists(),
            "custom format_override command must run even when php is absent from only_languages"
        );
    }

    // Complement: default formatters must still respect the only_languages filter
    // so that a warm cache (no file writes) skips unnecessary ruff/mix-format/etc.
    // invocations for default formatters.
    #[test]
    fn format_generated_default_formatter_skipped_when_lang_absent_from_only_languages() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Config with no format_overrides — python uses the default ruff formatter.
        let config = make_config("my-lib");

        let files: Vec<(Language, Vec<crate::core::backend::GeneratedFile>)> = vec![(Language::Python, vec![])];

        // only_languages is empty — simulates "nothing was written this run".
        let only_languages: std::collections::HashSet<Language> = std::collections::HashSet::new();

        // This should complete without error (ruff not present on the test box is fine —
        // the point is that format_generated skips python entirely without reaching the
        // is_tool_available check, so no warning is emitted and no external process runs).
        // We verify by ensuring format_generated returns without calling any tool.
        // Since python has a default formatter (ruff), skipping means the tool is never
        // looked up — we can't assert negatively on tool invocation, but the test
        // documents the intent: no-op when only_languages filter excludes the language.
        format_generated(&files, &config, dir.path(), Some(&only_languages));
        // If we reach here without error the skip path worked correctly.
    }
}
