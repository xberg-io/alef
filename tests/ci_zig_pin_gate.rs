use alef::core::template_versions::toolchain::MIN_ZIG_VERSION;

#[test]
fn ci_pins_zig_to_the_generated_code_toolchain_version() {
    let workflow = include_str!("../.github/workflows/ci.yml");
    let expected =
        format!("uses: xberg-io/actions/setup-zig@v1\n        with:\n          version: \"{MIN_ZIG_VERSION}\"");

    assert!(
        workflow.contains(&expected),
        "CI must pin setup-zig to {MIN_ZIG_VERSION}; the action's rolling `latest` default can install an incompatible \
         Zig release"
    );
}
