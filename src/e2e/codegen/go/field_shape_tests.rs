use std::collections::HashSet;
use std::path::PathBuf;

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::assertion_field_shape::resolve_assertion_field_shape;
use super::go_batch::{GoBatchCase, GoBatchLayout, GoCaseOutcome, run_go_batch};
use super::test_function::{GoTestFunctionContext, render_test_function};

/// Import path of the throwaway module that holds every rendered-assertion case as its own
/// package. One module means one `go test` process for the whole set; separate packages keep
/// each case compiled on its own, so an unused import or a build error still fails only the
/// case that caused it. ~keep
const SHAPE_BATCH_MODULE: &str = "example.com/shapes";

/// Every rendered-assertion case that must compile and run. Asserted as a set against the
/// packages `go test` reports on: a batch that silently selects fewer packages exits 0 and
/// is otherwise indistinguishable from a real pass. ~keep
const RENDERED_SHAPE_CASE_COUNT: usize = 45;

/// A case whose rendered Go cannot build. It shares the batch with the real cases to prove
/// the single invocation still surfaces a failure instead of swallowing it, and that doing
/// so does not disturb the verdict on any other case. ~keep
const BROKEN_SOURCE_CONTROL: &str = "compile_control_broken_source";

const PSEUDO_FIELD_SUFFIXES: [&str; 3] = ["length", "count", "size"];

const PSEUDO_FIELD_ASSERTIONS: [&str; 6] = [
    "greater_than",
    "less_than_or_equal",
    "count_min",
    "count_equals",
    "min_length",
    "max_length",
];

const DATA_INTERFACE_STRING_FAMILIES: [(&str, &str); 5] = [
    ("equals", "value"),
    ("contains", "value"),
    ("contains_all", "value"),
    ("not_contains", "absent"),
    ("contains_any", "value"),
];

const SAMPLE_DATA_INTERFACE: &str = "package sample\ntype Choice interface{}\ntype Envelope struct { Choice Choice }\nfunc Inspect() (*Envelope, error) { return &Envelope{Choice: \"value\"}, nil }\n";

const SAMPLE_RAW_MESSAGE: &str = "package sample\nimport \"encoding/json\"\ntype Envelope struct { Payload *json.RawMessage }\nfunc Inspect() (*Envelope, error) { raw := json.RawMessage(`{\"value\":\"sample\"}`); return &Envelope{Payload: &raw}, nil }\n";

const SAMPLE_LABEL_POINTER: &str = "package sample\ntype Envelope struct { Label *string }\nfunc Inspect() (*Envelope, error) { value := \"sample\"; return &Envelope{Label: &value}, nil }\n";

const SAMPLE_LABEL_NIL: &str = "package sample\ntype Envelope struct { Label *string }\nfunc Inspect() (*Envelope, error) { return &Envelope{Label: nil}, nil }\n";

const SAMPLE_LIMIT_POINTER: &str = "package sample\ntype Envelope struct { Limit *int64 }\nfunc Inspect() (*Envelope, error) { value := int64(5); return &Envelope{Limit: &value}, nil }\n";

/// Package one rendered assertion as a Go package inside the batch module: the sample
/// package under test plus the external test file that exercises it.
fn rendered_case(name: &str, rendered: &str, sample_source: &str) -> GoBatchCase {
    assert!(
        emitted_test_functions(rendered) >= 1,
        "a rendered case must emit at least one Go test function:\n{rendered}"
    );
    let mut imports = vec![
        "\"testing\"".to_owned(),
        format!("sample \"{SHAPE_BATCH_MODULE}/{name}\""),
    ];
    if rendered.contains("strings.") {
        imports.push("\"strings\"".to_owned());
    }
    if rendered.contains("jsonString(") {
        imports.push("\"encoding/json\"".to_owned());
    }
    let assertion_stub = if rendered.contains("assert.") {
        "type assertions struct{}\nvar assert assertions\nfunc (assertions) NotNil(*testing.T, any, ...string) {}\nfunc (assertions) GreaterOrEqual(*testing.T, any, any, ...string) {}\nfunc (assertions) LessOrEqual(*testing.T, any, any, ...string) {}\nfunc (assertions) Equal(*testing.T, any, any, ...string) {}\n"
    } else {
        ""
    };
    let json_stub = if rendered.contains("jsonString(") {
        "func jsonString(t *testing.T, value any) string { t.Helper(); data, err := json.Marshal(value); if err != nil { t.Fatal(err) }; return string(data) }\n"
    } else {
        ""
    };
    let source = format!(
        "package sample_test\nimport ({})\n{assertion_stub}{json_stub}\n{rendered}",
        imports.join("\n")
    );
    GoBatchCase {
        name: name.to_owned(),
        files: vec![
            ("sample.go".to_owned(), sample_source.to_owned()),
            ("shape_test.go".to_owned(), source),
        ],
    }
}

fn emitted_test_functions(rendered: &str) -> usize {
    rendered.lines().filter(|line| line.starts_with("func Test")).count()
}

fn render_field_assertion(
    field: FieldDef,
    assertion_field: &str,
    enums: &[EnumDef],
    configured_optional: bool,
    assertion_type: &str,
    value: Option<serde_json::Value>,
) -> String {
    let mut optional = HashSet::new();
    if configured_optional {
        optional.insert(field.name.clone());
    }
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        fields_optional: optional,
        ..Default::default()
    };
    let uses_values = matches!(assertion_type, "contains_all" | "contains_any" | "not_contains");
    let values = uses_values.then(|| vec![value.clone().expect("string family value")]);
    let fixture = Fixture {
        id: "field_shape".into(),
        description: "field shape".into(),
        assertions: vec![Assertion {
            assertion_type: assertion_type.into(),
            field: Some(assertion_field.into()),
            value: (!uses_values).then_some(value).flatten(),
            values,
            ..Default::default()
        }],
        ..Default::default()
    };
    render_fixture(config, fixture, field, enums)
}

fn render_fixture(config: E2eConfig, fixture: Fixture, field: FieldDef, enums: &[EnumDef]) -> String {
    let type_defs = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![field],
        ..Default::default()
    }];
    let functions = vec![FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    let mut output = String::new();
    render_test_function(
        &mut output,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample",
            e2e_config: &config,
            adapters: &[],
            data_enum_names: &HashSet::new(),
            config: &Default::default(),
            type_defs: &type_defs,
            enums,
            errors: &[],
            functions: &functions,
        },
    );
    output
}

fn data_choice_enum() -> EnumDef {
    EnumDef {
        name: "Choice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            fields: vec![FieldDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn label_field() -> FieldDef {
    FieldDef {
        name: "label".into(),
        ty: TypeRef::String,
        default: Some("default_label".into()),
        typed_default: Some(DefaultValue::StringLiteral("default".into())),
        ..Default::default()
    }
}

fn optional_data_interface_case() -> GoBatchCase {
    let output = render_field_assertion(
        FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            optional: true,
            ..Default::default()
        },
        "choice",
        &[data_choice_enum()],
        true,
        "is_true",
        None,
    );

    assert!(
        !output.contains("*result.Choice"),
        "sealed interfaces are not pointers:\n{output}"
    );
    rendered_case(
        "optional_data_interface_nullable_not_dereferenced",
        &output,
        SAMPLE_DATA_INTERFACE,
    )
}

fn required_unresolved_named_case() -> GoBatchCase {
    let output = render_field_assertion(
        FieldDef {
            name: "payload".into(),
            ty: TypeRef::Named("ForeignPayload".into()),
            ..Default::default()
        },
        "payload",
        &[],
        false,
        "contains",
        Some(serde_json::json!("sample")),
    );

    assert!(
        output.contains("*result.Payload"),
        "unresolved named fields are pointers:\n{output}"
    );
    rendered_case(
        "required_unresolved_named_raw_message_pointer",
        &output,
        SAMPLE_RAW_MESSAGE,
    )
}

fn required_default_string_count_case() -> GoBatchCase {
    let output = render_field_assertion(
        label_field(),
        "label",
        &[],
        false,
        "count_min",
        Some(serde_json::json!(1)),
    );

    assert!(output.contains("len(*result.Label)"), "{output}");
    rendered_case("required_default_string_count_pointer", &output, SAMPLE_LABEL_POINTER)
}

fn required_default_number_comparison_case() -> GoBatchCase {
    let output = render_field_assertion(
        FieldDef {
            name: "limit".into(),
            ty: TypeRef::Primitive(PrimitiveType::I64),
            default: Some("default_limit".into()),
            typed_default: Some(DefaultValue::IntLiteral(5)),
            ..Default::default()
        },
        "limit",
        &[],
        false,
        "greater_than",
        Some(serde_json::json!(1)),
    );

    assert!(output.contains("*result.Limit < 2"), "{output}");
    rendered_case(
        "required_default_number_comparison_pointer",
        &output,
        SAMPLE_LIMIT_POINTER,
    )
}

fn pointer_pseudo_field_compiles_case(suffix: &str, assertion_type: &str) -> GoBatchCase {
    let expected = match assertion_type {
        "greater_than" => 0,
        "less_than_or_equal" | "max_length" => 10,
        "count_equals" => 6,
        _ => 1,
    };
    let output = render_field_assertion(
        label_field(),
        &format!("label.{suffix}"),
        &[],
        false,
        assertion_type,
        Some(serde_json::json!(expected)),
    );
    assert!(!output.contains("len(*result.Label) != nil"), "{output}");
    assert!(!output.contains("len(len(*result.Label))"), "{output}");
    rendered_case(
        &format!("pointer_pseudo_{suffix}_{assertion_type}_compiles"),
        &output,
        SAMPLE_LABEL_POINTER,
    )
}

fn pointer_pseudo_field_nil_safe_case(suffix: &str, assertion_type: &str) -> GoBatchCase {
    let expected = match assertion_type {
        "less_than_or_equal" | "max_length" => 10,
        _ => 1,
    };
    let output = render_field_assertion(
        label_field(),
        &format!("label.{suffix}"),
        &[],
        false,
        assertion_type,
        Some(serde_json::json!(expected)),
    );
    rendered_case(
        &format!("pointer_pseudo_{suffix}_{assertion_type}_nil_safe"),
        &output,
        SAMPLE_LABEL_NIL,
    )
}

fn data_interface_string_family_case(assertion_type: &str, expected: &str) -> GoBatchCase {
    let output = render_field_assertion(
        FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            ..Default::default()
        },
        "choice",
        &[data_choice_enum()],
        false,
        assertion_type,
        Some(serde_json::json!(expected)),
    );
    assert!(output.contains("jsonString(t, result.Choice)"), "{output}");
    rendered_case(
        &format!("data_interface_string_{assertion_type}"),
        &output,
        SAMPLE_DATA_INTERFACE,
    )
}

/// Every case the batch must run, in a stable order. The names double as the batch's case
/// inventory, so each one names the exact fixture it replaced.
fn rendered_shape_cases() -> Vec<GoBatchCase> {
    let mut cases = vec![
        optional_data_interface_case(),
        required_unresolved_named_case(),
        required_default_string_count_case(),
        required_default_number_comparison_case(),
    ];
    for suffix in PSEUDO_FIELD_SUFFIXES {
        for assertion_type in PSEUDO_FIELD_ASSERTIONS {
            cases.push(pointer_pseudo_field_compiles_case(suffix, assertion_type));
            cases.push(pointer_pseudo_field_nil_safe_case(suffix, assertion_type));
        }
    }
    for (assertion_type, expected) in DATA_INTERFACE_STRING_FAMILIES {
        cases.push(data_interface_string_family_case(assertion_type, expected));
    }
    cases
}

fn broken_source_control_case() -> GoBatchCase {
    GoBatchCase {
        name: BROKEN_SOURCE_CONTROL.to_owned(),
        files: vec![
            ("sample.go".to_owned(), "package sample\n".to_owned()),
            (
                "shape_test.go".to_owned(),
                format!(
                    "package sample_test\nimport (\n\"testing\"\nsample \"{SHAPE_BATCH_MODULE}/{BROKEN_SOURCE_CONTROL}\"\n)\nfunc TestBrokenControl(t *testing.T) {{ _ = sample.MissingSymbol }}\n"
                ),
            ),
        ],
    }
}

#[test]
fn rendered_assertion_shapes_compile_and_run_in_one_go_test() {
    let mut cases = rendered_shape_cases();
    assert_eq!(
        cases.len(),
        RENDERED_SHAPE_CASE_COUNT,
        "the rendered-shape inventory changed; update RENDERED_SHAPE_CASE_COUNT deliberately"
    );
    let passing: Vec<String> = cases.iter().map(|case| case.name.clone()).collect();
    let emitted_tests: usize = cases
        .iter()
        .map(|case| {
            case.files
                .iter()
                .map(|(_, content)| emitted_test_functions(content))
                .sum::<usize>()
        })
        .sum();
    cases.push(broken_source_control_case());
    let inventory: Vec<String> = cases.iter().map(|case| case.name.clone()).collect();

    let layout = GoBatchLayout {
        root_files: vec![(
            PathBuf::from("go.mod"),
            format!("module {SHAPE_BATCH_MODULE}\n\ngo 1.24\n"),
        )],
        module_dir: PathBuf::new(),
        module_path: SHAPE_BATCH_MODULE.to_owned(),
        extra_args: Vec::new(),
    };
    let Some(report) = run_go_batch(&layout, &cases) else {
        return;
    };

    report.assert_inventory(&inventory);
    for name in &passing {
        report.assert_outcome(name, GoCaseOutcome::Passed);
        assert!(
            report.case(name).test_case_count >= 1,
            "case `{name}` selected no Go test to run:\n{}",
            report.case(name).output
        );
    }
    report.assert_outcome(BROKEN_SOURCE_CONTROL, GoCaseOutcome::Failed);
    report.assert_output_contains(BROKEN_SOURCE_CONTROL, "undefined: sample.MissingSymbol");
    assert!(
        emitted_tests >= RENDERED_SHAPE_CASE_COUNT,
        "every rendered case must contribute at least one Go test: {emitted_tests}"
    );
    assert_eq!(
        report.total_test_cases(),
        emitted_tests,
        "the batch executed a different number of Go tests than it generated"
    );
}

#[test]
fn optional_data_interface_field_is_nullable_but_not_dereferenced() {
    optional_data_interface_case();
}

#[test]
fn required_unresolved_named_field_uses_raw_message_pointer_shape() {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "payload".into(),
            ty: TypeRef::Named("ForeignPayload".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts_with_enums(&types, &[], "go"),
        Some("Envelope".into()),
    );
    assert_eq!(resolver.target_field_is_pointer("payload"), Some(true));

    required_unresolved_named_case();
}

#[test]
fn optional_vec_assertions_follow_go_slice_shape_over_global_optionality() {
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        fields_optional: HashSet::from(["items".into()]),
        fields_array: HashSet::from(["items".into()]),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "optional_vec_shape".into(),
        assertions: vec![Assertion {
            assertion_type: "min_length".into(),
            field: Some("items".into()),
            value: Some(serde_json::json!(1)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let field = FieldDef {
        name: "items".into(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        optional: true,
        ..Default::default()
    };
    let output = render_fixture(config, fixture, field, &[]);

    assert!(output.contains("len(result.Items)"), "expected slice length:\n{output}");
    assert!(
        !output.contains("len(*result.Items)"),
        "must not dereference slice:\n{output}"
    );
}

#[test]
fn optional_local_has_plain_value_shape() {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "title".into(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts_with_enums(&types, &[], "go"),
        Some("Envelope".into()),
    );
    let assertion = Assertion {
        assertion_type: "equals".into(),
        field: Some("title".into()),
        value: Some(serde_json::json!("sample")),
        ..Default::default()
    };
    let locals = std::collections::HashMap::from([("title".into(), "title".into())]);
    let shape = resolve_assertion_field_shape(&assertion, &resolver, &locals);

    assert!(!shape.is_optional);
    assert!(!shape.is_pointer);
    assert!(!shape.is_nullable);
}

#[test]
fn required_default_string_count_dereferences_authoritative_pointer() {
    required_default_string_count_case();
}

#[test]
fn required_default_number_comparison_dereferences_authoritative_pointer() {
    required_default_number_comparison_case();
}

#[test]
fn pointer_length_and_count_pseudo_fields_compile_as_scalars() {
    for suffix in PSEUDO_FIELD_SUFFIXES {
        for assertion_type in PSEUDO_FIELD_ASSERTIONS {
            pointer_pseudo_field_compiles_case(suffix, assertion_type);
            pointer_pseudo_field_nil_safe_case(suffix, assertion_type);
        }
    }
}

/// Negative control for the `count_min`/`count_equals` no-silent-skip fix in
/// `assertion_render_helpers.rs::render_count_assertion`. `label.length` is a `.length`
/// PSEUDO field -- a derived scalar measurement (a string's length) taken through `label`'s
/// optional pointer, not a named collection field in its own right. `label` being nil means
/// "not populated", the same "no presence claim" semantics `render_guarded_scalar_comparison`
/// already gives optional scalars like `QualityScore` -- so this must stay guard-only, unlike
/// a real collection field (`elements`, `chunks`, `detected_languages`), which now fails on
/// nil instead of silently skipping. Mirrors the `go_batch`
/// `pointer_pseudo_length_count_min_nil_safe` case, which runs this exact rendered shape
/// against a nil `*string` and requires `Test_FieldShape` to PASS. ~keep
#[test]
fn pointer_pseudo_length_count_min_stays_guard_only_on_nil() {
    let output = render_field_assertion(
        label_field(),
        "label.length",
        &[],
        false,
        "count_min",
        Some(serde_json::json!(1)),
    );
    assert!(
        output.contains("if result.Label != nil {"),
        "expected a guard on the optional pointer, got: {output}"
    );
    assert!(
        !output.contains("} else {"),
        "a pseudo-length measurement through an optional pointer must not gain a failing else, got: {output}"
    );
}

/// `count_equals` shares `render_count_assertion` with `count_min` -- confirm the `Equal`
/// method path gets the same pseudo-length exemption.
#[test]
fn pointer_pseudo_count_count_equals_stays_guard_only_on_nil() {
    let output = render_field_assertion(
        label_field(),
        "label.count",
        &[],
        false,
        "count_equals",
        Some(serde_json::json!(1)),
    );
    assert!(
        output.contains("if result.Label != nil {"),
        "expected a guard on the optional pointer, got: {output}"
    );
    assert!(
        !output.contains("} else {"),
        "a pseudo-count measurement through an optional pointer must not gain a failing else, got: {output}"
    );
}

#[test]
fn data_interface_string_assertion_families_compile_with_wire_json() {
    for (assertion_type, expected) in DATA_INTERFACE_STRING_FAMILIES {
        data_interface_string_family_case(assertion_type, expected);
    }
}

#[test]
fn go_result_shapes_follow_emitted_type_partitions() {
    let (types, enums, excluded) = partitioned_type_fixture();
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::go_ir_result_field_facts(&types, &enums, &excluded),
        Some("Envelope".into()),
    );

    for field in ["excluded", "opaque", "visitor", "enum_value"] {
        assert_eq!(resolver.target_field_is_pointer(field), Some(true), "{field}");
    }
}

/// Constructs the concrete case where `test_function.rs`'s pre-fix guess
/// (`is_optional && !is_array`) and `assertion_field_shape.rs`'s `unwrap_or(false)` diverge:
/// a path that crosses a data-enum (sealed-interface) variant boundary. `Choice`'s payload is
/// never recorded in `field_types` (`ir_result_fields.rs` only extends it across `structs` and
/// `pointer_variant_enums`), so the walk to `choice.value` genuinely cannot resolve --
/// `target_field_is_pointer` returns `None` -- while `is_optional`/`is_array` still answer from
/// the config-declared `fields_optional` set independently of that walk. Before harmonizing
/// `test_function.rs` onto `unwrap_or(false)`, this divergence made it guess "pointer" for a
/// leaf that is a plain, non-optional `string` field on `Choice`'s payload struct -- the
/// `SheetCount`-shaped compile error PR #330 fixed, reappearing via a different unwalkable
/// boundary. ~keep
#[test]
fn unresolved_data_enum_crossing_diverges_between_the_two_pointer_fallback_guesses() {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "choice".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("Choice".into()))),
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let enums = vec![data_choice_enum()];
    let optional: HashSet<String> = ["choice.value".into()].into_iter().collect();
    let resolver = FieldResolver::new(
        &Default::default(),
        &optional,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts_with_enums(&types, &enums, "go"),
        Some("Envelope".into()),
    );

    // The anchored IR cannot cross `Choice`'s variant boundary, so this path is genuinely
    // unresolved -- `None`, not a positive "not a pointer" answer.
    assert_eq!(resolver.target_field_is_pointer("choice.value"), None);

    // `assertion_field_shape.rs`'s (and, after this fix, `test_function.rs`'s) answer for this
    // exact unresolved path.
    assert!(!resolver.target_field_is_pointer("choice.value").unwrap_or(false));

    // `test_function.rs`'s pre-fix guess for the identical unresolved path: config-declared
    // optional and not an array, so it answered `true`.
    let old_guess = resolver.is_optional("choice.value") && !resolver.is_array("choice.value");
    assert!(
        old_guess,
        "the two fallbacks must genuinely diverge for this case, or the regression is moot"
    );
}

fn partitioned_type_fixture() -> (Vec<TypeDef>, Vec<EnumDef>, HashSet<String>) {
    let named_field = |name: &str, target: &str| FieldDef {
        name: name.into(),
        ty: TypeRef::Named(target.into()),
        ..Default::default()
    };
    let types = vec![
        TypeDef {
            name: "Envelope".into(),
            fields: vec![
                named_field("excluded", "Excluded"),
                named_field("opaque", "Opaque"),
                named_field("visitor", "VisitorContext"),
                named_field("enum_value", "HiddenChoice"),
            ],
            ..Default::default()
        },
        TypeDef {
            name: "Excluded".into(),
            ..Default::default()
        },
        TypeDef {
            name: "Opaque".into(),
            is_opaque: true,
            ..Default::default()
        },
        TypeDef {
            name: "VisitorContext".into(),
            ..Default::default()
        },
    ];
    let enums = vec![EnumDef {
        name: "HiddenChoice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let excluded = HashSet::from(["Excluded".into(), "VisitorContext".into(), "HiddenChoice".into()]);
    (types, enums, excluded)
}
/// A root result type reached through one `Vec<Struct>` hop before an `Option<Vec<T>>`/
/// `Option<Vec<String>>` leaf -- the exact shape `results[0].chunks` and
/// `results[0].detected_languages[0]` have in the real downstream crate. Every existing fixture in
/// this file puts the field directly on a flat "Envelope" root, which never exercises the
/// multi-hop `walk_to_owner_from` traversal the real compile failures went through.
fn nested_option_vec_fixture() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ExtractionResult".into(),
            fields: vec![FieldDef {
                name: "results".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("PageResult".into()))),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "PageResult".into(),
            fields: vec![
                FieldDef {
                    name: "chunks".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".into()))),
                    optional: true,
                    ..Default::default()
                },
                FieldDef {
                    name: "detected_languages".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        TypeDef {
            name: "Chunk".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

fn nested_option_vec_resolver() -> FieldResolver {
    let types = nested_option_vec_fixture();
    FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    // Mirrors the real `build_call_field_resolver_with_facts` wiring (both maps anchored at the
    // same root) -- the fix under test.
    .with_ir_collection_map(FieldResolver::ir_collection_fields(&types), Some("ExtractionResult".into()))
    .with_ir_result_fields(
        FieldResolver::go_ir_result_field_facts(&types, &[], &HashSet::new()),
        Some("ExtractionResult".into()),
    )
}

fn shape_for(resolver: &FieldResolver, field: &str) -> super::assertion_field_shape::AssertionFieldShape {
    let assertion = Assertion {
        assertion_type: "not_empty".into(),
        field: Some(field.into()),
        ..Default::default()
    };
    resolve_assertion_field_shape(&assertion, resolver, &std::collections::HashMap::new())
}

/// THE E2 REGRESSION: an `Option<Vec<T>>` result field reached through a `results[0].` hop must
/// never be treated as a pointer -- the Go binding backend always flattens it to a plain nilable
/// slice (`go_optional_type`'s `TypeRef::Vec(_) => go_type(ty)` arm). Before this fix, an
/// unresolved (or IR-collection-blind) `is_array_for_len` made the `unwrap_or(is_optional &&
/// !is_array_for_len)` guess fire `true`, and Go's e2e generator emitted `len(*result.Chunks)`
/// against a `[]Chunk` -- `cannot indirect ... variable of type []pkg.Chunk`.
#[test]
fn nested_option_vec_field_through_a_struct_array_is_not_pointer() {
    let resolver = nested_option_vec_resolver();
    let shape = shape_for(&resolver, "results[0].chunks");
    assert!(
        !shape.is_pointer,
        "an Option<Vec<T>> result field must never be dereferenced"
    );
    assert!(shape.is_slice, "the IR collection map must recognize this as a slice");
    assert!(shape.is_optional, "the field itself is still Option-wrapped");
    assert!(shape.is_nullable, "nullable via is_optional, not is_pointer");
}

/// THE SECOND E2 REGRESSION: `results[0].detected_languages[0]` names ONE element of an
/// `Option<Vec<String>>`, not the collection. Bracket-stripped field-name resolution answers
/// `is_optional`/`is_array`/`target_field_is_pointer` identically for `detected_languages` and
/// `detected_languages[0]`, so without the `leaf_is_indexed_element` guard the collection's own
/// optionality leaked onto the element and Go's e2e generator emitted `result.DetectedLanguages[0]
/// == nil` against a plain `string` -- `invalid operation: mismatched types string and untyped
/// nil`.
#[test]
fn indexed_element_of_an_optional_collection_is_not_treated_as_nullable() {
    let resolver = nested_option_vec_resolver();
    let element_shape = shape_for(&resolver, "results[0].detected_languages[0]");
    assert!(
        !element_shape.is_optional,
        "a single string element is never itself Option-wrapped"
    );
    assert!(!element_shape.is_pointer, "a string element is never a pointer");
    assert!(
        !element_shape.is_nullable,
        "must not compile a `== nil` check against a string"
    );
    assert!(!element_shape.is_slice, "one element of a slice is not itself a slice");

    // Control: the BARE collection (no trailing index) keeps its real Option<Vec<T>> shape --
    // proving the guard fires only on the indexed leaf, not on `detected_languages` in general.
    let collection_shape = shape_for(&resolver, "results[0].detected_languages");
    assert!(
        collection_shape.is_optional,
        "control: the un-indexed field is genuinely optional"
    );
    assert!(
        collection_shape.is_slice,
        "control: the un-indexed field is genuinely a slice"
    );
}

/// THE FALLBACK ITSELF, isolated from the `ir_collection_map` wiring fix: with no IR wired in at
/// all (only `fields_optional` config, as a consumer's `alef.toml` looks before any IR-derived
/// answer exists), `target_field_is_pointer` can never resolve -- it always returns `None`,
/// regardless of the field. The old `.unwrap_or(is_optional && !is_array_for_len)` guess then
/// took "optional AND not a configured array" as proof of pointer-ness, which is backwards: it is
/// proof of nothing, since neither half of that expression answers "is this a pointer" at all.
/// A consumer who marks a field optional without ALSO remembering to list it in `fields_array`
/// (the exact gap `ir_collection_map` closes for the fields it can reach) got a spurious `*`.
#[test]
fn unresolved_optional_field_is_never_guessed_as_a_pointer() {
    let mut optional = HashSet::new();
    optional.insert("items".to_string());
    let resolver = FieldResolver::new(
        &Default::default(),
        &optional,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    let shape = shape_for(&resolver, "items");
    assert!(
        resolver.target_field_is_pointer("items").is_none(),
        "control: no IR is wired in, so the authoritative answer must be unavailable"
    );
    assert!(shape.is_optional, "control: fields_optional still marks it optional");
    assert!(
        !shape.is_pointer,
        "an unresolved field must never be GUESSED as a pointer"
    );
}

/// A root result type with a `Map<String, String>` field -- the exact shape
/// `DocumentMetadata::open_graph` has downstream -- and a `Map<String, Option<String>>`
/// field standing in for a map whose VALUE type genuinely can be absent in Go (`map[string]
/// *string`), both reached the same bracket-index way `alef.toml`'s `fields_optional` declares a
/// map key lookup (`open_graph[title]`).
fn map_key_fixture() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "DocumentMetadata".into(),
        fields: vec![
            FieldDef {
                name: "open_graph".into(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                ..Default::default()
            },
            FieldDef {
                name: "pointer_labels".into(),
                ty: TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Optional(Box::new(TypeRef::String))),
                ),
                ..Default::default()
            },
        ],
        ..Default::default()
    }]
}

fn map_key_resolver() -> FieldResolver {
    let types = map_key_fixture();
    let mut optional = HashSet::new();
    optional.insert("open_graph".to_string());
    optional.insert("open_graph[title]".to_string());
    optional.insert("pointer_labels[title]".to_string());
    FieldResolver::new(
        &Default::default(),
        &optional,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::go_ir_result_field_facts(&types, &[], &HashSet::new()),
        Some("DocumentMetadata".into()),
    )
}

/// THE H2M REGRESSION (shipped alef 0.79.4 through 0.82.2): `open_graph[title]` names a
/// config-declared optional map-key lookup into `Map<String, String>` -- correct cross-language
/// modeling of "the key might be absent" (Python's `.get()`, TypeScript's `undefined`), but Go's
/// `m["key"]` on a `map[string]string` always yields a `string`, even for a missing key or a nil
/// map. Before the fix this leaf inherited the container's declared optionality unchanged and the
/// `equals` assertion family emitted `result.Metadata.Document.OpenGraph["title"] == nil` --
/// `invalid operation: mismatched types string and untyped nil`, a Go compile failure in the
/// generated e2e suite, not a warning.
#[test]
fn map_key_lookup_into_a_scalar_value_map_is_not_treated_as_nullable() {
    let resolver = map_key_resolver();
    let shape = shape_for(&resolver, "open_graph[title]");
    assert!(
        !shape.is_optional,
        "a string value can never be nil, regardless of config"
    );
    assert!(!shape.is_pointer, "a map[string]string element is never a Go pointer");
    assert!(!shape.is_nullable, "must not compile a `== nil` check against a string");

    // Control: the bare map field (no key lookup) keeps its real declared optionality --
    // proving the override fires only on the indexed leaf, not on `open_graph` in general.
    let container_shape = shape_for(&resolver, "open_graph");
    assert!(
        container_shape.is_optional,
        "control: the un-indexed field is still config-declared optional"
    );
}

/// THE GUARDRAIL: a map whose VALUE type genuinely can be `nil` in Go (here `map[string]
/// *string`) must keep its nil comparison. The fix must not blanket-strip the guard for every
/// bracket-indexed leaf -- only for leaves the IR can positively prove are backed by a plain Go
/// value type.
#[test]
fn map_key_lookup_into_a_pointer_value_map_keeps_its_nil_guard() {
    let resolver = map_key_resolver();
    let shape = shape_for(&resolver, "pointer_labels[title]");
    assert!(
        shape.is_nullable,
        "a map[string]*string element can genuinely be nil and must keep its guard"
    );
}
