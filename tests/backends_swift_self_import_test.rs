//! A generated Swift file must never import the SwiftPM module it is itself part of.
//!
//! Swift answers a self-import with `file ... is part of module '<M>'; ignoring import`, which is
//! harmless under a default build and fatal under warnings-as-errors. The controls below assert
//! the opposite direction too: imports of *other* targets must survive, so a fix that simply
//! drops every import cannot pass.

use alef::backends::swift::SwiftBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

const SWIFT_SOURCES_DIR: &str = "Sources";

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, fallible: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: if fallible {
            Some("SampleError".to_string())
        } else {
            None
        },
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_type(name: &str, is_trait: bool, methods: Vec<MethodDef>, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        original_rust_path: String::new(),
        fields,
        methods,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait,
        has_default: !is_trait,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: !is_trait,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: CoreWrapper::None,
    }
}

/// A crate with one serde struct (so the facade genuinely needs `import RustBridge`) and one
/// bridged trait (so the trait-bridge protocol/adapter files land in `Sources/RustBridge/`).
fn sample_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample".into(),
        version: "0.1.0".into(),
        types: vec![
            make_type(
                "SampleConfig",
                false,
                vec![],
                vec![make_field("label", TypeRef::String)],
            ),
            make_type(
                "SampleHandler",
                true,
                vec![
                    make_method(
                        "handle",
                        vec![make_param("payload", TypeRef::String)],
                        TypeRef::Unit,
                        true,
                    ),
                    make_method("describe", vec![], TypeRef::String, true),
                ],
                vec![],
            ),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn sample_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "SampleHandler"
registry_getter = "sample::registry::get_sample_handler_registry"
register_fn = "register_sample_handler"
unregister_fn = "unregister_sample_handler"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn generate() -> Vec<GeneratedFile> {
    SwiftBackend
        .generate_bindings(&sample_api(), &sample_config())
        .expect("swift bindings must generate")
}

/// The SwiftPM module a generated file belongs to: the directory under `Sources/`.
fn owning_module(file: &GeneratedFile) -> Option<String> {
    if file.path.extension()?.to_str()? != "swift" {
        return None;
    }
    let target_dir = file.path.parent()?;
    let module = target_dir.file_name()?.to_str()?.to_string();
    let sources = target_dir.parent()?.file_name()?.to_str()?;
    (sources == SWIFT_SOURCES_DIR).then_some(module)
}

fn files_importing(files: &[GeneratedFile], module: &str) -> Vec<String> {
    let import = format!("import {module}");
    files
        .iter()
        .filter(|f| owning_module(f).is_some_and(|m| m == module))
        .filter(|f| f.content.lines().any(|line| line.trim() == import))
        .map(|f| f.path.display().to_string())
        .collect()
}

#[test]
fn should_not_import_the_module_the_generated_file_belongs_to() {
    let files = generate();

    let mut offenders: Vec<String> = Vec::new();
    for file in &files {
        let Some(module) = owning_module(file) else {
            continue;
        };
        let self_import = format!("import {module}");
        if file.content.lines().any(|line| line.trim() == self_import) {
            offenders.push(format!("{} imports its own module '{module}'", file.path.display()));
        }
    }
    offenders.sort();

    assert!(
        offenders.is_empty(),
        "generated Swift files must not import their own SwiftPM module -- Swift warns \
         `file ... is part of module '<M>'; ignoring import`:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn should_still_import_a_genuinely_external_module() {
    let files = generate();

    let facade_imports = files_importing(&files, "Sample");
    assert!(
        facade_imports.is_empty(),
        "control precondition: nothing in Sources/Sample should import Sample"
    );

    let importers: Vec<String> = files
        .iter()
        .filter(|f| owning_module(f).is_some_and(|m| m == "Sample"))
        .filter(|f| f.content.lines().any(|line| line.trim() == "import RustBridge"))
        .map(|f| f.path.display().to_string())
        .collect();

    assert!(
        !importers.is_empty(),
        "a file in Sources/Sample must keep `import RustBridge` -- RustBridge is a different \
         SwiftPM target and the facade cannot name its symbols without it. Generated files: {:?}",
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
    );
}

#[test]
fn should_keep_other_imports_in_the_rust_bridge_target() {
    let files = generate();

    let importers: Vec<String> = files
        .iter()
        .filter(|f| owning_module(f).is_some_and(|m| m == "RustBridge"))
        .filter(|f| f.content.lines().any(|line| line.trim() == "import Foundation"))
        .map(|f| f.path.display().to_string())
        .collect();

    assert!(
        !importers.is_empty(),
        "removing the self-import must not strip `import Foundation` from Sources/RustBridge files"
    );
}
