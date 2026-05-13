//! Gleam-specific naming helpers for `ResolvedCrateConfig`.

use alef_core::config::ResolvedCrateConfig;

/// Get the Gleam app name.
///
/// Returns `[gleam] app_name` if set, otherwise derives a snake_case name from
/// the crate name by replacing hyphens with underscores.
pub fn gleam_app_name(config: &ResolvedCrateConfig) -> String {
    config
        .gleam
        .as_ref()
        .and_then(|g| g.app_name.as_ref())
        .cloned()
        .unwrap_or_else(|| config.name.replace('-', "_"))
}

/// Get the Gleam NIF module name (Erlang atom for `@external(erlang, "<nif>", ...)` lookups).
///
/// Defaults to `"Elixir.<PascalCase>.Native"` to match the atom registered by
/// `rustler::init!` in the Rustler backend.
pub fn gleam_nif_module(config: &ResolvedCrateConfig) -> String {
    use heck::ToUpperCamelCase;
    config
        .gleam
        .as_ref()
        .and_then(|g| g.nif_module.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            let pascal = config
                .elixir
                .as_ref()
                .and_then(|e| e.app_name.as_deref())
                .unwrap_or(&config.name)
                .to_upper_camel_case();
            format!("Elixir.{pascal}.Native")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn gleam_app_name_replaces_hyphens() {
        let r = minimal();
        assert_eq!(gleam_app_name(&r), "test_lib");
    }

    #[test]
    fn gleam_app_name_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.gleam]
app_name = "my_gleam_app"
"#,
        );
        assert_eq!(gleam_app_name(&r), "my_gleam_app");
    }

    #[test]
    fn gleam_nif_module_defaults_to_elixir_pascal_native() {
        let r = minimal();
        assert_eq!(gleam_nif_module(&r), "Elixir.TestLib.Native");
    }

    #[test]
    fn gleam_nif_module_uses_elixir_app_name_when_set() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "my_app"
"#,
        );
        assert_eq!(gleam_nif_module(&r), "Elixir.MyApp.Native");
    }

    #[test]
    fn gleam_nif_module_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.gleam]
nif_module = "CustomModule.Native"
"#,
        );
        assert_eq!(gleam_nif_module(&r), "CustomModule.Native");
    }
}
