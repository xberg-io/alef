//! Real compile-and-execute oracle for `gen_constructor_field_inits`.
//!
//! Every test in the sibling `tests.rs` asserts on the TEXT the function returns. Text is not
//! behavior: the emitted `prelude`/`field_inits` are Rust source for a `#[php(constructor)]
//! pub fn new(...)` that ext-php-rs compiles into the extension's native constructor -- PHP never
//! executes this logic directly, and the only PHP-visible artifact (the PHPStan stub) is a
//! declaration with no body, so `php -l`/PHPStan parsing it proves syntax and nothing about the
//! value a real call would produce. Building the real `.so` needs `cargo build`, which the
//! automation driving this change forbids at this layer.
//!
//! So this module drops one level instead of skipping the question: it takes the REAL
//! `prelude`/`field_inits` strings [`super::gen_constructor_field_inits`] returns, embeds them in
//! a standalone Rust program that builds one value through them, compiles that program with a
//! bare `rustc` invocation (no `Cargo.toml`, no dependency resolution, so no nested `cargo`), runs
//! the resulting binary, and asserts on the field values it actually prints. That is a real
//! compile-and-execute oracle for the exact logic under test, using the one toolchain call this
//! layer permits standalone.
//!
//! Refusal (`classify_omitted_field`'s `bail!` arm) has no counterpart here: when generation
//! refuses, no source is emitted at all, so there is nothing to compile or run. The sibling
//! `tests.rs` refusal tests already exercise that path at the only level it exists -- calling the
//! real function and observing the real `Err` it returns. ~keep

use std::process::{Command, Output};

use super::*;

/// `CORE_TYPE`/`names`/`field`/`optional_field`/`rule_list`/`nested_policy`/`policy`/`build`
/// below are deliberately re-declared rather than reached into from the sibling `tests` module:
/// this file is a SIBLING of `tests`, not a child of it, so `tests`'s helpers are private to a
/// module tree this one is not in. Keeping the two test modules mutually independent also keeps
/// "does the emitted text say the right thing" and "does the emitted text, compiled, DO the right
/// thing" from becoming coupled through shared fixture-builder internals. ~keep
const CORE_TYPE: &str = "sample_core::FetchPolicy";

fn names(values: &[&str]) -> AHashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn field(name: &str, ty: TypeRef, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        typed_default,
        ..Default::default()
    }
}

fn optional_field(name: &str, ty: TypeRef, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        optional: true,
        ..field(name, ty, typed_default)
    }
}

/// See the sibling `tests.rs`'s identically-named helper for why this must stay a type this
/// crate's `untagged_data_enum_names` set names, not a plain non-opaque, non-enum struct.
fn rule_list() -> TypeRef {
    TypeRef::Vec(Box::new(TypeRef::Named("Rule".to_string())))
}

/// See the sibling `tests.rs`'s identically-named helper for why this must stay a type this
/// crate's `untagged_data_enum_names` set names, not a plain nested struct.
fn nested_policy() -> TypeRef {
    TypeRef::Named("SsrfPolicy".to_string())
}

fn policy(has_default: bool, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "FetchPolicy".to_string(),
        rust_path: CORE_TYPE.to_string(),
        has_default,
        has_serde: true,
        fields,
        ..Default::default()
    }
}

fn build(typ: &TypeDef) -> anyhow::Result<ConstructorInit> {
    build_with_retained_cfg(typ, &[])
}

fn build_with_retained_cfg(typ: &TypeDef, never_skip_cfg_field_names: &[String]) -> anyhow::Result<ConstructorInit> {
    let optional_field_names: AHashSet<&str> = typ
        .fields
        .iter()
        .filter(|field| field.optional || field.has_bare_serde_enum_default())
        .map(|field| field.name.as_str())
        .collect();
    gen_constructor_field_inits(
        typ,
        &names(&["Mode"]),
        &names(&["Client"]),
        &names(&["SsrfPolicy", "Rule"]),
        never_skip_cfg_field_names,
        &optional_field_names,
    )
}

/// Compiles `source` with a bare `rustc` invocation and runs the resulting binary.
///
/// Panics if compilation itself fails -- that is a bug in the harness or in the string a caller
/// spliced into it, not a signal under test. Only the RUN's exit status/stdout/stderr is left for
/// the caller to assert on, because the fabrication this module refuses is a runtime value, not a
/// syntax error; a broken-control test that never gets this far would prove nothing. ~keep
fn compile_and_run(source: &str) -> Output {
    let dir = tempfile::tempdir().expect("tempdir for rustc harness");
    let source_path = dir.path().join("oracle.rs");
    std::fs::write(&source_path, source).expect("write harness source");
    let binary_path = dir.path().join(if cfg!(windows) { "oracle.exe" } else { "oracle" });

    let compile = Command::new("rustc")
        .arg("--edition")
        .arg("2024")
        .arg("-o")
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("invoke rustc");
    assert!(
        compile.status.success(),
        "harness source failed to compile -- this is a bug in the test, not the generator under \
         test:\n{}\nsource:\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );

    Command::new(&binary_path)
        .output()
        .expect("run compiled harness binary")
}

/// Shared `SsrfMode` enum every scenario's `FetchPolicy` uses for its `ssrf` field. ~keep
const SSRF_MODE_ENUM: &str = r#"
#[derive(Debug, PartialEq)]
enum SsrfMode {
    #[allow(dead_code)]
    Open,
    Strict,
}
"#;

/// `FetchPolicy` with both fields the two-field scenarios below omit from their constructor,
/// plus the `impl Default` the emitted `<Self as Default>::default()` reads back from. `ssrf`
/// defaults to `Some(SsrfMode::Strict)` and `allow_list` to a non-empty list deliberately: `None`
/// and `[]` are also what a fabrication would produce, so a fixture that already held them could
/// not tell a correct read-back from an invented one.
///
/// Must declare exactly the fields the scenario's `field_inits` initialises -- `Self { .. }`
/// requires every field, so a support struct with an extra field the emitted text never mentions
/// fails to compile, and [`compile_and_run`] treats a compile failure as a harness bug rather than
/// a signal under test. The single-field scenarios use [`ssrf_only_support`] instead. ~keep
fn two_field_support() -> String {
    format!(
        "{SSRF_MODE_ENUM}\n#[derive(Debug)]\nstruct FetchPolicy {{\n    allow_list: Vec<String>,\n    ssrf: \
         Option<SsrfMode>,\n}}\n\nimpl Default for FetchPolicy {{\n    fn default() -> Self {{\n        \
         FetchPolicy {{\n            allow_list: vec![\"internal.example\".to_string()],\n            ssrf: \
         Some(SsrfMode::Strict),\n        }}\n    }}\n}}\n"
    )
}

/// `FetchPolicy` with only the `ssrf` field, for the single-field scenarios -- see
/// [`two_field_support`] for why the field set must match the scenario's `field_inits` exactly.
fn ssrf_only_support() -> String {
    format!(
        "{SSRF_MODE_ENUM}\n#[derive(Debug)]\nstruct FetchPolicy {{\n    ssrf: Option<SsrfMode>,\n}}\n\nimpl \
         Default for FetchPolicy {{\n    fn default() -> Self {{\n        FetchPolicy {{ ssrf: \
         Some(SsrfMode::Strict) }}\n    }}\n}}\n"
    )
}

/// Wraps a `ConstructorInit`'s `prelude`/`field_inits` in a standalone program that builds one
/// `FetchPolicy` through them and asserts on its actual fields, exactly as the real generated
/// `#[php(constructor)] pub fn new(...)` builds `Self { .. }` through the same two strings.
fn harness_source(support: &str, init: &ConstructorInit, assertion: &str) -> String {
    format!(
        "{support}\nimpl FetchPolicy {{\n    fn constructed() -> Self {{\n        {prelude}Self {{ {field_inits} }}\n    }}\n}}\n\nfn main() {{\n    let value = FetchPolicy::constructed();\n    {assertion}\n    println!(\"oracle ok: {{value:?}}\");\n}}\n",
        prelude = init.prelude,
        field_inits = init.field_inits,
    )
}

const SSRF_SURVIVES_ASSERTION: &str = r#"assert_eq!(value.ssrf, Some(SsrfMode::Strict), "authored Some(..) core default must survive construction, got {:?}", value.ssrf);"#;

fn cfg_field(name: &str) -> FieldDef {
    let mut field = field(name, TypeRef::String, Some(DefaultValue::Empty));
    field.cfg = Some("feature = \"enterprise\"".to_string());
    field
}

#[test]
fn compiled_constructor_omits_a_cfg_field_stripped_from_the_binding_struct() {
    let typ = policy(
        true,
        vec![
            field("name", TypeRef::String, Some(DefaultValue::Empty)),
            cfg_field("feature_only"),
        ],
    );
    let init = build(&typ).expect("a stripped cfg field needs no initializer");
    let source = harness_source(
        "#[derive(Debug)]\nstruct FetchPolicy { name: String }\n",
        &init,
        r#"assert_eq!(value.name, "enabled");"#,
    )
    .replace(
        "fn constructed() -> Self {",
        "fn constructed() -> Self { let name = \"enabled\".to_string();",
    );
    let output = compile_and_run(&source);

    assert!(
        output.status.success(),
        "constructor and stripped binding struct must compile together:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn compiled_constructor_initializes_a_cfg_field_retained_by_policy() {
    let typ = policy(
        true,
        vec![
            field("name", TypeRef::String, Some(DefaultValue::Empty)),
            cfg_field("feature_only"),
        ],
    );
    let init = build_with_retained_cfg(&typ, &["feature_only".to_string()])
        .expect("a retained cfg field is initialized without a PHP parameter");
    let source = harness_source(
        "#[derive(Debug)]\nstruct FetchPolicy { name: String, feature_only: String }\n",
        &init,
        r#"assert_eq!(value.name, "enabled"); assert_eq!(value.feature_only, "");"#,
    )
    .replace(
        "fn constructed() -> Self {",
        "fn constructed() -> Self { let name = \"enabled\".to_string();",
    );
    let output = compile_and_run(&source);

    assert!(
        output.status.success(),
        "constructor and retained binding struct must compile together:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Positive oracle, both halves of the brief's claim in one construction: a manually-authored
/// non-empty `allow_list` and a `Some(..)`-holding `ssrf` must both come out of the compiled
/// constructor unchanged, not replaced by the empty/`None` a fabrication would substitute.
#[test]
fn compiled_constructor_preserves_a_real_non_empty_list_and_a_real_some_default() {
    let typ = policy(
        true,
        vec![
            field(
                "allow_list",
                rule_list(),
                Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
            ),
            optional_field(
                "ssrf",
                nested_policy(),
                Some(DefaultValue::EnumVariant("Strict".to_string())),
            ),
        ],
    );
    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    let source = harness_source(
        &two_field_support(),
        &init,
        &format!(
            r#"assert_eq!(value.allow_list, vec!["internal.example".to_string()], "allow_list must survive construction, got {{:?}}", value.allow_list);
    {SSRF_SURVIVES_ASSERTION}"#
        ),
    );
    let output = compile_and_run(&source);

    assert!(
        output.status.success(),
        "compiled constructor must preserve the real core default at runtime:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("internal.example"),
        "the surviving value must be the authored one, not merely a successful exit"
    );
}

/// Broken control for the test above, through the IDENTICAL harness and assertions: substitutes
/// the fragment `Default::default()` -- the module's own docs name it "the unconditional answer
/// this replaced" -- for both fields in place of the real `gen_constructor_field_inits` output.
/// Must fail the same runtime assertion the fixed generator passes, proving the harness actually
/// discriminates fabricated output from real output rather than merely running a program.
#[test]
fn compiled_constructor_rejects_a_fabricated_type_zero_in_place_of_the_real_core_default() {
    let typ = policy(
        true,
        vec![
            field(
                "allow_list",
                rule_list(),
                Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
            ),
            optional_field(
                "ssrf",
                nested_policy(),
                Some(DefaultValue::EnumVariant("Strict".to_string())),
            ),
        ],
    );
    let mut init = build(&typ).expect("a type with a Default impl can always be read back from");
    // The fabrication this module exists to refuse, injected directly rather than produced by the
    // (now-fixed) function, so this control still exercises the assertion even though nothing in
    // the current generator can reach this text any more.
    init.field_inits = "allow_list: Default::default(), ssrf: Default::default()".to_string();

    let source = harness_source(
        &two_field_support(),
        &init,
        &format!(
            r#"assert_eq!(value.allow_list, vec!["internal.example".to_string()], "allow_list must survive construction, got {{:?}}", value.allow_list);
    {SSRF_SURVIVES_ASSERTION}"#
        ),
    );
    let output = compile_and_run(&source);

    assert!(
        !output.status.success(),
        "the fabricated Default::default() fragment must fail the SAME assertion the fixed \
         generator passes; if this ever succeeds, the oracle has stopped discriminating between \
         correct and fabricated generated code:\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("allow_list must survive construction"),
        "failure must be the specific allow_list assertion, not an unrelated panic, got:\n{stderr}"
    );
}

/// Broken control tied to the specific historical defect this commit range removed: the deleted
/// `OmittedInit::Absent` arm emitted the literal fragment `format!("{{}}: None", field.name)` for
/// an omitted `Option` field. That arm's own trigger condition never overlapped with an available
/// core `Default` (the `has_default` check always won first -- see `git log -p` on this file), so
/// this does not resurrect the exact old trigger; it substitutes the arm's literal output directly
/// to prove that fragment fails the same runtime assertion a real read-back passes, which is the
/// guarantee that must keep holding if anything like that arm is ever reintroduced.
#[test]
fn compiled_constructor_rejects_the_removed_fabricated_none_fragment() {
    let typ = policy(
        true,
        vec![optional_field(
            "ssrf",
            nested_policy(),
            Some(DefaultValue::EnumVariant("Strict".to_string())),
        )],
    );
    let mut init = build(&typ).expect("a type with a Default impl can always be read back from");
    init.field_inits = "ssrf: None".to_string();

    let source = harness_source(&ssrf_only_support(), &init, SSRF_SURVIVES_ASSERTION);
    let output = compile_and_run(&source);

    assert!(
        !output.status.success(),
        "the fabricated `None` fragment must fail the same assertion a real read-back passes:\n\
         stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("authored Some(..) core default must survive construction"),
        "failure must be the specific assertion, not an unrelated panic, got:\n{stderr}"
    );
}

/// Legitimate control in the opposite direction: an authored `None` default must be honored EVEN
/// when the owning type's core `Default` would answer `Some(..)`, proving the constructor does
/// not just always defer to the core default once one happens to be available. `tests.rs`'s
/// `should_lower_an_omitted_option_field_whose_recorded_default_is_none` pins this fact on the
/// emitted text; this pins it on the runtime value that text actually produces once compiled.
#[test]
fn compiled_constructor_honors_an_authored_none_default_over_a_some_holding_core_default() {
    let typ = policy(
        true,
        vec![optional_field("ssrf", nested_policy(), Some(DefaultValue::None))],
    );
    let init = build(&typ).expect("a recorded null default is authored, not invented");
    assert_eq!(
        init.field_inits, "ssrf: Default::default()",
        "must take the TypeZero path, not read the core default, for a recorded None"
    );

    let source = harness_source(
        &ssrf_only_support(),
        &init,
        r#"assert_eq!(value.ssrf, None, "an authored null default must be honored even though the core Default holds Some(..), got {:?}", value.ssrf);"#,
    );
    let output = compile_and_run(&source);

    assert!(
        output.status.success(),
        "compiled constructor must honor the authored None default:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
