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

/// Task #385 candidate reproduction: the same workspace-sibling item re-exported from two
/// different local modules must resolve to a single `ApiSurface` entry, not one per `use` path.
/// The second `pub use` merges from the sibling surface cached on the first extraction, and the
/// merge skips an item already present by name. ~keep
#[test]
fn test_pub_use_same_external_item_from_two_local_modules_not_duplicated() {
    let tmp = std::env::temp_dir().join("alef_test_reexport_two_local_paths");
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
        "pub struct ServerConfig { pub host: String }\n",
    )
    .unwrap();

    let our_lib = tmp.join("crates/my_crate/src/lib.rs");
    std::fs::create_dir_all(our_lib.parent().unwrap()).unwrap();
    std::fs::write(
        &our_lib,
        r#"
pub use other_crate::ServerConfig;
pub mod api {
    pub use other_crate::ServerConfig;
}
"#,
    )
    .unwrap();

    let sources: Vec<&Path> = vec![our_lib.as_path()];
    let surface = extract(&sources, "my_crate", "0.1.0", Some(&tmp)).unwrap();

    assert_eq!(
        surface.types.len(),
        1,
        "ServerConfig re-exported from two local modules must appear once, not twice"
    );
    assert_eq!(surface.types[0].name, "ServerConfig");
    assert_eq!(surface.types[0].rust_path, "my_crate::ServerConfig");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Write a workspace under `tmp` whose `[workspace.dependencies]` name each `(crate, lib.rs)`
/// in `crates`, and return the host crate's `lib.rs`.
fn write_sibling_workspace(tmp: &Path, crates: &[(&str, &str)], host_lib: &str) -> PathBuf {
    let _ = std::fs::remove_dir_all(tmp);
    let mut deps = String::new();
    for (name, lib) in crates {
        std::fs::create_dir_all(tmp.join(format!("crates/{name}/src"))).unwrap();
        std::fs::write(tmp.join(format!("crates/{name}/src/lib.rs")), lib).unwrap();
        deps.push_str(&format!("{name} = {{ path = \"crates/{name}\" }}\n"));
    }
    std::fs::write(
        tmp.join("Cargo.toml"),
        format!("[workspace]\nmembers = []\n\n[workspace.dependencies]\n{deps}"),
    )
    .unwrap();
    let host = tmp.join("crates/host/src/lib.rs");
    std::fs::create_dir_all(host.parent().unwrap()).unwrap();
    std::fs::write(&host, host_lib).unwrap();
    host
}

fn sorted_type_names(surface: &ApiSurface) -> Vec<String> {
    let mut names: Vec<String> = surface.types.iter().map(|ty| ty.name.clone()).collect();
    names.sort();
    names
}

/// A sibling crate reached by two `pub use`s naming disjoint items must contribute both sets,
/// whichever re-export is walked first. The sibling used to be extracted once per run with the
/// first re-export's name filter, so the second re-export silently found nothing.
#[test]
fn test_pub_use_same_sibling_with_disjoint_names_in_either_order() {
    let proto = "pub struct Alpha;\npub struct Beta;\npub struct Gamma;\n";
    let orders = [
        (
            "module_first",
            "pub mod wire {\n    pub use proto::Alpha;\n}\npub use proto::{Beta, Gamma};\n",
        ),
        (
            "use_first",
            "pub use proto::{Beta, Gamma};\npub mod wire {\n    pub use proto::Alpha;\n}\n",
        ),
    ];
    for (label, host_lib) in orders {
        let tmp = std::env::temp_dir().join(format!("alef_test_reexport_disjoint_{label}"));
        let host = write_sibling_workspace(&tmp, &[("proto", proto)], host_lib);

        let surface = extract(&[host.as_path()], "host", "0.1.0", Some(&tmp)).unwrap();

        assert_eq!(
            sorted_type_names(&surface),
            ["Alpha", "Beta", "Gamma"],
            "order: {label}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// The same holds when the first visit reaches the sibling transitively, through another
/// sibling that re-exports only part of it.
#[test]
fn test_pub_use_sibling_reached_transitively_first_still_yields_later_names() {
    let tmp = std::env::temp_dir().join("alef_test_reexport_transitive_revisit");
    let host = write_sibling_workspace(
        &tmp,
        &[
            ("proto", "pub struct Alpha;\npub struct Beta;\n"),
            ("api", "pub use proto::Alpha;\n"),
        ],
        "pub use api::Alpha;\npub use proto::Beta;\n",
    );

    let surface = extract(&[host.as_path()], "host", "0.1.0", Some(&tmp)).unwrap();

    assert_eq!(sorted_type_names(&surface), ["Alpha", "Beta"]);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Two siblings that glob re-export each other must not recurse forever, and every item the
/// host names from either of them must still be found exactly once.
#[test]
fn test_pub_use_sibling_cycle_terminates_and_yields_named_items() {
    let tmp = std::env::temp_dir().join("alef_test_reexport_sibling_cycle");
    let host = write_sibling_workspace(
        &tmp,
        &[
            ("left", "pub struct LeftItem;\npub use right::*;\n"),
            ("right", "pub struct RightItem;\npub use left::*;\n"),
        ],
        "pub use left::{LeftItem, RightItem};\npub use right::RightItem as Again;\npub use right::LeftItem;\n",
    );

    let surface = extract(&[host.as_path()], "host", "0.1.0", Some(&tmp)).unwrap();

    assert_eq!(sorted_type_names(&surface), ["LeftItem", "RightItem"]);
    let _ = std::fs::remove_dir_all(&tmp);
}
