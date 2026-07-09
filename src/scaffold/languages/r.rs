use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::core::version::to_r_version;
use crate::{scaffold::cargo_package_header, scaffold::render_extra_deps, scaffold::scaffold_meta};
use std::path::PathBuf;

pub(crate) fn scaffold_r(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    // R / CRAN rejects SemVer dash-form prereleases; convert to the four-component form.
    let version = to_r_version(&api.version);
    let package_name = config.r_package_name();

    let mut description = meta.description.clone();
    if description.ends_with('.') {
        description.pop();
    }

    let authors_r = if meta.authors.is_empty() {
        anyhow::bail!("R scaffold requires package metadata authors; set package_metadata.authors or scaffold.authors");
    } else if let Some((given, family, email)) = parse_r_author(meta.authors.first().unwrap_or(&String::new())) {
        format!("Authors@R: person(\"{given}\", \"{family}\", email = \"{email}\", role = c(\"aut\", \"cre\"))")
    } else {
        format!(
            "Authors@R: person(\"{}\", role = c(\"aut\", \"cre\"))",
            meta.authors.first().unwrap_or(&"Author Name".to_string())
        )
    };
    let repository_lines = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("URL: {repository}\nBugReports: {repository}/issues"))
        .unwrap_or_default();
    let license = meta.license.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "R scaffold requires package metadata license; set package_metadata.license or scaffold.license"
        )
    })?;

    let content = format!(
        r#"Package: {package}
Title: {title}
Version: {version}
{authors}
Description: {description}
    Rust bindings generated with extendr.
{repository_lines}
License: {license}
Depends: R (>= 4.2)
Imports: jsonlite
Suggests:
    testthat (>= 3.0.0),
    withr,
    roxygen2
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
        repository_lines = repository_lines,
        license = license,
        rextendr = tv::cran::REXTENDR,
    );

    // R linting (jarl) and formatting (air) are poly-native — no `.lintr` is
    // emitted; configuration lives in the repo-root poly.toml.
    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/r/DESCRIPTION"),
        content,
        generated_header: true,
    }])
}

fn parse_r_author(author: &str) -> Option<(String, String, String)> {
    let (name, email) = author.rsplit_once('<')?;
    let email = email.strip_suffix('>')?.trim();
    if email.is_empty() {
        return None;
    }
    let name = name.trim();
    let (given, family) = name.rsplit_once(' ')?;
    if given.is_empty() || family.is_empty() {
        return None;
    }
    Some((given.to_string(), family.to_string(), email.to_string()))
}

pub(crate) fn scaffold_r_cargo(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    // The R crate at `packages/r/src/rust/Cargo.toml` is EXCLUDED from the workspace
    // (it has its own Cargo.lock and is not a workspace member), so it must NOT use
    // workspace inheritance for package fields. Always use concrete values instead.
    let ws = crate::scaffold::WorkspacePackageInheritance::default();
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-r"), version, "2024", &meta, &ws);

    // extendr requires staticlib (for R's dyn.load) + lib (for Rust tests).
    // "cdylib" alone causes linker failures on macOS/Linux.
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    // Trait-bridge impls emitted under any `[[crates.trait_bridges]]` carry
    // `#[async_trait::async_trait]` on their methods — declare the crate here so
    // the macro resolves at compile time (other backends like PHP, Ruby already
    // declare it unconditionally for the same reason).
    let has_trait_bridges = !config.trait_bridges.is_empty();

    // Collect all [dependencies] entries then sort alphabetically so the emitted
    // Cargo.toml is cargo-sort canonical without a post-processing step.
    let configured_features = config.features_for_language(Language::R);
    let core_default_features = config.r.as_ref().and_then(|r| r.default_features).unwrap_or(true);
    let features_str = if configured_features.is_empty() {
        String::new()
    } else if core_default_features {
        let quoted: Vec<String> = configured_features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    } else {
        let quoted: Vec<String> = configured_features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", default-features = false, features = [{}]", quoted.join(", "))
    };
    let mut dep_lines: Vec<String> = vec![
        crate::scaffold::render_core_dep(
            &config.name,
            &format!("../../../../crates/{core_crate_dir}"),
            &features_str,
            version,
        ),
        format!("extendr-api = \"{}\"", tv::cargo::EXTENDR_API),
        "serde = { version = \"1\", features = [\"derive\"] }".to_owned(),
        "serde_json = \"1\"".to_owned(),
    ];
    if has_async {
        dep_lines.push("tokio = { version = \"1\", features = [\"rt-multi-thread\"] }".to_owned());
    }
    if has_trait_bridges {
        dep_lines.push("async-trait = \"0.1\"".to_owned());
    }
    dep_lines.extend(render_extra_deps(config, Language::R).lines().map(ToOwned::to_owned));
    dep_lines.sort();
    let deps_section = dep_lines.join("\n");

    // Collect every feature name referenced by a `#[cfg(feature = "X")]` attribute
    // on any type, field, enum variant, or top-level function the extendr backend
    // emits into the R crate's `lib.rs`, and emit a forwarding `[features]` table so
    // the binding crate re-exports each one to the core dep. Without this, the
    // `#[cfg(feature = "X")]` gates produce `error: unexpected cfg condition value: X`
    // under `RUSTFLAGS=-D warnings` because the R crate's own Cargo.toml does not
    // declare that feature (it only enables it on the core dependency). Mirrors the
    // dart/swift backend behaviour.
    let cfg_features = crate::codegen::cfg::collect_cfg_features(api);
    let features_block = if cfg_features.is_empty() {
        String::new()
    } else {
        let mut lines: Vec<String> = Vec::with_capacity(cfg_features.len() + 1);
        if configured_features.is_empty() || core_default_features {
            // Enable every cfg-forwarded feature by default when the consumer did not
            // configure a no-default curated feature set, preserving the historical native R build.
            let default_list: Vec<String> = cfg_features.iter().map(|name| format!("\"{name}\"")).collect();
            lines.push(format!("default = [{}]", default_list.join(", ")));
        }
        for name in &cfg_features {
            lines.push(format!(
                r#"{name} = ["{core_dep_key}/{name}"]"#,
                core_dep_key = config.name
            ));
        }
        format!("\n[features]\n{}\n", lines.join("\n"))
    };

    // async-trait is declared so trait-bridge async impl macros resolve at compile
    // time, but the extendr shim itself never `use`s it, so cargo-machete flags it
    // as unused at the leaf-crate level. Ignore it when emitted. cargo-sort places
    // `[package.metadata.*]` immediately after `[package]`, before `[lib]`.
    let machete_block = if has_trait_bridges {
        "[package.metadata.cargo-machete]\nignored = [\"async-trait\"]\n\n".to_string()
    } else {
        String::new()
    };

    let cargo_content = format!(
        r#"{pkg_header}

{machete_block}[lib]
crate-type = ["staticlib", "lib"]

[dependencies]
{deps_section}
{features_block}"#,
        pkg_header = pkg_header,
        machete_block = machete_block,
        deps_section = deps_section,
        features_block = features_block,
    );

    let r_package_name = config.r_package_name();
    let lib_name = r_package_name.replace('-', "_");
    // The Rust crate emits its staticlib as `lib{crate_name}.a`, where crate_name
    // is `{core_crate_dir}-r` (with hyphens replaced by underscores by cargo).
    // This is independent of the R package's user-facing name.
    let rust_lib_name = format!("{}_r", core_crate_dir).replace('-', "_");

    // Makevars — tells R CMD INSTALL how to build the staticlib and link it.
    // Includes a binary-package target for CI: `make binary` produces {pkg_name}_{version}_{platform}.tgz
    //
    // Downstream consumers may inject extra link flags via
    // `[crates.r] extra_makevars_prelude` and `extra_pkg_libs` — used when the
    // staticlib transitively dlopens a system library (e.g. libheif) that R CMD
    // INSTALL must also link against.
    let prelude = config
        .r
        .as_ref()
        .map(|r| r.extra_makevars_prelude.clone())
        .unwrap_or_default();
    let extra_libs = config.r.as_ref().map(|r| r.extra_pkg_libs.clone()).unwrap_or_default();
    let prelude_block = if prelude.is_empty() {
        String::new()
    } else {
        format!("{}\n", prelude.join("\n"))
    };
    let extra_libs_str = if extra_libs.is_empty() {
        String::new()
    } else {
        format!(" {}", extra_libs.join(" "))
    };
    let makevars_content = format!(
        "CARGO_BUILD_ARGS = --release\nSTATLIB = ./rust/target/release/lib{rust_lib_name}.a\n{prelude_block}PKG_LIBS = -L./rust/target/release -l{rust_lib_name}{extra_libs_str} $(LAPACK_LIBS) $(BLAS_LIBS) $(FLIBS)\n\nall: $(SHLIB)\n\n$(STATLIB):\n\tcargo build --manifest-path ./rust/Cargo.toml $(CARGO_BUILD_ARGS)\n\n$(SHLIB): $(STATLIB)\n\nbinary:\n\tcd .. && R CMD INSTALL --build .\n\nclean:\n\trm -f $(SHLIB) $(STATLIB)\n\tcargo clean --manifest-path ./rust/Cargo.toml\n",
        rust_lib_name = rust_lib_name,
        prelude_block = prelude_block,
        extra_libs_str = extra_libs_str,
    );

    // Makevars.in — autoconf variant; same content.
    let makevars_in_content = makevars_content.clone();

    // Makevars.win.in — Windows variant; cargo produces a .lib, not .a.
    // Includes a binary-package target for CI: `make binary` produces {pkg_name}_{version}_{platform}.tgz
    let makevars_win_content = format!(
        "CARGO_BUILD_ARGS = --release --target x86_64-pc-windows-gnu\nSTATLIB = ./rust/target/x86_64-pc-windows-gnu/release/{rust_lib_name}.lib\nPKG_LIBS = -L./rust/target/x86_64-pc-windows-gnu/release -l{rust_lib_name} -lws2_32 -ladvapi32 -luserenv -lbcrypt -lntdll\n\nall: $(SHLIB)\n\n$(STATLIB):\n\tcargo build --manifest-path ./rust/Cargo.toml $(CARGO_BUILD_ARGS)\n\n$(SHLIB): $(STATLIB)\n\nbinary:\n\tcd .. && R CMD INSTALL --build .\n\nclean:\n\trm -f $(SHLIB) $(STATLIB)\n\tcargo clean --manifest-path ./rust/Cargo.toml\n",
        rust_lib_name = rust_lib_name,
    );

    // entrypoint.c — C shim required by extendr. R looks up `R_init_{pkg}` on
    // package load. The extendr macro emits `R_init_{pkg}_extendr` (the trailing
    // `_extendr` is hardcoded by the extendr-macros crate), so this shim
    // forwards the call.
    let entrypoint_c_content = format!(
        "// Generated entrypoint: forwards to the extendr-generated init function.\n// Do not edit — regenerate with `alef generate`.\n#include <R_ext/Visibility.h>\n\nvoid R_init_{lib_name}_extendr(void *dll);\n\nvoid attribute_visible R_init_{lib_name}(void *dll) {{\n    R_init_{lib_name}_extendr(dll);\n}}\n",
        lib_name = lib_name,
    );

    // NAMESPACE is intentionally NOT scaffolded here. The extendr backend's
    // `generate_public_api` owns it and emits the full directive set
    // (`useDynLib` + every `export(...)` / `S3method(...)` entry). Scaffolding
    // ran after generate, so a stub here would clobber the real NAMESPACE with
    // a useDynLib-only file, leaving the package with no exported symbols.
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
    ])
}
