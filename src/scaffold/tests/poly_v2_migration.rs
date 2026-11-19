//! Regression coverage for the v1 -> v2 `poly.toml` conversion: the managed merge only ever adds
//! tables, so without it an already-scaffolded consumer keeps its v1 tables beside the v2 ones
//! alef now emits, a mix poly 0.36 rejects.

use super::*;

/// Every v1 table shape alef has emitted into a consumer's `poly.toml`.
const V1_POLY_TOML: &str = r#"[discovery]
exclude = ["/fixtures/**"]

[lint]
workspace = false

[lint.markdown.rumdl]
disable = ["MD013", "MD025"]

[fmt.markdown.rumdl]
disable = ["MD013"]

[lint.python.ruff]
select = ["E", "F"]
ignore = ["E501"]

[lint.php.mago]
select = ["correctness"]

[lint.typos.extend_words]
flate = "flate"

# Comments ride along with their table. ~keep
[lint.uncomment]
enabled = true
preserve_patterns = ["SAFETY"]

[fmt.shell.shfmt]
enabled = true
indent_width = 2

[tools.clang-format]
enabled = true

[per-file-ignores]
"**/tests/**" = ["S101"]

[hooks]
stages = ["pre-commit"]
"#;

/// What `poly migrate --to-v2 --write` (poly 0.36.0) writes for [`V1_POLY_TOML`], less the
/// `required_version` key alef's generated tables carry instead.
const V2_POLY_TOML: &str = r#"[discovery]
exclude = ["/fixtures/**"]

[workspace]
run = "never"

[engines.mago.lint]
select = ["correctness"]

[engines.ruff.lint]
select = ["E", "F"]
ignore = ["E501"]

[engines.rumdl.lang.markdown.lint]
disable = ["MD013", "MD025"]

[engines.rumdl.lang.markdown.format]
disable = ["MD013"]

[engines.shfmt.format]
enabled = true
indent_width = 2

[engines.typos]
enabled = true

[engines.typos.extend_words]
flate = "flate"

[engines.uncomment]
enabled = true
preserve_patterns = ["SAFETY"]

[tools.clang-format]
enabled = true

[per_file_ignores]
"**/tests/**" = ["S101"]

[hooks]
stages = ["pre-commit"]
"#;

#[test]
fn should_convert_every_alef_v1_table_the_way_poly_migrate_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("poly.toml"), V1_POLY_TOML).expect("write v1 poly.toml");

    let changed = migrate_poly_toml_to_v2(dir.path()).expect("migration must not error");
    assert!(changed, "a v1 poly.toml must be reported as converted");

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read converted file");
    let converted: toml::Value = toml::from_str(&on_disk).expect("converted poly.toml must parse");
    let expected: toml::Value = toml::from_str(V2_POLY_TOML).expect("expected fixture parses");
    assert_eq!(converted, expected, "converted file:\n{on_disk}");
    assert!(
        on_disk.contains("# Comments ride along with their table. ~keep\n[engines.uncomment]"),
        "a moved table keeps its leading comment:\n{on_disk}"
    );

    let changed_again = migrate_poly_toml_to_v2(dir.path()).expect("second pass must not error");
    assert!(!changed_again, "a v2 poly.toml must be left alone");
}

#[test]
fn should_keep_v1_keys_it_cannot_place_so_poly_reports_the_mixed_file() {
    let mut doc = "[lint]\nshared = [\"A\"]\n\n[lint.python.ruff]\nselect = [\"ANN\"]\n"
        .parse::<toml_edit::DocumentMut>()
        .expect("fixture parses");
    assert!(convert_v1_poly_tables(doc.as_table_mut()));
    let converted: toml::Value = toml::from_str(&doc.to_string()).expect("converted parses");

    assert_eq!(converted["engines"]["ruff"]["lint"]["select"][0].as_str(), Some("ANN"));
    assert_eq!(converted["lint"]["shared"][0].as_str(), Some("A"));
}

#[test]
fn should_place_a_multi_language_engine_under_its_language() {
    let mut doc = "[fmt.c.clang-format]\nstyle = \"LLVM\"\n"
        .parse::<toml_edit::DocumentMut>()
        .expect("fixture parses");
    assert!(convert_v1_poly_tables(doc.as_table_mut()));
    let converted: toml::Value = toml::from_str(&doc.to_string()).expect("converted parses");

    assert_eq!(
        converted["engines"]["clang-format"]["lang"]["c"]["format"]["style"].as_str(),
        Some("LLVM")
    );
    assert!(converted.get("fmt").is_none());
}

#[test]
fn should_map_v1_merge_provenance_paths_to_their_v2_tables() {
    assert_eq!(
        v2_poly_provenance_path("per-file-ignores.**/tests/**"),
        "per_file_ignores.**/tests/**"
    );
    assert_eq!(
        v2_poly_provenance_path("lint.python.ruff.select"),
        "engines.ruff.lint.select"
    );
    assert_eq!(v2_poly_provenance_path("discovery.exclude"), "discovery.exclude");
}
