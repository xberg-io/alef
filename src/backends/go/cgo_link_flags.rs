//! Per-platform linker arguments the generated cgo preamble appends after the FFI library's `-l`.

use crate::core::config::ResolvedCrateConfig;
use anyhow::{Result, bail};

/// GOOS values cgo accepts in a `#cgo` build constraint.
const GOOS: &[&str] = &[
    "aix",
    "android",
    "darwin",
    "dragonfly",
    "freebsd",
    "illumos",
    "ios",
    "linux",
    "netbsd",
    "openbsd",
    "solaris",
    "windows",
];

/// GOARCH values cgo accepts in a `#cgo` build constraint.
const GOARCH: &[&str] = &[
    "386", "amd64", "arm", "arm64", "loong64", "mips", "mips64", "mips64le", "mipsle", "ppc64", "ppc64le", "riscv64",
    "s390x",
];

/// One `#cgo <constraint> LDFLAGS: <flags>` preamble line, ready for the template.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct LinkFlagLine {
    pub constraint: String,
    pub flags: String,
}

/// The `[crates.go] link_flags` entries as preamble lines, in constraint order.
///
/// The constraint and every flag are interpolated into a C comment that cgo parses line by
/// line, so both are checked against an allowlist here instead of being escaped: a constraint
/// must be `GOOS` or `GOOS,GOARCH`, and a flag may not carry a newline, any other control
/// character, or the `*/` that would close the preamble early. ~keep
pub(crate) fn cgo_link_flag_lines(config: &ResolvedCrateConfig) -> Result<Vec<LinkFlagLine>> {
    let Some(go) = config.go.as_ref() else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::with_capacity(go.link_flags.len());
    for (constraint, flags) in &go.link_flags {
        validate_constraint(constraint)?;
        if flags.is_empty() {
            continue;
        }
        for flag in flags {
            if flag.trim().is_empty() || flag.chars().any(char::is_control) || flag.contains("*/") {
                bail!("[crates.go.link_flags.{constraint}]: invalid linker flag {flag:?}");
            }
        }
        lines.push(LinkFlagLine {
            constraint: constraint.clone(),
            flags: flags.join(" "),
        });
    }
    Ok(lines)
}

fn validate_constraint(constraint: &str) -> Result<()> {
    let mut parts = constraint.split(',');
    let os_ok = parts.next().is_some_and(|os| GOOS.contains(&os));
    let arch_ok = parts.next().is_none_or(|arch| GOARCH.contains(&arch));
    if !os_ok || !arch_ok || parts.next().is_some() {
        bail!(
            "[crates.go.link_flags]: invalid cgo constraint {constraint:?}; expected a GOOS \
             (`darwin`, `linux`, `windows`, ...) or `GOOS,GOARCH` (`windows,arm64`)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backend::Backend as _;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::ApiSurface;

    fn resolved(go_extra: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "demo"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "demo"
[crates.go]
module = "github.com/test/demo"
{go_extra}
"#
        );
        let config: NewAlefConfig = toml::from_str(&toml).unwrap();
        config.resolve().unwrap().remove(0)
    }

    fn binding_go(config: &ResolvedCrateConfig) -> String {
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            version: "0.1.0".to_string(),
            ..ApiSurface::default()
        };
        let files = crate::backends::go::GoBackend
            .generate_bindings(&api, config)
            .expect("generate the Go backend files");
        files
            .into_iter()
            .find(|file| file.path.to_string_lossy().ends_with("binding.go"))
            .expect("binding.go is generated")
            .content
    }

    #[test]
    fn preamble_links_windows_arm64_from_the_platform_dir_the_packager_ships() {
        let content = binding_go(&resolved(""));
        assert!(
            content.contains("#cgo windows,arm64 LDFLAGS: -L${SRCDIR}/.lib/windows-aarch64 -ldemo"),
            "missing windows,arm64 LDFLAGS line:\n{content}"
        );
    }

    #[test]
    fn link_flags_render_after_the_library_flag_one_line_per_constraint() {
        let content = binding_go(&resolved(
            r#"
[crates.go.link_flags]
darwin = ["-framework", "Security", "-framework", "CoreFoundation", "-liconv", "-lresolv"]
"windows,amd64" = ["-lws2_32", "-luserenv"]
"#,
        ));
        let darwin = "#cgo darwin LDFLAGS: -framework Security -framework CoreFoundation -liconv -lresolv";
        let windows = "#cgo windows,amd64 LDFLAGS: -lws2_32 -luserenv";
        assert!(content.contains(darwin), "missing darwin line:\n{content}");
        assert!(content.contains(windows), "missing windows line:\n{content}");
        let last_library_flag = content.rfind("-ldemo").expect("library flag is present");
        assert!(
            content.find(darwin).unwrap() > last_library_flag,
            "system libraries must follow the static library they resolve symbols for"
        );
    }

    #[test]
    fn no_link_flags_adds_no_extra_ldflags_lines() {
        let config = resolved("");
        assert!(cgo_link_flag_lines(&config).unwrap().is_empty());
        let content = binding_go(&config);
        assert_eq!(content.matches("LDFLAGS:").count(), 6, "six platform lines only");
    }

    #[test]
    fn rejects_constraints_and_flags_that_could_break_out_of_the_preamble() {
        let bad_constraint = resolved("[crates.go.link_flags]\n\"darwin\\nfoo\" = [\"-lx\"]\n");
        assert!(cgo_link_flag_lines(&bad_constraint).is_err());
        let bad_arch = resolved("[crates.go.link_flags]\n\"darwin,arm64,extra\" = [\"-lx\"]\n");
        assert!(cgo_link_flag_lines(&bad_arch).is_err());
        let newline = resolved("[crates.go.link_flags]\ndarwin = [\"-lx\\n#cgo LDFLAGS: -evil\"]\n");
        assert!(cgo_link_flag_lines(&newline).is_err());
        let close = resolved("[crates.go.link_flags]\ndarwin = [\"-lx */\"]\n");
        assert!(cgo_link_flag_lines(&close).is_err());
    }
}
