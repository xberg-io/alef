//! The mixed case: one language excluded from an `options_field` trait bridge, the rest not.
//!
//! `exclude_languages` used to suppress the bridge but keep the carrier FIELD on every
//! language's config DTO, so an excluded backend exposed a field typed as a bridge handle it
//! had no way to construct -- reported for node, elixir and dart, and measured for pyo3
//! (alef #480). Pruning the field per language is only correct if it stays per language: the
//! excluded backend must lose it and every other backend must keep it, from the SAME run.
//!
//! A uniform fixture cannot show that. If the prune leaked across languages, a
//! python-only or node-only assertion would still pass. Both halves are asserted here, from
//! one `pipeline::generate` call, for that reason. ~keep

use alef::cli::pipeline;
use alef::core::config::{Language, NewAlefConfig};

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

pub fn inspect(settings: RunOptions) -> Result<String, String> {
    let _ = settings;
    Ok(String::new())
}
"#;

const CONFIG: &str = r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "toolkit"
sources = ["__SOURCE__"]
version_from = "__VERSION__"

[crates.python]
module_name = "_toolkit"

[[crates.trait_bridges]]
trait_name = "ProgressListener"
type_alias = "ProgressHandle"
param_name = "on_progress"
bind_via = "options_field"
options_type = "RunOptions"
options_field = "on_progress"
exclude_languages = ["python"]
"#;

fn toml_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn generate_both() -> Vec<(Language, Vec<alef::core::backend::GeneratedFile>)> {
    let dir = tempfile::tempdir().expect("fixture dir");
    let source = dir.path().join("lib.rs");
    let manifest = dir.path().join("Cargo.toml");
    let config_path = dir.path().join("alef.toml");
    std::fs::write(&source, SOURCE).expect("write source");
    std::fs::write(&manifest, "[package]\nname = \"toolkit\"\nversion = \"0.1.0\"\n").expect("write manifest");
    let text = CONFIG
        .replace("__SOURCE__", &toml_path(&source))
        .replace("__VERSION__", &toml_path(&manifest));
    std::fs::write(&config_path, &text).expect("write alef.toml");

    let raw: NewAlefConfig = toml::from_str(&text).expect("parse alef.toml");
    let resolved = raw.resolve().expect("resolve config").remove(0);
    let api = pipeline::extract(&resolved, &config_path, true).expect("extract fixture");

    pipeline::generate(
        &api,
        &resolved,
        &[Language::Python, Language::Node],
        true,
        &config_path,
        false,
    )
    .expect("generate both languages")
}

/// Concatenated content of every generated file for `language`, so an assertion cannot pass
/// by looking at the one artifact that happens to agree. ~keep
fn surface(files: &[(Language, Vec<alef::core::backend::GeneratedFile>)], language: Language) -> String {
    let (_, emitted) = files
        .iter()
        .find(|(lang, _)| *lang == language)
        .unwrap_or_else(|| panic!("`{language}` produced no output at all, so this test examined nothing"));
    assert!(
        !emitted.is_empty(),
        "`{language}` produced an empty file list, so this test examined nothing"
    );
    emitted
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of `struct <name> { ... }` in `surface`, or `None` when it is not declared.
///
/// Asserting on the whole concatenated surface is not enough and passed for the wrong reason
/// once already: node's bridge is ACTIVE, so its wrapper and registration code mention
/// `on_progress` regardless of whether the DTO still carries the field. A language-blind
/// prune left that string in place and a `surface.contains(..)` assertion stayed green. The
/// question is what the DTO declares, so the assertion has to read the DTO. ~keep
fn struct_body<'a>(surface: &'a str, name: &str) -> Option<&'a str> {
    let at = surface.find(&format!("struct {name} {{"))?;
    let open = surface[at..].find('{')? + at;
    let close = surface[open..].find('}')? + open;
    Some(&surface[open..=close])
}

#[test]
fn an_excluded_language_loses_the_carrier_while_every_other_language_keeps_it() {
    let generated = generate_both();

    let python = surface(&generated, Language::Python);
    let py_dto = struct_body(&python, "RunOptions").expect("pyo3 must emit a `RunOptions` mirror struct");
    assert!(
        !py_dto.contains("on_progress"),
        "python is in `exclude_languages`, so its config DTO must not carry the bridge's \
         field -- it has no bridge with which to construct a `ProgressHandle`; struct:\n{py_dto}"
    );

    let node = surface(&generated, Language::Node);
    let node_dto = struct_body(&node, "JsRunOptions").expect("napi must emit a `JsRunOptions` mirror struct");
    assert!(
        node_dto.contains("on_progress"),
        "node is NOT excluded, so its config DTO must KEEP the carrier -- a prune that removes \
         it here has leaked across languages and stripped the field from every backend the \
         bridge still targets; struct:\n{node_dto}"
    );
}
