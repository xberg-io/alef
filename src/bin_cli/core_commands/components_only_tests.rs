//! A crate that declares components and no binding languages owns only its component outputs.
//!
//! `alef generate` used to treat it like a bindings project: it stamped its version onto sibling
//! workspace members, relocked `Cargo.lock`, seeded `rust-toolchain.toml` and the scaffold files,
//! and `alef verify` then demanded docs reference pages and frozen scaffold files that were never
//! meant to exist, so it could not pass. Every test runs the real `generate`/`verify` commands
//! against a fixture workspace, and each negative assertion has a positive twin (the producer
//! crate exists) so a run that wrote nothing at all cannot satisfy them.

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::{CwdGuard, git_commit_all, git_init, write_file};
use std::path::{Path, PathBuf};

const VENDORED_MANIFEST: &str = "[package]\nname = \"vendored\"\nversion = \"0.8.0\"\nedition = \"2024\"\n\n[dependencies]\ndemo = { path = \"../demo\", version = \"1.0.0\" }\n";
const CARGO_LOCK: &str = "# sentinel: alef must not relock this\nversion = 4\n";

fn alef_toml(producer_dir: Option<&str>) -> String {
    let producer_dir = producer_dir.map_or_else(String::new, |dir| format!("producer_dir = \"{dir}\"\n"));
    format!(
        r#"[workspace]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]
version_from = "crates/demo/Cargo.toml"

[[crates.component_contracts]]
name = "extractor"
trait_path = "demo::Extractor"
interface_version = 1

[[crates.components]]
name = "pdf"
provides = [{{ contract = "extractor", implementation = "demo::PdfExtractor" }}]
features = ["pdf"]
default_features = false
targets = ["x86_64-unknown-linux-gnu"]
{producer_dir}"#
    )
}

const DEMO_SOURCE: &str = "pub struct Error;\n\npub trait Extractor {\n    fn extract(&self, input: &[u8]) -> Result<Vec<u8>, Error>;\n}\n\npub struct PdfExtractor;\n\nimpl Extractor for PdfExtractor {\n    fn extract(&self, input: &[u8]) -> Result<Vec<u8>, Error> {\n        Ok(input.to_vec())\n    }\n}\n";

fn context(root: &Path) -> DispatchContext {
    DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    }
}

fn generate(root: &Path) {
    super::handle(
        Commands::Generate {
            lang: None,
            clean: false,
            skip_frb: false,
            strict: false,
            skip_compile: true,
        },
        &context(root),
    )
    .expect("alef generate must succeed against a components-only fixture");
}

fn verify(root: &Path) -> anyhow::Result<()> {
    super::handle(
        Commands::Verify {
            exit_code: false,
            report_only: false,
            compile: false,
            lint: false,
            lang: None,
        },
        &context(root),
    )
    .map(|_| ())
}

/// A committed components-only workspace with an unrelated member that carries its own version,
/// already generated once. Returns the temp dir (kept alive by the caller) and its canonical root.
fn generated_fixture(producer_dir: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    git_init(&root);
    let relocated_member = producer_dir.map_or_else(String::new, |dir| format!(", \"{dir}\""));
    write_file(
        &root,
        "Cargo.toml",
        &format!("[workspace]\nresolver = \"3\"\nmembers = [\"crates/demo\", \"crates/vendored\"{relocated_member}]\n"),
    );
    write_file(
        &root,
        "crates/demo/Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"1.2.3\"\nedition = \"2024\"\n\n[features]\npdf = []\n",
    );
    write_file(&root, "crates/demo/src/lib.rs", DEMO_SOURCE);
    write_file(&root, "crates/vendored/Cargo.toml", VENDORED_MANIFEST);
    write_file(&root, "crates/vendored/src/lib.rs", "");
    write_file(&root, "Cargo.lock", CARGO_LOCK);
    write_file(&root, "alef.toml", &alef_toml(producer_dir));
    git_commit_all(&root, "fixture");

    {
        let _cwd = CwdGuard::enter(&root);
        generate(&root);
    }
    git_commit_all(&root, "generated");
    (dir, root)
}

fn producer_lib(root: &Path, producer_dir: &str) -> PathBuf {
    root.join(producer_dir).join("src/lib.rs")
}

/// Asserts `verify` rejected a hand-edited producer for the right reason: the failure is the
/// out-of-date verdict, and the stale-file list `verify` prints names the producer path with a
/// hash that no longer matches its content.
fn assert_reports_producer_drift(error: &anyhow::Error, root: &Path, producer_dir: &str) {
    assert!(
        error.to_string().contains("out of date"),
        "verify must fail with the out-of-date verdict, got: {error}"
    );
    let expected = format!("{producer_dir}/src/lib.rs");
    let stale = crate::bin_cli::helpers::verify_walk(root).expect("walk generated files");
    let entry = stale
        .iter()
        .find(|mismatch| mismatch.path.replace('\\', "/").ends_with(&expected))
        .unwrap_or_else(|| {
            panic!(
                "{expected} must be reported stale, got {:?}",
                stale.iter().map(|m| &m.path).collect::<Vec<_>>()
            )
        });
    assert_ne!(
        entry.embedded, entry.computed,
        "a stale entry means the embedded hash no longer matches the content"
    );
}

#[test]
fn generate_writes_only_the_producer_and_leaves_unowned_files_untouched() {
    let (_dir, root) = generated_fixture(None);

    assert!(
        root.join("crates/demo-pdf-component/Cargo.toml").is_file(),
        "the producer crate must be generated, or the absence checks below prove nothing"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("crates/vendored/Cargo.toml")).unwrap(),
        VENDORED_MANIFEST,
        "a workspace member with its own version must not be rewritten"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("Cargo.lock")).unwrap(),
        CARGO_LOCK,
        "Cargo.lock must not be relocked"
    );
    for absent in [
        "rust-toolchain.toml",
        ".gitattributes",
        "poly.toml",
        "rustfmt.toml",
        "docs/reference",
    ] {
        assert!(
            !root.join(absent).exists(),
            "{absent} must not be seeded for a components-only crate"
        );
    }
}

#[test]
fn verify_passes_after_a_fresh_generate() {
    let (_dir, root) = generated_fixture(None);
    let _cwd = CwdGuard::enter(&root);

    verify(&root).expect("a fresh components-only generate must verify clean");
}

#[test]
fn verify_fails_after_editing_a_generated_producer_file() {
    let (_dir, root) = generated_fixture(None);
    let lib = producer_lib(&root, "crates/demo-pdf-component");
    let original = std::fs::read_to_string(&lib).expect("producer lib.rs exists");
    std::fs::write(&lib, format!("{original}\n// hand edit\n")).unwrap();
    let _cwd = CwdGuard::enter(&root);

    let error = verify(&root).expect_err("a hand-edited producer file must fail verification");
    assert_reports_producer_drift(&error, &root, "crates/demo-pdf-component");
}

#[test]
fn verify_covers_a_producer_relocated_with_producer_dir() {
    let (_dir, root) = generated_fixture(Some("producers/pdf"));
    let lib = producer_lib(&root, "producers/pdf");
    assert!(lib.is_file(), "the producer must be generated into producer_dir");
    {
        let _cwd = CwdGuard::enter(&root);
        verify(&root).expect("a relocated producer must verify clean after generate");
    }

    let original = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(&lib, format!("{original}\n// hand edit\n")).unwrap();
    let _cwd = CwdGuard::enter(&root);

    let error = verify(&root).expect_err("a hand-edited relocated producer must fail verification");
    assert_reports_producer_drift(&error, &root, "producers/pdf");
}

#[test]
fn generate_removes_marked_former_scaffold_files_and_keeps_unmarked_ones() {
    let (_dir, root) = generated_fixture(None);
    write_file(
        &root,
        "rust-toolchain.toml",
        "# generated by alef - do not edit\n[toolchain]\nchannel = \"stable\"\n",
    );
    write_file(&root, "rustfmt.toml", "max_width = 100\n");
    write_file(
        &root,
        "docs/reference/api-rust.md",
        "<!-- generated by alef - do not edit -->\n# API\n",
    );
    write_file(&root, "docs/reference/notes.md", "# hand written\n");
    {
        let _cwd = CwdGuard::enter(&root);
        generate(&root);
    }
    git_commit_all(&root, "regenerated");

    assert!(
        !root.join("rust-toolchain.toml").exists(),
        "a marked former scaffold file must be removed"
    );
    assert!(
        !root.join("docs/reference/api-rust.md").exists(),
        "a marked former docs page must be removed"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("rustfmt.toml")).unwrap(),
        "max_width = 100\n",
        "an unmarked file is user-owned and must be kept"
    );
    assert!(root.join("docs/reference/notes.md").is_file());
    assert!(root.join("crates/demo-pdf-component/Cargo.toml").is_file());

    let _cwd = CwdGuard::enter(&root);
    verify(&root).expect("verify must pass once the formerly owned files are gone");
}

fn generate_selected_components(context: &DispatchContext, all: bool) {
    if all {
        crate::bin_cli::all_commands::handle(
            Commands::All {
                clean: false,
                clobber_create_once_seeds: false,
                strict: false,
                skip_frb: false,
                skip_snippet_validation: true,
                skip_compile: true,
            },
            context,
        )
        .expect("alef all must generate the selected components-only crate");
    } else {
        super::handle(
            Commands::Generate {
                lang: None,
                clean: false,
                skip_frb: false,
                strict: false,
                skip_compile: true,
            },
            context,
        )
        .expect("alef generate must generate the selected components-only crate");
    }
}

fn assert_selected_components_preserve_sibling_outputs(all: bool) {
    let (_dir, root) = generated_fixture(None);
    let config = format!(
        "{}\n[[crates]]\nname = \"vendored\"\nlanguages = [\"python\"]\n\
         sources = [\"crates/vendored/src/lib.rs\"]\nversion_from = \"crates/vendored/Cargo.toml\"\n",
        alef_toml(None)
    );
    write_file(&root, "alef.toml", &config);
    let sibling_files = [
        (
            "rust-toolchain.toml",
            "# generated by alef - do not edit\n[toolchain]\nchannel = \"stable\"\n",
        ),
        (
            "docs/reference/api-python.md",
            "<!-- generated by alef - do not edit -->\n<span class=\"version-badge\">v0.8.0</span>\n",
        ),
        ("CITATION.cff", "cff-version: 1.2.0\nversion: 0.8.0\n"),
        (
            "uv.lock",
            "version = 1\n[[package]]\nname = \"vendored\"\nversion = \"0.7.0\"\nsource = { virtual = \".\" }\n",
        ),
        (
            "pyproject.toml",
            "[project]\nname = \"vendored\"\nversion = \"0.8.0\"\n",
        ),
    ];
    for (path, content) in sibling_files {
        write_file(&root, path, content);
    }
    git_commit_all(&root, "add bindings sibling and shared files");

    let producer = producer_lib(&root, "crates/demo-pdf-component");
    std::fs::remove_file(&producer).expect("remove the test producer to prove generation ran");
    let _cwd = CwdGuard::enter(&root);
    let context = DispatchContext {
        crate_filter: vec!["demo".into()],
        ..context(&root)
    };
    generate_selected_components(&context, all);

    assert!(producer.is_file(), "the selected producer must be regenerated");
    for (path, content) in sibling_files {
        assert_eq!(
            std::fs::read_to_string(root.join(path)).as_deref().ok(),
            Some(content),
            "the selected components-only crate must preserve {path}"
        );
    }
}

#[test]
fn generate_preserves_shared_files_of_an_unselected_bindings_crate() {
    assert_selected_components_preserve_sibling_outputs(false);
}

#[test]
fn all_preserves_shared_files_of_an_unselected_bindings_crate() {
    assert_selected_components_preserve_sibling_outputs(true);
}

#[test]
fn generate_never_reformats_files_it_does_not_own() {
    let (_dir, root) = generated_fixture(None);
    let changelog = "# Changelog\n\n\n\n## 1.0.0   \n- entry\n\n\n";
    let stray = "pub fn   stray( ) {  }\n";
    write_file(&root, "CHANGELOG.md", changelog);
    write_file(&root, "crates/vendored/src/stray.rs", stray);
    // Removing a generated file forces a write this run, so the format pass is reached.
    let producer = producer_lib(&root, "crates/demo-pdf-component");
    std::fs::remove_file(&producer).unwrap();
    {
        let _cwd = CwdGuard::enter(&root);
        generate(&root);
    }

    assert!(
        producer.is_file(),
        "the producer must be rewritten, or the format pass was never reached"
    );
    assert_eq!(std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap(), changelog);
    assert_eq!(
        std::fs::read_to_string(root.join("crates/vendored/src/stray.rs")).unwrap(),
        stray
    );
}
