//! The set of directories a partial regen formats must match the set it stamps.
//!
//! [`poly_paths`]'s `Some(_)` branch used to return `package_dir(lang)` alone, while the
//! stamping side (`current_gen_paths` -> `finalize_hashes`) and the orphan sweep
//! (`generate_sweep_roots`) both span `package_dir(lang)` *and* `output_for(lang)`. For most
//! languages those two coincide or nest, so the gap was invisible. For Python they do not:
//! `package_dir(Python)` is `packages/python` (the wheel) while the PyO3 glue crate lives at
//! `crates/<name>-py/src` -- so `alef generate --lang python` stamped a Rust file it had never
//! formatted, and the next whole-tree format pass immediately made that stamp stale.
//!
//! `output_for(lang)` alone was still not enough: it names the glue crate's `src` directory, one
//! level below the crate root that holds the glue crate's own `Cargo.toml`. Neither
//! `package_dir` nor `output_for` names that root for languages whose default output template is
//! `<crate-root>/src` (python, ffi, php), so a partial regen never formatted the generated
//! manifest -- it shipped non-canonical from generation, and `alef verify` only caught the drift
//! the first time something else reformatted the file. See
//! `a_partial_python_regen_formats_the_glue_crate_root_holding_cargo_toml`. ~keep

use super::scope::poly_paths;
use super::*;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};

fn config_for(languages: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = [{languages}]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid config");
    cfg.resolve().unwrap().remove(0)
}

/// The measured divergence this test exists for. Asserting it directly means the regression
/// below cannot quietly turn into a tautology if `package_dir(Python)` is ever changed to
/// point at the crate: the moment the two agree, this fails and says so. ~keep
#[test]
fn python_package_dir_and_output_dir_are_genuinely_different_trees() {
    let config = config_for(r#""python""#);
    assert_eq!(config.package_dir(Language::Python), "packages/python");
    assert_eq!(
        config.output_for("python"),
        Some(Path::new("crates/sample-model-py/src")),
        "the PyO3 glue crate is generated outside the wheel package directory"
    );
}

/// A partial regen must format the generated Rust glue crate, not just the wheel package.
///
/// Asserts the glue crate *root* rather than `output_for`'s `src` subdirectory: the root also
/// holds the glue crate's `Cargo.toml` (see
/// `a_partial_python_regen_formats_the_glue_crate_root_holding_cargo_toml`), and
/// `collapse_nested_paths` folds the now-redundant `src` entry into the enclosing root it sits
/// inside, exactly as it already does for node/wasm/ffi in
/// `a_nested_output_dir_does_not_add_a_redundant_poly_path`.
#[test]
fn a_partial_python_regen_formats_the_generated_glue_crate_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = config_for(r#""python""#);
    let wheel = base.join(config.package_dir(Language::Python));
    let glue = base.join(config.output_for("python").expect("python output dir"));
    let glue_root = glue.parent().expect("glue src dir has a parent").to_path_buf();
    std::fs::create_dir_all(&wheel).expect("wheel dir");
    std::fs::create_dir_all(&glue).expect("glue crate src dir");

    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);

    assert!(
        paths.contains(&wheel),
        "the wheel package directory must still be formatted: {paths:?}"
    );
    assert!(
        paths.contains(&glue_root),
        "the generated PyO3 glue crate is stamped by `finalize_hashes` and swept by \
         `generate_sweep_roots`, so it must be formatted by the same run that writes it -- \
         otherwise generate emits non-canonical Rust and stamps it, and the next whole-tree \
         format pass makes the stamp stale: {paths:?}"
    );
}

/// `output_for` is nested inside `package_dir` for the languages whose binding crate *is* the
/// package (node, wasm, ffi). Handing poly both would make it walk the same subtree twice, so
/// the nested entry must be dropped -- while the enclosing directory, which is what actually
/// covers the crate manifest as well as its sources, is kept.
#[test]
fn a_nested_output_dir_does_not_add_a_redundant_poly_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = config_for(r#""node""#);
    let crate_root = base.join(config.package_dir(Language::Node));
    let sources = base.join(config.output_for("node").expect("node output dir"));
    std::fs::create_dir_all(&sources).expect("node src dir");

    let only: HashSet<Language> = [Language::Node].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Node]);

    assert_eq!(
        paths,
        vec![crate_root],
        "the crate root already contains its own src/, so only the enclosing path is handed to poly"
    );
}

/// The Python binding crate's own manifest -- `crates/<name>-py/Cargo.toml` -- lives in the
/// glue crate's root, one directory above `output_for("python")` (`crates/<name>-py/src`).
/// Neither `package_dir` (the wheel at `packages/python`) nor `output_for` (the src dir) names
/// that root, so a partial Python regen never handed it to poly: the manifest shipped
/// non-canonical straight out of generation, and `alef verify` only caught it the first time
/// something else reformatted the file.
#[test]
fn a_partial_python_regen_formats_the_glue_crate_root_holding_cargo_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = config_for(r#""python""#);
    let glue_src = base.join(config.output_for("python").expect("python output dir"));
    std::fs::create_dir_all(&glue_src).expect("glue crate src dir");
    let crate_root = glue_src.parent().expect("glue src dir has a parent").to_path_buf();
    std::fs::write(crate_root.join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);

    assert!(
        paths.contains(&crate_root),
        "the glue crate root holding Cargo.toml must be formatted, not just its src/ subdirectory: {paths:?}"
    );
}

/// The same crate-root gap reproduced for every other language whose default output template
/// is `<crate-root>/src` (see `default_binding_crate_root`): FFI and PHP, alongside Python.
/// Table-driven so a future language added to that same default-template list is covered by
/// construction rather than requiring a new hand-written case. ~keep
#[test]
fn every_default_crate_root_language_formats_its_manifest_directory() {
    for (languages, lang, lang_name) in [(r#""ffi""#, Language::Ffi, "ffi"), (r#""php""#, Language::Php, "php")] {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let config = config_for(languages);
        let output_dir = base.join(config.output_for(lang_name).expect("output dir"));
        std::fs::create_dir_all(&output_dir).expect("glue crate src dir");
        let crate_root = output_dir.parent().expect("output dir has a parent").to_path_buf();
        std::fs::write(crate_root.join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

        let only: HashSet<Language> = [lang].into_iter().collect();
        let paths = poly_paths(&config, base, Some(&only), &[lang]);

        assert!(
            paths.contains(&crate_root),
            "[{lang_name}] the manifest directory {crate_root:?} must be formatted: {paths:?}"
        );
    }
}

/// A language whose output directory does not exist yet must still be dropped, exactly as
/// before -- the new entry must not resurrect the "poly formats a path that isn't there" case.
#[test]
fn a_missing_output_dir_is_still_dropped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = config_for(r#""python""#);
    std::fs::create_dir_all(base.join(config.package_dir(Language::Python))).expect("wheel dir");

    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);

    assert_eq!(
        paths,
        vec![base.join("packages/python")],
        "an output directory this run did not create must not be handed to poly"
    );
}
