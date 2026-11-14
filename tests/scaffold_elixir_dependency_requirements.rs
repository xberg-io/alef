use alef::core::config::{NewAlefConfig, validation::validate_resolved};
use alef::core::managed_versions;
use alef::{ApiSurface, Language};
use semver::{Version, VersionReq};

const DEPENDENCIES: [&str; 3] = ["alloc-no-stdlib", "alloc-stdlib", "brotli-decompressor"];

fn scaffold_dependencies(requirement: Option<&str>) -> toml::Value {
    let mut source = r#"
[workspace]
languages = ["elixir"]

[workspace.package_metadata]
license = "MIT"

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#
    .to_owned();
    if let Some(requirement) = requirement {
        source.push_str("\n[crates.elixir.dependency_versions]\n");
        for dependency in DEPENDENCIES {
            source.push_str(&format!("\"cargo:{dependency}\" = \"{requirement}\"\n"));
        }
    }
    let parsed: NewAlefConfig = toml::from_str(&source).expect("fixture config parses");
    let resolved = parsed.resolve().expect("fixture config resolves").remove(0);
    validate_resolved(&resolved).expect("fixture config validates");
    let files =
        alef::scaffold::scaffold(&ApiSurface::default(), &resolved, &[Language::Elixir]).expect("scaffold succeeds");
    let manifests: Vec<_> = files
        .iter()
        .filter(|file| {
            file.path
                .to_string_lossy()
                .ends_with("native/sample_lib_nif/Cargo.toml")
        })
        .collect();
    assert_eq!(manifests.len(), 1, "one native manifest must be generated");
    let manifest: toml::Value = toml::from_str(&manifests[0].content).expect("manifest parses");
    manifest["target"]["cfg(false)"]["dependencies"].clone()
}

fn assert_requirement(dependencies: &toml::Value, dependency: &str, expected: &str) {
    let emitted = dependencies
        .get(dependency)
        .and_then(toml::Value::as_str)
        .expect("dependency is a string requirement");
    let parsed = VersionReq::parse(emitted)
        .unwrap_or_else(|error| panic!("invalid {dependency} requirement {emitted:?}: {error}"));
    assert_eq!(emitted, expected, "{dependency} requirement must be preserved");
    assert_eq!(
        parsed,
        VersionReq::parse(expected).expect("expected requirement parses")
    );
}

#[test]
fn should_keep_default_brotli_allocator_versions_exact() {
    let dependencies = scaffold_dependencies(None);
    for dependency in DEPENDENCIES {
        let key = format!("cargo:{dependency}");
        let default = managed_versions::lookup(Language::Elixir, &key)
            .expect("dependency is registered")
            .default;
        Version::parse(default).expect("default is a full version");
        assert_requirement(&dependencies, dependency, &format!("={default}"));
    }
}

#[test]
fn should_keep_bare_full_version_overrides_exact() {
    for version in ["99.88.77", "99.88.77-rc.1", "99.88.77+build.2"] {
        let dependencies = scaffold_dependencies(Some(version));
        for dependency in DEPENDENCIES {
            assert_requirement(&dependencies, dependency, &format!("={version}"));
        }
    }
}

#[test]
fn should_preserve_explicit_cargo_requirements_for_brotli_allocator_overrides() {
    for requirement in [
        "^3.0.0",
        "~0.3.0",
        ">=6.0.1, <7.0.0",
        "3.*",
        "*",
        "3",
        "3.0",
        "=3.0.0",
        "^3.0.0-rc.1",
    ] {
        let dependencies = scaffold_dependencies(Some(requirement));
        for dependency in DEPENDENCIES {
            assert_requirement(&dependencies, dependency, requirement);
        }
    }
}
