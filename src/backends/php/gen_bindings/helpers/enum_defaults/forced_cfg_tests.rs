use crate::backends::php::gen_bindings::{enum_cfg, host_enum_feature_tests};

#[test]
fn forced_feature_is_not_guarded_by_an_undeclared_php_feature_in_string_conversion() {
    let root = tempfile::tempdir().expect("fixture");
    host_enum_feature_tests::write_fixture(root.path(), true, None, false, false);
    let config = host_enum_feature_tests::config(root.path(), true);
    let mut api = host_enum_feature_tests::surface(true);
    let declared = crate::scaffold::languages::php::php_declared_features(&api, &[]);
    enum_cfg::specialize(&mut api, &config, &declared).expect("specialize");
    let expression = super::gen_string_to_enum_expr("value", "Mode", false, &api.enums, "core_lib", "mode");
    assert!(expression.contains("core_lib::Mode::Remote"), "{expression}");
    assert!(!expression.contains("#[cfg("), "{expression}");
    assert!(!expression.contains(r#"feature = "remote""#), "{expression}");
}
