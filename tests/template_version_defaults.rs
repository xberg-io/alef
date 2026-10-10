use alef::core::template_versions::{gem, maven, nuget};

#[test]
fn generated_dependencies_use_supported_stable_release_lines() {
    assert_eq!(gem::SORBET_RUNTIME, "~> 0.6");
    assert_eq!(gem::STEEP, "~> 2.1");
    assert_eq!(maven::ASSERTJ, "3.27.7");
    assert_eq!(nuget::MICROSOFT_NET_TEST_SDK, "18.10.1");
}
