use super::*;

#[test]
fn mirror_error_introspection_uses_safe_from_conversion_not_transmute() {
    let error = ErrorDef {
        exception_base: None,
        name: "AppError".to_string(),
        rust_path: "demo_crate::error::AppError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "Auth".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![
                    make_field("message", TypeRef::String, false),
                    make_field("status", TypeRef::Primitive(PrimitiveType::U16), false),
                ],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "Timeout".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            },
        ],
        doc: String::new(),
        methods: vec![MethodDef {
            name: "status_code".to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::U16),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![error],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("impl From<&AppError> for demo_crate::error::AppError"),
        "safe From<&MirrorEnum> impl must be emitted for error types with methods: {lib}"
    );
    assert!(
        lib.contains("let real: demo_crate::error::AppError = self.into();"),
        "introspection method must use safe self.into() conversion: {lib}"
    );
    assert!(
        !lib.contains("as *const Self as *const"),
        "unsafe raw-pointer transmute must NOT appear in mirror error impl: {lib}"
    );
    assert!(
        lib.contains("status: *f_status as u16"),
        "u16 field must be cast from i64 with `as u16` in From impl: {lib}"
    );
}

#[test]
fn mirror_error_from_impl_handles_optional_string_duration_and_sanitized_fields() {
    use alef::core::ir::CoreWrapper;
    let make_error_field = |name: &str, ty: TypeRef, optional: bool, sanitized: bool| FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let status_code_method = MethodDef {
        name: "code".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::I64),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let error = ErrorDef {
        exception_base: None,
        name: "ApiErr".to_string(),
        rust_path: "demo_crate::ApiErr".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "Budget".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![
                    make_error_field("message", TypeRef::String, false, false),
                    make_error_field("model", TypeRef::String, true, false),
                ],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "RateLimit".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![
                    make_error_field("message", TypeRef::String, false, false),
                    make_error_field("retry_after", TypeRef::Duration, true, false),
                ],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "Serialization".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![make_error_field("field0", TypeRef::String, false, true)],
                has_source: true,
                has_from: true,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
        ],
        doc: String::new(),
        methods: vec![status_code_method],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![error],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("model: f_model.as_ref().map(|v| v.clone())"),
        "optional String field must map through the option in From impl: {lib}"
    );
    assert!(
        lib.contains("retry_after: f_retry_after.as_ref().map(|v| std::time::Duration::from_millis(*v as u64))"),
        "optional Duration field must map through the option in From impl: {lib}"
    );
    assert!(
        lib.contains("model: Option<String>,") && lib.contains("retry_after: Option<i64>,"),
        "optional error variant fields must stay optional in the mirror enum: {lib}"
    );
    assert!(
        lib.contains("unreachable!"),
        "sanitized variant must produce unreachable wildcard arm in From impl: {lib}"
    );
    let to_core = lib
        .split("impl From<&ApiErr> for ")
        .nth(1)
        .and_then(|implementation| implementation.split("\nimpl ").next());
    assert!(
        to_core.is_some_and(|implementation| !implementation.contains("Serialization")),
        "sanitized arm in mirror-to-core impl: {lib}"
    );
}

/// Tuple-variant error types (fields named `_0`, `_1`, … as set by the extractor for
/// `syn::Fields::Unnamed`) must use positional constructor syntax in the `From<&Mirror>`
/// impl. The mirror enum always uses struct syntax `{ field0: T }` (FRB requirement),
/// but the core type expects `CoreError::Variant(value)` — not `CoreError::Variant { field0: value }`.
///
/// Regression: dart bridge crate failing E0559 "variant `X` has no field named `field0`" for a tuple variant.
#[test]
fn mirror_error_from_impl_uses_tuple_syntax_for_tuple_variants() {
    let make_positional_field = |idx: usize, ty: TypeRef| FieldDef {
        version: Default::default(),
        name: format!("_{idx}"),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let make_named_field = |name: &str, ty: TypeRef| FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let error = ErrorDef {
        exception_base: None,
        name: "GraphQLError".to_string(),
        rust_path: "demo_crate::GraphQLError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "ExecutionError".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![make_positional_field(0, TypeRef::String)],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "NetworkError".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![
                    make_positional_field(0, TypeRef::String),
                    make_positional_field(1, TypeRef::Primitive(PrimitiveType::U16)),
                ],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "SchemaError".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![make_named_field("message", TypeRef::String)],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "Unknown".to_string(),
                error_code: None,
                message_template: None,
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            },
        ],
        doc: String::new(),
        methods: vec![MethodDef {
            name: "kind".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: true,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![error],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("GraphQLError::ExecutionError { field0: f_field0 }"),
        "tuple variant pattern must still use mirror struct syntax {{field0: …}}: {lib}"
    );
    assert!(
        lib.contains("Self::ExecutionError(f_field0.clone())"),
        "tuple variant constructor must use positional tuple syntax Self::ExecutionError(…): {lib}"
    );
    assert!(
        !lib.contains("Self::ExecutionError { field0:"),
        "tuple variant constructor must NOT use struct syntax Self::ExecutionError {{ field0: … }}: {lib}"
    );

    assert!(
        lib.contains("GraphQLError::NetworkError { field0: f_field0, field1: f_field1 }"),
        "two-field tuple variant pattern must use struct syntax: {lib}"
    );
    assert!(
        lib.contains("Self::NetworkError(f_field0.clone(),"),
        "two-field tuple variant constructor must use positional tuple syntax: {lib}"
    );

    assert!(
        lib.contains("Self::SchemaError {"),
        "named-field struct variant constructor must still use struct syntax: {lib}"
    );
    assert!(
        lib.contains("message: f_message.clone()"),
        "named-field struct variant must initialize `message` from the bound pattern: {lib}"
    );

    assert!(
        lib.contains("GraphQLError::Unknown => Self::Unknown"),
        "unit variant arm must be unchanged: {lib}"
    );
}
