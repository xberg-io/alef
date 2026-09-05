use super::*;

fn error_api(name: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: name.to_string(),
            rust_path: format!("test::{name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "InvalidInput".to_string(),
                error_code: None,
                message_template: Some("invalid input: {0}".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn dispatcher_for(error_name: &str) -> String {
    CsharpBackend
        .generate_bindings(&error_api(error_name), &minimal_csharp_config("test"))
        .unwrap()
        .into_iter()
        .find(|file| file.content.contains("internal static Exception FromLastError"))
        .expect("a generated file must carry the FromLastError dispatcher")
        .content
}

#[test]
fn test_error_helper_preserves_base_error_acronym_class_name() {
    let dispatcher = dispatcher_for("GraphQLError");

    assert!(
        dispatcher.contains("if (code == 2) return new GraphQLErrorException(message);"),
        "{dispatcher}"
    );
    assert!(!dispatcher.contains("GraphQlErrorException"));
}

/// FFI code 1 is infrastructure-owned and must not be hijacked by a user variant. ~keep
#[test]
fn test_invalid_input_variant_does_not_hijack_ffi_conversion_error_code() {
    let dispatcher = dispatcher_for("RequestError");

    assert!(
        !dispatcher.contains("code == 1"),
        "FFI code 1 must not dispatch to a user variant: {dispatcher}"
    );
    assert!(
        dispatcher.contains("if (message.StartsWith(\"invalid input:\")) return new InvalidInputException(message);"),
        "InvalidInput must dispatch by message prefix: {dispatcher}"
    );
}
