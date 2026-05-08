//! FFI-related methods for `ResolvedCrateConfig`.

use super::ResolvedCrateConfig;

impl ResolvedCrateConfig {
    /// Get the FFI prefix (e.g., `"kreuzberg"`). Used by FFI, Go, Java, C# backends.
    ///
    /// Returns `[ffi] prefix` if set, otherwise derives from the crate name by
    /// replacing hyphens with underscores.
    pub fn ffi_prefix(&self) -> String {
        self.ffi
            .as_ref()
            .and_then(|f| f.prefix.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the FFI native library name (for Go cgo, Java Panama, C# P/Invoke).
    ///
    /// Resolution order:
    /// 1. `[ffi] lib_name` explicit override
    /// 2. Directory name of the user-supplied `[crates.output] ffi` path with
    ///    hyphens replaced by underscores (e.g. `crates/html-to-markdown-ffi/src/`
    ///    → `html_to_markdown_ffi`). Walks components from the end and skips
    ///    `src`/`lib`/`include` to find the crate directory.
    /// 3. `{ffi_prefix}_ffi` fallback
    pub fn ffi_lib_name(&self) -> String {
        // 1. Explicit override in [ffi] section.
        if let Some(name) = self.ffi.as_ref().and_then(|f| f.lib_name.as_ref()) {
            return name.clone();
        }

        // 2. Derive from the user-supplied `[crates.output] ffi` path. We use
        //    `explicit_output` (the raw user input) — NOT `output_paths` — so a
        //    template-derived FFI dir does not accidentally drive the lib name.
        if let Some(ffi_path) = self.explicit_output.ffi.as_ref() {
            let crate_dir = ffi_path
                .components()
                .filter_map(|c| {
                    if let std::path::Component::Normal(s) = c {
                        s.to_str()
                    } else {
                        None
                    }
                })
                .rev()
                .find(|&s| s != "src" && s != "lib" && s != "include");
            if let Some(dir) = crate_dir {
                return dir.replace('-', "_");
            }
        }

        // 3. Default fallback.
        format!("{}_ffi", self.ffi_prefix())
    }

    /// Get the FFI header name.
    ///
    /// Returns `[ffi] header_name` if set, otherwise `"{ffi_prefix}.h"`.
    pub fn ffi_header_name(&self) -> String {
        self.ffi
            .as_ref()
            .and_then(|f| f.header_name.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("{}.h", self.ffi_prefix()))
    }

    /// Get the relative path to the FFI crate from the e2e test directory.
    ///
    /// Used by C e2e tests to locate the compiled FFI library when building
    /// against a local checkout rather than a downloaded release.
    ///
    /// Resolution order:
    /// 1. Directory name of the user-supplied `[crates.output] ffi` path,
    ///    skipping trailing `src`/`lib`/`include` components, prefixed with
    ///    `../../` so the path resolves from `e2e/c/` back to the repo root.
    ///    E.g. `crates/my-lib-ffi/src/` → `../../crates/my-lib-ffi`.
    /// 2. `../../crates/{name}-ffi` fallback derived from the crate name.
    pub fn ffi_crate_path(&self) -> String {
        if let Some(ffi_path) = self.explicit_output.ffi.as_ref() {
            // Walk path components from the end, skipping src/lib/include.
            let components: Vec<&str> = ffi_path
                .components()
                .filter_map(|c| {
                    if let std::path::Component::Normal(s) = c {
                        s.to_str()
                    } else {
                        None
                    }
                })
                .collect();
            // Find the crate directory component (first non-leaf from the right
            // after skipping src/lib/include).
            if let Some(idx) = components
                .iter()
                .rposition(|&s| s != "src" && s != "lib" && s != "include")
            {
                // Reconstruct the path up to and including this component.
                let meaningful = &components[..=idx];
                return format!("../../{}", meaningful.join("/"));
            }
        }
        format!("../../crates/{}-ffi", self.name)
    }

    /// Get the relative path to the WASM crate's `pkg/` directory from the
    /// e2e test directory.
    ///
    /// Used by WASM e2e tests to import the wasm-pack build output when
    /// working against a local checkout rather than a published npm package.
    ///
    /// Resolution order:
    /// 1. Directory name of the user-supplied `[crates.output] wasm` path,
    ///    skipping trailing `src`/`lib`/`include` components, prefixed with
    ///    `../../` and suffixed with `/pkg`.
    ///    E.g. `crates/my-lib-wasm/src/` → `../../crates/my-lib-wasm/pkg`.
    /// 2. `../../crates/{name}-wasm/pkg` fallback derived from the crate name.
    pub fn wasm_crate_path(&self) -> String {
        if let Some(wasm_path) = self.explicit_output.wasm.as_ref() {
            let components: Vec<&str> = wasm_path
                .components()
                .filter_map(|c| {
                    if let std::path::Component::Normal(s) = c {
                        s.to_str()
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(idx) = components
                .iter()
                .rposition(|&s| s != "src" && s != "lib" && s != "include")
            {
                let meaningful = &components[..=idx];
                return format!("../../{}/pkg", meaningful.join("/"));
            }
        }
        format!("../../crates/{}-wasm/pkg", self.name)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal_ffi() -> super::super::ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn ffi_prefix_defaults_to_snake_case_name() {
        let r = minimal_ffi();
        assert_eq!(r.ffi_prefix(), "my_lib");
    }

    #[test]
    fn ffi_prefix_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "custom_prefix"
"#,
        );
        assert_eq!(r.ffi_prefix(), "custom_prefix");
    }

    #[test]
    fn ffi_lib_name_falls_back_to_prefix_ffi() {
        let r = minimal_ffi();
        assert_eq!(r.ffi_lib_name(), "my_lib_ffi");
    }

    #[test]
    fn ffi_lib_name_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
lib_name = "libmy_custom"
"#,
        );
        assert_eq!(r.ffi_lib_name(), "libmy_custom");
    }

    #[test]
    fn ffi_header_name_defaults_to_prefix_h() {
        let r = minimal_ffi();
        assert_eq!(r.ffi_header_name(), "my_lib.h");
    }

    #[test]
    fn ffi_lib_name_derives_from_explicit_output_path() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.output]
ffi = "crates/html-to-markdown-ffi/src/"
"#,
        );
        // Step 2 of resolution: derive from `[crates.output] ffi` path,
        // skipping `src`/`lib`/`include` and replacing hyphens with underscores.
        assert_eq!(r.ffi_lib_name(), "html_to_markdown_ffi");
    }

    #[test]
    fn ffi_lib_name_explicit_lib_name_overrides_output_path_derivation() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
lib_name = "explicit_wins"

[crates.output]
ffi = "crates/html-to-markdown-ffi/src/"
"#,
        );
        // Step 1 (explicit lib_name) takes precedence over step 2 (output path).
        assert_eq!(r.ffi_lib_name(), "explicit_wins");
    }

    #[test]
    fn ffi_lib_name_template_derived_output_does_not_drive_lib_name() {
        // No explicit `[crates.output] ffi`. The template-derived path
        // (e.g. `packages/ffi/my-lib/`) must NOT drive the lib_name —
        // it falls through to step 3 (`{ffi_prefix}_ffi`).
        let r = minimal_ffi();
        assert_eq!(r.ffi_lib_name(), "my_lib_ffi");
    }

    #[test]
    fn ffi_header_name_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
header_name = "custom.h"
"#,
        );
        assert_eq!(r.ffi_header_name(), "custom.h");
    }
}
