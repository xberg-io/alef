use super::render_enum;
use crate::core::config::Language;
use crate::docs::test_helpers::make_test_config;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

mod fixtures;

const ORIGINAL_RENDERER_SHA256: &str = "5abb7591ae9f85ac58b03666a631aad7064be84867605239ee9f46107ef4fbd3";
const TEST_MODULE_WIRING: &str = "\n#[cfg(test)]\nmod tests;\n";
const FIXTURE_COUNT: usize = 3;
const LANGUAGE_COUNT: usize = 20;
const CASE_COUNT: usize = FIXTURE_COUNT * LANGUAGE_COUNT;
const MANIFEST_NAME: &str = "manifest.json";
const CAPTURE_PREFIX: &str = "alef-quality338-enum-render-capture-";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: usize,
    source_tree: String,
    renderer_sha256: String,
    fixtures_sha256: String,
    harness_sha256: String,
    cases: Vec<CaseRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseRecord {
    name: String,
    bytes: usize,
    sha256: String,
}

struct RenderedCase {
    name: String,
    output: String,
}

#[test]
#[ignore = "explicit original-byte capture; requires a fresh task-local ALEF_ENUM_BASELINE_DIR"]
fn captures_original_enum_bytes_without_overwriting_existing_evidence() {
    assert_original_renderer();
    let source_tree = required_env("ALEF_ENUM_BASELINE_TREE");
    assert_hex_digest(&source_tree, 40);
    let directory = baseline_directory();
    let cases = rendered_cases();
    fs::create_dir(&directory).expect("capture directory must not already exist");
    let records = cases.iter().map(|case| write_case(&directory, case)).collect();
    let manifest = Manifest {
        schema_version: 1,
        source_tree,
        renderer_sha256: ORIGINAL_RENDERER_SHA256.into(),
        fixtures_sha256: digest(include_bytes!("tests/fixtures.rs")),
        harness_sha256: digest(include_bytes!("tests.rs")),
        cases: records,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).expect("serialize capture manifest");
    create_new_file(&directory.join(MANIFEST_NAME), &bytes);
    assert_directory_entries(&directory, &manifest);
    println!("captured_cases={CASE_COUNT} manifest_sha256={}", digest(&bytes));
}

#[test]
#[ignore = "explicit frozen-byte comparison; requires independently pinned ALEF_ENUM_BASELINE_MANIFEST_SHA256"]
fn enum_rendering_matches_independently_pinned_original_bytes() {
    let directory = baseline_directory();
    let expected_manifest = required_env("ALEF_ENUM_BASELINE_MANIFEST_SHA256");
    assert_hex_digest(&expected_manifest, 64);
    let bytes = fs::read(directory.join(MANIFEST_NAME)).expect("read frozen manifest");
    assert_eq!(digest(&bytes), expected_manifest, "frozen manifest changed");
    let manifest: Manifest = serde_json::from_slice(&bytes).expect("parse frozen manifest");
    assert_manifest(&manifest);
    assert_directory_entries(&directory, &manifest);
    let actual = rendered_cases();
    for (case, record) in actual.iter().zip(&manifest.cases) {
        compare_case(&directory, case, record);
    }
    assert_eq!(actual.len(), CASE_COUNT);
    println!("compared_cases={CASE_COUNT}");
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|error| panic!("{name} must be explicitly supplied: {error}"))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn enum_evidence_digests_match_known_lowercase_sha256_values() {
    assert_eq!(
        digest(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

fn assert_hex_digest(value: &str, length: usize) {
    assert_eq!(value.len(), length, "incorrect digest length");
    assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()), "invalid digest");
}

fn assert_original_renderer() {
    let source = include_str!("../enum_render.rs");
    let original = source
        .strip_suffix(TEST_MODULE_WIRING)
        .expect("only test module wiring may be added");
    assert_eq!(digest(original.as_bytes()), ORIGINAL_RENDERER_SHA256);
}

fn baseline_directory() -> PathBuf {
    let directory = PathBuf::from(required_env("ALEF_ENUM_BASELINE_DIR"));
    assert!(directory.is_absolute(), "capture directory must be absolute");
    assert!(
        directory.components().all(|part| !matches!(part, Component::ParentDir)),
        "parent traversal is forbidden"
    );
    let reports = reports_directory(Path::new(env!("CARGO_MANIFEST_DIR")));
    let parent = directory.parent().expect("capture directory has a parent");
    assert_eq!(parent.canonicalize().expect("resolve capture parent"), reports);
    let name = directory
        .file_name()
        .expect("capture name")
        .to_str()
        .expect("UTF-8 capture name");
    assert!(name.starts_with(CAPTURE_PREFIX), "unexpected capture directory name");
    if let Ok(metadata) = fs::symlink_metadata(&directory) {
        assert!(
            !metadata.file_type().is_symlink(),
            "capture directory cannot be a symlink"
        );
    }
    directory
}

fn reports_directory(checkout: &Path) -> PathBuf {
    let checkout = checkout.canonicalize().expect("resolve repository checkout");
    let repository_parent = checkout.parent().expect("checkout has a parent");
    let root = match repository_parent.parent() {
        Some(worktrees) if worktrees.file_name().is_some_and(|name| name == ".worktrees") => {
            worktrees.parent().expect("worktrees have a polyrepo parent")
        }
        _ => repository_parent,
    };
    root.join("reports")
        .canonicalize()
        .expect("resolve task-local reports directory")
}

#[test]
fn enum_evidence_directory_resolves_for_main_and_worktree_checkouts() {
    let directory = tempfile::tempdir().expect("layout fixture");
    let reports = directory.path().join("reports");
    fs::create_dir(&reports).expect("reports directory");
    let expected = reports.canonicalize().expect("canonical reports directory");
    for relative in ["alef", ".worktrees/alef/enum-render"] {
        let checkout = directory.path().join(relative);
        fs::create_dir_all(&checkout).expect("checkout layout");
        assert_eq!(reports_directory(&checkout), expected, "{relative}");
    }
}

fn rendered_cases() -> Vec<RenderedCase> {
    assert_eq!(Language::ALL.len(), LANGUAGE_COUNT);
    let fixtures = fixtures::fixtures();
    assert_eq!(fixtures.len(), FIXTURE_COUNT);
    let config = make_test_config();
    let mut cases = Vec::with_capacity(CASE_COUNT);
    for fixture in fixtures {
        assert_eq!(fixture.api.enums.len(), 1);
        for lang in Language::ALL {
            let name = format!("{}__{lang:?}", fixture.name);
            let output = render_enum(&fixture.api.enums[0], lang, &config, &fixture.api, "Demo");
            assert_fixture_behavior(fixture.name, lang, &output);
            cases.push(RenderedCase { name, output });
        }
    }
    let names: BTreeSet<_> = cases.iter().map(|case| &case.name).collect();
    assert_eq!(names.len(), CASE_COUNT, "missing or duplicate cases");
    assert_eq!(cases.len(), CASE_COUNT);
    cases
}

fn assert_fixture_behavior(fixture: &str, lang: Language, output: &str) {
    assert!(!output.is_empty());
    let table = "| Value | Description |";
    if fixture == "untagged" && lang == Language::Wasm {
        assert!(output.contains("export type WasmEmbeddingInput = string | string[];"));
        assert!(!output.contains(table));
        return;
    }
    assert!(output.contains(table));
    if fixture == "normal" {
        assert_eq!(output.contains("internal_only"), lang == Language::Rust);
        assert!(output.contains("**Since:** `v1.7`"));
        assert!(output.contains("Deprecated since `v2.3`"));
        assert!(output.contains("\\|"), "table pipes must be escaped");
    }
}

fn write_case(directory: &Path, case: &RenderedCase) -> CaseRecord {
    create_new_file(&directory.join(format!("{}.bin", case.name)), case.output.as_bytes());
    CaseRecord {
        name: case.name.clone(),
        bytes: case.output.len(),
        sha256: digest(case.output.as_bytes()),
    }
}

fn create_new_file(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .expect("create fresh evidence file");
    file.write_all(bytes).expect("write complete evidence file");
    file.sync_all().expect("persist evidence file");
}

fn assert_manifest(manifest: &Manifest) {
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.renderer_sha256, ORIGINAL_RENDERER_SHA256);
    assert_eq!(manifest.fixtures_sha256, digest(include_bytes!("tests/fixtures.rs")));
    assert_hex_digest(&manifest.source_tree, 40);
    assert_hex_digest(&manifest.harness_sha256, 64);
    assert_eq!(manifest.cases.len(), CASE_COUNT);
}

fn assert_directory_entries(directory: &Path, manifest: &Manifest) {
    let mut expected: BTreeSet<_> = manifest.cases.iter().map(|case| format!("{}.bin", case.name)).collect();
    assert_eq!(expected.len(), CASE_COUNT);
    expected.insert(MANIFEST_NAME.into());
    let actual: BTreeSet<_> = fs::read_dir(directory)
        .expect("read capture directory")
        .map(|entry| {
            entry
                .expect("read capture entry")
                .file_name()
                .into_string()
                .expect("UTF-8 case name")
        })
        .collect();
    assert_eq!(actual, expected, "capture has missing or unexpected entries");
}

fn compare_case(directory: &Path, actual: &RenderedCase, record: &CaseRecord) {
    assert_eq!(actual.name, record.name, "case order or membership changed");
    assert!(record.bytes > 0);
    let path = directory.join(format!("{}.bin", actual.name));
    assert!(fs::symlink_metadata(&path).expect("baseline file metadata").is_file());
    let expected = fs::read(path).expect("read frozen original bytes");
    assert_eq!(expected.len(), record.bytes, "{} baseline size", actual.name);
    assert_eq!(digest(&expected), record.sha256, "{} baseline hash", actual.name);
    assert_eq!(
        actual.output.as_bytes(),
        expected,
        "{} output bytes changed",
        actual.name
    );
}
