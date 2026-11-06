use std::path::Path;

fn dependency_requirement<'a>(dependencies: &'a toml::Table, name: &str) -> &'a str {
    let dependency = dependencies
        .get(name)
        .unwrap_or_else(|| panic!("Cargo.toml must declare {name}"));
    dependency
        .as_str()
        .or_else(|| dependency.get("version").and_then(toml::Value::as_str))
        .unwrap_or_else(|| panic!("{name} must declare a version requirement"))
}

#[test]
fn unicode_tables_are_unpinned() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read Alef Cargo.toml");
    let parsed: toml::Value = toml::from_str(&manifest).expect("parse Alef Cargo.toml");
    let dependencies = parsed
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml has dependencies");

    for name in ["icu_casemap", "unicode-general-category"] {
        let requirement = dependency_requirement(dependencies, name);
        semver::VersionReq::parse(requirement).unwrap_or_else(|error| panic!("parse {name} requirement: {error}"));
        assert!(
            !requirement.starts_with('='),
            "{name} must not be exactly pinned; the behavioral checks below guard the tables"
        );
    }
}

#[test]
fn upgraded_case_tables_preserve_collisions_for_accepted_nuget_names() {
    use alef::codegen::coordinates::{nuget_ordinal_fold, validate_nuget_package_id};

    // Unicode 18 adds uppercase partners for these accepted Unicode 16 letters, but the newer
    // partners remain outside our pinned identifier grammar. Collision classes must stay stable. ~keep
    let pairs = [
        ('\u{0277}', '\u{a7dd}'),
        ('\u{027c}', '\u{a7e2}'),
        ('\u{ab4b}', '\u{ab6c}'),
        ('\u{ab4c}', '\u{ab6d}'),
    ];
    let mut comparisons = 0;
    for (index, (left, uppercase)) in pairs.iter().enumerate() {
        let left = format!("Sample{left}");
        assert!(validate_nuget_package_id(&left).is_ok(), "{left}");
        let newer_uppercase = format!("Sample{uppercase}");
        assert!(
            validate_nuget_package_id(&newer_uppercase).is_err(),
            "{newer_uppercase}"
        );
        for (other_index, (right, _)) in pairs.iter().enumerate() {
            let right = format!("sample{right}");
            assert!(validate_nuget_package_id(&right).is_ok(), "{right}");
            assert_eq!(
                nuget_ordinal_fold(&left) == nuget_ordinal_fold(&right),
                index == other_index
            );
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 16);
}
