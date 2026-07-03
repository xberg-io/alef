//! Post-generation formatter support for e2e test projects.
//!
//! Formatting is delegated to the `poly` (polylint) CLI as a system dependency —
//! the same tool the main generate pipeline uses (see `cli::pipeline::format`).
//! For each language directory that had files generated, `run_formatters` runs a
//! single `poly fmt --fix` pass, which formats every language poly supports
//! (Python via ruff, JS/TS/JSON via oxc, Rust via rustfmt, Go via gofmt, …). A
//! missing `poly` binary is a best-effort no-op.
//!
//! Two escape hatches remain:
//! * a per-language `E2eConfig.format` override (`sh -c`, with `{dir}` expanded)
//!   replaces the poly pass for that language;
//! * a residual `go mod tidy` runs for Go directories — it is not formatting but
//!   is required to populate `go.sum` from `go.mod` so the e2e Go suite builds.

use crate::core::backend::GeneratedFile;
use crate::e2e::config::E2eConfig;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Run per-language formatters for all languages that had files generated.
///
/// E2e files are written to `{output}/{lang}/...`, so the language is the first
/// path component after the output prefix. For each language directory: a user
/// `E2eConfig.format[lang]` override runs as a shell command (`{dir}` expanded);
/// otherwise poly formats the directory in-process. Failures are logged as
/// warnings and never abort the process.
pub fn run_formatters(files: &[GeneratedFile], e2e_config: &E2eConfig) {
    let output_prefix = Path::new(e2e_config.effective_output());
    let languages: HashSet<String> = files
        .iter()
        .filter_map(|f| {
            let remainder = f.path.strip_prefix(output_prefix).ok()?;
            let first = remainder.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        })
        .collect();

    for lang in &languages {
        let dir = format!("{}/{}", e2e_config.effective_output(), lang);
        let dir_path = PathBuf::from(&dir);

        // User override takes precedence and replaces the poly pass entirely.
        if let Some(custom) = e2e_config.format.get(lang.as_str()) {
            let cmd = custom.replace("{dir}", &dir);
            eprintln!("  Formatting {lang}: {cmd}");
            run_shell(&cmd, lang);
            continue;
        }

        // Default: shell out to `poly fmt --fix` over the directory. poly walks up
        // from `dir_path` for a `poly.toml` (falling back to poly's zero-config
        // defaults when none is found).
        eprintln!("  Formatting {lang} with poly: {dir}");
        crate::cli::pipeline::poly_format(std::slice::from_ref(&dir_path), &dir_path);

        // Residual: `go mod tidy` populates `go.sum` from `go.mod` (poly cannot —
        // it is dependency resolution, not formatting) so the Go suite builds.
        if lang == "go" {
            run_go_mod_tidy(&dir);
        }
    }
}

/// Run a best-effort shell command; log non-success as a warning.
fn run_shell(cmd: &str, lang: &str) {
    match std::process::Command::new("sh").args(["-c", cmd]).status() {
        Ok(s) if s.success() => {}
        Ok(s) => warn!("Formatter for {lang} exited with {s}: {cmd}"),
        Err(e) => warn!("Failed to run formatter for {lang}: {e}"),
    }
}

/// Populate `go.sum` from `go.mod` in the e2e Go directory. Best-effort.
fn run_go_mod_tidy(dir: &str) {
    let cmd = format!("(cd {dir} && go mod tidy)");
    run_shell(&cmd, "go");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `E2eConfig` whose output directory is `out`, defaults otherwise.
    fn e2e_config_for(out: &Path) -> E2eConfig {
        E2eConfig {
            output: out.to_string_lossy().into_owned(),
            ..E2eConfig::default()
        }
    }

    /// A user override in `E2eConfig.format` must replace the poly pass: the
    /// `{dir}` placeholder is expanded and the command is run verbatim.
    #[test]
    fn user_override_command_is_expanded_and_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out = base.join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let sentinel = out.join("python/was_run.txt");
        let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");

        let mut e2e_config = e2e_config_for(&out);
        e2e_config
            .format
            .insert("python".to_owned(), format!("touch {sentinel_str}"));

        let files = vec![GeneratedFile {
            path: out.join("python/main.py"),
            content: "x = 1\n".to_owned(),
            generated_header: false,
        }];

        assert!(!sentinel.exists());
        run_formatters(&files, &e2e_config);
        assert!(
            sentinel.exists(),
            "user override command must run with {{dir}} expanded"
        );
    }

    /// The default path shells out to `poly fmt --fix`. When `poly` is installed a
    /// badly-spaced Python file ends up ruff-formatted; when it is absent the pass
    /// is a best-effort no-op (file untouched, no panic).
    #[test]
    fn default_path_formats_python_with_poly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out = base.join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x=1").unwrap();

        let e2e_config = e2e_config_for(&out);

        let files = vec![GeneratedFile {
            path: out.join("python/main.py"),
            content: "x=1".to_owned(),
            generated_header: false,
        }];

        run_formatters(&files, &e2e_config);

        let formatted = std::fs::read_to_string(&py).unwrap();
        if which::which("poly").is_ok() {
            assert_eq!(
                formatted, "x = 1\n",
                "with poly installed, `poly fmt --fix` must reformat the e2e Python file"
            );
        } else {
            assert_eq!(formatted, "x=1", "without poly the file must be left untouched");
        }
    }

    /// A language poly does not know still runs cleanly (poly no-ops on unknown
    /// files); the process must not panic or abort.
    #[test]
    fn unknown_language_dir_is_best_effort() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out = base.join("e2e-out");
        std::fs::create_dir_all(out.join("cobol")).unwrap();

        let e2e_config = e2e_config_for(&out);

        let files = vec![GeneratedFile {
            path: out.join("cobol/main.cob"),
            content: "       IDENTIFICATION DIVISION.\n".to_owned(),
            generated_header: false,
        }];

        // Must complete without panicking.
        run_formatters(&files, &e2e_config);
    }
}
