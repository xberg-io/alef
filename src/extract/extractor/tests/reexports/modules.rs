use super::*;

#[test]
fn test_extract_via_top_level_function_with_multiple_sources() {
    // Test the public `extract()` entry point with two source files that would
    // be visited. The second file should be skipped if already processed via mod.
    let tmp = std::env::temp_dir().join("alef_test_extract_multi");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub struct Config {
    pub timeout: u32,
}

pub fn run(config: Config) -> bool {
    true
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "1.0.0", None).unwrap();

    assert_eq!(surface.crate_name, "my_crate");
    assert_eq!(surface.version, "1.0.0");
    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "Config");
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].name, "run");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_derive_module_path_via_extract_with_submodule_files() {
    // Verify that extract() with multiple source files derives correct module paths.
    // When cache/types.rs is given as an explicit source with src/ as root,
    // the items should have rust_path `my_crate::cache::types::Item`.
    let tmp = std::env::temp_dir().join("alef_test_derive_module_path");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(tmp.join("src/cache/mod.rs"), "pub mod types;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/types.rs"),
        r#"
pub struct CacheEntry {
    pub key: String,
    pub value: String,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    let entry = &surface.types[0];
    assert_eq!(entry.name, "CacheEntry");
    assert_eq!(entry.rust_path, "my_crate::cache::types::CacheEntry");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_parent_reexport_shortening_via_extract() {
    // When cache/types.rs defines CacheEntry and cache/mod.rs has `pub use types::CacheEntry;`,
    // the extract() pass should shorten the rust_path to `my_crate::cache::CacheEntry`.
    let tmp = std::env::temp_dir().join("alef_test_parent_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/mod.rs"),
        r#"
pub mod types;
pub use types::CacheEntry;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/cache/types.rs"),
        r#"
pub struct CacheEntry {
    pub key: String,
    pub ttl: u64,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    let entry = &surface.types[0];
    assert_eq!(entry.name, "CacheEntry");
    assert_eq!(
        entry.rust_path, "my_crate::cache::CacheEntry",
        "Named re-export in parent mod.rs should shorten the rust_path"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_parent_reexport_glob_shortening_via_extract() {
    // When cache/mod.rs has `pub use types::*;`, all items from types.rs should
    // be shortened to `my_crate::cache::ItemName`.
    let tmp = std::env::temp_dir().join("alef_test_parent_glob_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/mod.rs"),
        r#"
pub mod types;
pub use types::*;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/cache/types.rs"),
        r#"
pub struct Entry {
    pub key: String,
}

pub struct Store {
    pub size: u32,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 2);
    for ty in &surface.types {
        assert!(
            ty.rust_path.starts_with("my_crate::cache::"),
            "Glob re-export should shorten path to parent level, got: {}",
            ty.rust_path
        );
        assert!(
            !ty.rust_path.contains("types"),
            "types:: should not appear in shortened path, got: {}",
            ty.rust_path
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// reexports.rs coverage: collect_use_names rename, resolve_use_tree group/bare,
// find_crate_source heuristic & dependencies table, extract_module file-based,
// private module pruning with named re-exports
// ---------------------------------------------------------------------------

#[test]
fn test_extract_module_external_file_based() {
    // Test that an external `pub mod foo;` declaration is followed to foo.rs
    let tmp = std::env::temp_dir().join("alef_test_extract_mod_external");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub mod models;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/models.rs"),
        r#"
pub struct ModelItem {
    pub id: u32,
    pub name: String,
}

pub fn find_model(id: u32) -> ModelItem {
    unimplemented!()
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "ModelItem");
    assert_eq!(surface.types[0].rust_path, "my_crate::models::ModelItem");
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].rust_path, "my_crate::models::find_model");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_extract_module_mod_rs_subdir() {
    // `pub mod cache;` that resolves to `cache/mod.rs`
    let tmp = std::env::temp_dir().join("alef_test_extract_mod_subdir");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/cache")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod cache;\n").unwrap();
    std::fs::write(
        tmp.join("src/cache/mod.rs"),
        r#"
pub struct CacheClient {
    pub url: String,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "my_crate", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1);
    assert_eq!(surface.types[0].name, "CacheClient");
    assert_eq!(surface.types[0].rust_path, "my_crate::cache::CacheClient");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_private_module_with_named_reexport_prunes_non_reexported() {
    // A private module (`mod foo;`) with `pub use foo::Public` should expose
    // only Public, not the private types.
    let source = r#"
        mod inner;
        pub use inner::Public;

        mod inner {
            pub struct Public { pub value: u32 }
            pub struct Hidden { pub secret: String }
        }
    "#;

    let surface = extract_from_source(source);
    // Only Public should survive, Hidden is pruned
    assert_eq!(surface.types.len(), 1, "Only re-exported items should survive");
    assert_eq!(surface.types[0].name, "Public");
}

#[test]
fn test_private_module_glob_reexport_exposes_all() {
    // `mod inner; pub use inner::*;` — all public items from inner should be exposed
    let source = r#"
        mod inner;
        pub use inner::*;

        mod inner {
            pub struct Alpha { pub x: u32 }
            pub struct Beta { pub y: String }
        }
    "#;

    let surface = extract_from_source(source);
    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Alpha"), "Alpha should be exposed via glob re-export");
    assert!(names.contains(&"Beta"), "Beta should be exposed via glob re-export");
}

#[test]
fn test_pub_use_clears_binding_excluded_on_skipped_source() {
    // `#[cfg(feature = "X")] pub use mod::fn` re-exports a concrete-signature
    // function from a sibling module. The source carries `#[cfg_attr(alef, alef(skip))]`
    // because it is intended to be reached only through the crate-root re-export.
    // The re-export must un-exclude the entry so it lands in the binding surface.
    let tmp = std::env::temp_dir().join("alef_test_pub_use_clears_skip");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub mod inner;

#[cfg(feature = "real")]
pub use inner::do_thing;

#[cfg(not(feature = "real"))]
pub fn do_thing(input: String) -> Result<String, String> {
    Err("real feature not enabled".to_string())
}
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/inner.rs"),
        r#"
#[cfg(feature = "real")]
#[cfg_attr(alef, alef(skip))]
pub fn do_thing(input: String) -> Result<String, String> {
    Ok(input)
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "demo", "0.1.0", None).unwrap();

    let entries: Vec<&_> = surface.functions.iter().filter(|f| f.name == "do_thing").collect();
    assert_eq!(
        entries.len(),
        2,
        "expected both the inner source (un-excluded by re-export) and the stub to land in the surface; got {entries:?}"
    );
    assert!(
        entries.iter().all(|f| !f.binding_excluded),
        "binding_excluded must be cleared by the pub use re-export; got {entries:?}"
    );
    let cfgs: Vec<&str> = entries.iter().filter_map(|f| f.cfg.as_deref()).collect();
    assert!(
        cfgs.iter().any(|c| c.contains("\"real\"") && !c.contains("not")),
        "real cfg gate must be present; got cfgs={cfgs:?}"
    );
    assert!(
        cfgs.iter().any(|c| c.contains("not") && c.contains("\"real\"")),
        "stub cfg gate must be preserved; got cfgs={cfgs:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_pub_use_synthesises_paired_entry_for_generic_source() {
    // `#[cfg(feature = "X")] pub use mod::fn` re-exports a GENERIC function which
    // extract_function drops (FFI cannot represent generics). A same-named
    // concrete stub exists under the disjoint cfg `not(feature = "X")`. The
    // re-export must synthesise a paired concrete entry under cfg "X" so the
    // binding has a callable surface in every feature combination.
    let tmp = std::env::temp_dir().join("alef_test_pub_use_pairs_generic");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub mod inner;

#[cfg(feature = "real")]
pub use inner::do_thing;

#[cfg(not(feature = "real"))]
pub fn do_thing(input: String) -> Result<String, String> {
    Err("real feature not enabled".to_string())
}
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/inner.rs"),
        r#"
#[cfg(feature = "real")]
pub fn do_thing<T: AsRef<str>>(input: T) -> Result<String, String> {
    Ok(input.as_ref().to_string())
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "demo", "0.1.0", None).unwrap();

    let entries: Vec<&_> = surface.functions.iter().filter(|f| f.name == "do_thing").collect();
    assert_eq!(
        entries.len(),
        2,
        "expected the stub plus a synthesised paired entry under the re-export's cfg, got {entries:?}"
    );
    assert!(
        entries.iter().all(|f| !f.binding_excluded),
        "neither the stub nor the synthesised paired entry should be binding_excluded"
    );
    let cfgs: Vec<&str> = entries.iter().filter_map(|f| f.cfg.as_deref()).collect();
    assert!(
        cfgs.iter().any(|c| c.contains("\"real\"") && !c.contains("not")),
        "paired entry must carry the re-export's cfg gate; got cfgs={cfgs:?}"
    );
    assert!(
        cfgs.iter().any(|c| c.contains("not") && c.contains("\"real\"")),
        "stub cfg gate must be preserved; got cfgs={cfgs:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
