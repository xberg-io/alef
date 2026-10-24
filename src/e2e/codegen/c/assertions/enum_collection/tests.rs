use std::collections::{HashMap, HashSet};

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::go::test_backend::enum_collection_support as support;

#[test]
fn issue684_c_collection_accessors_follow_actual_json_field_boundary() {
    for wire in [false, true] {
        let rendered = render(wire);
        assert_eq!(
            rendered.accessors.matches("expected enum collection payload").count(),
            3,
            "{}",
            rendered.accessors
        );
        assert!(!rendered.accessors.contains("\"parts\""), "{}", rendered.accessors);
        assert_eq!(rendered.assertions.len(), 3);
        assert!(rendered.assertions[0].contains("alef_json_array_count"));
        assert!(rendered.assertions[1].contains("hello"));
        assert!(rendered.assertions[2].contains(" world"));
        if wire {
            assert!(rendered.accessors.contains("\"wireText\""))
        }
        verify_ffi_json_producer(wire);
    }
}

#[test]
#[ignore = "native C proof executes real generated runner JSON helpers"]
fn issue684_c_collection_assertions_execute_producer_backed_cases() {
    for wire in [false, true] {
        verify_native(wire)
    }
}

#[test]
fn issue684_c_refuses_an_enum_prefix_that_exposes_a_typed_scalar() {
    use crate::e2e::field_access::FieldResolver;
    let api = support::surface(false);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::from(["content".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&api.types, &api.enums),
        Some("Message".to_string()),
    );
    let sources = super::super::FieldConfigSources {
        result_fields: super::super::EffectiveConfigSource::Global,
        fields: super::super::EffectiveConfigSource::Global,
    };
    let mut output = String::new();
    let mut handles = Vec::new();
    let error = super::emit_ir_nested_accessor(
        &mut output,
        &resolver,
        super::NativeAccessorArgs {
            prefix: "sample",
            resolved: "content.parts",
            local_var: "parts",
            root_expression: "result",
            fields_c_types: &HashMap::from([("message.content".to_string(), "uint64_t".to_string())]),
            fields_enum: &HashSet::new(),
            intermediate_handles: &mut handles,
            result_type_name: "Message",
            raw_field: "content.parts",
            type_defs: &api.types,
            config_sources: &sources,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("must expose JSON"), "{error}");
    assert!(error.to_string().contains("uint64_t"), "{error}");
    assert!(!output.contains("expected enum collection payload"));
}

struct Rendered {
    accessors: String,
    assertions: Vec<String>,
    locals: Vec<String>,
    handles: Vec<(String, String)>,
}

fn render(wire: bool) -> Rendered {
    let api = support::surface(wire);
    let resolver = support::resolver(&api, "c");
    let sources = super::super::FieldConfigSources {
        result_fields: super::super::EffectiveConfigSource::Global,
        fields: super::super::EffectiveConfigSource::Global,
    };
    let mut rendered = Rendered {
        accessors: String::new(),
        assertions: Vec::new(),
        locals: Vec::new(),
        handles: Vec::new(),
    };
    let mut accessed = Vec::new();
    for assertion in support::assertions() {
        let field = assertion.field.as_ref().unwrap();
        let local = field.replace(['.', '['], "_").replace(']', "");
        let outcome = super::emit_ir_nested_accessor(
            &mut rendered.accessors,
            &resolver,
            super::NativeAccessorArgs {
                prefix: "sample",
                resolved: field,
                local_var: &local,
                root_expression: "result",
                fields_c_types: &HashMap::new(),
                fields_enum: &HashSet::new(),
                intermediate_handles: &mut rendered.handles,
                result_type_name: "Reply",
                raw_field: field,
                type_defs: &api.types,
                config_sources: &sources,
            },
        )
        .unwrap();
        assert!(outcome.is_none());
        accessed.push((field.clone(), local.clone(), true));
        rendered.locals.push(local);
    }
    rendered.assertions = render_assertions(&resolver, &accessed);
    rendered
}

fn verify_ffi_json_producer(wire: bool) {
    use quote::ToTokens;
    let files = crate::backends::ffi::FfiBackend
        .generate_bindings(
            &support::surface(wire),
            &ResolvedCrateConfig {
                name: "sample".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
    let functions = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|extension| extension == "rs"))
        .flat_map(|file| syn::parse_file(&file.content).unwrap().items)
        .filter_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "sample_reply_choices" => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(functions.len(), 1, "actual FFI choices getter must be generated");
    let source = functions[0].to_token_stream().to_string();
    assert!(
        source.contains("serde_json :: to_string") && source.contains("choices"),
        "{source}"
    );
}

fn verify_native(wire: bool) {
    let directory = tempfile::tempdir().unwrap();
    let binary = compile_probe(wire, directory.path());
    support::verify_cases(
        support::core_cases(wire),
        [
            "PASS:3",
            "expected 2 elements",
            "FAIL_CASE:1",
            "FAIL_CASE:2",
            "expected enum collection payload",
            "expected enum collection payload",
        ],
        |case| {
            std::process::Command::new(&binary)
                .arg(case.to_string())
                .current_dir(directory.path())
                .output()
                .unwrap()
        },
    );
}

fn render_assertions(
    resolver: &crate::e2e::field_access::FieldResolver,
    accessed: &[(String, String, bool)],
) -> Vec<String> {
    support::assertions()
        .into_iter()
        .map(|assertion| {
            let mut output = String::new();
            super::super::render_assertion(
                &mut output,
                &assertion,
                "result",
                "sample",
                resolver,
                accessed,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            );
            assert!(!output.contains("skipped"), "{output}");
            output
        })
        .collect()
}

fn compile_probe(wire: bool, directory: &std::path::Path) -> std::path::PathBuf {
    let cc = support::tool("cc", directory);
    let rendered = render(wire);
    std::fs::write(
        directory.join("test_runner.h"),
        super::super::super::runner::render_test_runner_header(&[], &[]),
    )
    .unwrap();
    let handles = rendered
        .handles
        .iter()
        .map(|(name, kind)| crate::alef_context! { name => name, free_string => kind == "free_string" })
        .collect::<Vec<_>>();
    let source = crate::e2e::template_env::render(
        "c/enum_collection_native_probe.c.jinja",
        crate::alef_context! {
            accessors => rendered.accessors, assertions => rendered.assertions, locals => rendered.locals, handles => handles,
        },
    );
    let path = directory.join("probe.c");
    let binary = directory.join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&path, source).unwrap();
    let output = std::process::Command::new(cc)
        .args(["-std=c11", "-D_POSIX_C_SOURCE=200809L", "-Werror", "-Wall", "-Wextra"])
        .arg(path)
        .arg("-o")
        .arg(&binary)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    binary
}

/// The test file the generator writes for a real call must read the parts through the union
/// crossing. A resolver assembled by hand in a test can hide a missing enum map.
#[test]
fn the_generated_test_file_reads_the_parts_of_a_content_union() {
    use crate::e2e::codegen::call_ir::CallIr;
    use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::{Assertion, Fixture};

    let mut api = support::surface(false);
    api.functions.push(crate::core::ir::FunctionDef {
        name: "chat".to_string(),
        return_type: crate::core::ir::TypeRef::Named("Reply".to_string()),
        ..Default::default()
    });
    let fixture = Fixture {
        id: "parts_reply".into(),
        description: "Reply with a list of parts".into(),
        call: Some("chat".into()),
        assertions: vec![Assertion {
            assertion_type: "count_equals".into(),
            field: Some("choices[0].message.content.parts".into()),
            value: Some(serde_json::json!(2)),
            ..Default::default()
        }],
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    let mut call = CallConfig {
        function: "chat".to_string(),
        ..CallConfig::default()
    };
    call.overrides.insert(
        "c".to_string(),
        CallOverride {
            function: Some("sample_chat".to_string()),
            ..CallOverride::default()
        },
    );
    e2e.calls.insert("chat".into(), call);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let rendered = super::super::super::render_test_file(
        "chat",
        &[&fixture],
        "sample.h",
        "sample",
        "result",
        &e2e,
        "c",
        &resolver,
        &config,
        &api.types,
        &api.enums,
        &[],
        CallIr {
            functions: &api.functions,
            type_defs: &api.types,
        },
    )
    .expect("a call returning a content union renders");
    assert!(rendered.contains("expected enum collection payload"), "{rendered}");
    assert!(
        !rendered.contains("alef_json_get_string(content_json, \"parts\")"),
        "{rendered}"
    );
}
