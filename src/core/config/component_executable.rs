//! Configuration of components shipped as prebuilt executables.
//!
//! An executable component (`producer_language = "executable"`) is built by a consumer-supplied
//! command rather than by a producer crate alef generates. It speaks the framed stdio transport
//! (ADR-0017), so hosts treat it as a stdio component without listing it in
//! `component_host.stdio_components`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Placeholders accepted by [`ComponentExecutableConfig::build`].
pub const EXECUTABLE_BUILD_PLACEHOLDERS: &[&str] = &["target", "out", "profile"];

/// How to build the executable of a `producer_language = "executable"` component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentExecutableConfig {
    /// Shell command that builds the executable for one target, run with `sh -c` from the
    /// component's `producer_dir`. Placeholders: `{target}` (Rust target triple), `{out}` (absolute
    /// path the command must write the executable to; required) and `{profile}` (`release` or
    /// `debug`). Each substituted value is inserted POSIX single-quoted, so a path containing
    /// spaces or quotes stays one word: write `-o {out}`, never `-o "{out}"`. On Windows `{out}`
    /// uses forward slashes. The same values are exported unquoted as `ALEF_COMPONENT_TARGET`,
    /// `ALEF_COMPONENT_OUT` and `ALEF_COMPONENT_PROFILE`. Mapping a triple to the consumer's
    /// toolchain (for example GOOS/GOARCH) is the command's job, and cross-compilation is allowed.
    /// A POSIX `sh` must be on `PATH` of the build host, Windows included (Git for Windows or
    /// MSYS2 provide one).
    pub build: String,
    /// Payload file name; defaults to the component name. `.exe` is appended for `*-windows-*`
    /// targets unless `target_files` names the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Exact payload file name per target triple, overriding `file` and the `.exe` rule.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_files: BTreeMap<String, String>,
    /// Glob patterns (relative to the directory of `alef.toml`) of the files whose content decides
    /// whether `alef component plan` reuses a previous build. A directory matches every file under
    /// it; `.git` directories and alef's own `.alef` build directory are skipped. An entry that
    /// names an existing path is taken literally before it is tried as a glob. Symbolic links are
    /// hashed by their target string and never followed. Defaults to the component's
    /// `producer_dir`; when set it replaces that default, so list the producer directory too when
    /// it should still count, plus files outside it such as a shared `go.work`. Unlike
    /// `producer_dir`, entries may use `..` to reach files outside the producer directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Shell command whose trimmed standard output identifies the toolchain (for example
    /// `go version`); it is part of the fingerprint, so a compiler upgrade rebuilds. Run with
    /// `sh -c` from the `producer_dir` by `plan`, `build` and `package`, never by `lock`. Because
    /// `lock` does not run it, `lock --reuse` re-verification covers the input files and the build
    /// command but not the toolchain: the toolchain is trusted to the plan that produced the
    /// reused artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain_version_cmd: Option<String>,
}

impl ComponentExecutableConfig {
    /// File name of the executable built for `target`.
    #[must_use]
    pub fn file_name(&self, component: &str, target: &str) -> String {
        if let Some(name) = self.target_files.get(target) {
            return name.clone();
        }
        let stem = self.file.as_deref().unwrap_or(component);
        if target.contains("windows") && !stem.to_ascii_lowercase().ends_with(".exe") {
            format!("{stem}.exe")
        } else {
            stem.to_string()
        }
    }

    /// The build command with `{target}`, `{out}` and `{profile}` substituted, each value
    /// POSIX single-quoted. On Windows `{out}` is converted to forward slashes first so `sh` does
    /// not read its backslashes as escapes.
    #[must_use]
    pub fn render_build(&self, target: &str, out: &str, debug: bool) -> String {
        self.render_build_for(target, out, debug, cfg!(windows))
    }

    fn render_build_for(&self, target: &str, out: &str, debug: bool, windows: bool) -> String {
        let out = if windows {
            out.replace('\\', "/")
        } else {
            out.to_string()
        };
        self.build
            .replace("{target}", &shell_quote(target))
            .replace("{out}", &shell_quote(&out))
            .replace("{profile}", &shell_quote(if debug { "debug" } else { "release" }))
    }

    /// Check the fields that do not depend on the rest of the component.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason when a field is invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.build.trim().is_empty() {
            return Err("executable.build must not be empty".into());
        }
        if !self.build.contains("{out}") {
            return Err("executable.build must contain the `{out}` placeholder".into());
        }
        let mut rest = self.build.as_str();
        while let Some(start) = rest.find('{') {
            let Some(end) = rest[start..].find('}') else { break };
            let name = &rest[start + 1..start + end];
            if !name.is_empty()
                && name.bytes().all(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                && !EXECUTABLE_BUILD_PLACEHOLDERS.contains(&name)
                && !is_shell_parameter(rest, start)
            {
                return Err(format!(
                    "executable.build placeholder `{{{name}}}` is unknown; expected one of {}",
                    EXECUTABLE_BUILD_PLACEHOLDERS
                        .iter()
                        .map(|name| format!("{{{name}}}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            rest = &rest[start + end + 1..];
        }
        if let Some(file) = &self.file {
            validate_file_name("executable.file", file)?;
        }
        for (target, file) in &self.target_files {
            if !super::component::is_valid_target_triple(target) {
                return Err(format!(
                    "executable.target_files key `{target}` is not a valid Rust target triple"
                ));
            }
            validate_file_name("executable.target_files value", file)?;
        }
        for pattern in &self.inputs {
            if pattern.trim().is_empty() || pattern.starts_with('/') || pattern.contains('\\') {
                return Err(format!(
                    "executable.inputs entry `{pattern}` must be a relative glob using `/` separators"
                ));
            }
            glob::Pattern::new(pattern)
                .map_err(|error| format!("executable.inputs entry `{pattern}` is not a valid glob: {error}"))?;
        }
        if self
            .toolchain_version_cmd
            .as_deref()
            .is_some_and(|cmd| cmd.trim().is_empty())
        {
            return Err("executable.toolchain_version_cmd must not be empty".into());
        }
        Ok(())
    }

    /// Re-root `inputs` onto `config_dir`, like `producer_dir`.
    pub fn rebase_paths(&mut self, config_dir: &std::path::Path) {
        for pattern in &mut self.inputs {
            *pattern = config_dir.join(&*pattern).to_string_lossy().replace('\\', "/");
        }
    }
}

/// `${NAME}` is a shell parameter expansion, not a placeholder.
fn is_shell_parameter(text: &str, brace: usize) -> bool {
    text[..brace].ends_with('$')
}

/// `value` as one POSIX single-quoted word.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn validate_file_name(label: &str, value: &str) -> Result<(), String> {
    let safe = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if safe {
        Ok(())
    } else {
        Err(format!(
            "{label} `{value}` must be a file name of ASCII letters, digits, `-`, `_` and `.`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(build: &str) -> ComponentExecutableConfig {
        ComponentExecutableConfig {
            build: build.into(),
            file: None,
            target_files: BTreeMap::new(),
            inputs: Vec::new(),
            toolchain_version_cmd: None,
        }
    }

    #[test]
    fn file_name_defaults_to_the_component_and_adds_exe_on_windows() {
        let executable = config("make {out}");
        assert_eq!(executable.file_name("sh", "x86_64-unknown-linux-gnu"), "sh");
        assert_eq!(executable.file_name("sh", "x86_64-pc-windows-msvc"), "sh.exe");
        let mut named = config("make {out}");
        named.file = Some("alef-sh.exe".into());
        assert_eq!(named.file_name("sh", "aarch64-pc-windows-msvc"), "alef-sh.exe");
        named
            .target_files
            .insert("aarch64-apple-darwin".into(), "sh-mac".into());
        assert_eq!(named.file_name("sh", "aarch64-apple-darwin"), "sh-mac");
        assert_eq!(named.file_name("sh", "x86_64-unknown-linux-gnu"), "alef-sh.exe");
    }

    #[test]
    fn render_build_substitutes_every_placeholder() {
        let executable = config("go build -o {out} # {target} {profile} {target}");
        assert_eq!(
            executable.render_build("aarch64-apple-darwin", "/tmp/out/sh", false),
            "go build -o '/tmp/out/sh' # 'aarch64-apple-darwin' 'release' 'aarch64-apple-darwin'"
        );
        assert!(executable.render_build("t", "/o", true).contains("'debug'"));
    }

    #[test]
    fn render_build_quotes_an_out_path_with_a_space_and_a_quote() {
        let executable = config("printf x > {out}");
        let rendered = executable.render_build("t", "/tmp/it's here/sh", false);
        assert_eq!(rendered, "printf x > '/tmp/it'\\''s here/sh'");
    }

    #[test]
    fn render_build_converts_a_windows_out_path_to_forward_slashes() {
        let executable = config("go build -o {out} .");
        assert_eq!(
            executable.render_build_for("t", r"C:\Users\a b\sh.exe", false, true),
            "go build -o 'C:/Users/a b/sh.exe' ."
        );
        assert_eq!(
            executable.render_build_for("t", r"/tmp/a\b", false, false),
            "go build -o '/tmp/a\\b' ."
        );
    }

    #[test]
    fn validation_rejects_a_near_miss_placeholder_in_any_case() {
        for build in ["make {out} {OUT}", "make {out} {Target}", "make {out} {Profile}"] {
            let error = config(build).validate().unwrap_err();
            assert!(error.contains("is unknown"), "{build}: {error}");
        }
        assert!(config("make {out} ${HOME} \"${Target_Dir}\"").validate().is_ok());
    }

    #[test]
    fn validation_rejects_a_build_without_out_unknown_placeholders_and_unsafe_names() {
        assert!(config("go build -o {out} .").validate().is_ok());
        assert!(config("").validate().unwrap_err().contains("must not be empty"));
        assert!(config("go build .").validate().unwrap_err().contains("{out}"));
        let unknown = config("go build -o {out} {triple}").validate().unwrap_err();
        assert!(unknown.contains("{triple}"), "{unknown}");
        // Shell and JSON braces are not placeholders.
        assert!(config("sh -c 'echo ${HOME}' {out}").validate().is_ok());
        let mut bad = config("make {out}");
        bad.file = Some("../sh".into());
        assert!(bad.validate().unwrap_err().contains("executable.file"));
        let mut bad = config("make {out}");
        bad.inputs = vec!["/abs/**".into()];
        assert!(bad.validate().unwrap_err().contains("executable.inputs"));
        let mut bad = config("make {out}");
        bad.inputs = vec!["a/[".into()];
        assert!(bad.validate().unwrap_err().contains("valid glob"));
        let mut bad = config("make {out}");
        bad.target_files.insert("Not A Triple".into(), "x".into());
        assert!(bad.validate().unwrap_err().contains("target_files"));
        let mut bad = config("make {out}");
        bad.toolchain_version_cmd = Some("  ".into());
        assert!(bad.validate().unwrap_err().contains("toolchain_version_cmd"));
    }
}
