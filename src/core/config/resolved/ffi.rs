//! FFI-related methods for `ResolvedCrateConfig`.

use crate::core::config::KotlinFfiStyle;

use super::ResolvedCrateConfig;

impl ResolvedCrateConfig {
    /// Get the base name used to derive the JNI crate directory and package name.
    ///
    /// Resolution order:
    /// 1. `[crates.jni] crate_dir` explicit override — allows consumers whose
    ///    `config.name` carries a language suffix (e.g. `"sample-markdown-rs"`)
    ///    to place the JNI crate at `crates/<override>-jni/` instead.
    /// 2. `config.name` fallback — preserves the existing behaviour and keeps
    ///    parser-pack's `parser-language-pack-jni` working correctly even though
    ///    its `core_crate_dir()` differs (`parser-core-core`).
    pub fn jni_crate_base(&self) -> &str {
        self.jni
            .as_ref()
            .and_then(|j| j.crate_dir.as_deref())
            .unwrap_or(&self.name)
    }

    /// Get the JNI native library name used by Android JNI Bridge objects.
    ///
    /// Returns `<ffi_prefix>_jni`, parallel to [`Self::ffi_lib_name`].
    /// This is the library name passed to `System.loadLibrary(...)` in the
    /// emitted Kotlin Bridge object when `KotlinFfiStyle::Jni` is active.
    pub fn jni_lib_name(&self) -> String {
        format!("{}_jni", self.ffi_prefix())
    }

    /// Returns the configured Kotlin FFI emission style.
    ///
    /// Reads `[crates.kotlin] ffi_style`; defaults to `KotlinFfiStyle::Panama`.
    /// The `alef-backend-kotlin-android` backend overrides this to `Jni`
    /// unconditionally via [`Self::with_kotlin_ffi_style`].
    pub fn kotlin_ffi_style(&self) -> KotlinFfiStyle {
        self.kotlin.as_ref().map(|k| k.ffi_style).unwrap_or_default()
    }

    /// Return a clone of this config with the Kotlin FFI style forced to `style`.
    ///
    /// Used by the Android backend so all downstream emitters see `Jni` mode
    /// regardless of what the user wrote in `[crates.kotlin] ffi_style`.
    pub fn with_kotlin_ffi_style(mut self, style: KotlinFfiStyle) -> Self {
        self.kotlin.get_or_insert_with(Default::default).ffi_style = style;
        self
    }

    /// Get the FFI prefix (e.g., `"sample_core"`). Used by FFI, Go, Java, C# backends.
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
    ///    hyphens replaced by underscores (e.g. `crates/sample-markdown-ffi/src/`
    ///    → `sample_markdown_ffi`). Walks components from the end and skips
    ///    `src`/`lib`/`include` to find the crate directory.
    /// 3. `{ffi_prefix}_ffi` fallback
    pub fn ffi_lib_name(&self) -> String {
        if let Some(name) = self.ffi.as_ref().and_then(|f| f.lib_name.as_ref()) {
            return name.clone();
        }

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

    /// Resolve the Rust expression used by FFI plugin shims
    /// (`plugin_impl_initialize`, `plugin_impl_shutdown`) to construct an
    /// error value from a runtime `String` named `msg`.
    ///
    /// Returns `[ffi] plugin_error_constructor` verbatim when set; otherwise
    /// `None` so callers can fall back to a generic constructor that doesn't
    /// depend on a specific error variant shape.
    pub fn ffi_plugin_error_constructor(&self) -> Option<String> {
        self.ffi
            .as_ref()
            .and_then(|f| f.plugin_error_constructor.as_ref())
            .cloned()
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
            if let Some(idx) = components
                .iter()
                .rposition(|&s| s != "src" && s != "lib" && s != "include")
            {
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

    /// Get the relative path to the JNI crate from the kotlin-android package
    /// root (packages/kotlin-android/). Used to invoke `cargo build --release`
    /// for the host JNI library.
    ///
    /// Returns `../../crates/{jni_crate_base}-jni`.
    pub fn jni_crate_path(&self) -> String {
        format!("../../crates/{}-jni", self.jni_crate_base())
    }
}

#[cfg(test)]
mod tests {
    use crate::core::config::new_config::NewAlefConfig;

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
ffi = "crates/sample-markdown-ffi/src/"
"#,
        );
        assert_eq!(r.ffi_lib_name(), "sample_markdown_ffi");
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
ffi = "crates/sample-markdown-ffi/src/"
"#,
        );
        assert_eq!(r.ffi_lib_name(), "explicit_wins");
    }

    #[test]
    fn ffi_lib_name_template_derived_output_does_not_drive_lib_name() {
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
