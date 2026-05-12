//! Tests for the e2e language resolution: e2e generation must only emit code
//! for languages the consumer has actually scaffolded — never for backends
//! that don't have a binding crate.

use alef_core::config::Language;
use alef_e2e::default_e2e_languages;

#[test]
fn default_includes_each_scaffolded_language() {
    let scaffolded = vec![Language::Python, Language::Node, Language::Ruby];
    let names = default_e2e_languages(&scaffolded);
    assert!(names.contains(&"python".to_string()));
    assert!(names.contains(&"node".to_string()));
    assert!(names.contains(&"ruby".to_string()));
}

#[test]
fn default_maps_ffi_to_c_generator() {
    let scaffolded = vec![Language::Ffi];
    let names = default_e2e_languages(&scaffolded);
    assert!(
        names.contains(&"c".to_string()),
        "Language::Ffi should map to the `c` e2e generator (got {names:?})"
    );
    assert!(
        !names.contains(&"ffi".to_string()),
        "no e2e generator is named `ffi`; FFI binding is exercised via the C harness"
    );
}

#[test]
fn default_always_includes_rust_source() {
    let scaffolded = vec![Language::Python];
    let names = default_e2e_languages(&scaffolded);
    assert!(
        names.contains(&"rust".to_string()),
        "rust e2e tests exercise the source crate and must always be emitted"
    );
}

#[test]
fn default_does_not_duplicate_rust_when_already_listed() {
    let scaffolded = vec![Language::Rust, Language::Python];
    let names = default_e2e_languages(&scaffolded);
    let rust_count = names.iter().filter(|n| *n == "rust").count();
    assert_eq!(rust_count, 1, "rust must appear exactly once (got {names:?})");
}

#[test]
fn default_excludes_brew_and_other_non_binding_generators() {
    let scaffolded = vec![
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Ffi,
        Language::Go,
        Language::Java,
        Language::Csharp,
        Language::Elixir,
        Language::Wasm,
        Language::R,
    ];
    let names = default_e2e_languages(&scaffolded);
    // brew is a CLI test runner with no Language enum variant — it must be
    // opt-in via [e2e].languages.
    assert!(
        !names.contains(&"brew".to_string()),
        "brew must not appear in default e2e languages; it requires explicit opt-in"
    );
}

#[test]
fn default_excludes_languages_not_in_scaffold_list() {
    let scaffolded = vec![Language::Python, Language::Node];
    let names = default_e2e_languages(&scaffolded);
    // kotlin, dart, swift, zig should NOT be generated unless
    // scaffolded — this is the bug the regression test guards against.
    for unscaffolded in ["kotlin", "dart", "swift", "zig"] {
        assert!(
            !names.contains(&unscaffolded.to_string()),
            "{unscaffolded} must not appear when only {scaffolded:?} are scaffolded (got {names:?})"
        );
    }
}

#[test]
fn default_handles_empty_scaffold_list() {
    let names = default_e2e_languages(&[]);
    // Even with no bindings configured, the rust source-language e2e suite is
    // emitted so users get at least the core-crate exercise out of the box.
    assert_eq!(names, vec!["rust".to_string()]);
}
