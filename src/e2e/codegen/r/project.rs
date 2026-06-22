//! R e2e project file rendering.

use crate::core::hash::{self, CommentStyle};
use crate::core::version::to_r_version;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

/// Emit an R snippet that calls `Sys.setenv(KEY = VALUE)` for every
/// `[e2e.env]` entry when not already set. The check via `Sys.getenv(..., unset = "")`
/// preserves any value supplied by the parent shell (setdefault semantics).
/// Returns an empty string when the env map is empty. Keys are sorted
/// alphabetically for deterministic output.
pub(super) fn render_env_block(env: &HashMap<String, String>) -> String {
    if env.is_empty() {
        return String::new();
    }
    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    let mut out = String::new();
    let _ = writeln!(out, "# Suite-level environment defaults from [e2e.env]. Each entry");
    let _ = writeln!(out, "# uses setdefault semantics: only applied when not already set.");
    for key in keys {
        let value = &env[key];
        // R double-quoted strings: escape `\` and `"`.
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(out, "if (Sys.getenv(\"{key}\", unset = \"\") == \"\") {{");
        let _ = writeln!(out, "  args <- list(\"{escaped}\")");
        let _ = writeln!(out, "  names(args) <- \"{key}\"");
        let _ = writeln!(out, "  do.call(Sys.setenv, args)");
        let _ = writeln!(out, "}}");
    }
    let _ = writeln!(out);
    out
}

pub(super) fn render_description(
    pkg_name: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let dep_line = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            let r_version = to_r_version(pkg_version);
            format!("Imports: {pkg_name} ({r_version})\n")
        }
        crate::e2e::config::DependencyMode::Local => String::new(),
    };
    format!(
        r#"Package: e2e.r
Title: E2E Tests for {pkg_name}
Version: 0.1.0
Description: End-to-end test suite.
{dep_line}Suggests: testthat (>= 3.0.0)
Config/testthat/edition: 3
"#
    )
}

pub(super) fn render_setup_fixtures(test_documents_path: &str, env: &HashMap<String, String>) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out);
    let env_block = render_env_block(env);
    if !env_block.is_empty() {
        out.push_str(&env_block);
    }
    let _ = writeln!(
        out,
        "# Resolve fixture paths against the repo's `test_documents/` directory."
    );
    let _ = writeln!(
        out,
        "# testthat sources setup-*.R with the working directory at tests/,"
    );
    let _ = writeln!(
        out,
        "# so test_documents lives three directories up: tests/ -> e2e/r/ -> e2e/ -> repo root."
    );
    let _ = writeln!(
        out,
        "# Each `test_that()` block has its working directory reset back to tests/, so"
    );
    let _ = writeln!(
        out,
        "# fixture lookups must be performed via this helper rather than relying on `setwd`."
    );
    let _ = writeln!(
        out,
        ".alef_test_documents <- normalizePath(\"{test_documents_path}\", mustWork = FALSE)"
    );
    let _ = writeln!(out, ".resolve_fixture <- function(path) {{");
    let _ = writeln!(out, "  if (dir.exists(.alef_test_documents)) {{");
    let _ = writeln!(out, "    file.path(.alef_test_documents, path)");
    let _ = writeln!(out, "  }} else {{");
    let _ = writeln!(out, "    path");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    // FormatMetadata is an internally-tagged enum (serde tag = "format_type")
    // so the JSON shape varies. `simplifyVector = FALSE` hands us a per-variant
    // list — keyed by the snake_case variant name (`image`, `excel`, ...) — that
    // points at the inner metadata struct, with all other variants set to NULL.
    // Collapse both shapes here so terminal `metadata$format` assertions see
    // the human-readable format string (e.g. "PNG") instead of the wrapper list.
    let _ = writeln!(
        out,
        ".alef_format_value <- function(x) {{
  if (is.list(x)) {{
    for (variant in names(x)) {{
      v <- x[[variant]]
      if (is.list(v) && !is.null(v[[\"format\"]]) && is.character(v[[\"format\"]])) {{
        return(v[[\"format\"]])
      }}
    }}
    if (!is.null(x[[\"format\"]]) && is.character(x[[\"format\"]])) {{
      return(x[[\"format\"]])
    }}
    if (!is.null(x[[\"format_type\"]])) {{
      return(x[[\"format_type\"]])
    }}
  }}
  x
}}"
    );
    out
}

pub(super) fn render_test_runner(
    pkg_name: &str,
    pkg_path: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "library(testthat)");
    match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // In registry mode, load the installed CRAN package. This must happen before
            // test_dir() runs so that all package functions are available to the tests.
            let _ = writeln!(out, "library({})", pkg_name);
        }
        crate::e2e::config::DependencyMode::Local => {
            // Use devtools::load_all() to load the local R package without requiring
            // a full install, matching the e2e test runner convention.
            let _ = writeln!(out, "devtools::load_all(\"{pkg_path}\")");
        }
    }
    let _ = writeln!(out);
    // Surface every failure rather than aborting at the default max_fails=10 —
    // partial pass counts are essential for triage during e2e bring-up.
    let _ = writeln!(out, "testthat::set_max_fails(Inf)");
    // Resolve the tests/ directory relative to this script. testthat reads
    // setup-*.R from there before each file runs, where path resolution
    // against test_documents/ is handled by the `.resolve_fixture` helper.
    let _ = writeln!(
        out,
        ".script_dir <- tryCatch(dirname(normalizePath(sys.frame(1)$ofile)), error = function(e) getwd())"
    );
    let _ = writeln!(out, "test_dir(file.path(.script_dir, \"tests\"))");
    out
}

pub(super) fn render_install_r(pkg_name: &str, pkg_version: &str, github_repo: &str) -> String {
    let github_repo = github_repo.trim_end_matches('/');
    let mut out = String::new();
    let _ = writeln!(out, "# alef-generated installer for registry-mode R test_app.");
    let _ = writeln!(out, "# Installs the configured R package from GitHub releases.");
    let _ = writeln!(out, "# Requires `R` on PATH.");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Version override: pass as commandArgs()[6] to test an");
    let _ = writeln!(out, "# arbitrary tag; defaults to the alef-pinned version from");
    let _ = writeln!(out, "# [crates.e2e.registry.packages.r].version.");
    let _ = writeln!(out, "args <- commandArgs(trailingOnly = TRUE)");
    let _ = writeln!(out, "VERSION <- if (length(args) > 0) args[1] else \"{pkg_version}\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Construct the GitHub release tarball URL.");
    let _ = writeln!(out, "url <- sprintf(");
    let _ = writeln!(out, "  \"{github_repo}/releases/download/v%s/{pkg_name}_%s.tar.gz\",");
    let _ = writeln!(out, "  VERSION,");
    let _ = writeln!(out, "  VERSION");
    let _ = writeln!(out, ")");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "# Install from the release tarball without requiring devtools or remotes."
    );
    // `install.packages` signals download/build failures as *warnings*, not
    // errors, and returns normally — so a 404 or non-zero R CMD INSTALL would
    // slip past a plain `tryCatch(error=)`. Promote warnings to errors during
    // install, then verify the package is actually loadable before declaring
    // success. Either failure path exits non-zero without printing success.
    let _ = writeln!(out, "tryCatch(");
    let _ = writeln!(out, "  withCallingHandlers(");
    let _ = writeln!(
        out,
        "    install.packages(url, repos = NULL, type = \"source\", quiet = TRUE),"
    );
    let _ = writeln!(
        out,
        "    warning = function(w) stop(conditionMessage(w), call. = FALSE)"
    );
    let _ = writeln!(out, "  ),");
    let _ = writeln!(out, "  error = function(e) {{");
    let _ = writeln!(out, "    message(paste(\"Error installing {pkg_name} from\", url))");
    let _ = writeln!(out, "    message(conditionMessage(e))");
    let _ = writeln!(out, "    quit(status = 1)");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, ")");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Verify the package is installed and loadable; install.packages does");
    let _ = writeln!(out, "# not guarantee this even when no condition was raised.");
    let _ = writeln!(
        out,
        "if (!requireNamespace(\"{pkg_name}\", quietly = TRUE)) {{"
    );
    let _ = writeln!(
        out,
        "  message(paste(\"Error: {pkg_name} not available after install from\", url))"
    );
    let _ = writeln!(out, "  quit(status = 1)");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "message(paste(\"Successfully installed {pkg_name}\", VERSION))");
    out
}

#[cfg(test)]
mod description_tests {
    use super::render_description;
    use crate::e2e::config::DependencyMode;

    #[test]
    fn render_description_registry_release_uses_plain_version() {
        let out = render_description("mypkg", "1.2.3", DependencyMode::Registry);
        assert!(out.contains("Imports: mypkg (1.2.3)"), "got: {out}");
    }

    #[test]
    fn render_description_registry_prerelease_uses_r_version_form() {
        // 3.6.0-rc.1 → 3.6.0.9001 (CRAN-compatible dev-pin form)
        let out = render_description("mypkg", "3.6.0-rc.1", DependencyMode::Registry);
        assert!(
            out.contains("Imports: mypkg (3.6.0.9001)"),
            "pre-release must use CRAN dev-pin form, got: {out}"
        );
        assert!(
            !out.contains("3.6.0-rc.1"),
            "raw semver dash form must not appear in DESCRIPTION, got: {out}"
        );
    }

    #[test]
    fn render_description_local_omits_imports_line() {
        let out = render_description("mypkg", "3.6.0-rc.1", DependencyMode::Local);
        assert!(
            !out.contains("Imports:"),
            "local mode must not emit Imports line, got: {out}"
        );
    }
}

#[cfg(test)]
mod install_r_tests {
    use super::render_install_r;

    #[test]
    fn install_r_promotes_warnings_to_errors() {
        // install.packages signals download/build failures as warnings, not
        // errors. The installer must promote them so a 404 cannot slip past.
        let out = render_install_r("mypkg", "1.2.3", "https://github.com/org/repo");
        assert!(
            out.contains("withCallingHandlers"),
            "must wrap install in withCallingHandlers to catch warnings, got: {out}"
        );
        assert!(
            out.contains("warning = function(w) stop(conditionMessage(w)"),
            "must promote warnings to errors, got: {out}"
        );
    }

    #[test]
    fn install_r_verifies_loadability_before_success() {
        let out = render_install_r("mypkg", "1.2.3", "https://github.com/org/repo");
        let verify = out
            .find("requireNamespace(\"mypkg\"")
            .expect("must verify package loadability");
        let success = out
            .find("Successfully installed mypkg")
            .expect("must print success message");
        assert!(
            verify < success,
            "loadability check must precede the success message, got: {out}"
        );
    }

    #[test]
    fn install_r_exits_nonzero_on_failure() {
        // Both the install error path and the loadability check must exit non-zero.
        let out = render_install_r("mypkg", "1.2.3", "https://github.com/org/repo");
        assert_eq!(
            out.matches("quit(status = 1)").count(),
            2,
            "both failure paths must exit non-zero, got: {out}"
        );
    }

    #[test]
    fn install_r_success_message_is_not_unconditional() {
        // The success message must not sit inside the tryCatch body where it
        // would print regardless of the install outcome.
        let out = render_install_r("mypkg", "1.2.3", "https://github.com/org/repo");
        let trycatch_end = out.find("\n)\n").expect("tryCatch must close");
        let success = out
            .find("Successfully installed mypkg")
            .expect("must print success message");
        assert!(
            success > trycatch_end,
            "success message must come after the tryCatch block closes, got: {out}"
        );
    }
}

#[cfg(test)]
mod env_tests {
    use super::{render_env_block, render_setup_fixtures};
    use std::collections::HashMap;

    #[test]
    fn render_env_block_emits_setdefault_with_sorted_keys() {
        let mut env = HashMap::new();
        env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        env.insert("ALEF_FOO".to_string(), "bar".to_string());
        let block = render_env_block(&env);
        assert!(
            block.contains("if (Sys.getenv(\"ALEF_FOO\", unset = \"\") == \"\") {"),
            "got: {block}"
        );
        assert!(
            block.contains("if (Sys.getenv(\"E2E_ALLOW_PRIVATE_NETWORK\", unset = \"\") == \"\") {"),
            "got: {block}"
        );
        assert!(block.contains("names(args) <- \"ALEF_FOO\""), "got: {block}");
        let alef_pos = block.find("ALEF_FOO").unwrap();
        let e2e_pos = block.find("E2E_ALLOW_PRIVATE_NETWORK").unwrap();
        assert!(alef_pos < e2e_pos, "keys must be sorted alphabetically; got: {block}");
    }

    #[test]
    fn render_env_block_empty_when_no_env_configured() {
        let env = HashMap::new();
        assert_eq!(render_env_block(&env), "");
    }

    #[test]
    fn render_setup_fixtures_includes_env_block_when_env_configured() {
        let mut env = HashMap::new();
        env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        let out = render_setup_fixtures("../../../test_documents", &env);
        assert!(
            out.contains("if (Sys.getenv(\"E2E_ALLOW_PRIVATE_NETWORK\", unset = \"\") == \"\")"),
            "got: {out}"
        );
    }

    #[test]
    fn render_setup_fixtures_omits_env_block_when_env_empty() {
        let out = render_setup_fixtures("../../../test_documents", &HashMap::new());
        assert!(
            !out.contains("Suite-level environment defaults"),
            "no env block when env empty; got: {out}"
        );
    }
}
