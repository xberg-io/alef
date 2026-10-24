use std::collections::{HashMap, HashSet};

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::go::test_backend::enum_collection_support as support;

#[test]
fn issue684_go_collection_assertions_use_actual_raw_enum_representation() {
    for wire in [false, true] {
        let source = generated_binding(wire);
        assert!(source.contains("type Body json.RawMessage"), "{source}");
        assert!(source.contains("func (e Body) MarshalJSON()"), "{source}");
        let assertions = render_assertions(wire);
        assert_eq!(assertions.matches("json.Marshal(").count(), 3, "{assertions}");
        assert!(!assertions.contains(".Parts"), "{assertions}");
        assert!(!assertions.contains("skipped"), "{assertions}");
        if wire {
            assert!(assertions.contains("\"wireText\""), "{assertions}")
        }
    }
}

#[test]
#[ignore = "native Go proof compiles actual generated DTO projections"]
fn issue684_go_collection_assertions_execute_producer_backed_cases() {
    for wire in [false, true] {
        verify_native(wire)
    }
}

fn render_assertions(wire: bool) -> String {
    let api = support::surface(wire);
    let resolver = support::resolver(&api, "go");
    let context = super::super::AssertionRenderContext {
        effective_result_var: "result",
        import_alias: "sample",
        field_resolver: &resolver,
        optional_locals: &HashMap::new(),
        numeric_scalar_fields: &HashSet::new(),
        presence_checked_fields: &HashSet::new(),
        result_is_simple: false,
        result_is_array: false,
        is_streaming: false,
        streaming_item_type: None,
    };
    let mut output = String::new();
    for assertion in support::assertions() {
        super::super::render_assertion(&mut output, &context, &assertion);
    }
    output
}

fn generated_binding(wire: bool) -> String {
    let files = crate::backends::go::GoBackend
        .generate_bindings(
            &support::surface(wire),
            &ResolvedCrateConfig {
                name: "sample".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
    files
        .into_iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("generated Go binding")
        .content
}

fn verify_native(wire: bool) {
    let directory = tempfile::tempdir().unwrap();
    let go = support::tool("go", directory.path());
    prepare_probe(wire, directory.path(), &go);
    support::verify_cases(
        support::core_cases(wire),
        [
            "--- PASS: TestEnumPayload",
            "enum collection count mismatch",
            "parts[0].text",
            "parts[1].text",
            "expected enum collection payload",
            "expected enum collection payload",
        ],
        |case| {
            std::process::Command::new(&go)
                .args(["test", "-v", "-count=1", "-run", "^TestEnumPayload$"])
                .env("GOWORK", "off")
                .env("GOTOOLCHAIN", "local")
                .env("ALEF_PROBE_INPUT", case.to_string())
                .current_dir(directory.path())
                .output()
                .unwrap()
        },
    );
}

fn prepare_probe(wire: bool, directory: &std::path::Path, go: &std::path::Path) {
    std::fs::write(directory.join("go.mod"), "module sample\n\ngo 1.24\n").unwrap();
    let declarations = project_declarations(wire, directory, go);
    let source = crate::e2e::template_env::render(
        "go/enum_collection_native_probe.go.jinja",
        crate::alef_context! {
            declarations => declarations, assertions => render_assertions(wire),
        },
    );
    std::fs::write(directory.join("enum_test.go"), source).unwrap();
}

fn project_declarations(wire: bool, directory: &std::path::Path, go: &std::path::Path) -> String {
    let binding = directory.join("binding.txt");
    std::fs::write(&binding, generated_binding(wire)).unwrap();
    let projector = directory.join("project.go");
    std::fs::write(&projector, include_str!("project_types.go")).unwrap();
    let executable = directory.join(if cfg!(windows) { "project.exe" } else { "project" });
    let output = std::process::Command::new(go)
        .args(["build", "-o"])
        .arg(&executable)
        .arg(&projector)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", "local")
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let projection = std::process::Command::new(executable)
        .arg(binding)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        projection.status.success(),
        "{}",
        String::from_utf8_lossy(&projection.stderr)
    );
    std::fs::remove_file(projector).unwrap();
    String::from_utf8(projection.stdout).unwrap()
}

/// The resolver the generator builds for a real call must resolve the union crossing; a resolver
/// assembled by hand in a test can hide a missing enum map.
#[test]
fn the_per_call_resolver_resolves_the_union_crossing() {
    let mut api = support::surface(false);
    api.functions.push(crate::core::ir::FunctionDef {
        name: "chat".to_string(),
        return_type: crate::core::ir::TypeRef::Named("Reply".to_string()),
        ..Default::default()
    });
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..Default::default()
    };
    let facts = crate::e2e::codegen::go::test_function::call_resolver::GoCrateResolverFacts::new(
        &api.types, &api.enums, &config,
    );
    let call: crate::core::config::e2e::CallConfig = toml::from_str("function = \"chat\"").unwrap();
    let resolver = crate::e2e::codegen::go::test_function::call_resolver::build_call_field_resolver_with_facts(
        &crate::core::config::e2e::E2eConfig::default(),
        &call,
        &api.functions,
        &api.types,
        &facts,
    );
    let (prefix, owner, variant, suffix) = resolver
        .ir_tagged_union_split("choices[0].message.content.parts[0].text")
        .expect("the content enum must be reachable from the call's result type");
    assert_eq!(
        (prefix.as_str(), owner.as_str(), variant.as_str(), suffix.as_str()),
        ("choices[0].message.content", "Body", "Parts", "[0].text")
    );
}
