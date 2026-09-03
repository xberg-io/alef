use super::*;

/// Regression test: the PHPStan stub's constructor used to be built from
/// `binding_fields(&typ.fields)` with NO filtering, promoting every field — including ones
/// the real extension's `#[php(constructor)]` cannot accept (e.g. `Vec<Json>`, which has
/// no `#[php(prop)]` and is never a constructor param — see `is_php_prop_scalar_with_enums` /
/// `php_field_can_be_constructor_param` in `structs.rs`). PHPStan would then report a 2-arg
/// constructor call as correct when the real extension only accepts 1 arg, and `$r->value`
/// as a valid property read when the extension never registers that property.
///
/// The stub must instead call the SAME `php_field_can_be_constructor_param` predicate the real
/// extension's constructor uses, omit non-representable fields from the constructor entirely,
/// and still expose them via a `get<Field>()` stub so PHPStan sees the only working accessor.
///
/// `Vec<Json>` (rather than a bare or `Optional` `Json` field) is the fixture here because
/// `php_field_can_be_constructor_param`'s `Json` arm answers `true` for a bare/optional `Json`
/// field since commit a8da43884 ("accept nested struct, Vec<struct> and Json constructor
/// params") — such a field decodes a JSON `String` param via `serde_json::from_str` and IS now
/// a legitimate (plain, non-promoted) constructor parameter on both the stub and the real
/// extension; see `optional_json_field_is_a_plain_constructor_param_and_reachable_via_getter`
/// below and `json_constructor_param_tests.rs`'s `optional_json_is_representable`. The `Vec`
/// arm's own `TypeRef::Json => false` case (a `Vec` of `Json` values has no per-element decode
/// path) is the one shape that is still genuinely unrepresentable, so it is what this test
/// pins instead. ~keep
///
/// This only holds when the binding crate has serde: `structs.rs`'s `use_from_json` gate (and
/// its mirror, `type_stubs.rs`'s `stub_constructor_shape`) requires `serde_available` before it
/// will emit a positional `#[php(constructor)]` alongside the fields it can't represent — without
/// serde, `has_named_params` alone routes the type to a zero-param, unconditionally-throwing
/// `__construct` (no promoted properties at all, see `structs.rs`'s `has_named_params` branch).
/// The fixture crate dir must therefore have a real `Cargo.toml` with `serde`/`serde_json`
/// dependencies, or `php_serde_available` reads `false` and this test would be exercising the
/// throwing shape instead of the one it names. ~keep
#[test]
fn json_field_is_excluded_from_constructor_but_reachable_via_getter() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                make_field("value", TypeRef::Vec(Box::new(TypeRef::Json)), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let root = tempfile::tempdir().expect("tempdir");
    let output_dir = root.path().join("crates/test-lib-php/src");
    std::fs::create_dir_all(&output_dir).expect("create output dir");
    std::fs::write(
        root.path().join("crates/test-lib-php/Cargo.toml"),
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .expect("write Cargo.toml");
    let config = make_config_with_php_output(&output_dir);
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().expect("stub file must be generated");
    let content = &stubs.content;

    // --- positive: `value` (Vec<Json>) is not representable as a constructor param ---
    assert!(
        !content.contains("$value"),
        "Vec<Json> field `value` has no ext-php-rs constructor-param support and must not \
         appear as a promoted property or a plain constructor parameter:\n{content}"
    );

    assert!(
        content.contains("public function getValue(): ?string"),
        "field `value` must still be reachable via a getter returning the JSON-serialized \
         string (matching the real extension's `Option<String>` getter for Json fields):\n{content}"
    );

    assert!(
        content.contains("Not settable via the constructor"),
        "the omission of `value` from the constructor must be documented in the generated stub, \
         not silent (per the standing rule against silent alef drops):\n{content}"
    );

    // --- negative control: an ordinary String field must remain a real, promoted property ---
    assert!(
        content.contains("public readonly string $content"),
        "an ordinary required String field must still be a promoted constructor property — the \
         filter must be scoped to non-representable types, not strip ordinary fields:\n{content}"
    );

    assert!(
        content.contains("public function getContent(): string"),
        "an ordinary field must also still get its getter stub (every binding field does, per \
         the real extension's unconditional getter loop):\n{content}"
    );
}

/// A field whose type IS representable as a constructor param but is NOT itself a real PHP
/// property (e.g. `Bytes`, or `Vec<Named>` of an opaque/enum type) must appear as a plain
/// (non-promoted) constructor parameter — reachable from `new(...)`, but with no matching
/// `public readonly` property, since the real extension never emits `#[php(prop)]` for it.
///
/// Same serde-availability caveat as `json_field_is_excluded_from_constructor_but_reachable_via_getter`
/// above: `Bytes` fails `is_php_prop_scalar`, so it alone makes `has_named_params` true, and
/// without a real `serde`-having crate dir the whole type falls through to the zero-param
/// throwing `__construct` rather than the positional shape this test is named for. ~keep
#[test]
fn bytes_field_is_a_plain_constructor_param_without_a_promoted_property() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Blob".to_string(),
            rust_path: "test_lib::Blob".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("name", TypeRef::String, false),
                make_field("payload", TypeRef::Bytes, false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let root = tempfile::tempdir().expect("tempdir");
    let output_dir = root.path().join("crates/test-lib-php/src");
    std::fs::create_dir_all(&output_dir).expect("create output dir");
    std::fs::write(
        root.path().join("crates/test-lib-php/Cargo.toml"),
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .expect("write Cargo.toml");
    let config = make_config_with_php_output(&output_dir);
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().expect("stub file must be generated");
    let content = &stubs.content;

    assert!(
        content.contains("$payload"),
        "Bytes field must still be a constructor parameter (field_can_be_param allows Bytes):\n{content}"
    );
    assert!(
        !content.contains("public readonly string $payload"),
        "Bytes has no #[php(prop)] on the real extension's struct field, so the stub must not \
         promote `payload` to a property:\n{content}"
    );
    assert!(
        content.contains("public function getPayload(): string"),
        "Bytes field must still get a getter stub:\n{content}"
    );
}

/// Since commit a8da43884 ("accept nested struct, Vec<struct> and Json constructor params"),
/// `php_field_can_be_constructor_param`'s `Json` arm answers `true` for a bare or `Optional`
/// `Json` field — it decodes a JSON `String` param via `serde_json::from_str` on the real
/// extension's `#[php(constructor)]` (see `json_constructor_param_tests.rs`'s
/// `optional_json_is_representable` / `optional_json_field_also_wraps_constructor_in_php_result`).
/// The stub shares that exact predicate (this file's own header comment on the sibling test
/// above), so it must agree: `value` is a plain (non-promoted) constructor parameter here too,
/// not the excluded-and-getter-only shape the sibling `Vec<Json>` test pins. ~keep
#[test]
fn optional_json_field_is_a_plain_constructor_param_and_reachable_via_getter() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                make_field("value", TypeRef::Json, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let root = tempfile::tempdir().expect("tempdir");
    let output_dir = root.path().join("crates/test-lib-php/src");
    std::fs::create_dir_all(&output_dir).expect("create output dir");
    std::fs::write(
        root.path().join("crates/test-lib-php/Cargo.toml"),
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .expect("write Cargo.toml");
    let config = make_config_with_php_output(&output_dir);
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().expect("stub file must be generated");
    let content = &stubs.content;

    assert!(
        content.contains("?string $value = null"),
        "Optional(Json) field `value` must be a plain, nullable constructor parameter \
         (field_can_be_param's Json arm allows it):\n{content}"
    );
    assert!(
        !content.contains("public readonly ?string $value"),
        "Json has no #[php(prop)] on the real extension's struct field, so the stub must not \
         promote `value` to a property:\n{content}"
    );
    assert!(
        content.contains("public function getValue(): ?string"),
        "Optional(Json) field must still get a getter stub:\n{content}"
    );
}
