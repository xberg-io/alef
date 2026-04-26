use alef_core::config::{AlefConfig, FormatConfig, Language};
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

/// Post-generation formatter configuration and execution commands.
/// Maps each language to its default formatter and working directory (relative to project root).
struct FormatterSpec {
    command: &'static str,
    args: &'static [&'static str],
    work_dir: &'static str,
}

/// Get the default formatter spec for a language.
fn get_default_formatter(lang: Language) -> Option<FormatterSpec> {
    match lang {
        Language::Python => Some(FormatterSpec {
            command: "ruff",
            args: &["format"],
            work_dir: "packages/python/",
        }),
        Language::Node => Some(FormatterSpec {
            command: "biome",
            args: &["format", "--write", "."],
            work_dir: "packages/typescript/",
        }),
        Language::Ruby => Some(FormatterSpec {
            command: "cargo",
            args: &["fmt"],
            work_dir: "packages/ruby/",
        }),
        Language::Php => Some(FormatterSpec {
            command: "php-cs-fixer",
            args: &["fix"],
            work_dir: "packages/php/",
        }),
        Language::Elixir => Some(FormatterSpec {
            command: "mix",
            args: &["format"],
            work_dir: "packages/elixir/",
        }),
        Language::Go => Some(FormatterSpec {
            command: "gofmt",
            args: &["-w", "."],
            work_dir: "packages/go/",
        }),
        Language::Java => Some(FormatterSpec {
            command: "google-java-format",
            args: &["-i"],
            work_dir: "packages/java/src/",
        }),
        Language::Csharp => Some(FormatterSpec {
            command: "dotnet",
            args: &["format"],
            work_dir: "packages/csharp/",
        }),
        Language::Wasm => Some(FormatterSpec {
            command: "cargo",
            args: &["fmt"],
            work_dir: "packages/wasm/",
        }),
        Language::Ffi => Some(FormatterSpec {
            command: "cargo",
            args: &["fmt"],
            work_dir: "packages/ffi/",
        }),
        Language::R => Some(FormatterSpec {
            command: "cargo",
            args: &["fmt"],
            work_dir: "packages/r/",
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

        // Check if formatter binary is available
        if !is_tool_available(formatter_cmd.command) {
            warn!(
                "[{lang_str}] formatter not found: {} (skipping format)",
                formatter_cmd.command
            );
            continue;
        }

        // Run the formatter
        let work_dir = base_dir.join(formatter_cmd.work_dir);
        if !work_dir.exists() {
            debug!(
                "  [{lang_str}] package directory does not exist: {}, skipping",
                work_dir.display()
            );
            continue;
        }

        match run_formatter(formatter_cmd.command, formatter_cmd.args, &work_dir) {
            Ok(()) => {
                debug!("  [{lang_str}] formatted successfully");
            }
            Err(e) => {
                warn!("[{lang_str}] formatter error: {}", e);
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
