use super::*;
use crate::core::config::{NewAlefConfig, ReadmeConfig, ResolvedCrateConfig};

fn test_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
"#,
    )
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// Resolve a config carrying an explicit `[crates.output] ffi`, so the FFI crate's directory
/// stem and the alef crate name differ — the shape of every real consumer repo except
/// liter-llm, whose name happens to match its FFI directory, making it a vacuous test for
/// anything derived from that directory. ~keep
fn config_with_ffi_output(name: &str, ffi_output: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "{name}"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/{name}"

[crates.output]
ffi = "{ffi_output}"
"#
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// Reproduce a real consumer repo's crate layout: an alef crate `name` that differs from the
/// directory `stem` shared by its core, FFI, Node and Wasm crates, wired exactly as those repos
/// wire it (`[crates.node]`/`[crates.wasm] crate_dir`, `[crates.output] ffi`, and `sources`
/// pointing into the core crate). html-to-markdown and tree-sitter-language-pack both have this
/// shape; liter-llm's name equals its stem, so it can discriminate package names but never
/// paths. ~keep
fn consumer_shape(name: &str, stem: &str, node_package: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["ffi", "node", "wasm"]

[[crates]]
name = "{name}"
sources = ["crates/{stem}/src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/xberg-io/{name}"

[crates.node]
package_name = "{node_package}"
crate_dir = "crates/{stem}-node"

[crates.wasm]
crate_dir = "crates/{stem}-wasm"

[crates.output]
ffi = "crates/{stem}-ffi/src/"
"#
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// The two consumer shapes whose alef crate name differs from their crate directory stem, as
/// `(case, name, stem, node_package)`. Any path derived from the crate name is wrong for both,
/// and liter-llm cannot show that because its name and stem coincide. ~keep
const DISCRIMINATING_SHAPES: [(&str, &str, &str, &str); 2] = [
    (
        "html_to_markdown",
        "html-to-markdown-rs",
        "html-to-markdown",
        "@xberg-io/html-to-markdown",
    ),
    (
        "tree_sitter_language_pack",
        "tree-sitter-language-pack",
        "ts-pack-core",
        "@xberg-io/tree-sitter-language-pack",
    ),
];

fn test_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
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

mod function_surface;
mod hardcoded;
mod headings;
mod snippet_roots;
mod template;
