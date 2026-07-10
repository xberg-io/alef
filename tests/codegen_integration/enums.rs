use super::*;

// ==============================================================================
// ==============================================================================

#[test]
fn test_enum_has_data_variants_false_for_unit_variants() {
    let enum_def = simple_enum_def();
    assert!(
        !enum_has_data_variants(&enum_def),
        "unit-only enum should not have data variants"
    );
}

#[test]
fn test_enum_has_data_variants_true_when_fields_present() {
    let enum_def = EnumDef {
        name: "DataEnum".to_string(),
        rust_path: "my_crate::DataEnum".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Variant".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    assert!(
        enum_has_data_variants(&enum_def),
        "enum with fields should have data variants"
    );
}

#[test]
fn test_gen_enum_with_single_variant_uses_discriminant_zero() {
    let enum_def = EnumDef {
        name: "Single".to_string(),
        rust_path: "my_crate::Single".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Only".to_string(),
            fields: vec![],
            doc: String::new(),
            is_default: true,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("pub enum Single {"), "should have enum declaration");
    assert!(result.contains("Only = 0"), "single variant has discriminant 0");
    assert!(result.contains("#[default]"), "first variant gets #[default]");
}

#[test]
fn test_gen_enum_with_enum_attrs() {
    let enum_def = simple_enum_def();
    let mut cfg = default_cfg();
    let attrs = vec!["repr(u8)"];
    cfg.enum_attrs = &attrs;

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("#[repr(u8)]"), "should include enum attrs");
}

#[test]
fn test_gen_enum_always_derives_serde() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("serde::Serialize"), "should always derive Serialize");
    assert!(
        result.contains("serde::Deserialize"),
        "should always derive Deserialize"
    );
}

#[test]
fn test_gen_enum_discriminant_increments_correctly() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: "my_crate::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Inactive".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Pending".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Deleted".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(result.contains("Active = 0"), "first variant = 0");
    assert!(result.contains("Inactive = 1"), "second variant = 1");
    assert!(result.contains("Pending = 2"), "third variant = 2");
    assert!(result.contains("Deleted = 3"), "fourth variant = 3");
    // Only first variant has #[default]
    assert!(result.contains("#[default]"), "should have #[default]");
}

#[test]
fn test_gen_enum_with_pyo3_pyclass_attr_emits_upper_snake_case_for_all_variants() {
    // Every variant in a pyo3 pyclass enum gets #[pyo3(name = "UPPER_SNAKE_CASE")] so that
    let enum_def = EnumDef {
        name: "BatchStatus".to_string(),
        rust_path: "my_crate::BatchStatus".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Validating".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "InProgress".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let mut cfg = default_cfg();
    let attrs = ["pyclass(eq, eq_int)"];
    cfg.enum_attrs = &attrs;

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("#[pyo3(name = \"NONE\")]"),
        "Python keyword 'None' should be emitted as NONE, got: {}",
        result
    );
    assert!(
        result.contains("#[pyo3(name = \"VALIDATING\")]"),
        "variant 'Validating' should be emitted as VALIDATING"
    );
    assert!(
        result.contains("#[pyo3(name = \"IN_PROGRESS\")]"),
        "variant 'InProgress' should be emitted as IN_PROGRESS"
    );
}

#[test]
fn test_gen_enum_without_pyclass_does_not_rename_python_keywords() {
    let enum_def = EnumDef {
        name: "Formats".to_string(),
        rust_path: "my_crate::Formats".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "None".to_string(),
            fields: vec![],
            doc: String::new(),
            is_default: true,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        !result.contains("#[pyo3(name ="),
        "without pyclass, should not emit any pyo3 rename"
    );
    assert!(result.contains("None = 0"), "variant should still appear");
}
