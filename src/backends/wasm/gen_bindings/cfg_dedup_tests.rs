//! Regression coverage for `drop_cfg_disabled_functions`: dedup must never collapse a
//! mutually-exclusive real/stub cfg pair into a tautologically-gated call to the disabled
//! variant's `rust_path`. See the function's own doc comment in `cfg.rs` for the full mechanism.

use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeRef};

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn config_with_features(features_line: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        "\n[workspace]\nlanguages = [\"wasm\"]\n[[crates]]\nname = \"test-lib\"\nsources = \
         [\"src/lib.rs\"]\n[crates.wasm]\n{features_line}\n"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A same-named real-impl (`feature = "gated"`) / stub (`not(feature = "gated")`) pair, with
/// DIFFERENT `rust_path`s -- the shape a crate-root re-export clone does NOT produce, and the one
/// `dedup_same_name_functions`'s OR-merge is unsafe for.
fn real_and_stub_variants() -> (FunctionDef, FunctionDef) {
    let real = FunctionDef {
        name: "classify".to_string(),
        rust_path: "test_lib::deep::classify".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::I32),
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::I32),
        cfg: Some(r#"feature = "gated""#.to_string()),
        ..Default::default()
    };
    let stub = FunctionDef {
        name: "classify".to_string(),
        rust_path: "test_lib::classify".to_string(),
        params: vec![ParamDef {
            name: "_value".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::I32),
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::I32),
        cfg: Some(r#"not(feature = "gated")"#.to_string()),
        ..Default::default()
    };
    (real, stub)
}

fn lib_rs_of(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

/// The bug this guards: with the "gated" feature OFF, the disabled real variant's `rust_path`
/// (`test_lib::deep::classify`) must never appear in the generated source. Before the fix, dedup
/// ran first, OR-merged the pair's cfg into a tautology (`any(feature = "gated", not(feature =
/// "gated"))`), picked the real variant's params as canonical (non-underscore beats the stub
/// convention), and `prepend_cfg` emitted that always-true gate around a call to
/// `test_lib::deep::classify` -- a module the core crate compiles out whenever "gated" is off.
#[test]
fn disabled_real_variant_rust_path_is_never_called() {
    let (real, stub) = real_and_stub_variants();
    let api = ApiSurface {
        functions: vec![real, stub],
        ..empty_api()
    };
    let config = config_with_features("");

    let files = crate::backends::wasm::WasmBackend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let lib_rs = lib_rs_of(&files);

    assert!(
        !lib_rs.contains("test_lib::deep::classify"),
        "the disabled real variant's rust_path must never be called unconditionally:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("test_lib::classify"),
        "the surviving stub variant must still be emitted:\n{lib_rs}"
    );
}

/// The mirror direction, and the negative control for the test above: with "gated" ON, the real
/// variant is the one that must survive and be called through its own real `rust_path` -- proving
/// the fix does not simply drop the pair wholesale, only reconciles which one calls what.
#[test]
fn enabled_real_variant_rust_path_is_called_not_the_stub() {
    let (real, stub) = real_and_stub_variants();
    let api = ApiSurface {
        functions: vec![real, stub],
        ..empty_api()
    };
    let config = config_with_features("features = [\"gated\"]");

    let files = crate::backends::wasm::WasmBackend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let lib_rs = lib_rs_of(&files);

    assert!(
        lib_rs.contains("test_lib::deep::classify"),
        "the enabled real variant must be called through its own rust_path:\n{lib_rs}"
    );
}

/// Negative control for the whole mechanism: a single, ungated function sharing no name with
/// anything else must reach dedup as a no-op group of one and emit with no `#[cfg(...)]` at all --
/// proving `drop_cfg_disabled_functions` only ever removes genuinely disabled cfg variants, never
/// an unrelated, unconditional function.
#[test]
fn ungated_function_is_emitted_with_no_cfg_gate() {
    let plain = FunctionDef {
        name: "always_on".to_string(),
        rust_path: "test_lib::always_on".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::I32),
        cfg: None,
        ..Default::default()
    };
    let api = ApiSurface {
        functions: vec![plain],
        ..empty_api()
    };
    let config = config_with_features("");

    let files = crate::backends::wasm::WasmBackend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let lib_rs = lib_rs_of(&files);

    assert!(
        lib_rs.contains("test_lib::always_on"),
        "an ungated function must still be emitted:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("#[cfg("),
        "an ungated function must never gain a synthetic #[cfg(...)] gate:\n{lib_rs}"
    );
}
