use crate::e2e::codegen::test_support::typed_variant_list_test_support as support;

fn render(android: bool, enum_elements: bool, wire_rename: bool) -> String {
    let resolver = support::resolver(
        if android { "kotlin_android" } else { "kotlin" },
        enum_elements,
        wire_rename,
    );
    let mut source = String::new();
    for assertion in support::assertions(wire_rename) {
        assert!(super::try_render_generic_union_assertion(
            &mut source,
            &assertion,
            &resolver,
            "result",
            android,
            assertion.field.as_deref().unwrap()
        ));
    }
    source
}

#[test]
fn indexed_list_variant_assertions_check_count_and_exact_text() {
    for (enum_elements, wire_rename) in [(false, false), (true, false), (true, true)] {
        for android in [false, true] {
            support::verify(&render(android, enum_elements, wire_rename), ".size");
        }
    }
}

use crate::e2e::codegen::test_support::typed_variant_list_native_support as native;

#[test]
#[ignore = "requires target compiler, assertion framework and generated binding dependencies"]
fn indexed_list_variant_assertions_run_against_actual_generated_types() {
    for enum_elements in [false, true] {
        for android in [false, true] {
            let language = if android { "kotlin_android" } else { "kotlin" };
            native::run(language, enum_elements, &render(android, enum_elements, enum_elements));
        }
    }
}

/// The Android test package is a child of the binding package, so the test file must import the
/// union and the element union that the typed assertions name.
#[test]
fn android_test_file_imports_the_union_and_element_types_the_assertions_name() {
    use crate::core::ir::{FunctionDef, TypeRef};
    use crate::e2e::config::{CallConfig, E2eConfig};
    use crate::e2e::fixture::Fixture;

    let api = support::api(true, false);
    let fixture = Fixture {
        id: "parts".into(),
        description: "parts".into(),
        assertions: support::assertions(false),
        ..Fixture::default()
    };
    let functions = vec![FunctionDef {
        name: "read_envelope".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..FunctionDef::default()
    }];
    let e2e = E2eConfig {
        call: CallConfig {
            function: "read_envelope".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        ..Default::default()
    };
    let render = |android: bool| {
        crate::e2e::codegen::kotlin::test_file::render_test_file_inner(
            "parts",
            &[&fixture],
            "SampleClient",
            "readEnvelope",
            "dev.sample.android",
            "result",
            &[],
            None,
            false,
            &e2e,
            &std::collections::HashMap::new(),
            android,
            &crate::core::config::ResolvedCrateConfig::default(),
            &api.types,
            &api.enums,
            &functions,
        )
        .expect("the test file renders")
    };
    let android = render(true);
    for name in ["Payload", "Piece"] {
        assert!(
            android.contains(&format!("import dev.sample.android.{name}\n")),
            "{android}"
        );
    }
    assert!(!render(false).contains("import dev.sample.android.Payload\n"));
}
