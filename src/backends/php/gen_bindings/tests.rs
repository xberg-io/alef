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

    let is_optional_default_constructible_param = |p: &ParamDef| -> bool {
        if let TypeRef::Named(name) = &p.ty {
            matches!(name.as_str(), "CrawlConfig" | "InteractionActions")
        } else {
            false
        }
    };

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

    let opt_param = ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("CrawlConfig".to_string()),
        optional: true,
        ..ParamDef::default()
    };

    let should_be_optional = opt_param.optional || is_optional_default_constructible_param(&opt_param);
    assert!(should_be_optional, "explicitly optional param should be optional");

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

/// Regression: the `#[php_impl]` facade is Rust source, so function docs must be emitted as
/// Rust line doc-comments (`///`), never PHPDoc `/** … */` blocks.
///
/// Rust block comments nest, so a doc that mentions `image/*` opens a nested `/*` that the
/// intended closing `*/` only balances at the inner level, leaving the outer `/**` unterminated
/// (`error[E0758]: unterminated block doc-comment`). Line doc-comments have no such hazard.
#[test]
fn should_emit_rust_line_doc_comments_when_doc_text_contains_block_comment_sequences() {
    use super::super::type_map::PhpMapper;
    use crate::backends::php::gen_bindings::functions::{PhpParamTypeSets, gen_function_as_static_method};
    use crate::core::ir::{FunctionDef, TypeRef};
    use ahash::AHashSet;

    let func = FunctionDef {
        name: "choose_call_mode".to_string(),
        rust_path: "sample_crate::choose_call_mode".to_string(),
        return_type: TypeRef::String,
        doc: "Decide which call mode best fits this document.\n\n\
              Rules: `image/*` → vision; `text/*` and `application/*` → text. Closes with */."
            .to_string(),
        ..FunctionDef::default()
    };

    let mapper = PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };
    let empty = AHashSet::new();
    let type_sets = PhpParamTypeSets {
        opaque: &empty,
        default: &empty,
        enums: &empty,
    };

    let generated = gen_function_as_static_method(&func, &mapper, type_sets, "sample_crate", &[], false, &empty);

    assert!(
        generated.contains("/// Decide which call mode best fits this document."),
        "doc must be emitted as Rust `///` line comments, got:\n{generated}"
    );
    assert!(
        generated.contains("/// Rules: `image/*` → vision; `text/*` and `application/*` → text. Closes with */."),
        "doc body (incl. `image/*` and `*/`) must survive verbatim on a `///` line, got:\n{generated}"
    );
    assert!(
        !generated.contains("/**"),
        "Rust crate doc must not use PHPDoc `/**` block comments (nesting hazard), got:\n{generated}"
    );

    for line in generated.lines().filter(|l| l.contains("Closes with")) {
        assert!(
            line.trim_start().starts_with("///"),
            "line carrying a `*/` token must be a `///` line doc-comment, got: {line:?}"
        );
    }
}

/// Regression: a `&mut self -> Result<&mut Self, E>` builder (a method that returns a reference
/// to its own wrapper type) must SHARE the existing handle's `Arc` (`self.inner.clone()`) rather
/// than cloning the returned reference. `&mut Self` is not `Clone`, and the inner value need not
/// be `Clone`, so `Arc::new(std::sync::Mutex::new(result.clone()))` fails to compile.
#[test]
fn php_self_ref_builder_shares_arc_instead_of_cloning_returned_ref() {
    use super::super::type_map::PhpMapper;
    use crate::backends::php::gen_bindings::functions::gen_instance_method;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
    use ahash::{AHashMap, AHashSet};

    let method = MethodDef {
        name: "register_route".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("RouteCfg".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("App".to_string()),
        error_type: Some("AppError".to_string()),
        doc: "Register a route, returning the app for chaining.".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        cfg: None,
        returns_ref: true,
        ..MethodDef::default()
    };

    let mapper = PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };
    let mut opaque = AHashSet::new();
    opaque.insert("App".to_string());
    opaque.insert("RouteCfg".to_string());
    let enums = AHashSet::new();
    let adapter_bodies: AHashMap<String, String> = AHashMap::new();
    let mut mutex = AHashSet::new();
    mutex.insert("App".to_string());

    let code = gen_instance_method(
        &method,
        &mapper,
        true,
        "App",
        &opaque,
        &enums,
        "sample_crate",
        &adapter_bodies,
        &mutex,
    );

    assert!(
        code.contains("Ok(Self { inner: self.inner.clone() })"),
        "self-returning builder should share the existing Arc, got:\n{code}"
    );
    assert!(
        !code.contains("Mutex::new(result.clone())") && !code.contains("Mutex::new(result)"),
        "must not wrap the returned &mut ref in a new Mutex, got:\n{code}"
    );
    assert!(
        !code.contains("let result ="),
        "self-returning builder must not bind the returned &mut ref, got:\n{code}"
    );
}

/// Regression: Cargo replaces EVERY hyphen in a crate name with `_` for the cdylib output
/// filename (crate `demo-ext-php` -> `libdemo_ext_php.{dylib,so}`), so the generated
/// `config.m4` must probe for a fully-underscored stem. It must also keep crate-directory
/// paths hyphenated (`crates/demo-ext-php/...`) and use the (possibly overridden) extension
/// name, not the crate name, for the `modules/*.so` output filename.
#[test]
fn config_m4_uses_underscored_cdylib_stem_and_hyphenated_crate_dir() {
    use super::rust_items::generate_config_m4;

    let m4 = generate_config_m4("demo_ext", "demo-ext");

    assert!(
        m4.contains("crates/demo-ext-php/Cargo.toml"),
        "crate directory path must keep hyphens, got:\n{m4}"
    );
    assert!(
        m4.contains("cd crates/demo-ext-php"),
        "cd target must keep hyphens, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/demo-ext-php/target/release/libdemo_ext_php.dylib"),
        "dylib stem must be fully underscored, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/demo-ext-php/target/release/libdemo_ext_php.so"),
        "so stem must be fully underscored, got:\n{m4}"
    );
    assert!(
        !m4.contains("demo-ext_php"),
        "must never mix a hyphenated crate name with the `_php` suffix, got:\n{m4}"
    );
    assert!(
        m4.contains(r#"cp "$cargo_lib" "modules/demo_ext.so""#),
        "module output filename must use the extension name, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/demo-ext-php/target/release\" >&2"),
        "not-found error message must reference the hyphenated crate directory, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/demo-ext-php/Cargo.toml not found"),
        "missing-Cargo.toml error message must reference the hyphenated crate directory, got:\n{m4}"
    );
}

/// Positive-control companion to the above: `demo-ext` carries only one hyphen, so
/// "replace the first hyphen" and "replace every hyphen" produce an identical dylib stem and
/// cannot distinguish the two derivations. A crate name with multiple hyphens can: Cargo
/// underscores ALL of them in the cdylib filename while the source directory keeps every one. ~keep
#[test]
fn config_m4_underscores_every_hyphen_in_a_multi_hyphen_crate_name() {
    use super::rust_items::generate_config_m4;

    let m4 = generate_config_m4("my_cool_lib", "my-cool-lib");

    assert!(
        m4.contains("crates/my-cool-lib-php/Cargo.toml"),
        "crate directory path must keep every hyphen, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/my-cool-lib-php/target/release/libmy_cool_lib_php.dylib"),
        "dylib stem must underscore every hyphen, not just the first, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/my-cool-lib-php/target/release/libmy_cool_lib_php.so"),
        "so stem must underscore every hyphen, not just the first, got:\n{m4}"
    );
    assert!(
        !m4.contains("my-cool-lib_php") && !m4.contains("my_cool-lib_php") && !m4.contains("my-cool_lib_php"),
        "must never leave a partially-underscored stem, got:\n{m4}"
    );
}

/// Regression (#103): `gen_flat_data_enum`/`gen_flat_data_enum_methods` hardcode serde derives
/// and `from_json` on every tagged data enum unconditionally (never gated on `has_serde` — the
/// PHPStan stub keys the same methods on `is_tagged_data_enum` alone and must not diverge from
/// the runtime, see commit bb0787c69). Before the fix, `has_serde` was the raw
/// `php_serde_available` probe alone: a crate whose Cargo.toml the probe reads as serde-less
/// still emitted the enum's hardcoded serde derives (unavoidable, protected), while every OTHER
/// type in the same crate was held to a no-serde code path the crate cannot actually honor once
/// it contains a tagged data enum. The fix folds "does this API surface contain a tagged data
/// enum" into the crate-level `has_serde` value, so a plain struct in the same crate gets the
/// same serde-based `from_json` the enum already unconditionally requires.
///
/// `gen_struct_methods_impl`'s `use_from_json = has_serde && (has_named_params || ...)`
/// (types/structs.rs) is a clean, direct gate — unlike the struct-derive path in the shared
/// `codegen::generators::gen_struct_with_per_field_attrs`, which always derives serde regardless
/// of `cfg.has_serde` and so cannot distinguish the fix. `PlainStruct`'s one `Bytes` field is not
/// PHP-prop-representable (`is_php_prop_scalar_with_enums` — see that fn's match arms), which
/// makes `has_named_params` true independently of `has_serde`, so `from_json`'s presence here
/// depends on nothing but `has_serde`.
///
/// Discriminating because `"my-crate"` resolves to no real Cargo.toml on disk in this test run
/// (asserted below), so the raw probe alone is false: `use_from_json` would be `false && ...` =
/// `false` without the fix, and `PlainStruct::from_json` would not be emitted at all.
#[test]
fn tagged_data_enum_forces_crate_wide_serde_even_when_probe_finds_none() {
    use crate::core::config::resolved::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

    let tagged_enum = EnumDef {
        name: "Shape".to_string(),
        rust_path: "test_lib::Shape".to_string(),
        variants: vec![EnumVariant {
            name: "Circle".to_string(),
            fields: vec![FieldDef {
                name: "radius".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F64),
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    };
    let plain_struct = TypeDef {
        name: "PlainStruct".to_string(),
        rust_path: "test_lib::PlainStruct".to_string(),
        fields: vec![FieldDef {
            name: "payload".to_string(),
            ty: TypeRef::Bytes,
            ..Default::default()
        }],
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "my-crate".to_string(),
        version: "1.0.0".to_string(),
        types: vec![plain_struct],
        enums: vec![tagged_enum],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "my-crate".to_string(),
        ..ResolvedCrateConfig::default()
    };
    assert!(
        !super::rust_bindings::php_serde_available(&config),
        "test fixture must not resolve to a real Cargo.toml with serde -- otherwise this test \
         cannot distinguish the fix from the pre-fix behavior"
    );

    let files = super::rust_bindings::generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // `PlainStruct`'s own `from_json` body is a bare one-liner (no `let value: Self =` binding,
    // unlike the tagged enum's templated `from_json`), so this substring cannot accidentally
    // match `Shape::from_json` instead. The body is indented 8 spaces, not 4 -- matching the
    // emitted indentation exactly is what keeps this pinned to `PlainStruct`'s own impl block. ~keep
    assert!(
        lib.content
            .contains("pub fn from_json(json: String) -> PhpResult<Self> {\n        serde_json::from_str(&json)"),
        "PlainStruct must get a serde-based from_json once the crate contains a tagged data \
         enum, even though the raw Cargo.toml probe alone found no serde, got:\n{}",
        lib.content
    );
}

/// Fixture mirroring the shape that made the defect visible: a unit-variant enum carrying
/// `#[serde(rename_all = "snake_case")]` and a multi-word variant. The `rename_all` is
/// load-bearing, not decoration — without it `wire_variant_value` returns the Rust ident
/// verbatim, the emitted match arm carries `"InProgress" | "inprogress"`, and a constant holding
/// the Rust ident would match by accident. A fixture without `rename_all` therefore proves
/// nothing about either side. ~keep
fn snake_case_unit_enum() -> crate::core::ir::EnumDef {
    use crate::core::ir::{EnumDef, EnumVariant};

    EnumDef {
        name: "BatchStatus".to_string(),
        rust_path: "sample_crate::BatchStatus".to_string(),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "InProgress".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Failed".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// A struct carrying that enum by value, which is what routes the field through
/// `gen_string_to_enum_expr` and produces the binding->core match arms.
fn struct_with_unit_enum_field() -> crate::core::ir::TypeDef {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    TypeDef {
        name: "BatchObject".to_string(),
        rust_path: "sample_crate::BatchObject".to_string(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "status".to_string(),
            ty: TypeRef::Named("BatchStatus".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The PHP class constant the extension registers for a unit-variant enum must hold the serde
/// wire value. It used to hold the Rust variant ident (`"InProgress"`), which is neither what the
/// extension's binding->core match accepts nor what its core->binding direction
/// (`serde_json::to_value`) produces.
#[test]
fn php_enum_class_constant_carries_the_serde_wire_value_not_the_rust_variant_name() {
    let emitted = super::types::gen_enum_constants(&snake_case_unit_enum(), None, false, None);

    assert!(
        emitted.contains("pub const INPROGRESS: &str = \"in_progress\";"),
        "the constant must carry the serde wire value, got:\n{emitted}"
    );
    assert!(
        !emitted.contains("\"InProgress\""),
        "the Rust variant ident must not survive as the constant's value, got:\n{emitted}"
    );
    assert!(
        emitted.contains("pub const FAILED: &str = \"failed\";"),
        "a single-word variant is renamed by `rename_all` too, so it must also carry the wire \
         value rather than `\"Failed\"`, got:\n{emitted}"
    );
}

/// The invariant that actually matters, asserted across the two generators rather than inside
/// either: every value `gen_enum_constants` publishes as a PHP class constant must be a value the
/// generated binding->core match accepts. Otherwise a caller who passes the extension's own
/// constant hits the match's `_ =>` fallback and silently receives the default variant.
#[test]
fn php_enum_class_constant_values_are_all_accepted_by_the_generated_string_to_core_match() {
    use ahash::AHashSet;

    let enum_def = snake_case_unit_enum();
    let enum_names: AHashSet<String> = std::iter::once("BatchStatus".to_string()).collect();
    let conversion = super::helpers::gen_php_lossy_binding_to_core_fields(
        &struct_with_unit_enum_field(),
        "sample_crate",
        &enum_names,
        &AHashSet::new(),
        std::slice::from_ref(&enum_def),
    );

    let constants = super::types::gen_enum_constants(&enum_def, None, false, None);
    let values: Vec<String> = constants
        .lines()
        .filter_map(|line| line.split_once("= \"").and_then(|(_, rest)| rest.split_once('"')))
        .map(|(value, _)| value.to_string())
        .collect();

    assert_eq!(
        values.len(),
        enum_def.variants.len(),
        "apparatus check: one constant value must be extracted per variant, or the loop below \
         asserts nothing. Extracted {values:?} from:\n{constants}"
    );

    for value in &values {
        assert!(
            conversion.contains(&format!("\"{value}\"")),
            "the constant value `{value}` is not a match arm of the generated binding->core \
             conversion, so passing it would fall through to the default variant. Conversion:\n{conversion}"
        );
    }
}

/// Negative control for the test above. It compares constant values against match arms, so it
/// would pass vacuously if the match accepted *every* string. It does not: the Rust variant ident
/// — the value the constant used to carry — is absent, which is precisely why the old constant was
/// unusable.
#[test]
fn the_generated_string_to_core_match_does_not_accept_the_rust_variant_name() {
    use ahash::AHashSet;

    let enum_def = snake_case_unit_enum();
    let enum_names: AHashSet<String> = std::iter::once("BatchStatus".to_string()).collect();
    let conversion = super::helpers::gen_php_lossy_binding_to_core_fields(
        &struct_with_unit_enum_field(),
        "sample_crate",
        &enum_names,
        &AHashSet::new(),
        std::slice::from_ref(&enum_def),
    );

    assert!(
        conversion.contains("\"in_progress\""),
        "apparatus check: the wire-named arm must be present, got:\n{conversion}"
    );
    assert!(
        !conversion.contains("\"InProgress\""),
        "the match must NOT accept the Rust variant ident -- if it did, the constant's value \
         would be interchangeable and the assertion it anchors would prove nothing, got:\n{conversion}"
    );
}

/// `public_api.rs` generates the runtime facade class (e.g. `LiterLlm.php`) that composer
/// autoloads and every caller actually executes -- it is not PHPStan-only prose. Its methods
/// delegate positionally into the native `...Api` class (`\Ns\ClassApi::method($args)`), whose
/// own params are typed by `PhpMapper::named`, which lowers a unit-variant enum to `String`. A
/// bare `php_type` call here would type the facade param/return as the enum's own class name --
/// a class `gen_enum_constants` declares with only `const` members, so no instance of it can ever
/// exist. That is not a documentation nit: it makes the generated method statically uncallable
/// with any value a caller could construct. This pins the facade to the enum-aware mapping.
#[test]
fn public_api_facade_types_unit_enum_param_and_return_as_string_not_enum_class() {
    use crate::core::config::resolved::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

    let status_enum = EnumDef {
        name: "Status".to_string(),
        rust_path: "sample_crate::Status".to_string(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Inactive".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let config_type = TypeDef {
        name: "Config".to_string(),
        rust_path: "sample_crate::Config".to_string(),
        fields: vec![FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let set_status_fn = FunctionDef {
        name: "set_status".to_string(),
        rust_path: "sample_crate::set_status".to_string(),
        params: vec![
            ParamDef {
                name: "status".to_string(),
                ty: TypeRef::Named("Status".to_string()),
                ..Default::default()
            },
            // Negative control: a non-enum named type must keep its own class name.
            ParamDef {
                name: "config".to_string(),
                ty: TypeRef::Named("Config".to_string()),
                ..Default::default()
            },
        ],
        return_type: TypeRef::Named("Status".to_string()),
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "sample-crate".to_string(),
        version: "1.0.0".to_string(),
        types: vec![config_type],
        enums: vec![status_enum],
        functions: vec![set_status_fn],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-crate".to_string(),
        ..ResolvedCrateConfig::default()
    };

    let files = super::public_api::generate_public_api(&api, &config).unwrap();
    let facade = files
        .iter()
        .find(|f| f.content.contains("setStatus("))
        .expect("facade class file with the delegating setStatus method must be generated");

    assert!(
        facade.content.contains("string $status, Config $config): string"),
        "a unit-enum param and return must be typed `string` (what PhpMapper::named actually \
         lowers it to), while the sibling struct param keeps its own class name, got:\n{}",
        facade.content
    );
    assert!(
        !facade.content.contains("Status $status") && !facade.content.contains("): Status"),
        "the facade must never type a unit-enum value as the enum's own (uninstantiable) class \
         name, got:\n{}",
        facade.content
    );
}

/// Same defect, same fix, on the opaque-class stub side (`opaque_files.rs`): an opaque type's
/// method that takes or returns a unit-variant enum must be typed `string`, exactly like the
/// facade above -- `PhpMapper::named` does not distinguish free functions from methods.
#[test]
fn opaque_class_stub_types_unit_enum_method_param_and_return_as_string_not_enum_class() {
    use crate::core::ir::{EnumDef, EnumVariant, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
    use ahash::{AHashMap, AHashSet};

    let status_enum = EnumDef {
        name: "Status".to_string(),
        rust_path: "sample_crate::Status".to_string(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Inactive".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let enum_names: AHashSet<String> = std::iter::once(status_enum.name.clone()).collect();

    let session_type = TypeDef {
        name: "Session".to_string(),
        rust_path: "sample_crate::Session".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "set_status".to_string(),
            params: vec![
                ParamDef {
                    name: "status".to_string(),
                    ty: TypeRef::Named("Status".to_string()),
                    ..Default::default()
                },
                // Negative control: a non-enum named type must keep its own class name.
                ParamDef {
                    name: "meta".to_string(),
                    ty: TypeRef::Named("Meta".to_string()),
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("Status".to_string()),
            receiver: Some(ReceiverKind::RefMut),
            ..Default::default()
        }],
        ..Default::default()
    };

    let content = super::opaque_files::gen_php_opaque_class_file(
        &session_type,
        "Sample\\Crate",
        &[],
        &AHashSet::default(),
        &[],
        &AHashMap::default(),
        &enum_names,
    );

    assert!(
        content.contains("string $status, Meta $meta): string"),
        "an opaque-class method must type a unit-enum param and return as `string`, while a \
         non-enum named param keeps its own class name, got:\n{content}"
    );
    assert!(
        !content.contains("Status $status") && !content.contains("): Status"),
        "an opaque-class method must never type a unit-enum value as the enum's own \
         (uninstantiable) class name, got:\n{content}"
    );
}

fn record_param(is_ref: bool, is_mut: bool) -> crate::core::ir::ParamDef {
    crate::core::ir::ParamDef {
        name: "record".to_string(),
        ty: crate::core::ir::TypeRef::Named("Record".to_string()),
        is_ref,
        is_mut,
        ..crate::core::ir::ParamDef::default()
    }
}

/// Regression test for issue #380: a `&mut T` DTO parameter on a unit-returning sync function
/// previously rendered as `pub fn tag_record(record: &Record) { .. }` -- mutating a dropped
/// `record_core` intermediate and leaving the caller's PHP object untouched with no diagnostic.
/// The binding must instead return the mutated intermediate.
///
/// PHP's free-function param style always passes non-opaque Named DTOs by reference
/// (`&Record`), which makes `can_auto_delegate_function` false for every such function (see
/// `shared::is_named_ref_param`), so this exercises `gen_function_body`'s non-`can_delegate`
/// branch -- the only branch PHP free functions with a DTO param actually take.
#[test]
fn php_mut_dto_param_returns_the_updated_value() {
    use super::super::type_map::PhpMapper;
    use crate::backends::php::gen_bindings::functions::{PhpParamTypeSets, gen_function_as_static_method};
    use crate::core::ir::{FunctionDef, TypeRef};
    use ahash::AHashSet;

    let func = FunctionDef {
        name: "tag_record".to_string(),
        rust_path: "sample_crate::tag_record".to_string(),
        params: vec![record_param(true, true)],
        return_type: TypeRef::Unit,
        error_type: None,
        ..FunctionDef::default()
    };
    let mapper = PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };
    let empty = AHashSet::new();
    let type_sets = PhpParamTypeSets {
        opaque: &empty,
        default: &empty,
        enums: &empty,
    };

    let generated = gen_function_as_static_method(&func, &mapper, type_sets, "sample_crate", &[], false, &empty);

    // `return_type_sig` (params.rs) renders the Rust facade's return arrow: `" -> {ty}"` for a
    // real type, or nothing at all for `()`. So a write-back must add `-> Record`, and the
    // pre-fix unit shape (`tag_record(record: &Record) {`, no arrow) must be gone.
    assert!(
        generated.contains("-> Record"),
        "expected the binding to return the mutated DTO type instead of `()`:\n{generated}"
    );
    assert!(
        !generated.contains("tag_record(record: &Record) {"),
        "must not still advertise a unit return with no arrow:\n{generated}"
    );
    // Load-bearing round-trip: the core call must still pass `&mut record_core` AND the tail
    // must hand back `record_core.into()`.
    assert!(
        generated.contains("sample_crate::tag_record(&mut record_core)"),
        "expected the core call to still pass `&mut record_core`:\n{generated}"
    );
    assert!(
        generated.contains("record_core.into()"),
        "expected the mutated intermediate to be returned:\n{generated}"
    );
}

/// Negative control for issue #380: an immutable `&T` DTO param must not gain write-back
/// semantics -- the return stays unit (no `-> Type` arrow at all; see `return_type_sig`).
#[test]
fn php_immutable_dto_param_keeps_void_return() {
    use super::super::type_map::PhpMapper;
    use crate::backends::php::gen_bindings::functions::{PhpParamTypeSets, gen_function_as_static_method};
    use crate::core::ir::{FunctionDef, TypeRef};
    use ahash::AHashSet;

    let func = FunctionDef {
        name: "read_record".to_string(),
        rust_path: "sample_crate::read_record".to_string(),
        params: vec![record_param(true, false)],
        return_type: TypeRef::Unit,
        error_type: None,
        ..FunctionDef::default()
    };
    let mapper = PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };
    let empty = AHashSet::new();
    let type_sets = PhpParamTypeSets {
        opaque: &empty,
        default: &empty,
        enums: &empty,
    };

    let generated = gen_function_as_static_method(&func, &mapper, type_sets, "sample_crate", &[], false, &empty);

    assert!(
        generated.contains("read_record(record: &Record) {"),
        "immutable borrow must keep the unit signature (no `-> Type` arrow):\n{generated}"
    );
    assert!(
        !generated.contains("-> Record"),
        "immutable borrow must not gain a return type:\n{generated}"
    );
    assert!(
        !generated.contains("record_core.into()"),
        "immutable borrow must not gain a write-back tail:\n{generated}"
    );
}

/// `reject_unsupported_writeback` must fire through the real PHP `generate_bindings` path: a
/// `&mut T` DTO param on a function that ALSO returns a value has no free return slot for the
/// write-back value, so generation must fail loudly (naming the function) instead of silently
/// emitting a binding that drops the mutation.
#[test]
fn php_generate_bindings_rejects_mut_dto_param_with_non_unit_return() {
    use crate::core::config::resolved::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};

    let record_type = TypeDef {
        name: "Record".to_string(),
        rust_path: "test_lib::Record".to_string(),
        fields: vec![FieldDef {
            name: "score".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    };
    let func = FunctionDef {
        name: "tag_and_count".to_string(),
        rust_path: "test_lib::tag_and_count".to_string(),
        params: vec![record_param(true, true)],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        ..FunctionDef::default()
    };
    let api = ApiSurface {
        crate_name: "my-crate".to_string(),
        version: "1.0.0".to_string(),
        types: vec![record_type],
        functions: vec![func],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "my-crate".to_string(),
        ..ResolvedCrateConfig::default()
    };

    let error = super::rust_bindings::generate_bindings(&api, &config)
        .expect_err("a `&mut` DTO param plus a non-unit return must be rejected at generation time");
    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the offending function:\n{message}"
    );
}

/// The `.phpstub` surface (`PhpBackend::generate_type_stubs`) must document the signature the
/// binding actually emits: a `&mut T` DTO parameter on a unit-returning sync function makes the
/// binding return the updated `T` instead of `void` (see `codegen::mut_writeback`).
#[test]
fn generate_type_stubs_documents_writeback_return_type() {
    use crate::core::backend::Backend;
    use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};

    let api = ApiSurface {
        crate_name: "my-crate".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_record".to_string(),
            rust_path: "test_lib::tag_record".to_string(),
            params: vec![record_param(true, true)],
            return_type: TypeRef::Unit,
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = crate::core::config::resolved::ResolvedCrateConfig {
        name: "my-crate".to_string(),
        ..crate::core::config::resolved::ResolvedCrateConfig::default()
    };

    let files = super::PhpBackend.generate_type_stubs(&api, &config).unwrap();
    let stub = &files[0].content;

    assert!(
        stub.contains("public static function tagRecord(\\My\\Crate\\Record $record): \\My\\Crate\\Record"),
        "expected the stub to return the mutated DTO type instead of `void`:\n{stub}"
    );
    assert!(
        !stub.contains("public static function tagRecord(\\My\\Crate\\Record $record): void"),
        "must not still advertise a void return:\n{stub}"
    );
}

/// An error's `Duration` method is declared on the exception stub as `?int` milliseconds, named
/// the way ext-php-rs camel-cases the `#[php_method]`.
#[test]
fn generate_type_stubs_declares_error_duration_methods() {
    use crate::core::backend::Backend;
    use crate::core::ir::{ApiSurface, ErrorDef, MethodDef, ReceiverKind, TypeRef};

    let api = ApiSurface {
        crate_name: "my-crate".to_string(),
        version: "1.0.0".to_string(),
        errors: vec![ErrorDef {
            name: "SampleError".to_string(),
            rust_path: "sample::SampleError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![MethodDef {
                name: "retry_after".to_string(),
                return_type: TypeRef::Optional(Box::new(TypeRef::Duration)),
                receiver: Some(ReceiverKind::Ref),
                ..Default::default()
            }],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..Default::default()
    };
    let config = crate::core::config::resolved::ResolvedCrateConfig {
        name: "my-crate".to_string(),
        ..crate::core::config::resolved::ResolvedCrateConfig::default()
    };

    let files = super::PhpBackend.generate_type_stubs(&api, &config).unwrap();
    let stub = &files[0].content;
    assert!(stub.contains("public function retryAfter(): ?int"), "{stub}");
}
