//! An `AsyncMethod` adapter's declared return type must name the type the package publishes
//! under that word, exactly as `public_return_type_tests.rs` proves for a plain (non-adapter)
//! function -- and, before an earlier fix, an adapter did NOT hold that guarantee for a return
//! type `options.py` publishes as a public *input* dataclass (a type that is `has_default`,
//! `is_return_type = false`, and used as a parameter elsewhere in the surface).
//!
//! `emit_adapter_wrapper`'s return-conversion check (`adapter_return_converter`) used to be
//! handed only `options_dataclass_type_names` -- which already covers this shape, so that part
//! was never the bug; the bug was a NARROWER copy of that same set being consulted elsewhere and
//! disagreeing. `api.py`'s own import classification (`options_type_names` in
//! `orchestration.rs`) already consulted the type, so it still got imported from `.options` and
//! named in the `-> ReturnType` annotation -- while the wrapper body, unable to find a converter,
//! fell back to `return await engine.<name>(...)`, the untouched native `#[pyclass]`. The
//! annotation named the public type; the value was the private one.
//!
//! A pure return-position type (`is_return_type = true`) is a different case entirely: it is
//! never published by `options.py` at all, under any DTO output style -- see
//! `is_dataclass_backed_config` and `public_return_type_tests.rs`
//! (tree-sitter-language-pack#183). ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

const RETURN_TYPE: &str = "FetchOutcome";
const ADAPTER_NAME: &str = "fetch_url";
const NATIVE_ONLY_TYPE: &str = "FetchHandle";
const NATIVE_ADAPTER_NAME: &str = "open_handle";

fn config_with_adapter(returns: &str, adapter_name: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"

[crates.dto]
python = "typed-dict"

[[crates.adapters]]
name = "{adapter_name}"
pattern = "async_method"
core_path = "test_lib::Engine::{adapter_name}"
owner_type = "Engine"
returns = "{returns}"

[[crates.adapters.params]]
name = "uri"
type = "String"
"#
    ))
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

/// One `has_default`, non-`is_return_type` struct -- the shape `options.py` publishes as a
/// public *input* `@dataclass` (e.g. it is also accepted as a parameter somewhere else in the
/// real surface) which an adapter also happens to return, exactly as
/// `plain_function_dataclass_return_conversion_tests.rs`'s `ScanResult` fixture does for a plain
/// function -- and one native-pyclass-only struct that nothing in `options.py` ever publishes
/// (no `has_default`).
fn surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: RETURN_TYPE.to_string(),
                rust_path: format!("test_lib::{RETURN_TYPE}"),
                has_serde: true,
                has_default: true,
                is_return_type: false,
                fields: vec![FieldDef {
                    name: "url".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: NATIVE_ONLY_TYPE.to_string(),
                rust_path: format!("test_lib::{NATIVE_ONLY_TYPE}"),
                is_opaque: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// `api.py`, `options.py` and `__init__.py` as the backend writes them.
fn render_public_api(config: &ResolvedCrateConfig) -> (String, String, String) {
    let files = crate::backends::pyo3::Pyo3Backend
        .generate_public_api(&surface(), config)
        .expect("public API generation succeeds");
    let find = |suffix: &str| {
        files
            .iter()
            .find(|file| file.path.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} is generated"))
            .content
            .clone()
    };
    (find("api.py"), find("options.py"), find("__init__.py"))
}

/// REPRODUCTION: an adapter whose return type `options.py` publishes as a public *input*
/// `@dataclass` must convert the engine's native return value before handing it back -- the same
/// requirement a plain function wrapper already gets.
#[test]
fn an_adapter_converts_its_native_return_value_into_a_dataclass_published_type() {
    let config = config_with_adapter(RETURN_TYPE, ADAPTER_NAME);
    let (api_py, options_py, init_py) = render_public_api(&config);

    assert!(
        options_py.contains(&format!("class {RETURN_TYPE}:")),
        "the fixture must reach the case under test -- options.py has to publish {RETURN_TYPE}:\n{options_py}"
    );
    assert!(
        !options_py.contains(&format!("class {RETURN_TYPE}(TypedDict")),
        "the fixture must reach the dataclass case, not a TypedDict case -- tree-sitter-language\
         -pack#183 is exactly a return type wrongly published this way:\n{options_py}"
    );
    assert!(
        init_py.contains(&format!("from .options import {RETURN_TYPE}")),
        "__init__.py must be re-exporting the options definition:\n{init_py}"
    );
    assert!(
        api_py.contains(&format!(
            "async def {ADAPTER_NAME}(engine: Engine, uri: str) -> {RETURN_TYPE}:"
        )),
        "the wrapper must be annotated with the published name:\n{api_py}"
    );
    assert!(
        api_py.contains(&format!(
            "    return _from_native_fetch_outcome(await engine.{ADAPTER_NAME}(uri))\n"
        )),
        "the wrapper must convert the native return value before handing it back:\n{api_py}"
    );
    assert!(
        options_py.contains("def _from_native_fetch_outcome("),
        "options.py must define the converter the wrapper calls:\n{options_py}"
    );
}

/// CONTROL: an adapter whose return type `options.py` never publishes at all (a plain, opaque
/// native `#[pyclass]` with no `has_default`) must keep returning the engine's value untouched --
/// there is no public type to convert into, and applying a converter here would call an
/// undefined name.
#[test]
fn an_adapter_returning_a_type_options_py_never_publishes_stays_unconverted() {
    let config = config_with_adapter(NATIVE_ONLY_TYPE, NATIVE_ADAPTER_NAME);
    let (api_py, options_py, _init_py) = render_public_api(&config);

    assert!(
        !options_py.contains(&format!("class {NATIVE_ONLY_TYPE}")),
        "the fixture must reach the case under test -- options.py must not publish {NATIVE_ONLY_TYPE}:\n{options_py}"
    );
    assert!(
        api_py.contains(&format!("async def {NATIVE_ADAPTER_NAME}(")),
        "the wrapper must still be generated:\n{api_py}"
    );
    assert!(
        api_py.contains(&format!("    return await engine.{NATIVE_ADAPTER_NAME}(uri)\n")),
        "with nothing published under that name, the native value must pass through unconverted:\n{api_py}"
    );
    assert!(
        !api_py.contains("_from_native_fetch_handle"),
        "no converter exists for a type options.py never publishes:\n{api_py}"
    );
}
