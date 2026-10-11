//! Compile-shape and cross-artifact coverage for an `options_field` trait bridge whose handle
//! lives on a **required** config parameter (alef #476).
//!
//! `gen_bridge_field_function` had no direct test at all before this file: the one pyo3
//! options-field test sets `functions: vec![]`, so every bug in the emitted wrapper -- an
//! un-awaited async core call, a `match Some/None` against a non-`Option` parameter, a
//! hardcoded `options` identifier spliced in by a whole-file text pass -- was invisible by
//! construction. The assertions here are deliberately about the *emitted Rust*, and the
//! compile oracle behind them is `generated_output_downstream_gate`. ~keep

use std::path::Path;

use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

/// A synthetic consumer: an async entry point and a sync diagnostic, both taking the same
/// **required** config struct that carries the bridge handle.
const SOURCE: &str = r#"
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub trait ProgressListener: Send + Sync {
    fn on_step(&self, label: String);
}

pub type ProgressHandle = Arc<Mutex<dyn ProgressListener + Send + Sync>>;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RunOptions {
    pub retries: u32,
    #[serde(skip)]
    pub on_progress: Option<ProgressHandle>,
}

pub async fn execute(input: String, settings: RunOptions) -> Result<String, String> {
    let _ = (input, settings);
    Ok(String::new())
}

pub fn inspect(settings: RunOptions) -> Result<String, String> {
    let _ = settings;
    Ok(String::new())
}

pub fn convert(input: String, settings: impl Into<Option<RunOptions>>) -> Result<String, String> {
    let _ = (input, settings.into());
    Ok(String::new())
}

pub fn convert_many(input: String, settings: impl Into<Option<RunOptions>>) -> Result<String, String> {
    let _ = (input, settings.into());
    Ok(String::new())
}
"#;

const BRIDGE_TOML: &str = r#"
trait_name = "ProgressListener"
type_alias = "ProgressHandle"
param_name = "on_progress"
bind_via = "options_field"
options_type = "RunOptions"
options_field = "on_progress"
"#;

fn extract(dir: &Path) -> ApiSurface {
    let src = dir.join("lib.rs");
    std::fs::write(&src, SOURCE).expect("write fixture source");
    alef::extract::extractor::extract(&[src.as_path()], "sample_lib", "0.1.0", None).expect("extract fixture")
}

fn config_with(bridge_extra: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_sample_lib"

[crates.python.stubs]
output = "bindings/python/sample_lib"

[[crates.trait_bridges]]
{BRIDGE_TOML}{bridge_extra}
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("parse alef.toml");
    cfg.resolve().expect("resolve config").remove(0)
}

fn lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    Pyo3Backend
        .generate_bindings(api, config)
        .expect("generate pyo3 bindings")
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs")
        .content
}

/// The body of the emitted `pub fn <name>` wrapper, up to the next top-level attribute.
fn wrapper_body<'a>(lib: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("pub fn {name}");
    let start = lib.find(&marker)?;
    let rest = &lib[start..];
    Some(rest.split("\n#[").next().unwrap_or(rest))
}

#[test]
fn async_wrapper_awaits_the_core_call_inside_future_into_py() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    for (runtime, expected, absent) in [
        (
            None,
            "pyo3_async_runtimes::tokio::future_into_py",
            "alef_async_runtime::future_into_py",
        ),
        (
            Some("managed"),
            "alef_async_runtime::future_into_py",
            "pyo3_async_runtimes::tokio::future_into_py",
        ),
    ] {
        let mut config = config_with("");
        config.python.as_mut().expect("Python config").async_runtime = runtime.map(str::to_string);
        let lib = lib_rs(&api, &config);
        let body = wrapper_body(&lib, "execute").expect("execute wrapper");

        assert!(
            body.contains(&format!("{expected}(py, async move {{")),
            "the options-field wrapper must use the selected runtime {runtime:?}; body:\n{body}"
        );
        assert!(
            !body.contains(absent),
            "the options-field wrapper must not use the other runtime {runtime:?}; body:\n{body}"
        );
        assert!(
            body.contains("sample_lib::execute(input, settings_core).await"),
            "the async options-field wrapper must await the core call in runtime {runtime:?}; body:\n{body}"
        );
    }
}

#[test]
fn required_options_parameter_is_converted_without_an_option_match() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with(""));

    for name in ["execute", "inspect"] {
        let body = wrapper_body(&lib, name).expect("wrapper");
        assert!(
            body.contains("let mut settings_core: sample_lib::RunOptions = settings.into();"),
            "`{name}` takes a REQUIRED `RunOptions`, so the wrapper must convert it directly; \
             body:\n{body}"
        );
        assert!(
            !body.contains("Some(opts) => opts.clone().into()"),
            "`{name}` must not pattern-match `Some`/`None` against a non-`Option` parameter \
             (E0308); body:\n{body}"
        );
    }
}

#[test]
fn every_wrapper_reads_the_fallback_off_its_own_parameter_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with(""));

    assert!(
        !lib.contains("options.as_ref().and_then("),
        "the fallback must use the wrapper's own parameter name, never a hardcoded `options`; \
         lib.rs:\n{lib}"
    );

    // Both wrappers, not just the first: the deleted postprocess pass spliced at the FIRST
    // match in the whole file, so with two functions exactly one got patched. ~keep
    let patched = ["execute", "inspect"]
        .iter()
        .filter(|name| {
            wrapper_body(&lib, name)
                .expect("wrapper")
                .contains("settings.on_progress.as_ref()")
        })
        .count();
    assert_eq!(
        patched, 2,
        "both options-field wrappers must carry the handle fallback, found {patched}; lib.rs:\n{lib}"
    );
}

#[test]
fn generic_optional_options_are_not_wrapped_twice() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with(""));
    let body = wrapper_body(&lib, "convert").expect("convert wrapper");

    assert!(
        body.contains("sample_lib::convert(input, settings_core)"),
        "the core call must pass its existing Option<RunOptions> directly; body:\n{body}"
    );
    assert!(
        !body.contains("Some(settings_core)"),
        "wrapping an Option<RunOptions> produces Option<Option<RunOptions>>; body:\n{body}"
    );
}

#[test]
fn optional_default_rewrite_is_scoped_to_the_non_bridge_function() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with("exclude_functions = [\"convert\"]\n"));
    let bridge_body = wrapper_body(&lib, "execute").expect("execute wrapper");
    let excluded_body = wrapper_body(&lib, "convert").expect("convert wrapper");
    let prefixed_bridge_body = wrapper_body(&lib, "convert_many").expect("convert_many wrapper");

    assert!(
        !bridge_body.contains("Some(settings_core)"),
        "a rewrite required by another function must not create a nested optional in the bridge wrapper:\n{bridge_body}"
    );
    assert!(
        excluded_body.contains("sample_lib::convert(input, Some(settings_core))"),
        "the excluded non-bridge wrapper still needs its optional default adapted:\n{excluded_body}"
    );
    assert!(
        !prefixed_bridge_body.contains("Some(settings_core)"),
        "an exact-name rewrite for `convert` must not modify the `convert_many` bridge wrapper:\n{prefixed_bridge_body}"
    );
}

#[test]
fn a_bare_callable_is_rejected_by_name_instead_of_being_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with(""));
    let body = wrapper_body(&lib, "execute").expect("execute wrapper");

    assert!(
        body.contains("bound.hasattr(*name).unwrap_or(false)") && body.contains("\"on_step\", "),
        "the wrapper must check the host object provides a `ProgressListener` method; body:\n{body}"
    );
    assert!(
        body.contains("PyTypeError::new_err") && body.contains("a bare callable is not accepted"),
        "the rejection must name the rule, not surface as a bare AttributeError; body:\n{body}"
    );
}

#[test]
fn excluding_python_emits_no_wrapper_and_no_bridge_reference() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with("exclude_languages = [\"python\"]\n"));

    assert!(
        !lib.contains("PyProgressListenerBridge"),
        "an excluded bridge must leave no reference to a struct no pass emitted; lib.rs:\n{lib}"
    );
    assert!(
        !lib.contains("on_progress: Option<Py<PyAny>>"),
        "an excluded bridge must not add its keyword to any wrapper; lib.rs:\n{lib}"
    );
}

/// The declared parameter list of `<kw> <name>(...)`, balanced to its closing paren.
///
/// A signature assertion that reads a fixed number of lines silently passes whenever the
/// emitter wraps differently -- the exact zero-work shape this repo keeps hitting. ~keep
fn declared_params(content: &str, name: &str) -> Option<String> {
    // `pub fn execute<'py>(` and `def execute(` both have to match, and `execute_more(` must
    // not: anchor on the name, then accept only a generic list before the paren. ~keep
    let at = ["def ", "pub fn "]
        .iter()
        .flat_map(|kw| [format!("{kw}{name}("), format!("{kw}{name}<")])
        .filter_map(|marker| content.find(&marker))
        .min()?;
    let open = content[at..].find('(')? + at;
    let mut depth = 0usize;
    for (offset, ch) in content[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(content[open..open + offset + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// The parity assertion the whole four-predicate collapse exists for.
///
/// `exclude_functions` has to be honoured by the `#[pyfunction]`, the `.pyi` stub and the
/// `api.py` facade *identically*. Teaching only one of them ships a package whose stub
/// promises a keyword the extension module will reject. ~keep
#[test]
fn exclude_functions_is_honoured_by_the_pyfunction_the_stub_and_the_facade() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let config = config_with("exclude_functions = [\"inspect\"]\n");

    let lib = lib_rs(&api, &config);
    let stub = Pyo3Backend
        .generate_type_stubs(&api, &config)
        .expect("generate stubs")
        .into_iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "pyi"))
        .expect(".pyi")
        .content;
    let facade = Pyo3Backend
        .generate_public_api(&api, &config)
        .expect("generate public api")
        .into_iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py")
        .content;

    for (label, content) in [("lib.rs", &lib), (".pyi", &stub), ("api.py", &facade)] {
        let kept = declared_params(content, "execute")
            .unwrap_or_else(|| panic!("{label} must declare `execute`; content:\n{content}"));
        assert!(
            kept.contains("on_progress"),
            "{label}: `execute` must keep the bridge keyword; params:\n{kept}"
        );

        let excluded = declared_params(content, "inspect")
            .unwrap_or_else(|| panic!("{label} must declare `inspect`; content:\n{content}"));
        assert!(
            !excluded.contains("on_progress"),
            "{label}: `inspect` is in `exclude_functions`, so it must NOT gain the bridge \
             keyword -- this artifact disagrees with the others; params:\n{excluded}"
        );
    }
}
