use super::*;

#[test]
fn test_find_crate_source_no_workspace() {
    assert!(find_crate_source("some_crate", None).is_none());
}

#[test]
fn test_pub_use_reexport_from_workspace_crate() {
    let tmp = std::env::temp_dir().join("alef_test_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/other_crate/src")).unwrap();

    std::fs::write(
        tmp.join("Cargo.toml"),
        r#"
[workspace]
members = ["crates/other_crate"]

[workspace.dependencies]
other_crate = { path = "crates/other_crate" }
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("crates/other_crate/src/lib.rs"),
        r#"
/// Server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

/// CORS settings.
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
}

/// Internal helper, not re-exported.
pub struct InternalHelper {
    pub data: String,
}
"#,
    )
    .unwrap();

    let our_lib = tmp.join("crates/my_crate/src/lib.rs");
    std::fs::create_dir_all(our_lib.parent().unwrap()).unwrap();
    std::fs::write(
        &our_lib,
        r#"
pub use other_crate::{ServerConfig, CorsConfig};
"#,
    )
    .unwrap();

    let sources: Vec<&Path> = vec![our_lib.as_path()];
    let surface = extract(&sources, "my_crate", "0.1.0", Some(&tmp)).unwrap();

    assert_eq!(surface.types.len(), 2);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"ServerConfig"));
    assert!(names.contains(&"CorsConfig"));
    assert!(!names.contains(&"InternalHelper"));

    let server = surface.types.iter().find(|t| t.name == "ServerConfig").unwrap();
    assert_eq!(server.rust_path, "my_crate::ServerConfig");
    assert!(server.is_clone);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_pub_use_glob_reexport() {
    let tmp = std::env::temp_dir().join("alef_test_glob_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/other_crate/src")).unwrap();

    std::fs::write(
        tmp.join("Cargo.toml"),
        r#"
[workspace]
members = ["crates/other_crate"]

[workspace.dependencies]
other_crate = { path = "crates/other_crate" }
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("crates/other_crate/src/lib.rs"),
        r#"
pub struct Alpha { pub value: u32 }
pub struct Beta { pub name: String }
"#,
    )
    .unwrap();

    let our_lib = tmp.join("crates/my_crate/src/lib.rs");
    std::fs::create_dir_all(our_lib.parent().unwrap()).unwrap();
    std::fs::write(&our_lib, "pub use other_crate::*;\n").unwrap();

    let sources: Vec<&Path> = vec![our_lib.as_path()];
    let surface = extract(&sources, "my_crate", "0.1.0", Some(&tmp)).unwrap();

    assert_eq!(surface.types.len(), 2);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Alpha"));
    assert!(names.contains(&"Beta"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_find_crate_source_with_dependencies_table() {
    let tmp = std::env::temp_dir().join("alef_test_find_crate_dep");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/dep_crate/src")).unwrap();

    std::fs::write(
        tmp.join("Cargo.toml"),
        r#"
[dependencies]
dep_crate = { path = "crates/dep_crate" }
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("crates/dep_crate/src/lib.rs"),
        "pub struct DepType { pub x: u32 }\n",
    )
    .unwrap();

    let result = super::reexports::find_crate_source("dep_crate", Some(&tmp));
    assert!(result.is_some(), "Should find crate source via [dependencies] path dep");
    assert!(result.unwrap().ends_with("lib.rs"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_find_crate_source_heuristic_crates_dir() {
    let tmp = std::env::temp_dir().join("alef_test_find_crate_heuristic");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/my_lib/src")).unwrap();

    std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    std::fs::write(tmp.join("crates/my_lib/src/lib.rs"), "pub struct Heuristic;\n").unwrap();

    let result = super::reexports::find_crate_source("my_lib", Some(&tmp));
    assert!(result.is_some(), "Should find via heuristic crates/{{name}}/src/lib.rs");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_find_crate_source_hyphen_underscore_alt() {
    let tmp = std::env::temp_dir().join("alef_test_find_crate_alt");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/my-lib/src")).unwrap();
    std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    std::fs::write(tmp.join("crates/my-lib/src/lib.rs"), "pub struct AltType;\n").unwrap();

    let result = super::reexports::find_crate_source("my_lib", Some(&tmp));
    assert!(result.is_some(), "Should find crate via hyphen/underscore alt name");

    let _ = std::fs::remove_dir_all(&tmp);
}
