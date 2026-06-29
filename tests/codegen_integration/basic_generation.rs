use super::*;

#[test]
fn test_gen_struct_produces_struct_definition() {
    let typ = simple_type_def();
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(
        result.contains("pub struct MyConfig"),
        "should contain struct declaration"
    );
    assert!(result.contains("name: String"), "should contain String field");
    assert!(
        result.contains("count: Option<u32>"),
        "should contain optional u32 field"
    );
    assert!(
        result.contains("#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]"),
        "should have derives"
    );
}

#[test]
fn test_gen_function_produces_function_signature() {
    let func = simple_function_def();
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("pub fn process"), "should contain function name");
    assert!(result.contains("input: String"), "should contain input param");
    assert!(result.contains("-> u32"), "should contain return type");
}

#[test]
fn test_gen_enum_produces_enum_with_variants() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("pub enum OutputFormat"),
        "should contain enum declaration"
    );
    assert!(
        result.contains("Json = 0"),
        "should contain first variant with discriminant"
    );
    assert!(result.contains("Csv = 1"), "should contain second variant");
    assert!(result.contains("Plain = 2"), "should contain third variant");
    assert!(
        result.contains("#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]"),
        "should have derives"
    );
}

#[test]
fn test_gen_enum_produces_default_impl() {
    let enum_def = simple_enum_def();
    let cfg = default_cfg();

    let result = gen_enum(&enum_def, &cfg);

    assert!(
        result.contains("#[default]"),
        "should have #[default] attribute on first variant"
    );
    assert!(result.contains("Default"), "should derive Default");
}

#[test]
fn test_gen_struct_with_empty_fields() {
    let typ = TypeDef {
        name: "Empty".to_string(),
        rust_path: "my_crate::Empty".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_struct(&typ, &mapper, &cfg);

    assert!(result.contains("pub struct Empty"), "should generate empty struct");
}
