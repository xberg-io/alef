//! Regression coverage: the generated C# `Register<Trait>` facade must only be emitted when
//! the trait bridge actually has a native register function.
//!
//! Original defect: `gen_wrapper_class` emitted the register facade unconditionally while
//! guarding the *unregister* facade two lines below on `unregister_fn.is_some()`. A bridge
//! configured without a `register_fn` therefore produced
//!
//! ```csharp
//! var ec = NativeMethods.RegisterHtmlVisitor(name, bridge._vtable, handle, out var outError);
//! ```
//!
//! against a `NativeMethods` class that declares no such method — `CS0117`, and the whole
//! package fails to compile.
//!
//! The fix narrows emission rather than inventing a declaration. Declaring the extern would
//! be strictly worse: no `register`-shaped symbol exists in the Rust exports, the cbindgen
//! header, or the built dylib, so a declaration converts a compile-time error into a runtime
//! `EntryPointNotFoundException` — moving the failure past CI and into users' hands.
//!
//! Java (`gen_bindings/facade.rs`) and Go (`gen_bindings/mod.rs`) already gate on
//! `register_fn`; C# was the sole outlier.

use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::ApiSurface;

const TRAIT_WITHOUT_REGISTER: &str = "DocumentSink";
const TRAIT_WITH_REGISTER: &str = "TextBackend";

fn csharp_config() -> ResolvedCrateConfig {
    let toml_str = "[workspace]\nlanguages = [\"csharp\"]\n\
         [[crates]]\nname = \"sample_crate\"\nsources = [\"src/lib.rs\"]\n\
         [crates.csharp]\nnamespace = \"Sample\"\n\
         [crates.ffi]\nprefix = \"sample\"\nerror_style = \"last_error\"\n";
    let cfg: NewAlefConfig = toml::from_str(toml_str).expect("config parses");
    cfg.resolve().expect("config resolves").remove(0)
}

fn bridge(trait_name: &str, register_fn: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: register_fn.map(str::to_string),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: vec![],
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn generate(bridges: Vec<TraitBridgeConfig>) -> Vec<alef::core::backend::GeneratedFile> {
    let mut config = csharp_config();
    config.trait_bridges = bridges;
    CsharpBackend
        .generate_bindings(&empty_api(), &config)
        .expect("csharp generation succeeds")
}

fn concat(files: &[alef::core::backend::GeneratedFile]) -> String {
    files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn register_facade_is_not_emitted_when_bridge_has_no_register_fn() {
    let files = generate(vec![bridge(TRAIT_WITHOUT_REGISTER, None)]);
    let all = concat(&files);

    let call = format!("NativeMethods.Register{TRAIT_WITHOUT_REGISTER}");
    assert!(
        !all.contains(&call),
        "a bridge with no register_fn must not produce a call to {call} — \
         NativeMethods declares no such method, so this is CS0117 at build time. Output:\n{all}"
    );
}

/// The general invariant the specific case above is an instance of, mirroring the Kotlin
/// backend's `every_referenced_bridge_native_call_has_a_matching_external_fun_declaration`.
/// Stated this way it also catches future emitters that reference an undeclared P/Invoke.
///
/// ~keep Deliberately drives ONLY the no-`register_fn` bridge. A bridge that HAS a
/// `register_fn` also trips this assertion under this fixture -- but as an artifact, not a
/// defect: `NativeMethods` declarations are generated from the API surface's function list,
/// and this fixture's `ApiSurface` is empty, so `register_text_backend` has nothing to be
/// declared from. A real crate carries that function and the declaration is emitted. Including
/// that bridge here would make the test fail for a reason that has nothing to do with the bug.
#[test]
fn every_referenced_native_method_has_a_matching_declaration() {
    let files = generate(vec![bridge(TRAIT_WITHOUT_REGISTER, None)]);

    let declarations = files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with("NativeMethods.cs"))
        .map(|f| f.content.clone())
        .collect::<Vec<_>>()
        .join("\n");

    let mut missing = Vec::new();
    for file in &files {
        for (_, rest) in file
            .content
            .match_indices("NativeMethods.")
            .map(|(i, _)| (i, &file.content[i + "NativeMethods.".len()..]))
        {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.is_empty() || declarations.contains(&format!(" {name}(")) {
                continue;
            }
            missing.push(name);
        }
    }
    missing.sort();
    missing.dedup();

    assert!(
        missing.is_empty(),
        "every NativeMethods.X referenced in generated C# must have a matching extern \
         declaration in NativeMethods.cs; undeclared: {missing:?}"
    );
}

/// The fix must NARROW emission, not break it. Without this, deleting the register facade
/// outright would satisfy the two tests above while removing working functionality.
#[test]
fn register_facade_is_still_emitted_when_bridge_has_a_register_fn() {
    let files = generate(vec![bridge(TRAIT_WITH_REGISTER, Some("register_text_backend"))]);
    let all = concat(&files);

    assert!(
        all.contains(&format!("Register{TRAIT_WITH_REGISTER}")),
        "a bridge WITH a register_fn must still get its Register facade — \
         the guard must narrow emission, not remove the feature. Output:\n{all}"
    );
}
