use crate::backends::napi::NapiBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use crate::core::validation::ValidatedApiSurface;
use quote::ToTokens;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(super) struct Expected {
    pub(super) types: usize,
    pub(super) enums: usize,
    pub(super) functions: usize,
    pub(super) methods: usize,
}

pub(super) struct NativeFixture {
    directory: tempfile::TempDir,
    generated: String,
    protected: Vec<(PathBuf, Vec<u8>)>,
}

impl NativeFixture {
    pub(super) fn emit(source: &str, expected: Expected) -> Self {
        let directory = tempfile::tempdir().expect("owned native wrapper fixture");
        let root = directory.path();
        let config = write_inputs(root, source);
        let api = extract_api(root, &expected);
        let (generated, paths) = emit_sources(root, &api, &config);
        let protected = protected_files(root, paths);
        Self {
            directory,
            generated,
            protected,
        }
    }

    pub(super) fn assert_wrapper(&self, name: &str, method: bool) -> syn::Signature {
        let parsed = syn::parse_file(&self.generated).expect("complete native Rust must parse");
        let mut matches = Vec::new();
        for item in parsed.items {
            match item {
                syn::Item::Fn(function) if !method && function.sig.ident == name => matches.push(function.sig),
                syn::Item::Impl(implementation)
                    if method
                        && implementation.trait_.is_none()
                        && implementation.self_ty.to_token_stream().to_string() == "JsEngine" =>
                {
                    for item in implementation.items {
                        if let syn::ImplItem::Fn(function) = item
                            && function.sig.ident == name
                        {
                            matches.push(function.sig);
                        }
                    }
                }
                _ => {}
            }
        }
        assert_eq!(matches.len(), 1, "one real emitted wrapper for {name}, method={method}");
        matches.remove(0)
    }

    pub(super) fn assert_async_unit(&self, name: &str, method: bool) {
        let signature = self.assert_wrapper(name, method);
        assert!(
            signature.asyncness.is_some(),
            "the actual async boundary must be emitted"
        );
    }

    pub(super) fn assert_serde_dto(&self, name: &str) {
        let parsed = syn::parse_file(&self.generated).unwrap();
        let structure = parsed
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            })
            .expect("the real JSON-bearing DTO must be emitted");
        let serializer = structure
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("derive"))
            .any(|attribute| {
                let derives = attribute
                    .parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                    .unwrap();
                derives
                    .iter()
                    .any(|path| path.to_token_stream().to_string() == "serde :: Serialize")
            });
        assert!(
            serializer,
            "canonical manifest detection must enable this DTO's own serde derive"
        );
    }

    pub(super) fn check(&self) {
        let root = self.directory.path();
        let output = native_check(root);
        for (path, before) in &self.protected {
            assert_eq!(
                std::fs::read(path).unwrap(),
                *before,
                "core/canonical generated file changed: {}",
                path.display()
            );
        }
        assert!(
            output.status.success(),
            "complete native wrapper compilation failed\nstdout:\n{}\nstderr:\n{}\nFULL GENERATED SOURCE:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            self.generated
        );
    }
}

fn write_inputs(root: &Path, source: &str) -> ResolvedCrateConfig {
    let core = root.join("crates/wrapper-core/src");
    std::fs::create_dir_all(&core).unwrap();
    std::fs::write(core.join("lib.rs"), source).unwrap();
    std::fs::write(core.parent().unwrap().join("Cargo.toml"),
        "[package]\nname='wrapper-core'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nserde={version='1',features=['derive']}\nserde_json='1'\n").unwrap();
    let text = "[workspace]\nlanguages=['node']\n[[crates]]\nname='wrapper-core'\nsources=['crates/wrapper-core/src/lib.rs']\n[crates.node.extra_dependencies]\ntokio={version='1',features=['rt-multi-thread','macros']}\n";
    std::fs::write(root.join("alef.toml"), text).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver='3'\nmembers=['crates/wrapper-core','crates/wrapper-core-node']\n",
    )
    .unwrap();
    let raw: NewAlefConfig = toml::from_str(text).unwrap();
    let mut config = raw.resolve().unwrap().remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config.sources = vec![root.join("crates/wrapper-core/src/lib.rs")];
    config
        .output_paths
        .insert("node".into(), root.join("crates/wrapper-core-node/src"));
    assert!(config.components.is_empty());
    config
}

fn write(root: &Path, file: &GeneratedFile) -> PathBuf {
    let path = if file.path.is_absolute() {
        file.path.clone()
    } else {
        root.join(&file.path)
    };
    assert!(path.starts_with(root) && !file.path.components().any(|part| part == Component::ParentDir));
    assert_ne!(
        path,
        root.join("Cargo.toml"),
        "outer workspace belongs to the authored harness"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, file.content.as_bytes()).unwrap();
    path
}

fn protected_files(root: &Path, mut paths: Vec<PathBuf>) -> Vec<(PathBuf, Vec<u8>)> {
    paths.extend(
        [
            "Cargo.toml",
            "alef.toml",
            "crates/wrapper-core/Cargo.toml",
            "crates/wrapper-core/src/lib.rs",
        ]
        .map(|path| root.join(path)),
    );
    paths.sort();
    paths.dedup();
    assert!(
        paths.len() >= 6,
        "protect core files plus complete canonical native source and manifest"
    );
    paths
        .into_iter()
        .map(|path| {
            let data = std::fs::read(&path).unwrap();
            assert!(!data.is_empty());
            (path, data)
        })
        .collect()
}

fn extract_api(root: &Path, expected: &Expected) -> crate::core::ir::ApiSurface {
    let source_path = root.join("crates/wrapper-core/src/lib.rs");
    let api = crate::extract::extractor::extract(&[source_path.as_path()], "wrapper-core", "0.1.0", Some(root))
        .expect("actual public source extraction");
    assert_eq!(
        (api.types.len(), api.enums.len(), api.functions.len()),
        (expected.types, expected.enums, expected.functions)
    );
    let engine = api
        .types
        .iter()
        .find(|typ| typ.name == "Engine")
        .expect("actual opaque Engine");
    assert!(engine.is_opaque && engine.has_default);
    assert_eq!(engine.methods.len(), expected.methods);
    ValidatedApiSurface::new(&api, &[]).expect("unsuppressed public API validation");
    api
}

fn emit_sources(
    root: &Path,
    api: &crate::core::ir::ApiSurface,
    config: &ResolvedCrateConfig,
) -> (String, Vec<PathBuf>) {
    let scaffolded = crate::scaffold::scaffold(api, config, &[Language::Node]).expect("canonical Node scaffold");
    assert!(!scaffolded.is_empty());
    let mut paths = Vec::new();
    for file in &scaffolded {
        paths.push(write(root, file));
    }
    let output = root.join("crates/wrapper-core-node/src");
    assert!(
        crate::core::config::detect_serde_available(output.to_str().unwrap()),
        "observe the actual canonical manifest"
    );
    let files = NapiBackend
        .generate_bindings(api, config)
        .expect("complete public NAPI backend");
    assert_eq!(files.len(), 1, "complete native library, no selected-helper projection");
    assert_eq!(files[0].path, output.join("lib.rs"));
    paths.push(write(root, &files[0]));
    (files[0].content.clone(), paths)
}

fn native_check(root: &Path) -> std::process::Output {
    let channel = committed_channel();
    let cargo = installed_rust_tool(&channel, "cargo", root);
    let rustc = installed_rust_tool(&channel, "rustc", root);
    write_task(root);
    let path = std::env::join_paths(
        std::iter::once(cargo.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    crate::test_support::mise_task::mise_run_without_lock(root, "check")
        .env("PATH", path)
        .env("RUSTC", rustc)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("CARGO_BUILD_JOBS", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_DEV_OPT_LEVEL", "0")
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_ENCODED_RUSTFLAGS", "")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env_remove("CARGO_BUILD_TARGET")
        .output()
        .expect("execute the isolated offline native Cargo gate")
}

fn committed_channel() -> String {
    let tools: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust-toolchain.toml"
    )))
    .unwrap();
    tools["toolchain"]["channel"]
        .as_str()
        .expect("committed Rust channel")
        .to_string()
}

fn installed_rust_tool(channel: &str, name: &str, root: &Path) -> PathBuf {
    let output = Command::new("rustup")
        .args(["which", "--toolchain", channel, name])
        .current_dir(root)
        .output()
        .expect("look up the already-installed committed Rust toolchain");
    assert!(
        output.status.success(),
        "installed {channel} {name} required; no install is attempted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert!(path.is_absolute() && path.is_file());
    path
}

fn write_task(root: &Path) {
    std::fs::write(root.join("mise.toml"), "[settings]\ntask.run_auto_install=false\nlockfile=false\n[tasks.check]\nrun='cargo check --offline --lib -p wrapper-core-node'\n").unwrap();
}
