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
    let report = run_go_batch(&layout, &cases);

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
/// `results[0].detected_languages[0]` have in the real xberg crate. Every existing fixture in
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
/// against a `[]Chunk` -- `cannot indirect ... variable of type []xberg.Chunk`.
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
