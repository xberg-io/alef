//! A consumer's dependency bump has to survive a regeneration.
//!
//! Every manifest alef emits with `generated_header: true` is rewritten in full on each run
//! from literals baked into the emitter, so a hand-edited version in one of them is discarded
//! the next time `alef all` runs -- silently, and not until someone else regenerates. The
//! durable place for that bump is `[crates.<lang>.dependency_versions]`, and these tests are
//! what prove the table actually reaches each emitted file.
//!
//! Each case renders the same fixture twice, with and without a sentinel override, and asserts
//! both halves: the sentinel is present, **and** the line the default produced is gone.
//! Asserting only presence passes when both are emitted, which is a broken manifest that reads
//! as a working override.

use alef::core::config::NewAlefConfig;
use alef::{ApiSurface, Language, ResolvedCrateConfig};

const SCAFFOLDED_LANGUAGES: [Language; 5] = [
    Language::Python,
    Language::Java,
    Language::Elixir,
    Language::Ruby,
    Language::R,
];

/// Neutral fixture: no real consumer project, package or domain type appears anywhere here
/// (`tests/cli_no_project_special_casing.rs` enforces that over all of `tests/`).
const BASE_CONFIG: &str = r#"
[workspace]
languages = ["python", "java", "elixir", "ruby", "r"]

[workspace.package_metadata]
description = "Sample fixture crate"
license = "MIT"
repository = "https://example.invalid/sample-org/sample-lib"
authors = ["Sample Author <author@example.invalid>"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.java]
group_id = "invalid.example.sample"
"#;

/// Deliberately unmistakable: no registry default is, or will be bumped to, any of these.
const SENTINEL_PLAIN: &str = "99.88.77-alef-sentinel";
const SENTINEL_PEP440: &str = ">=99.88.77";
const SENTINEL_HEX: &str = "~> 99.88";
const SENTINEL_GEM: &str = "~> 99.88";

fn render(overrides_table: &str) -> Vec<alef::core::GeneratedFile> {
    let source = format!("{BASE_CONFIG}{overrides_table}");
    let parsed: NewAlefConfig = toml::from_str(&source).expect("fixture config parses");
    let resolved: ResolvedCrateConfig = parsed.resolve().expect("fixture config resolves").remove(0);
    alef::core::config::validation::validate_resolved(&resolved).expect("fixture config validates");
    alef::scaffold::scaffold(&ApiSurface::default(), &resolved, &SCAFFOLDED_LANGUAGES).expect("scaffold succeeds")
}

/// The content of the single scaffolded file whose path ends in `suffix`.
fn manifest(files: &[alef::core::GeneratedFile], suffix: &str) -> String {
    let matches: Vec<&alef::core::GeneratedFile> = files
        .iter()
        .filter(|file| file.path.to_string_lossy().ends_with(suffix))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one scaffolded file ending in `{suffix}`, got: {:?}",
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
    );
    matches[0].content.clone()
}

/// The one line in `content` mentioning `marker`.
fn line_with<'a>(content: &'a str, marker: &str) -> &'a str {
    let matches: Vec<&str> = content.lines().filter(|line| line.contains(marker)).collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one line mentioning `{marker}`, got {matches:?} in:\n{content}"
    );
    matches[0]
}

/// Both halves of the contract, for one dependency in one manifest.
///
/// The anti-vacuity guard is the third assertion: without it, a renderer that emitted the
/// sentinel *in addition to* the default -- a manifest that installs the wrong version and
/// may not even parse -- would pass on presence alone.
fn assert_override_replaces_default(baseline: &str, overridden: &str, marker: &str, sentinel: &str) {
    let default_line = line_with(baseline, marker);
    let overridden_line = line_with(overridden, marker);

    assert!(
        overridden_line.contains(sentinel),
        "the override must reach the emitted manifest; `{marker}` line was: {overridden_line}"
    );
    assert_ne!(
        default_line, overridden_line,
        "the `{marker}` line must change when the override is set"
    );
    assert!(
        !overridden.contains(default_line),
        "the default line must be gone, not emitted alongside the override.\n  default:   \
         {default_line}\n  overridden manifest:\n{overridden}"
    );
}

#[test]
fn pyproject_toml_honours_a_pypi_override() {
    let baseline = manifest(&render(""), "packages/python/pyproject.toml");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.python.dependency_versions]\n\"pypi:pyrefly\" = \"{SENTINEL_PEP440}\"\n"
        )),
        "packages/python/pyproject.toml",
    );

    assert_override_replaces_default(&baseline, &overridden, "\"pyrefly", SENTINEL_PEP440);
}

#[test]
fn the_python_binding_manifest_honours_a_cargo_override() {
    let baseline = manifest(&render(""), "-py/Cargo.toml");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.python.dependency_versions]\n\"cargo:pyo3\" = \"{SENTINEL_PLAIN}\"\n"
        )),
        "-py/Cargo.toml",
    );

    assert_override_replaces_default(&baseline, &overridden, "pyo3 = ", SENTINEL_PLAIN);
}

#[test]
fn pom_xml_honours_a_maven_override() {
    let baseline = manifest(&render(""), "packages/java/pom.xml");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.java.dependency_versions]\n\"maven:junit\" = \"{SENTINEL_PLAIN}\"\n"
        )),
        "packages/java/pom.xml",
    );

    assert_override_replaces_default(&baseline, &overridden, "<junit.version>", SENTINEL_PLAIN);
}

#[test]
fn mix_exs_honours_a_hex_override() {
    let baseline = manifest(&render(""), "mix.exs");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.elixir.dependency_versions]\n\"hex:credo\" = \"{SENTINEL_HEX}\"\n"
        )),
        "mix.exs",
    );

    assert_override_replaces_default(&baseline, &overridden, ":credo", SENTINEL_HEX);
}

#[test]
fn the_gemspec_honours_a_gem_override() {
    let baseline = manifest(&render(""), ".gemspec");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.ruby.dependency_versions]\n\"gem:sorbet-runtime\" = \"{SENTINEL_GEM}\"\n"
        )),
        ".gemspec",
    );

    assert_override_replaces_default(&baseline, &overridden, "sorbet-runtime", SENTINEL_GEM);
}

/// The `Gemfile` is a second file fed by the same table, and it is `generated_header: false`
/// -- so it is the one case where the override has to reach a create-once seed rather than a
/// rewritten manifest.
#[test]
fn the_gemfile_honours_a_gem_override() {
    let baseline = manifest(&render(""), "packages/ruby/Gemfile");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.ruby.dependency_versions]\n\"gem:rspec\" = \"{SENTINEL_GEM}\"\n"
        )),
        "packages/ruby/Gemfile",
    );

    assert_override_replaces_default(&baseline, &overridden, "\"rspec\"", SENTINEL_GEM);
}

#[test]
fn description_honours_a_cran_override() {
    let baseline = manifest(&render(""), "packages/r/DESCRIPTION");
    let overridden = manifest(
        &render(&format!(
            "\n[crates.r.dependency_versions]\n\"cran:rextendr\" = \"{SENTINEL_PLAIN}\"\n"
        )),
        "packages/r/DESCRIPTION",
    );

    assert_override_replaces_default(&baseline, &overridden, "Config/rextendr/version", SENTINEL_PLAIN);
}

/// `DESCRIPTION` is Debian Control File format and is deliberately excluded from alef's
/// marker/comment machinery (`cli::pipeline::generate::write::marker_header_syntax`), so it
/// gets no in-manifest notice -- the `alef verify` finding is its only surface. Asserting the
/// absence here keeps a later "add the notice everywhere" change from corrupting a file
/// `R CMD build` parses strictly.
#[test]
fn description_carries_no_comment_notice() {
    let description = manifest(&render(""), "packages/r/DESCRIPTION");

    assert!(
        !description.contains("dependency_versions"),
        "DESCRIPTION has no comment syntax; the notice must not be emitted into it:\n{description}"
    );
    assert!(
        !description.lines().any(|line| line.starts_with('#')),
        "DESCRIPTION must carry no `#` comment line:\n{description}"
    );
}

/// The five comment-capable manifest files must each say where the durable override lives,
/// and the four `#`-comment ones must carry `~keep` or poly's uncomment pass deletes the
/// notice between regenerations.
#[test]
fn every_comment_capable_manifest_names_the_override_table() {
    let files = render("");
    for (suffix, table, keep_marked) in [
        (
            "packages/python/pyproject.toml",
            "[crates.python.dependency_versions]",
            true,
        ),
        ("packages/java/pom.xml", "[crates.java.dependency_versions]", false),
        ("mix.exs", "[crates.elixir.dependency_versions]", true),
        (".gemspec", "[crates.ruby.dependency_versions]", true),
        ("packages/ruby/Gemfile", "[crates.ruby.dependency_versions]", true),
    ] {
        let content = manifest(&files, suffix);
        assert!(content.contains(table), "`{suffix}` must name `{table}`:\n{content}");
        assert_eq!(
            content.contains("~keep"),
            keep_marked,
            "`{suffix}` keep-marking must match its comment grammar -- the four `#` manifests \
             carry the marker so poly's uncomment pass spares the notice, and the pom must not, \
             because `scaffold::template_env::render` strips every marker from jinja output and \
             poly has no uncomment pass for XML:\n{content}"
        );
    }
}

/// An XML comment may not contain `--`, so the pom's notice would make every Maven parse fail
/// on a well-formedness error. Checked against the emitted bytes rather than the notice
/// builder: the builder is shared, but only this one manifest renders it as XML.
#[test]
fn the_pom_notice_is_well_formed_xml() {
    let pom = manifest(&render(""), "packages/java/pom.xml");
    let comment_start = pom.find("<!--").expect("the pom must carry the notice comment");
    let comment_end = pom[comment_start..].find("-->").expect("the notice comment must close") + comment_start;
    let body = &pom[comment_start + "<!--".len()..comment_end];

    assert!(
        !body.contains("--"),
        "an XML comment body may not contain `--`; got:\n{body}"
    );
}

/// An explicit pin has to beat the disk floor, including downward.
///
/// `scaffold::version_floor` raises every emitted `Cargo.toml` requirement to the one the
/// committed manifest declares, which is right when alef is guessing and wrong once the
/// consumer has stated an intent: the floor only ever raises, so a deliberate pin *below*
/// what is committed would be reverted on every run and never converge -- the exact silent
/// discard this table exists to end.
#[test]
fn an_explicit_pin_survives_a_higher_committed_version() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    std::fs::write(
        workspace.path().join("Cargo.toml"),
        "[workspace]\nmembers = []\n\n[workspace.dependencies]\nanyhow = \"1\"\n",
    )
    .expect("write workspace manifest");
    let manifest_path = workspace.path().join("crates/sample-lib-py/Cargo.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest has a parent")).expect("create crate dir");
    std::fs::write(&manifest_path, "[dependencies]\npyo3 = \"99.99\"\n").expect("write committed manifest");

    let source = format!("{BASE_CONFIG}\n[crates.python.dependency_versions]\n\"cargo:pyo3\" = \"0.1-pinned-low\"\n");
    let parsed: NewAlefConfig = toml::from_str(&source).expect("fixture config parses");
    let mut resolved: ResolvedCrateConfig = parsed.resolve().expect("fixture config resolves").remove(0);
    resolved.workspace_root = Some(workspace.path().to_path_buf());
    let files =
        alef::scaffold::scaffold(&ApiSurface::default(), &resolved, &SCAFFOLDED_LANGUAGES).expect("scaffold succeeds");

    let emitted = manifest(&files, "-py/Cargo.toml");
    assert!(
        line_with(&emitted, "pyo3 = ").contains("0.1-pinned-low"),
        "the explicit pin must beat the disk floor:\n{emitted}"
    );
    assert!(
        !emitted.contains("99.99"),
        "the committed version must not be floored back in over an explicit pin:\n{emitted}"
    );
}

/// The anti-vacuity control for the test above: with no pin declared, the floor must still
/// do its job. Without this, dropping the floor entirely would leave that test passing.
#[test]
fn the_disk_floor_still_raises_a_dependency_that_is_not_pinned() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    std::fs::write(
        workspace.path().join("Cargo.toml"),
        "[workspace]\nmembers = []\n\n[workspace.dependencies]\nanyhow = \"1\"\n",
    )
    .expect("write workspace manifest");
    let manifest_path = workspace.path().join("crates/sample-lib-py/Cargo.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest has a parent")).expect("create crate dir");
    std::fs::write(&manifest_path, "[dependencies]\npyo3 = \"99.99\"\n").expect("write committed manifest");

    let parsed: NewAlefConfig = toml::from_str(BASE_CONFIG).expect("fixture config parses");
    let mut resolved: ResolvedCrateConfig = parsed.resolve().expect("fixture config resolves").remove(0);
    resolved.workspace_root = Some(workspace.path().to_path_buf());
    let files =
        alef::scaffold::scaffold(&ApiSurface::default(), &resolved, &SCAFFOLDED_LANGUAGES).expect("scaffold succeeds");

    let emitted = manifest(&files, "-py/Cargo.toml");
    assert!(
        line_with(&emitted, "pyo3 = ").contains("99.99"),
        "with nothing pinned, the committed version must still floor the emitted one:\n{emitted}"
    );
}

/// The pom notice's exact emitted shape, so the jinja whitespace controls cannot silently
/// collapse it back onto one line the way `{%-` did before this was pinned down.
#[test]
fn the_pom_notice_renders_as_its_own_indented_comment_block() {
    let pom = manifest(&render(""), "packages/java/pom.xml");
    let lines: Vec<&str> = pom.lines().take(6).collect();

    assert_eq!(lines[0], "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    assert_eq!(lines[1], "<!--");
    assert_eq!(
        lines[2],
        "  Dependency versions here are alef-managed and are rewritten on every `alef build`."
    );
    assert_eq!(
        lines[3],
        "  To raise one durably, set it in `[crates.java.dependency_versions]`; a direct edit to \
         this file is discarded."
    );
    assert_eq!(lines[4], "-->");
    assert_eq!(lines[5], "<project");
}

/// The `#`-comment notice's exact emitted shape and placement in each manifest that takes
/// one. Pinned for the same reason as the pom's: the notice is prepended into a `format!`
/// template, where a missing newline welds it onto the first real line of the file.
#[test]
fn the_hash_notice_renders_above_the_first_real_line_of_each_manifest() {
    let files = render("");
    let notice = |table: &str| {
        vec![
            "# Dependency versions here are alef-managed and are rewritten on every `alef build`. ~keep".to_string(),
            format!("# To raise one durably, set it in `{table}`; a direct edit to this file is discarded."),
        ]
    };

    let pyproject = manifest(&files, "packages/python/pyproject.toml");
    let expected = notice("[crates.python.dependency_versions]");
    assert_eq!(pyproject.lines().take(2).collect::<Vec<_>>(), expected);
    assert_eq!(pyproject.lines().nth(2), Some(""));
    assert_eq!(pyproject.lines().nth(3), Some("[build-system]"));

    let mix = manifest(&files, "mix.exs");
    let expected = notice("[crates.elixir.dependency_versions]");
    assert_eq!(mix.lines().take(2).collect::<Vec<_>>(), expected);
    assert_eq!(mix.lines().nth(2), Some(""));

    // Ruby's two manifests keep `# frozen_string_literal: true` first: the magic comment is
    // only honoured at the top of the file.
    for suffix in [".gemspec", "packages/ruby/Gemfile"] {
        let content = manifest(&files, suffix);
        assert_eq!(content.lines().next(), Some("# frozen_string_literal: true"));
        assert_eq!(
            content.lines().skip(1).take(2).collect::<Vec<_>>(),
            notice("[crates.ruby.dependency_versions]"),
            "in {suffix}"
        );
    }
}

/// Every `versions.get("…")` key in every renderer is registered for that renderer's
/// language.
///
/// `ManagedVersions::get` panics on an unregistered key, but several lookups are conditional
/// (`has_trait_bridges`, `has_streaming`, `has_async`) and never execute for the default
/// `ApiSurface` the tests above render, so a typo in one of those would reach a consumer
/// before it reached a test. Scanning the source is what covers the branches rendering
/// cannot.
///
/// The per-file counts are asserted up front for the same reason: a regex that silently
/// matched nothing would leave this test passing having examined no keys at all.
#[test]
fn every_renderer_lookup_key_is_registered_for_its_language() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let renderers = [
        ("src/scaffold/languages/python.rs", Language::Python, 9usize),
        ("src/scaffold/languages/java.rs", Language::Java, 22),
        ("src/scaffold/languages/elixir.rs", Language::Elixir, 16),
        ("src/scaffold/languages/ruby/manifests.rs", Language::Ruby, 9),
        ("src/scaffold/languages/r.rs", Language::R, 7),
    ];

    for (relative, language, expected_sites) in renderers {
        let source = std::fs::read_to_string(root.join(relative)).unwrap_or_else(|error| {
            panic!("read {relative}: {error}");
        });
        let keys: Vec<&str> = source
            .match_indices("versions.get(\"")
            .map(|(index, matched)| {
                let rest = &source[index + matched.len()..];
                let end = rest
                    .find('"')
                    .unwrap_or_else(|| panic!("unterminated key in {relative}"));
                &rest[..end]
            })
            .collect();

        assert_eq!(
            keys.len(),
            expected_sites,
            "{relative} has {} lookup site(s), expected {expected_sites} -- update this count \
             deliberately when adding or removing one, so the scan can never pass having found \
             nothing. Found: {keys:?}",
            keys.len()
        );
        for key in keys {
            assert!(
                alef::core::managed_versions::lookup(language, key).is_some(),
                "`{key}` is looked up in {relative} but is not registered for {language}; valid \
                 keys: {:?}",
                alef::core::managed_versions::keys_for(language)
            );
        }
    }
}
