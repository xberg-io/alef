use crate::{cargo_package_header, core_dep_features, detect_workspace_inheritance, scaffold_meta};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
use alef_core::version::to_r_version;
use std::path::PathBuf;

pub(crate) fn scaffold_r(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    // R / CRAN rejects SemVer dash-form prereleases; convert to the four-component form.
    let version = to_r_version(&api.version);
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

    // extendr requires staticlib (for R's dyn.load) + lib (for Rust tests).
    // "cdylib" alone causes linker failures on macOS/Linux.
    let cargo_content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["staticlib", "lib"]

[dependencies]
{crate_name} = {{ path = "../../../../crates/{core_crate_dir}"{features} }}
extendr-api = "{extendr_api}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::R),
        extendr_api = tv::cargo::EXTENDR_API,
    );

    let r_package_name = config.r_package_name();
    let lib_name = r_package_name.replace('-', "_");

    // Makevars — tells R CMD INSTALL how to build the staticlib and link it.
    let makevars_content = format!(
        "CARGO_BUILD_ARGS = --release\nSTATLIB = ./rust/target/release/lib{lib_name}.a\nPKG_LIBS = -L./rust/target/release -l{lib_name} $(LAPACK_LIBS) $(BLAS_LIBS) $(FLIBS)\n\nall: $(SHLIB)\n\n$(STATLIB):\n\tcargo build --manifest-path ./rust/Cargo.toml $(CARGO_BUILD_ARGS)\n\n$(SHLIB): $(STATLIB)\n\nclean:\n\trm -f $(SHLIB) $(STATLIB)\n\tcargo clean --manifest-path ./rust/Cargo.toml\n",
        lib_name = lib_name,
    );

    // Makevars.in — autoconf variant; same content.
    let makevars_in_content = makevars_content.clone();

    // Makevars.win.in — Windows variant; cargo produces a .lib, not .a.
    let makevars_win_content = format!(
        "CARGO_BUILD_ARGS = --release --target x86_64-pc-windows-gnu\nSTATLIB = ./rust/target/x86_64-pc-windows-gnu/release/{lib_name}.lib\nPKG_LIBS = -L./rust/target/x86_64-pc-windows-gnu/release -l{lib_name} -lws2_32 -ladvapi32 -luserenv -lbcrypt -lntdll\n\nall: $(SHLIB)\n\n$(STATLIB):\n\tcargo build --manifest-path ./rust/Cargo.toml $(CARGO_BUILD_ARGS)\n\n$(SHLIB): $(STATLIB)\n\nclean:\n\trm -f $(SHLIB) $(STATLIB)\n\tcargo clean --manifest-path ./rust/Cargo.toml\n",
        lib_name = lib_name,
    );

    // entrypoint.c — C shim required by extendr.
    let entrypoint_c_content = format!(
        "// Generated entrypoint: calls the extendr-generated R_init function.\n// Do not edit — regenerate with `alef generate`.\n#include <R_ext/Visibility.h>\n\nvoid R_init_{lib_name}(void *dll);\n\nvoid attribute_visible R_init_{lib_name}_impl(void *dll) {{{{\n    R_init_{lib_name}(dll);\n}}}}\n",
        lib_name = lib_name,
    );

    // NAMESPACE — minimal bootstrap so `R CMD check` can find the package.
    let namespace_content = format!(
        "# Generated by alef — do not edit.\nuseDynLib({lib_name}, .registration = TRUE)\n",
        lib_name = lib_name,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/r/src/rust/Cargo.toml"),
            content: cargo_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/r/src/Makevars"),
            content: makevars_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/r/src/Makevars.in"),
            content: makevars_in_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/r/src/Makevars.win.in"),
            content: makevars_win_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/r/src/entrypoint.c"),
            content: entrypoint_c_content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/r/NAMESPACE"),
            content: namespace_content,
            generated_header: false,
        },
    ])
}
