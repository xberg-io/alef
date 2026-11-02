use super::*;

const API: &str = r#"
    pub mod api {
        use crate::Result;
        pub fn typed() -> Result<()> { unimplemented!() }
        pub async fn asynchronous() -> Result<()> { unimplemented!() }
        pub struct Reader;
        impl Reader { pub fn read(&self) -> Result<()> { unimplemented!() } }
        pub trait Service { fn call(&self) -> Result<()>; }
    }
"#;
const ERROR: &str = r#"
    pub mod error {
        #[derive(Debug, thiserror::Error)]
        pub enum PublicError { #[error("invalid input")] Invalid }
        pub type Result<T> = std::result::Result<T, PublicError>;
    }
    pub use error::{PublicError, Result};
"#;

fn extract_files(files: &[(&str, &str)], source_names: &[&str]) -> ApiSurface {
    let directory = tempfile::tempdir().expect("crate source directory");
    for (name, source) in files {
        let path = directory.path().join(name);
        std::fs::create_dir_all(path.parent().expect("source parent")).expect("create module directories");
        std::fs::write(path, source).expect("write Rust source");
    }
    let paths = source_names
        .iter()
        .map(|name| directory.path().join(name))
        .collect::<Vec<_>>();
    let sources = paths.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    extract(&sources, "forward_alias", "0.1.0", None).expect("extract public API")
}

fn assert_error(surface: &ApiSurface, function_name: &str, expected: &str) {
    let function = surface
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("expected public function");
    assert_eq!(function.error_type.as_deref(), Some(expected), "{function_name}");
}

fn assert_api_errors(surface: &ApiSurface) {
    assert_eq!(surface.functions.len(), 2);
    assert_eq!(surface.types.len(), 2);
    assert_eq!(surface.errors.len(), 1);
    for name in ["typed", "asynchronous"] {
        assert_error(surface, name, "PublicError");
    }
    for (owner, method) in [("Reader", "read"), ("Service", "call")] {
        let definition = surface.types.iter().find(|typ| typ.name == owner).expect("owner");
        let definition = definition
            .methods
            .iter()
            .find(|item| item.name == method)
            .expect("method");
        assert_eq!(
            definition.error_type.as_deref(),
            Some("PublicError"),
            "{owner}::{method}"
        );
    }
}

#[test]
fn forward_inline_alias_is_independent_of_module_order() {
    for source in [format!("{API}\n{ERROR}"), format!("{ERROR}\n{API}")] {
        assert_api_errors(&extract_files(&[("lib.rs", &source)], &["lib.rs"]));
    }
}

#[test]
fn forward_external_alias_is_independent_of_source_order() {
    let root = format!("{API}\npub mod error; pub use error::{{PublicError, Result}};");
    let error = r#"
        #[derive(Debug, thiserror::Error)]
        pub enum PublicError { #[error("invalid input")] Invalid }
        pub type Result<T> = std::result::Result<T, PublicError>;
    "#;
    for sources in [vec!["lib.rs", "error.rs"], vec!["error.rs", "lib.rs"]] {
        assert_api_errors(&extract_files(&[("lib.rs", &root), ("error.rs", error)], &sources));
    }
}

#[test]
fn forward_aliases_preserve_subsystem_qualified_and_foreign_results() {
    let source = format!(
        r#"{API}
        pub mod subsystem {{
            pub mod api {{
                use super::error::Result;
                pub fn local() -> Result<()> {{ unimplemented!() }}
                pub fn qualified() -> crate::error::Result<()> {{ unimplemented!() }}
                pub fn foreign() -> anyhow::Result<()> {{ unimplemented!() }}
            }}
            pub mod error {{
                #[derive(Debug, thiserror::Error)]
                pub enum LocalError {{ #[error("local failure")] Invalid }}
                pub type Result<T> = std::result::Result<T, LocalError>;
            }}
        }}
        {ERROR}"#
    );
    let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 5);
    assert_error(&surface, "typed", "PublicError");
    assert_error(&surface, "local", "LocalError");
    assert_error(&surface, "qualified", "PublicError");
    assert_error(&surface, "foreign", "anyhow::Error");
}

#[test]
fn forward_root_reexport_selects_its_alias_instead_of_the_first_error() {
    let source = r#"
        pub mod api { use crate::Result; pub fn run() -> Result<()> { unimplemented!() } }
        pub mod alpha { pub struct Other; pub type Result<T> = std::result::Result<T, Other>; }
        pub mod zeta { pub struct Selected; pub type Result<T> = std::result::Result<T, Selected>; }
        pub use zeta::Result;
    "#;
    let surface = extract_files(&[("lib.rs", source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 1);
    assert_error(&surface, "run", "Selected");
}

#[test]
fn glob_reexports_do_not_replace_the_rust_alias_namespace() {
    let source = r#"
        pub mod api { use crate::Result; pub fn root() -> Result<()> { unimplemented!() } }
        mod hidden {
            pub struct LocalError;
            type Result<T> = std::result::Result<T, LocalError>;
            pub fn local() -> Result<()> { unimplemented!() }
        }
        pub use hidden::*;
        pub mod error { pub struct RootError; pub type Result<T> = std::result::Result<T, RootError>; }
        pub use error::Result;
    "#;
    let surface = extract_files(&[("lib.rs", source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 2);
    assert_error(&surface, "root", "RootError");
    assert_error(&surface, "local", "LocalError");
}

#[test]
fn out_of_line_nested_aliases_are_collected_before_sibling_apis() {
    let root = "pub mod service; pub mod error; pub use error::{PublicError, Result};";
    let service = "use crate::Result; pub fn run() -> Result<()> { unimplemented!() }";
    let error = "pub struct PublicError; pub type Result<T> = std::result::Result<T, PublicError>;";
    for error_path in ["error.rs", "error/mod.rs"] {
        let surface = extract_files(
            &[("lib.rs", root), ("service.rs", service), (error_path, error)],
            &["lib.rs"],
        );
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "run", "PublicError");
    }
}

#[test]
fn precollected_alias_hints_do_not_leak_between_crates() {
    let source = format!("{API}\n{ERROR}");
    assert_api_errors(&extract_files(&[("lib.rs", &source)], &["lib.rs"]));
    let empty = extract_files(&[("lib.rs", "pub struct Empty;")], &["lib.rs"]);
    assert_eq!(empty.types.len(), 1);
    assert_eq!(empty.functions.len(), 0);
    let result: syn::Type = syn::parse_str("Result<()>").expect("result type syntax");
    assert_eq!(
        type_resolver::extract_result_error_type(&result).as_deref(),
        Some("anyhow::Error")
    );
}

#[test]
fn forward_aliases_follow_the_owning_directory_of_nested_module_files() {
    let surface = extract_files(
        &[
            ("lib.rs", "pub mod outer;"),
            ("outer.rs", "pub mod api; pub mod error; pub use error::Result;"),
            (
                "outer/api.rs",
                "use super::Result; pub fn run() -> Result<()> { unimplemented!() }",
            ),
            (
                "outer/error/mod.rs",
                "pub struct NestedError; pub type Result<T> = std::result::Result<T, NestedError>;",
            ),
        ],
        &["lib.rs"],
    );
    assert_eq!(surface.functions.len(), 1);
    assert_error(&surface, "run", "NestedError");
}

#[test]
fn forward_foreign_result_reexport_chains_do_not_inherit_local_errors() {
    for foreign in ["anyhow", "eyre", "std::io"] {
        let source = format!(
            r#"
            pub mod api {{
                pub use {foreign}::Result;
                pub mod child {{
                    use super::Result;
                    pub fn foreign() -> Result<()> {{ unimplemented!() }}
                }}
            }}
            pub mod error {{
                pub struct LocalError;
                pub type Result<T> = std::result::Result<T, LocalError>;
            }}
            pub use error::Result;
            "#
        );
        let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "foreign", "anyhow::Error");
    }
}

#[test]
fn forward_private_alias_modules_supply_metadata_without_exposing_private_items() {
    let source = r#"
        pub struct PublicError;
        pub struct OtherError;
        pub mod api {
            use crate::aliases::Result;
            pub fn local() -> Result<()> { unimplemented!() }
        }
        mod aliases {
            pub(crate) type Result<T> = std::result::Result<T, crate::OtherError>;
            pub struct PrivateSentinel;
        }
        pub mod error { pub type Result<T> = std::result::Result<T, crate::PublicError>; }
        pub use error::Result;
    "#;
    let surface = extract_files(&[("lib.rs", source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 1);
    assert_error(&surface, "local", "crate::OtherError");
    let mut names = surface.types.iter().map(|typ| typ.name.as_str()).collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["OtherError", "PublicError"]);
}

#[test]
fn foreign_result_globs_do_not_borrow_private_alias_errors() {
    for import in ["use crate::*;", "use std::io::*;"] {
        let source = format!(
            r#"
            pub use std::io::Result;
            mod hidden {{
                pub struct HiddenError;
                pub type Result<T> = std::result::Result<T, HiddenError>;
            }}
            pub mod api {{ {import} pub fn foreign() -> Result<()> {{ unimplemented!() }} }}
            "#
        );
        let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "foreign", "anyhow::Error");
    }
}

#[test]
fn nested_foreign_super_glob_keeps_the_parent_alias_identity() {
    let source = r#"
        pub struct RootError;
        pub type Result<T> = std::result::Result<T, RootError>;
        pub mod nested {
            pub use std::io::Result;
            pub mod child { use super::*; pub fn foreign() -> Result<()> { unimplemented!() } }
        }
    "#;
    let surface = extract_files(&[("lib.rs", source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 1);
    assert_error(&surface, "foreign", "anyhow::Error");
}

const GLOB_ALIASES: &str = r#"
    pub struct RootError;
    pub type Result<T> = std::result::Result<T, RootError>;
    pub mod empty { pub struct Noise; }
    pub mod chosen { pub struct Selected; pub type Result<T> = std::result::Result<T, Selected>; }
    pub mod exported { pub use crate::chosen::*; }
"#;

#[test]
fn local_result_globs_ignore_targets_that_export_no_result() {
    for imports in [
        "use crate::empty::*; use crate::exported::*;",
        "use crate::exported::*; use crate::empty::*;",
        "use std::collections::*; use crate::empty::*; use crate::exported::*;",
    ] {
        let source = format!(
            "{GLOB_ALIASES} pub mod api {{ {imports} pub fn selected() -> Result<()> {{ unimplemented!() }} }}"
        );
        let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "selected", "Selected");
    }
}

#[test]
fn named_and_declared_result_aliases_take_precedence_over_globs() {
    for alias in [
        "use crate::explicit::Result;",
        "type Result<T> = std::result::Result<T, crate::ExplicitError>;",
    ] {
        let source = format!(
            r#"{GLOB_ALIASES}
            pub struct ExplicitError;
            pub mod explicit {{ pub type Result<T> = std::result::Result<T, crate::ExplicitError>; }}
            pub mod api {{
                use crate::chosen::*;
                {alias}
                pub fn selected() -> Result<()> {{ unimplemented!() }}
            }}"#
        );
        let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "selected", "crate::ExplicitError");
    }
}

#[test]
fn result_glob_cycles_resolve_when_a_real_alias_is_available() {
    let source = r#"
        pub struct RootError;
        pub type Result<T> = std::result::Result<T, RootError>;
        pub mod first { pub use crate::second::*; }
        pub mod second {
            pub use crate::first::*;
            pub struct Selected;
            pub type Result<T> = std::result::Result<T, Selected>;
        }
        pub mod api { use crate::first::*; pub fn selected() -> Result<()> { unimplemented!() } }
    "#;
    let surface = extract_files(&[("lib.rs", source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 1);
    assert_error(&surface, "selected", "Selected");
}

#[test]
fn foreign_result_alias_declarations_do_not_borrow_private_errors() {
    for rhs in ["std::io::Result<T>", "anyhow::Result<T>", "eyre::Result<T>"] {
        let source = format!(
            r#"
            pub type Result<T> = {rhs};
            use imported::*;
            pub mod imported {{
                pub struct ImportedError;
                pub type Result<T> = std::result::Result<T, ImportedError>;
            }}
            mod hidden {{
                pub struct HiddenError;
                pub type Result<T> = std::result::Result<T, HiddenError>;
            }}
            pub fn foreign() -> Result<()> {{ unimplemented!() }}
            "#
        );
        let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "foreign", "anyhow::Error");
    }
}

#[test]
fn local_result_alias_declarations_resolve_their_qualified_target() {
    let source = r#"
        pub type Result<T> = crate::chain::Result<T>;
        use std::io::*;
        mod a_hidden { pub struct HiddenError; pub type Result<T> = std::result::Result<T, HiddenError>; }
        pub fn selected() -> Result<()> { unimplemented!() }
        pub mod chain { pub type Result<T> = super::error::Result<T>; }
        pub mod error { pub struct Selected; pub type Result<T> = std::result::Result<T, Selected>; }
    "#;
    let surface = extract_files(&[("lib.rs", source)], &["lib.rs"]);
    assert_eq!(surface.functions.len(), 1);
    assert_error(&surface, "selected", "Selected");
}

#[test]
fn absolute_foreign_imports_ignore_same_named_local_modules() {
    for import in ["use ::std::io::Result;", "use ::std::io::*;"] {
        let source = format!(
            r#"
            pub mod std {{
                pub mod io {{ pub struct Shadow; pub type Result<T> = ::std::result::Result<T, Shadow>; }}
            }}
            pub mod api {{ {import} pub fn foreign() -> Result<()> {{ unimplemented!() }} }}
            "#
        );
        let surface = extract_files(&[("lib.rs", &source)], &["lib.rs"]);
        assert_eq!(surface.functions.len(), 1);
        assert_error(&surface, "foreign", "anyhow::Error");
    }
}
