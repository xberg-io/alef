//! Cargo.toml generation for Rust e2e test crates.

use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;

/// Render a `Cargo.toml` for the Rust e2e test crate.
///
/// Generates all dependency lines based on which test features are needed
/// (mock server, HTTP tests, tokio, trait-bridge stubs, etc.).
///
/// The emitted file is idempotent under `cargo sort`: dependencies are in
/// alphabetical order and `[package.metadata.cargo-machete]` appears
/// immediately after `[package]`, which is the canonical position that
/// `cargo sort` would choose if it ran on the file.
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
    // An empty `[workspace]` table makes the e2e crate its own workspace root, so
    // it never gets pulled into a parent crate's workspace. This means consumers
    // don't have to remember to add `e2e/rust` to `workspace.exclude`, and
    // `cargo fmt`/`cargo build` work the same whether the parent has a
    // workspace or not.
    // Mock server requires axum (HTTP router) and tokio-stream (SSE streaming).
    // The standalone binary additionally needs serde (derive) and walkdir.
    // Http integration tests require axum-test for the test server.
    let needs_axum = needs_mock_server || needs_http_tests;

    // Build the cargo-machete ignore list.
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

    // Build metadata and bin sections as self-contained block strings.
    // [package.metadata.cargo-machete] must appear immediately after [package]
    // and before [[bin]] — that is the canonical position cargo-sort would
    // choose, so emitting them in this order keeps the output idempotent.
    let machete_section = if machete_ignored.is_empty() {
        String::new()
    } else {
        format!(
            "[package.metadata.cargo-machete]\nignored = [{}]",
            machete_ignored.join(", ")
        )
    };
    let bin_section = if needs_mock_server || needs_http_tests {
        String::from("[[bin]]\nname = \"mock-server\"\npath = \"src/main.rs\"")
    } else {
        String::new()
    };

    // E2e package version tracks the consumer crate version so `alef sync-versions`
    // doesn't rewrite the generated file on every prek run. Fall back to "0.1.0" only
    // when no consumer version is known (test fixtures, etc.).
    let pkg_version = version.unwrap_or("0.1.0");
    let header = hash::header(CommentStyle::Hash);

    // Collect all dependency entries into a sortable Vec so the emitted
    // [dependencies] block is alphabetically ordered.  cargo-sort rewrites any
    // non-alphabetical block, causing prek to oscillate between alef's order
    // and cargo-sort's canonical order on every run.
    let mut dep_entries: Vec<(String, String)> = Vec::new();
    dep_entries.push((dep_name.to_string(), dep_spec.clone()));
    if effective_needs_serde_json {
        dep_entries.push((
            "serde_json".to_string(),
            format!("serde_json = \"{}\"", tv::cargo::SERDE_JSON),
        ));
    }
    if needs_anyhow {
        dep_entries.push(("anyhow".to_string(), "anyhow = \"1\"".to_string()));
        dep_entries.push((
            "async-trait".to_string(),
            format!("async-trait = \"{async_trait}\"", async_trait = tv::cargo::ASYNC_TRAIT),
        ));
    }
    if needs_axum {
        dep_entries.push(("axum".to_string(), format!("axum = \"{axum}\"", axum = tv::cargo::AXUM)));
        dep_entries.push((
            "serde".to_string(),
            format!(
                "serde = {{ version = \"{}\", features = [\"derive\"] }}",
                tv::cargo::SERDE
            ),
        ));
        dep_entries.push((
            "walkdir".to_string(),
            format!("walkdir = \"{walkdir}\"", walkdir = tv::cargo::WALKDIR),
        ));
        if needs_mock_server {
            dep_entries.push((
                "tokio-stream".to_string(),
                format!(
                    "tokio-stream = \"{tokio_stream}\"",
                    tokio_stream = tv::cargo::TOKIO_STREAM
                ),
            ));
        }
        if needs_http_tests {
            dep_entries.push(("axum-test".to_string(), "axum-test = \"20\"".to_string()));
            dep_entries.push(("bytes".to_string(), "bytes = \"1\"".to_string()));
        }
        if needs_tower_http {
            dep_entries.push((
                "tower-http".to_string(),
                format!(
                    "tower-http = {{ version = \"{tower_http}\", features = [\"cors\", \"fs\"] }}",
                    tower_http = tv::cargo::TOWER_HTTP
                ),
            ));
            dep_entries.push((
                "tempfile".to_string(),
                format!("tempfile = \"{tempfile}\"", tempfile = tv::cargo::TEMPFILE),
            ));
        }
    }
    if needs_tokio {
        dep_entries.push((
            "tokio".to_string(),
            format!(
                "tokio = {{ version = \"{}\", features = [\"full\"] }}",
                tv::cargo::TOKIO
            ),
        ));
    }
    dep_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let dep_block = dep_entries
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Assemble the middle block (between [package] and [dependencies]).
    // Each present section is separated by exactly one blank line.
    let mut middle_sections: Vec<&str> = Vec::new();
    if !machete_section.is_empty() {
        middle_sections.push(&machete_section);
    }
    if !bin_section.is_empty() {
        middle_sections.push(&bin_section);
    }
    let middle_block = middle_sections.join("\n\n");
    let middle_separator = if middle_block.is_empty() { "" } else { "\n\n" };

    format!(
        "{header}\n[workspace]\n\n[package]\nname = \"{e2e_name}\"\nversion = \"{pkg_version}\"\nedition = \"2021\"\nlicense = \"MIT\"\npublish = false{middle_separator}{middle_block}\n\n[dependencies]\n{dep_block}\n"
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

    #[test]
    fn render_cargo_toml_deps_are_alphabetically_sorted() {
        // Regression: alef emitted deps in code order (consumer crate first,
        // then serde_json, axum, serde, walkdir, tokio-stream, tokio).
        // cargo-sort rewrites unsorted blocks causing prek to oscillate.
        let out = render_cargo_toml(
            "my-crate",
            "my_crate",
            "../../crates/my-crate",
            true,  // needs_serde_json
            true,  // needs_mock_server
            false, // needs_http_tests
            true,  // needs_tokio
            false, // needs_tower_http
            false, // needs_anyhow
            DependencyMode::Local,
            Some("1.2.3"),
            &[],
        );
        // Extract the [dependencies] block and verify key order.
        let deps_start = out.find("[dependencies]\n").expect("missing [dependencies]");
        let deps_block = &out[deps_start + "[dependencies]\n".len()..];
        let dep_keys: Vec<&str> = deps_block
            .lines()
            .take_while(|l| !l.starts_with('['))
            .filter(|l| !l.is_empty())
            .map(|l| l.split_once(" = ").map(|(k, _)| k).unwrap_or(l))
            .collect();
        let mut sorted = dep_keys.clone();
        sorted.sort();
        assert_eq!(
            dep_keys, sorted,
            "dependencies must be in alphabetical order (cargo-sort canonical), got:\n{out}"
        );
    }

    #[test]
    fn render_cargo_toml_machete_section_precedes_bin_section() {
        // Regression: older alef put [package.metadata.cargo-machete] at the
        // end of the file.  cargo-sort moves it to immediately after [package],
        // before [[bin]], causing prek to oscillate.
        let out = render_cargo_toml(
            "my-crate",
            "my_crate",
            "../../crates/my-crate",
            true,  // needs_serde_json → triggers machete section
            true,  // needs_mock_server → triggers [[bin]] section
            false, // needs_http_tests
            true,  // needs_tokio
            false, // needs_tower_http
            false, // needs_anyhow
            DependencyMode::Local,
            Some("1.2.3"),
            &[],
        );
        let machete_pos = out
            .find("[package.metadata.cargo-machete]")
            .expect("[package.metadata.cargo-machete] missing");
        let bin_pos = out.find("[[bin]]").expect("[[bin]] missing");
        let deps_pos = out.find("[dependencies]").expect("[dependencies] missing");
        assert!(
            machete_pos < bin_pos,
            "[package.metadata.cargo-machete] must precede [[bin]] — cargo-sort canonical order:\n{out}"
        );
        assert!(bin_pos < deps_pos, "[[bin]] must precede [dependencies]:\n{out}");
        // Must not appear at the end (after [dependencies]).
        assert!(
            machete_pos < deps_pos,
            "[package.metadata.cargo-machete] must not appear after [dependencies]:\n{out}"
        );
    }
}
