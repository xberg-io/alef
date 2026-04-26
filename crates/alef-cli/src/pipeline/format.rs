use alef_core::config::{AlefConfig, Language};
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

/// One formatter invocation (command + args).
struct FormatterCommand {
    command: &'static str,
    args: &'static [&'static str],
}

/// Post-generation formatter configuration.
/// Each language may run a sequence of formatter commands in a working directory.
struct FormatterSpec {
    /// Commands to run in sequence; on first failure the rest are skipped (warning logged).
    commands: &'static [FormatterCommand],
    /// Working directory relative to project root; empty string = project root.
    work_dir: &'static str,
}

/// Get the default formatter spec for a language.
///
/// Notes on Rust formatting: backends that emit Rust code (FFI, PyO3, NAPI, Magnus,
/// ext-php-rs, Rustler, wasm-bindgen) all live inside the consumer workspace, so a
/// single `cargo fmt --all` from the project root covers every generated `.rs` file.
/// We attach this to `Language::Ffi` because FFI is always present when any of the
/// C-FFI-bridged languages (Go/Java/C#) are enabled, and harmless when only WASM
/// or pure-Rust bindings are used.
fn get_default_formatter(lang: Language) -> Option<FormatterSpec> {
    match lang {
        // ruff check --fix runs lint autofixes (unused imports, missing TypeAlias
        // annotations, import sorting); ruff format applies whitespace formatting.
        // Both must run — `format` alone leaves I001/F401/TC008 issues that fail CI.
        Language::Python => Some(FormatterSpec {
            commands: &[
                FormatterCommand {
                    command: "ruff",
                    args: &["check", "--fix", "."],
                },
                FormatterCommand {
                    command: "ruff",
                    args: &["format", "."],
                },
            ],
            work_dir: "packages/python/",
        }),
        Language::Node => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "biome",
                args: &["format", "--write", "."],
            }],
            work_dir: "packages/typescript/",
        }),
        Language::Ruby => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "rubocop",
                args: &["-A", "--no-server"],
            }],
            work_dir: "packages/ruby/",
        }),
        Language::Php => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "php-cs-fixer",
                args: &["fix"],
            }],
            work_dir: "packages/php/",
        }),
        Language::Elixir => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "mix",
                args: &["format"],
            }],
            work_dir: "packages/elixir/",
        }),
        Language::Go => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "gofmt",
                args: &["-w", "."],
            }],
            work_dir: "packages/go/",
        }),
        Language::Java => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "google-java-format",
                args: &["-i"],
            }],
            work_dir: "packages/java/src/",
        }),
        Language::Csharp => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "dotnet",
                args: &["format"],
            }],
            work_dir: "packages/csharp/",
        }),
        Language::Wasm => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "cargo",
                args: &["fmt", "-p", "wasm"],
            }],
            work_dir: "packages/wasm/",
        }),
        // FFI runs `cargo fmt --all` from project root: this formats every generated
        // Rust crate in the consumer workspace (FFI, PyO3, NAPI-RS, Magnus, ext-php-rs,
        // Rustler, wasm-bindgen). Was previously `cargo fmt` in `packages/ffi/` —
        // which has no Cargo.toml, so it silently no-op'd and CI's `cargo fmt --check`
        // would fail on the unformatted FFI lib.rs.
        Language::Ffi => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "cargo",
                args: &["fmt", "--all"],
            }],
            work_dir: "",
        }),
        Language::R => Some(FormatterSpec {
            commands: &[FormatterCommand {
                command: "Rscript",
                args: &["-e", "styler::style_pkg('packages/r')"],
            }],
            work_dir: "",
        }),
        Language::Rust => None,
    }
}

/// Run language-native formatters on emitted packages after generation.
/// For each language in the output, if formatting is enabled and the formatter binary
/// is available, run the formatter in the package directory.
/// Formatter errors are logged as warnings and do not fail the generate command.
pub fn format_generated(
    files: &[(Language, Vec<alef_core::backend::GeneratedFile>)],
    config: &AlefConfig,
    base_dir: &Path,
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
        } else if let Some(spec) = get_default_formatter(*lang) {
            spec
        } else {
            // No formatter for this language (e.g., Rust is formatted in-memory before writing)
            debug!("  [{lang_str}] no default formatter configured");
            continue;
        };

        // Resolve work dir (empty string = project root)
        let work_dir = if formatter_cmd.work_dir.is_empty() {
            base_dir.to_path_buf()
        } else {
            base_dir.join(formatter_cmd.work_dir)
        };
        if !work_dir.exists() {
            debug!(
                "  [{lang_str}] package directory does not exist: {}, skipping",
                work_dir.display()
            );
            continue;
        }

        // Run each command in sequence; stop on first failure (warning logged)
        for step in formatter_cmd.commands {
            if !is_tool_available(step.command) {
                warn!("[{lang_str}] formatter not found: {} (skipping format)", step.command);
                break;
            }
            match run_formatter(step.command, step.args, &work_dir) {
                Ok(()) => {
                    debug!("  [{lang_str}] {} {:?} ok", step.command, step.args);
                }
                Err(e) => {
                    warn!("[{lang_str}] {} {:?} failed: {}", step.command, step.args, e);
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
