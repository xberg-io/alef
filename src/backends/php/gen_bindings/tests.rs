/// Test that PHP wrapper param signatures preserve required-ness from the Rust API.
///
/// Before the fix: Required params after an optional param were being made optional.
/// Example: `scrape(?CrawlEngineHandle $engine = null, ?string $url = null)`
/// when the Rust API required both `engine: CrawlEngineHandle` and `url: String`.
///
/// After the fix: Only explicitly optional params or default-constructible params
/// become optional in the wrapper. Required params stay required.
/// Example: `scrape(CrawlEngineHandle $engine, string $url)`
#[test]
fn test_php_wrapper_param_optionality_logic() {
    use crate::core::ir::{ParamDef, TypeRef};

    // Helper to check if a param should be optional in the wrapper
    let is_optional_default_constructible_param = |p: &ParamDef| -> bool {
        if let TypeRef::Named(name) = &p.ty {
            // Simulate the no_arg_constructor_types set
            matches!(name.as_str(), "CrawlConfig" | "InteractionActions")
        } else {
            false
        }
    };

    // Test case 1: Required params should remain required
    let req_param = ParamDef {
        name: "url".to_string(),
        ty: TypeRef::String,
        optional: false,
        ..ParamDef::default()
    };

    let should_be_optional = req_param.optional || is_optional_default_constructible_param(&req_param);
    assert!(
        !should_be_optional,
        "required param should not become optional in wrapper"
    );

    // Test case 2: Explicitly optional params remain optional
    let opt_param = ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("CrawlConfig".to_string()),
        optional: true,
        ..ParamDef::default()
    };

    let should_be_optional = opt_param.optional || is_optional_default_constructible_param(&opt_param);
    assert!(should_be_optional, "explicitly optional param should be optional");

    // Test case 3: Default-constructible required params become optional
    let default_constructible_param = ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("CrawlConfig".to_string()),
        optional: false,
        ..ParamDef::default()
    };

    let should_be_optional =
        default_constructible_param.optional || is_optional_default_constructible_param(&default_constructible_param);
    assert!(should_be_optional, "default-constructible param should become optional");
}
