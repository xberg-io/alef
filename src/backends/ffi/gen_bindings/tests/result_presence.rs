use super::super::FfiBackend;
use super::super::functions::{
    gen_free_function, gen_free_function_result_presence_wrapper, gen_method_result_presence_wrapper,
    gen_method_wrapper,
};
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

fn optional_u64(name: &str, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        error_type: error_type.map(str::to_string),
        ..FunctionDef::default()
    }
}

/// Wiring test: the real `Backend::generate_bindings` entry point must emit a `_has_result`
/// companion for every free function and method whose return type is `Optional<T>` where `T`'s
/// leaf is ambiguous (ints/floats/bool/Duration), for both the fallible and infallible shape --
/// and must NOT emit one for an owned-receiver method (a second call would consume an
/// already-removed handle) or for a leaf that already has a real null (`String`). This proves the
/// gates in `lib_rs.rs`'s free-function and method loops both reach the return path, not just the
/// field path fields.rs already covers. ~keep
#[test]
fn generated_bindings_emit_result_presence_companion_for_every_ambiguous_optional_return_only() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![
            optional_u64("get_port", None),
            optional_u64("checked_port", Some("String")),
        ],
        types: vec![TypeDef {
            name: "SampleConfig".to_string(),
            rust_path: "my_lib::SampleConfig".to_string(),
            is_clone: true,
            methods: vec![
                MethodDef {
                    name: "margin_fraction".to_string(),
                    return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                    receiver: Some(ReceiverKind::Ref),
                    ..MethodDef::default()
                },
                MethodDef {
                    name: "consume".to_string(),
                    return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                    receiver: Some(ReceiverKind::Owned),
                    ..MethodDef::default()
                },
                MethodDef {
                    name: "label".to_string(),
                    return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                    receiver: Some(ReceiverKind::Ref),
                    ..MethodDef::default()
                },
            ],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "smp"
"#,
    );
    let backend = FfiBackend;
    let lib = backend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .unwrap()
        .content;

    assert!(
        lib.contains("fn smp_get_port_has_result(\n) -> i32"),
        "infallible free function returning Option<u64> must get a presence companion, got:\n{lib}"
    );
    assert!(
        lib.contains("i32::from(result.is_some())"),
        "infallible presence companion must check result.is_some(), got:\n{lib}"
    );
    assert!(
        lib.contains("fn smp_checked_port_has_result(\n) -> i32"),
        "fallible free function returning Option<u64> must get a presence companion, got:\n{lib}"
    );
    assert!(
        lib.contains("i32::from(val.is_some())"),
        "fallible presence companion must check val.is_some() inside the Ok arm, got:\n{lib}"
    );
    assert!(
        lib.contains("fn smp_sample_config_margin_fraction_has_result(\n    this: AlefHandle) -> i32"),
        "&self method returning Option<f64> must get a presence companion, got:\n{lib}"
    );
    assert!(
        !lib.contains("smp_sample_config_consume_has_result"),
        "owned-receiver method must NOT get a presence companion (second call would consume an \
         already-removed handle), got:\n{lib}"
    );
    assert!(
        !lib.contains("smp_sample_config_label_has_result"),
        "String-returning method already has a real null on None and must NOT get a presence \
         companion, got:\n{lib}"
    );
}

/// Compiles and runs the ACTUAL generated `_has_result` companions together with their sibling
/// getters, proving at runtime -- not just by string-matching the rendered source -- that `None`
/// and a zero-valued `Some` are distinguishable through BOTH the free-function and the method
/// codegen path, and for both the infallible and fallible (`Result<Option<T>, E>`) shape. Mirrors
/// `presence_companion_distinguishes_none_from_zero_valued_some_at_runtime` in fields.rs, which
/// covers the struct-field path; this covers the return path that field companion never touched.
/// ~keep
#[test]
fn result_presence_companion_distinguishes_none_from_zero_valued_some_at_runtime() {
    let port_present = FunctionDef {
        name: "port_present".to_string(),
        rust_path: "sample_core::port_present".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        ..FunctionDef::default()
    };
    let port_absent = FunctionDef {
        name: "port_absent".to_string(),
        rust_path: "sample_core::port_absent".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        ..FunctionDef::default()
    };
    let checked_present = FunctionDef {
        name: "checked_present".to_string(),
        rust_path: "sample_core::checked_present".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        error_type: Some("String".to_string()),
        ..FunctionDef::default()
    };
    let checked_absent = FunctionDef {
        name: "checked_absent".to_string(),
        rust_path: "sample_core::checked_absent".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        error_type: Some("String".to_string()),
        ..FunctionDef::default()
    };
    let checked_err = FunctionDef {
        name: "checked_err".to_string(),
        rust_path: "sample_core::checked_err".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        error_type: Some("String".to_string()),
        ..FunctionDef::default()
    };
    let functions = [
        &port_present,
        &port_absent,
        &checked_present,
        &checked_absent,
        &checked_err,
    ];

    let typ = TypeDef {
        name: "SampleConfig".to_string(),
        rust_path: "sample_core::SampleConfig".to_string(),
        is_clone: true,
        ..TypeDef::default()
    };
    let margin_method = MethodDef {
        name: "margin_fraction".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };

    let empty_map = ahash::AHashMap::new();
    let empty_set = ahash::AHashSet::new();

    let mut generated = String::new();
    for func in functions {
        generated.push_str(&gen_free_function(
            func,
            "smp",
            "sample_core",
            &empty_map,
            &empty_set,
            &empty_set,
            None,
            false,
        ));
        generated.push('\n');
        if let Some(presence) =
            gen_free_function_result_presence_wrapper(func, "smp", "sample_core", &empty_map, &empty_set)
        {
            generated.push_str(&presence);
            generated.push('\n');
        }
    }
    generated.push_str(&gen_method_wrapper(
        &typ,
        &margin_method,
        "smp",
        "sample_core",
        &empty_map,
        &empty_set,
        &empty_set,
    ));
    generated.push('\n');
    let method_presence =
        gen_method_result_presence_wrapper(&typ, &margin_method, "smp", "sample_core", &empty_map, &empty_set)
            .expect("Option<f64> &self method must get a presence companion");
    generated.push_str(&method_presence);
    generated.push('\n');

    let last_error = crate::backends::ffi::template_env::render(
        "last_error.jinja",
        minijinja::context! {
            prefix => "smp",
            builtin_prefix => "",
            error_code_impls => Vec::<String>::new(),
            has_error_code_impls => false,
            taxonomy => Vec::<String>::new(),
            no_error_code => 0,
            conversion_error_code => 1,
            unknown_error_code => 2,
            panic_error_code => 3,
            invalid_handle_error_code => 4,
        },
    );
    // Same excision fields.rs's compile-and-run test uses: `insert_serialized_handle` pulls in
    // serde/serde_json, unneeded here (SampleConfig has no lifetime params) and unavailable to a
    // bare `rustc` invocation with no Cargo dependency graph.
    let mut handle_registry =
        crate::backends::ffi::template_env::render("handle_registry.rs.jinja", minijinja::context! {});
    let serialized_start = handle_registry
        .find("struct SerializedHandle")
        .expect("serialized helper start");
    let core_registry_resume = handle_registry[serialized_start..]
        .find("fn with_handle")
        .map(|offset| serialized_start + offset)
        .expect("core registry helpers resume");
    handle_registry.replace_range(serialized_start..core_registry_resume, "");

    let source = format!(
        r#"
use std::cell::RefCell;
use std::ffi::{{c_char, CString}};

mod sample_core {{
    #[derive(Clone, Default)]
    pub struct SampleConfig {{
        pub margin: Option<f64>,
    }}

    impl SampleConfig {{
        pub fn margin_fraction(&self) -> Option<f64> {{
            self.margin
        }}
    }}

    pub fn port_present() -> Option<u64> {{ Some(0) }}
    pub fn port_absent() -> Option<u64> {{ None }}
    pub fn checked_present() -> Result<Option<u64>, String> {{ Ok(Some(0)) }}
    pub fn checked_absent() -> Result<Option<u64>, String> {{ Ok(None) }}
    pub fn checked_err() -> Result<Option<u64>, String> {{ Err("boom".to_string()) }}
}}

{last_error}
{handle_registry}
{generated}

fn main() {{
  unsafe {{
    // Direction 1: the getter alone cannot tell `None` from `Some(zero)` for either codegen
    // path -- both collapse to the same sentinel. Asserted first so a future regression that
    // removes the presence companion's reason to exist is still caught.
    assert_eq!(smp_port_present(), 0);
    assert_eq!(smp_port_absent(), 0);
    assert_eq!(smp_checked_present(), 0);
    assert_eq!(smp_checked_absent(), 0);

    let absent = insert_handle(sample_core::SampleConfig::default()).expect("insert absent");
    let present_zero = insert_handle(sample_core::SampleConfig {{ margin: Some(0.0) }})
        .expect("insert present-zero");
    assert_eq!(smp_sample_config_margin_fraction(absent), 0.0);
    assert_eq!(smp_sample_config_margin_fraction(present_zero), 0.0);

    // Direction 2: the presence companion distinguishes them, for both the free-function and
    // the method codegen path, and for both the infallible and fallible shape.
    assert_eq!(smp_port_present_has_result(), 1);
    assert_eq!(smp_port_absent_has_result(), 0);
    assert_eq!(smp_checked_present_has_result(), 1);
    assert_eq!(smp_checked_absent_has_result(), 0);
    assert_eq!(smp_checked_err_has_result(), -1);
    assert_eq!(smp_sample_config_margin_fraction_has_result(absent), 0);
    assert_eq!(smp_sample_config_margin_fraction_has_result(present_zero), 1);

    // An invalid handle reports -1 on the presence channel too, distinct from both 0 and 1.
    assert_eq!(smp_sample_config_margin_fraction_has_result(0), -1);
  }}
}}
"#
    );

    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("result_presence.rs");
    let binary_path = directory.path().join("result-presence-test");
    std::fs::write(&source_path, &source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        compile.status.success(),
        "{}\n---source---\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}
