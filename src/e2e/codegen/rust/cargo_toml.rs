//! Cargo.toml generation for Rust e2e test crates.

use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;

/// Render a `Cargo.toml` for the Rust e2e test crate.
///
/// Generates all dependency lines based on which test features are needed
/// (mock server, HTTP tests, tokio, trait-bridge stubs, etc.).
#[allow(clippy::too_many_arguments)]
pub fn render_cargo_toml(
    crate_name: &str,
    dep_name: &str,
    crate_path: &str,
    needs_serde_json: bool,
    needs_mock_server: bool,
    needs_http_tests: bool,
    needs_tokio: bool,
    needs_tower_http: bool,
    needs_anyhow: bool,
    dep_mode: crate::e2e::config::DependencyMode,
    version: Option<&str>,
    features: &[String],
) -> String {
    let e2e_name = format!("{dep_name}-e2e-rust");
    // Use only the features explicitly configured in alef.toml.
    // Do NOT auto-add "serde" — the target crate may not have that feature.
    // serde_json is added as a separate dependency when needed.
    let effective_features: Vec<&str> = features.iter().map(|s| s.as_str()).collect();
    let features_str = if effective_features.is_empty() {
        String::new()
    } else {
        format!(", default-features = false, features = {:?}", effective_features)
    };
    let dep_spec = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            let ver = version.unwrap_or("0.1.0");
            if crate_name != dep_name {
                format!("{dep_name} = {{ package = \"{crate_name}\", version = \"{ver}\"{features_str} }}")
            } else if effective_features.is_empty() {
                format!("{dep_name} = \"{ver}\"")
            } else {
                format!("{dep_name} = {{ version = \"{ver}\"{features_str} }}")
            }
        }
        crate::e2e::config::DependencyMode::Local => {
            if crate_name != dep_name {
                format!("{dep_name} = {{ package = \"{crate_name}\", path = \"{crate_path}\"{features_str} }}")
            } else if effective_features.is_empty() {
                format!("{dep_name} = {{ path = \"{crate_path}\" }}")
            } else {
                format!("{dep_name} = {{ path = \"{crate_path}\"{features_str} }}")
            }
        }
    };
    // serde_json is needed either when args use json_object/handle, or when the
    // mock server binary is present (it uses serde_json::Value for fixture bodies),
    // or when http integration tests are generated (they serialize fixture bodies).
    let effective_needs_serde_json = needs_serde_json || needs_mock_server || needs_http_tests;
    let serde_line = if effective_needs_serde_json {
        "\nserde_json = \"1\""
    } else {
        ""
    };
    // An empty `[workspace]` table makes the e2e crate its own workspace root, so
    // it never gets pulled into a parent crate's workspace. This means consumers
    // don't have to remember to add `e2e/rust` to `workspace.exclude`, and
    // `cargo fmt`/`cargo build` work the same whether the parent has a
    // workspace or not.
    // Mock server requires axum (HTTP router) and tokio-stream (SSE streaming).
    // The standalone binary additionally needs serde (derive) and walkdir.
    // Http integration tests require axum-test for the test server.
    let needs_axum = needs_mock_server || needs_http_tests;
    let mock_lines = if needs_axum {
        let mut lines = format!(
            "\naxum = \"{axum}\"\nserde = {{ version = \"1\", features = [\"derive\"] }}\nwalkdir = \"{walkdir}\"",
            axum = tv::cargo::AXUM,
            walkdir = tv::cargo::WALKDIR,
        );
        if needs_mock_server {
            lines.push_str(&format!(
                "\ntokio-stream = \"{tokio_stream}\"",
                tokio_stream = tv::cargo::TOKIO_STREAM
            ));
        }
        if needs_http_tests {
            lines.push_str("\naxum-test = \"20\"\nbytes = \"1\"");
        }
        if needs_tower_http {
            lines.push_str(&format!(
                "\ntower-http = {{ version = \"{tower_http}\", features = [\"cors\", \"fs\"] }}\ntempfile = \"{tempfile}\"",
                tower_http = tv::cargo::TOWER_HTTP,
                tempfile = tv::cargo::TEMPFILE,
            ));
        }
        lines
    } else {
        String::new()
    };
    let mut machete_ignored: Vec<&str> = Vec::new();
    if effective_needs_serde_json {
        machete_ignored.push("\"serde_json\"");
    }
    if needs_axum {
        machete_ignored.push("\"axum\"");
        machete_ignored.push("\"serde\"");
        machete_ignored.push("\"walkdir\"");
    }
    if needs_mock_server {
        machete_ignored.push("\"tokio-stream\"");
    }
    if needs_http_tests {
        machete_ignored.push("\"axum-test\"");
        machete_ignored.push("\"bytes\"");
    }
    if needs_tower_http {
        machete_ignored.push("\"tower-http\"");
        machete_ignored.push("\"tempfile\"");
    }
    // anyhow and async-trait are deps used by trait-bridge stubs; machete would
    // flag them as unused since they're only referenced in generated impl code.
    if needs_anyhow {
        machete_ignored.push("\"anyhow\"");
        machete_ignored.push("\"async-trait\"");
    }
    let machete_section = if machete_ignored.is_empty() {
        String::new()
    } else {
        format!(
            "\n[package.metadata.cargo-machete]\nignored = [{}]\n",
            machete_ignored.join(", ")
        )
    };
    let tokio_line = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"full\"] }"
    } else {
        ""
    };
    // Trait-bridge stubs use `#[async_trait]` on impl blocks (required when the
    // trait itself is `#[async_trait]`-decorated).  `anyhow` is kept as a direct
    // dep for any crates that still reference `anyhow::Error` in fixture code.
    let anyhow_line = if needs_anyhow {
        format!(
            "\nanyhow = \"1\"\nasync-trait = \"{async_trait}\"",
            async_trait = tv::cargo::ASYNC_TRAIT,
        )
    } else {
        String::new()
    };
    let bin_section = if needs_mock_server || needs_http_tests {
        "\n[[bin]]\nname = \"mock-server\"\npath = \"src/main.rs\"\n"
    } else {
        ""
    };
    // E2e package version tracks the consumer crate version so `alef sync-versions`
    // doesn't rewrite the generated file on every prek run. Fall back to "0.1.0" only
    // when no consumer version is known (test fixtures, etc.).
    let pkg_version = version.unwrap_or("0.1.0");
    let header = hash::header(CommentStyle::Hash);
    format!(
        r#"{header}
[workspace]

[package]
name = "{e2e_name}"
version = "{pkg_version}"
edition = "2021"
license = "MIT"
publish = false
{bin_section}
[dependencies]
{dep_spec}{serde_line}{anyhow_line}{mock_lines}{tokio_line}
{machete_section}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::DependencyMode;

    #[test]
    fn render_cargo_toml_local_no_features_produces_path_dep() {
        // When crate_name ("my-crate") differs from dep_name ("my_crate") a
        // `package = …` key is required to tell Cargo the actual crate name.
        let out = render_cargo_toml(
            "my-crate",
            "my_crate",
            "../../crates/my-crate",
            false,
            false,
            false,
            false,
            false,
            false,
            DependencyMode::Local,
            None,
            &[],
        );
        assert!(
            out.contains("my_crate = { package = \"my-crate\", path = \"../../crates/my-crate\" }"),
            "got:\n{out}"
        );
        assert!(out.contains("edition = \"2021\""));
    }

    #[test]
    fn render_cargo_toml_local_same_name_produces_simple_path_dep() {
        // When crate_name and dep_name are identical no `package` key is needed.
        let out = render_cargo_toml(
            "my_crate",
            "my_crate",
            "../../crates/my_crate",
            false,
            false,
            false,
            false,
            false,
            false,
            DependencyMode::Local,
            None,
            &[],
        );
        assert!(
            out.contains("my_crate = { path = \"../../crates/my_crate\" }"),
            "got:\n{out}"
        );
    }

    #[test]
    fn render_cargo_toml_has_no_issues_docs_header_line() {
        // Regression: older alef injected a `# Issues & docs: …` header line via
        // header_for_config. cargo-sort always strips it, so every prek run
        // oscillated between cargo-sort removing it and alef re-adding it.
        // The e2e rust Cargo.toml must use the plain hash::header (no issues_url).
        let out = render_cargo_toml(
            "my-crate",
            "my_crate",
            "../../crates/my-crate",
            false,
            false,
            false,
            false,
            false,
            false,
            DependencyMode::Local,
            None,
            &[],
        );
        assert!(
            !out.contains("Issues & docs:"),
            "e2e Cargo.toml must not contain 'Issues & docs:' — cargo-sort strips it, \
             causing prek to loop forever:\n{out}"
        );
    }
}
