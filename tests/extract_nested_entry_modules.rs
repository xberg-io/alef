use alef::extract::extractor::extract;
use std::process::Command;

fn assert_answer(root_file: &str, files: &[(&str, &str)], expected_path: &str) {
    let directory = tempfile::tempdir().unwrap();
    for (relative, content) in files {
        let path = directory.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let source = directory.path().join(root_file);
    let compiled = Command::new("rustc")
        .args(["--edition=2024", "--crate-name=sample", "--crate-type=lib"])
        .arg(&source)
        .arg("--out-dir")
        .arg(directory.path())
        .output()
        .expect("compile the dependency-free module fixture");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let library = directory.path().join("libsample.rlib");
    assert!(
        library
            .metadata()
            .expect("rustc must produce the fixture library")
            .len()
            > 0
    );
    let surface = extract(&[source.as_path()], "sample", "0.1.0", None).unwrap();
    assert!(
        surface.unresolved_modules.is_empty(),
        "valid Rust child modules must resolve: {:?}",
        surface.unresolved_modules
    );
    let answers: Vec<_> = surface
        .functions
        .iter()
        .filter(|function| function.name == "answer")
        .map(|function| function.rust_path.as_str())
        .collect();
    assert_eq!(answers, [expected_path]);
}

fn assert_nested_entry(name: &str) {
    let file = format!("src/nested/{name}.rs");
    let child = format!("src/nested/{name}/child.rs");
    let declaration = format!("pub mod {name};");
    assert_answer(
        "src/lib.rs",
        &[
            ("src/lib.rs", "pub mod nested;"),
            ("src/nested/mod.rs", &declaration),
            (&file, "mod child; pub use child::answer;"),
            (&child, "pub fn answer() -> i32 { 42 }"),
        ],
        &format!("sample::nested::{name}::answer"),
    );
}

#[test]
fn should_resolve_children_of_a_nested_main_module() {
    assert_nested_entry("main");
}

#[test]
fn should_resolve_children_of_a_nested_lib_module() {
    assert_nested_entry("lib");
}

fn assert_glob_reexported_entry(name: &str) {
    let root = if name == "main" { "src/lib.rs" } else { "src/main.rs" };
    let declaration = format!("mod {name}; pub use {name}::*;");
    let file = format!("src/{name}.rs");
    let child = format!("src/{name}/child.rs");
    assert_answer(
        root,
        &[
            (root, &declaration),
            (&file, "mod child; pub use child::answer;"),
            (&child, "pub fn answer() -> i32 { 42 }"),
        ],
        "sample::answer",
    );
}

#[test]
fn should_resolve_children_of_a_glob_reexported_main_module() {
    assert_glob_reexported_entry("main");
}

#[test]
fn should_resolve_children_of_a_glob_reexported_lib_module() {
    assert_glob_reexported_entry("lib");
}

fn assert_crate_entry(root: &str) {
    assert_answer(
        root,
        &[
            (root, "mod child; pub use child::answer;"),
            ("src/child.rs", "pub fn answer() -> i32 { 42 }"),
        ],
        "sample::answer",
    );
}

#[test]
fn should_resolve_children_beside_the_real_main_entry() {
    assert_crate_entry("src/main.rs");
}

#[test]
fn should_resolve_children_beside_the_real_lib_entry() {
    assert_crate_entry("src/lib.rs");
}
