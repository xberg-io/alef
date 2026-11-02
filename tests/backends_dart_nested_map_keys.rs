use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, NewtypeContainer, NewtypeWrapper, TypeRef};
use quote::ToTokens;

#[path = "support/mise_task.rs"]
mod mise_task;

const SOURCE: &str = r#"
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
pub struct Wide(pub u64);
#[derive(Eq, PartialEq, Ord, PartialOrd, Hash)] pub struct Key(pub u64);
pub static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn nested_cow_tree(values: Vec<BTreeMap<Cow<'static, str>, Wide>>) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(values.len(), 1); assert_eq!(values[0].len(), 1);
    assert_eq!(values[0].get("a").unwrap().0, u64::MAX);
}
pub fn nested_cow_hashed(values: Vec<ahash::AHashMap<Cow<'static, str>, Wide>>) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(values.len(), 1); assert_eq!(values[0].len(), 1);
    assert_eq!(values[0].get("a").unwrap().0, u64::MAX);
}
pub fn map_value_cow(values: HashMap<String, BTreeMap<Cow<'static, str>, Wide>>) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(values.len(), 1); assert_eq!(values["outer"].len(), 1);
    assert_eq!(values["outer"].get("a").unwrap().0, u64::MAX);
}
pub fn root_string(values: BTreeMap<String, Wide>) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(values.len(), 1); assert_eq!(values.get("a").unwrap().0, u64::MAX);
}
pub fn root_unsigned(values: BTreeMap<u64, Wide>) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(values.len(), 1); assert_eq!(values.get(&u64::MAX).unwrap().0, u64::MAX);
}
pub fn root_newtype(values: BTreeMap<Key, Wide>) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(values.len(), 1); assert_eq!(values.get(&Key(u64::MAX)).unwrap().0, u64::MAX);
}
"#;

fn extract_source(root: &std::path::Path) -> ApiSurface {
    let source = root.join("lib.rs");
    std::fs::write(&source, SOURCE).unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 6);
    assert!(api.functions.iter().all(|f| matches!(f.return_type, TypeRef::Unit)
        && f.params.len() == 1
        && f.params[0].newtype_wrapper.is_some()));
    api
}

#[test]
fn extracted_nested_map_key_sources_keep_the_actual_container_paths() {
    let directory = tempfile::tempdir().unwrap();
    let api = extract_source(directory.path());
    for function in &api.functions {
        let wrapper = NewtypeWrapper::decode(function.params[0].newtype_wrapper.as_deref().unwrap()).unwrap();
        let actual: Vec<_> = wrapper
            .explicit_paths()
            .iter()
            .map(|path| path.containers.clone())
            .collect();
        let expected = match function.name.as_str() {
            "nested_cow_tree" | "nested_cow_hashed" => vec![vec![NewtypeContainer::Vec, NewtypeContainer::MapValue]],
            "map_value_cow" => vec![vec![NewtypeContainer::MapValue, NewtypeContainer::MapValue]],
            "root_newtype" => vec![vec![NewtypeContainer::MapKey], vec![NewtypeContainer::MapValue]],
            _ => vec![vec![NewtypeContainer::MapValue]],
        };
        assert_eq!(actual, expected, "source function {}", function.name);
    }
}

fn compile_and_execute(name: &str, argument: &str) {
    use alef::snippets::validators::run_command_streams;
    let directory = tempfile::tempdir().unwrap();
    let api = extract_source(directory.path());
    let config: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=[\"dart\"]\n[[crates]]\nname=\"sample\"\nsources=[\"src/lib.rs\"]\n")
            .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let source = &files
        .iter()
        .find(|f| f.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    let functions: Vec<_> = syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == name => Some(function),
            _ => None,
        })
        .collect();
    assert_eq!(functions.len(), 1, "compile the selected actual generated function");
    let declaration = functions[0].to_token_stream();
    let program = format!(
        "mod sample {{ {SOURCE} }}\n{declaration}\nfn main() {{ {name}({argument}); assert_eq!(sample::CALLS.load(std::sync::atomic::Ordering::SeqCst), 1); println!(\"core_calls=1\"); }}"
    );
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(directory.path().join("src/main.rs"), program).unwrap();
    let lock: toml::Value = toml::from_str(include_str!("../Cargo.lock")).unwrap();
    let ahash: Vec<_> = lock["package"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"].as_str() == Some("ahash"))
        .collect();
    assert_eq!(ahash.len(), 1);
    let version = ahash[0]["version"].as_str().unwrap();
    std::fs::write(directory.path().join("Cargo.toml"), format!("[package]\nname=\"dart-map-key-probe\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[dependencies]\nahash=\"={version}\"\n")).unwrap();
    std::fs::write(
        directory.path().join("rust-toolchain.toml"),
        include_str!("../rust-toolchain.toml"),
    )
    .unwrap();
    std::fs::write(directory.path().join("mise.toml"), format!("[tasks.compile]\nrun='cargo build --offline --jobs 1'\n[tasks.execute]\nrun='./target/debug/dart-map-key-probe{}'\n", std::env::consts::EXE_SUFFIX)).unwrap();
    for task in ["compile", "execute"] {
        let mut command = mise_task::mise_run(directory.path(), task);
        command.env("RUSTC_WRAPPER", "");
        if let Some(cache) = std::env::var_os("CARGO_HOME") {
            command.env("CARGO_HOME", cache);
        }
        let output = run_command_streams(&mut command, 60).unwrap();
        assert!(output.success, "{name} {task}: {}\n{}", output.stdout, output.stderr);
        if task == "execute" {
            assert_eq!(
                output.stdout.lines().filter(|line| *line == "core_calls=1").count(),
                1,
                "{name}: actual source function must execute exactly once"
            );
        }
    }
}

#[test]
fn nested_cow_btree_keys_reach_the_actual_native_argument() {
    compile_and_execute(
        "nested_cow_tree",
        "vec![std::collections::HashMap::from([(String::from(\"a\"), u64::MAX)])]",
    );
}

#[test]
fn nested_cow_ahash_keys_reach_the_actual_native_argument() {
    compile_and_execute(
        "nested_cow_hashed",
        "vec![std::collections::HashMap::from([(String::from(\"a\"), u64::MAX)])]",
    );
}

#[test]
fn map_value_cow_keys_reach_the_actual_native_argument() {
    compile_and_execute(
        "map_value_cow",
        "std::collections::HashMap::from([(String::from(\"outer\"), std::collections::HashMap::from([(String::from(\"a\"), u64::MAX)]))])",
    );
}

#[test]
fn root_string_keys_remain_source_strings() {
    compile_and_execute(
        "root_string",
        "std::collections::HashMap::from([(String::from(\"a\"), u64::MAX)])",
    );
}

#[test]
fn root_unsigned_keys_retain_the_full_u64_range() {
    compile_and_execute(
        "root_unsigned",
        "std::collections::HashMap::from([(u64::MAX, u64::MAX)])",
    );
}

#[test]
fn root_newtype_keys_use_their_real_constructor() {
    compile_and_execute(
        "root_newtype",
        "std::collections::HashMap::from([(u64::MAX, u64::MAX)])",
    );
}
