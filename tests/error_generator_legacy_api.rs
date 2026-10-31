//! External call sites keep the pre-shared-family public generator signatures.

use alef::codegen::error_gen::{
    gen_csharp_error_types, gen_java_error_types, gen_java_error_types_with_parent, gen_php_error_converter,
};
use alef::core::ir::{ErrorDef, ErrorVariant};

#[test]
fn legacy_public_generator_calls_remain_source_compatible() {
    let error = ErrorDef {
        name: "StorageError".into(),
        rust_path: "demo::StorageError".into(),
        variants: vec![ErrorVariant {
            name: "Missing".into(),
            is_unit: true,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let java = gen_java_error_types(&error, "demo", "Demo");
    let java_parent = gen_java_error_types_with_parent(&error, "demo", "Demo", None);
    let csharp = gen_csharp_error_types(&error, "Demo", None, &[]);
    let php = gen_php_error_converter(&error, "demo");
    assert_eq!(java.len(), 2);
    assert_eq!(java, java_parent);
    assert_eq!(csharp.len(), 2);
    assert!(php.contains("StorageError::Missing"), "{php}");
}
