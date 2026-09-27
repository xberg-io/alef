//! Regression coverage for alef issue #457: CI installs pyrefly through a hard-pinned
//! `uv tool install pyrefly==<version>` line in `.github/workflows/ci.yml` (the pyo3 kwarg-unpack
//! harness and the generated-package gate both resolve pyrefly from PATH via
//! `which::which("pyrefly")`, not from any config), while
//! `template_versions::pypi::PYREFLY` sets an independent floor that only feeds the
//! `pyrefly>=X` dev dependency alef scaffolds into a *generated consumer's* own
//! `pyproject.toml`. Nothing tied those two together, and the CI pin sat at `1.2.0` for a month
//! after `template_versions::pypi::PYREFLY` implied `>=1.2.0` was sufficient -- until
//! `backends::pyo3::kwarg_unpack_tests`'s negative control started requiring `[open-unpacking]`,
//! a diagnostic pyrefly's `strict` preset only promotes from `Ignore` to `Error` starting at
//! `1.3.0`. `1.2.0` and `1.2.1` report zero errors for the exact sabotaged input that test
//! checks, so CI went red on all three OSes while every developer machine running a newer
//! pyrefly stayed green.
//!
//! Unlike `tests/ci_poly_pin_gate.rs`, which only reads the workflow file as text, this gate
//! checks the real pyrefly binary resolved from PATH -- the same resolution the production
//! harnesses use -- against the real registry constant, so it catches drift in either direction:
//! a `template_versions::pypi::PYREFLY` bump that outruns the CI pin, or a CI pin that outruns a
//! floor nobody remembered to raise. It shares the pyo3 kwarg-unpack harness's own skip
//! contract: unavailable locally is a skip, `ALEF_REQUIRE_PYREFLY=1` (set for every CI `test` job
//! leg) turns that skip into a hard failure. ~keep

/// Parses the trailing token of `pyrefly --version`'s stdout (`"pyrefly 1.3.1\n"`) into a
/// [`semver::Version`], rather than assuming a fixed prefix -- the binary's own reported name is
/// not a contract this test should depend on.
fn parse_pyrefly_version(version_output: &str) -> Option<semver::Version> {
    let token = version_output.split_whitespace().last()?;
    semver::Version::parse(token).ok()
}

#[test]
#[allow(clippy::print_stderr)] // narrow: reports a toolchain skip on a machine without pyrefly ~keep
fn pyrefly_on_path_satisfies_the_template_versions_floor() {
    let pyrefly = match which::which("pyrefly") {
        Ok(path) => path,
        Err(error) if std::env::var_os("ALEF_REQUIRE_PYREFLY").is_some() => {
            panic!("ALEF_REQUIRE_PYREFLY is set but pyrefly is unavailable: {error}")
        }
        Err(error) => {
            eprintln!(
                "SKIP pyrefly_on_path_satisfies_the_template_versions_floor: pyrefly is not on \
                 PATH ({error}). Set ALEF_REQUIRE_PYREFLY=1 to turn this skip into a failure."
            );
            return;
        }
    };

    let output = std::process::Command::new(&pyrefly)
        .arg("--version")
        .output()
        .expect("pyrefly --version must run");
    let version_output = String::from_utf8_lossy(&output.stdout);
    let resolved = parse_pyrefly_version(&version_output)
        .unwrap_or_else(|| panic!("could not parse a semver version out of `pyrefly --version`: {version_output:?}"));

    let floor = alef::core::template_versions::pypi::PYREFLY;
    let requirement = semver::VersionReq::parse(floor).unwrap_or_else(|error| {
        panic!("`template_versions::pypi::PYREFLY` (`{floor}`) must parse as a semver requirement: {error}")
    });

    assert!(
        requirement.matches(&resolved),
        "the pyrefly resolved from PATH ({resolved}) does not satisfy \
         `template_versions::pypi::PYREFLY` (`{floor}`) -- alef issue #457: pyrefly older than \
         1.3.0 does not promote `[open-unpacking]` from `Ignore` to `Error` under the `strict` \
         preset, so a pin below this floor makes `backends::pyo3::kwarg_unpack_tests`'s negative \
         control pass vacuously. Either the CI pin in `.github/workflows/ci.yml` (`uv tool \
         install pyrefly==...`) drifted below the registry floor, or the floor was raised without \
         checking whether the installed toolchain (this machine, or the CI image) still satisfies \
         it. Bump both together."
    );
}

/// [`parse_pyrefly_version`] must actually discriminate: it should extract the real version from
/// realistic output and refuse output that carries none, rather than silently returning
/// [`None`] either way and making the gate above vacuous.
#[test]
fn parse_pyrefly_version_rejects_output_with_no_trailing_semver() {
    assert_eq!(
        parse_pyrefly_version("pyrefly 1.3.1\n"),
        Some(semver::Version::new(1, 3, 1)),
        "real `pyrefly --version` output must parse"
    );
    assert_eq!(
        parse_pyrefly_version("pyrefly 1.2.0\n"),
        Some(semver::Version::new(1, 2, 0)),
        "an older real version must still parse -- this is the exact input the gate above must reject downstream"
    );
    assert_eq!(
        parse_pyrefly_version("pyrefly is not installed\n"),
        None,
        "output with no trailing semver token must not resolve to a fabricated version"
    );
    assert_eq!(
        parse_pyrefly_version(""),
        None,
        "empty output must not resolve to a version"
    );
}

/// The comparison the gate above runs (`VersionReq::matches`) must actually discriminate against
/// the exact versions alef issue #457 was about, not just against synthetic examples -- proven
/// against the real pinned floor string, not a hand-typed `">=1.3.0"` that could drift from it.
/// Empirically confirmed by running the real pyrefly 1.2.0, 1.2.1, 1.3.0 and 1.3.1 binaries
/// against the sabotaged `backends::pyo3::kwarg_unpack_tests` fixture: the first two report zero
/// errors (the harness's negative control silently stops discriminating), the last two report
/// `[open-unpacking]` as required.
#[test]
fn template_versions_floor_rejects_the_two_pyrefly_releases_that_missed_open_unpacking() {
    let requirement = semver::VersionReq::parse(alef::core::template_versions::pypi::PYREFLY)
        .expect("`template_versions::pypi::PYREFLY` must parse as a semver requirement");

    assert!(
        !requirement.matches(&semver::Version::new(1, 2, 0)),
        "1.2.0 does not promote `[open-unpacking]` under `strict` and must be rejected"
    );
    assert!(
        !requirement.matches(&semver::Version::new(1, 2, 1)),
        "1.2.1 does not promote `[open-unpacking]` under `strict` and must be rejected"
    );
    assert!(
        requirement.matches(&semver::Version::new(1, 3, 0)),
        "1.3.0 is the first release that promotes `[open-unpacking]` under `strict` and must be accepted"
    );
    assert!(
        requirement.matches(&semver::Version::new(1, 3, 1)),
        "1.3.1 must be accepted -- this is the version installed on the machine the fix was verified on"
    );
}
