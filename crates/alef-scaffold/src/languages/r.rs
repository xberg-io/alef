use crate::{cargo_package_header, core_dep_features, detect_workspace_inheritance, scaffold_meta};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
use std::path::PathBuf;

pub(crate) fn scaffold_r(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let package_name = config.r_package_name();

    let mut description = meta.description.clone();
    if description.ends_with('.') {
        description.pop();
    }

    let authors_r = if meta.authors.is_empty() {
        r#"Authors@R: person("Author", "Name", email = "author@example.com", role = c("aut", "cre"))"#.to_string()
    } else {
        format!(
            "Authors@R: person(\"{}\", email = \"author@example.com\", role = c(\"aut\", \"cre\"))",
            meta.authors.first().unwrap_or(&"Author Name".to_string())
        )
    };

    let content = format!(
        r#"Package: {package}
Title: {title}
Version: {version}
{authors}
Description: {description}
    Rust bindings generated with extendr.
URL: {repository}
BugReports: {repository}/issues
License: {license}
Depends: R (>= 4.2)
Imports: jsonlite
Suggests:
    testthat (>= 3.0.0),
    withr,
    roxygen2,
    lintr,
    styler
SystemRequirements: Cargo (Rust's package manager), rustc (>= 1.91)
Config/rextendr/version: {rextendr}
Encoding: UTF-8
Roxygen: list(markdown = TRUE)
RoxygenNote: 7.3.3
Config/testthat/edition: 3
"#,
        package = package_name,
        title = meta.description,
        version = version,
        authors = authors_r,
        description = description,
        repository = meta.repository,
        license = meta.license,
        rextendr = tv::cran::REXTENDR,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/r/DESCRIPTION"),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/r/.lintr"),
            content: r#"linters: linters_with_defaults(
    line_length_linter(120),
    object_name_linter = NULL,
    object_usage_linter = NULL,
    commented_code_linter = NULL
  )
"#
            .to_string(),
            generated_header: false,
        },
    ])
}

pub(crate) fn scaffold_r_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.crate_config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-r"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../../../../crates/{core_crate_dir}"{features} }}
extendr-api = {{ version = "{extendr_api}", features = ["use-precompiled-bindings"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::R),
        extendr_api = tv::cargo::EXTENDR_API,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/r/src/rust/Cargo.toml".to_string()),
        content,
        generated_header: true,
    }])
}
