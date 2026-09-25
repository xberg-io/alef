//! The `_to_rust_<snake>` converter's native-constructor call and that constructor's own
//! signature must agree on the exact keyword set.
//!
//! `**({"field": value} if value is not None else {})` hides the keyword from the checker: pyrefly
//! then tries every remaining parameter against the unpacked value and reports one
//! `[bad-argument-type]` per pair, so N such unpacks in one call cost N*(N-1) errors in a dozen
//! lines. The omission is only meaningful when the public field can actually be absent, which is
//! exactly when `options.py` defaults it to `None` -- a `#[serde(default)]` enum field renders
//! `= "start"` there and can never be absent. ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{
    ApiSurface, DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef,
};

const CONFIG_TYPE: &str = "LayoutSpec";
const SERDE_DEFAULT: &str = "/* serde(default) */";
const ENUM_FIELDS: [(&str, &str, &str, &str); 3] = [
    ("alignment", "Alignment", "Start", "End"),
    ("density", "Density", "Loose", "Tight"),
    ("casing", "Casing", "Lower", "Upper"),
];

fn python_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#,
    )
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

fn unit_enum(name: &str, default_variant: &str, other_variant: &str) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        variants: vec![
            EnumVariant {
                name: default_variant.to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: other_variant.to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// `#[serde(default)]` on a non-`Option` enum field: `options.py` renders it as the enum's
/// `#[default]` variant string, so the public field is never `None`.
fn serde_default_enum_field(field_name: &str, enum_name: &str) -> FieldDef {
    FieldDef {
        name: field_name.to_string(),
        ty: TypeRef::Named(enum_name.to_string()),
        default: Some(SERDE_DEFAULT.to_string()),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    }
}

/// `#[serde(default = "some_fn")]`: alef cannot render the function's value as a Python literal,
/// so `options.py` defaults the field to `None` and the field genuinely can be absent.
fn function_default_field(field_name: &str, type_name: &str) -> FieldDef {
    FieldDef {
        name: field_name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        typed_default: Some(DefaultValue::FunctionCall("test_lib::default_theme".to_string())),
        ..Default::default()
    }
}

fn surface(config_fields: Vec<FieldDef>, enums: Vec<EnumDef>, extra_types: Vec<TypeDef>) -> ApiSurface {
    let mut types = vec![TypeDef {
        name: CONFIG_TYPE.to_string(),
        rust_path: format!("test_lib::{CONFIG_TYPE}"),
        has_serde: true,
        has_default: true,
        fields: config_fields,
        ..Default::default()
    }];
    types.extend(extra_types);
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types,
        enums,
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "test_lib::render".to_string(),
            params: vec![ParamDef {
                name: "spec".to_string(),
                ty: TypeRef::Named(CONFIG_TYPE.to_string()),
                ..Default::default()
            }],
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn render_facade_and_stub(api: &ApiSurface) -> (String, String) {
    let backend = crate::backends::pyo3::Pyo3Backend;
    let config = python_config();
    let stub = backend
        .generate_type_stubs(api, &config)
        .expect("stub generation succeeds")
        .into_iter()
        .find(|file| file.path.extension().is_some_and(|ext| ext == "pyi"))
        .expect("a .pyi stub is generated")
        .content;
    let facade = backend
        .generate_public_api(api, &config)
        .expect("public API generation succeeds")
        .into_iter()
        .find(|file| file.path.ends_with("api.py"))
        .expect("api.py is generated")
        .content;
    (facade, stub)
}

/// The argument list of `return _rust.<CONFIG_TYPE>(` in the rendered facade, one entry per
/// top-level comma.
fn constructor_call_arguments(facade: &str) -> Vec<String> {
    let marker = format!("return _rust.{CONFIG_TYPE}(");
    let start = facade
        .find(&marker)
        .map(|idx| idx + marker.len())
        .unwrap_or_else(|| panic!("`{marker}` is missing from:\n{facade}"));
    split_top_level(&balanced_slice(&facade[start..], facade))
}

/// The parameter names of `<CONFIG_TYPE>.__init__` in the rendered `.pyi`, `self` excluded.
fn stub_constructor_parameters(stub: &str) -> Vec<String> {
    let class_marker = format!("\nclass {CONFIG_TYPE}:");
    let class_start = stub
        .find(&class_marker)
        .unwrap_or_else(|| panic!("`class {CONFIG_TYPE}:` is missing from:\n{stub}"));
    let init_marker = "def __init__(";
    let init_start = stub[class_start..]
        .find(init_marker)
        .map(|idx| class_start + idx + init_marker.len())
        .unwrap_or_else(|| panic!("`{CONFIG_TYPE}.__init__` is missing from:\n{stub}"));
    split_top_level(&balanced_slice(&stub[init_start..], stub))
        .into_iter()
        .map(|entry| entry.split(':').next().unwrap_or_default().trim().to_string())
        .filter(|name| name.as_str() != "self")
        .collect()
}

/// Everything up to the paren that closes an already-opened call.
fn balanced_slice(rest: &str, whole: &str) -> String {
    let mut depth = 1usize;
    let mut inner = String::new();
    for ch in rest.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return inner;
                }
            }
            _ => {}
        }
        inner.push(ch);
    }
    panic!("unbalanced call parentheses in:\n{whole}");
}

fn split_top_level(inner: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    entries.push(current.trim().to_string());
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        entries.push(current.trim().to_string());
    }
    entries
}

fn enum_field_surface() -> ApiSurface {
    surface(
        ENUM_FIELDS
            .iter()
            .map(|(field, enum_name, _, _)| serde_default_enum_field(field, enum_name))
            .collect(),
        ENUM_FIELDS
            .iter()
            .map(|(_, enum_name, default_variant, other)| unit_enum(enum_name, default_variant, other))
            .collect(),
        Vec::new(),
    )
}

/// Every `#[serde(default)]` enum field is passed as a plain keyword argument, spelled exactly.
#[test]
fn should_pass_serde_default_enum_fields_as_plain_kwargs_when_options_never_defaults_them_to_none() {
    let (facade, _stub) = render_facade_and_stub(&enum_field_surface());
    let arguments = constructor_call_arguments(&facade);

    let expected: Vec<String> = ENUM_FIELDS
        .iter()
        .map(|(field, enum_name, _, _)| format!("{field}=_coerce_enum(_rust.{enum_name}, value.{field})"))
        .collect();
    assert_eq!(
        arguments, expected,
        "each serde(default) enum field must be passed by keyword, not hidden behind a \
         `**({{...}} if ... else {{}})` unpack:\n{facade}"
    );
}

/// The unpack form must not appear at all for this shape -- asserting only that the plain form is
/// present would pass while an unpack was emitted alongside it.
#[test]
fn should_not_emit_a_kwargs_unpack_when_no_field_can_be_absent() {
    let (facade, _stub) = render_facade_and_stub(&enum_field_surface());
    let arguments = constructor_call_arguments(&facade);

    let unpacks: Vec<&String> = arguments.iter().filter(|entry| entry.starts_with("**")).collect();
    assert_eq!(
        unpacks,
        Vec::<&String>::new(),
        "a field `options.py` defaults to a real value can never be absent, so the omission \
         unpack is dead code that costs one pyrefly [bad-argument-type] per other unpack in the \
         same call:\n{facade}"
    );
}

/// The keyword set the converter passes and the keyword set the native constructor declares must
/// be the same set -- an unpack removes a keyword from the former without removing the parameter.
#[test]
fn constructor_call_and_native_constructor_signature_agree_on_the_parameter_set() {
    let (facade, stub) = render_facade_and_stub(&enum_field_surface());

    let mut called: Vec<String> = constructor_call_arguments(&facade)
        .into_iter()
        .map(|entry| entry.split('=').next().unwrap_or_default().trim().to_string())
        .collect();
    called.sort();
    let mut declared = stub_constructor_parameters(&stub);
    declared.sort();

    assert_eq!(
        called, declared,
        "the `_to_rust_*` constructor call and the native `__init__` must name the same \
         parameters:\nfacade:\n{facade}\nstub:\n{stub}"
    );
}

/// The omission unpack is still emitted where it is load-bearing: a field whose Rust default is a
/// function call has no Python literal, so `options.py` defaults it to `None` and the field really
/// can be absent.
#[test]
fn should_keep_the_kwargs_unpack_when_options_defaults_the_field_to_none() {
    let theme = TypeDef {
        name: "Theme".to_string(),
        rust_path: "test_lib::Theme".to_string(),
        ..Default::default()
    };
    let api = surface(vec![function_default_field("theme", "Theme")], Vec::new(), vec![theme]);
    let (facade, _stub) = render_facade_and_stub(&api);
    let arguments = constructor_call_arguments(&facade);

    assert_eq!(
        arguments,
        vec![r#"**_optional_layout_spec_theme(value.theme)"#.to_string()],
        "a field `options.py` defaults to `None` must stay omittable -- passing `None` to a \
         non-`Option` pyo3 parameter fails extraction:\n{facade}"
    );
}

#[test]
fn should_not_emit_optional_kwarg_helper_when_nested_converter_owns_the_call_site() {
    let theme = TypeDef {
        name: "Theme".to_string(),
        rust_path: "test_lib::Theme".to_string(),
        has_default: true,
        ..Default::default()
    };
    let api = surface(vec![function_default_field("theme", "Theme")], Vec::new(), vec![theme]);
    let (facade, _stub) = render_facade_and_stub(&api);
    let helper_name = "_optional_layout_spec_theme";

    assert_eq!(
        constructor_call_arguments(&facade),
        vec!["theme=_to_rust_theme(value.theme)".to_string()],
        "the nested converter owns the constructor argument:\n{facade}"
    );
    assert!(
        !facade.contains(helper_name),
        "a helper with no call site is dead generated code:\n{facade}"
    );
}

#[test]
fn mixed_optional_primitive_defaults_use_heterogeneous_kwarg_helper() {
    let fields = [
        ("minimum_score", TypeRef::Primitive(crate::core::ir::PrimitiveType::F64)),
        (
            "minimum_words",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize),
        ),
        ("require_text", TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)),
    ]
    .into_iter()
    .map(|(name, ty)| FieldDef {
        name: name.to_string(),
        ty,
        typed_default: Some(DefaultValue::FunctionCall(format!("test_lib::default_{name}"))),
        ..Default::default()
    })
    .collect();
    let facade = render_facade_and_stub(&surface(fields, Vec::new(), Vec::new())).0;
    let arguments = constructor_call_arguments(&facade);

    assert_eq!(
        arguments,
        vec![
            r#"**_optional_layout_spec_minimum_score(value.minimum_score)"#.to_string(),
            r#"**_optional_layout_spec_minimum_words(value.minimum_words)"#.to_string(),
            r#"**_optional_layout_spec_require_text(value.require_text)"#.to_string(),
        ],
        "mixed optional values must flow through a heterogeneous kwargs type: {facade}"
    );
    // A `TypedDict`, not a plain `dict[str, V]`: pyrefly resolves an unpacked `dict[str, V]`
    // against every remaining parameter, so three helpers with `float`/`int`/`bool` values on one
    // constructor call cost one `[bad-argument-type]` per incompatible pair (3 errors here, under
    // the `default` preset as well as `strict`). The one-key `TypedDict` is what pins each value to
    // its own keyword. ~keep
    for (field_name, pascal_name, python_type) in [
        ("minimum_score", "MinimumScore", "float"),
        ("minimum_words", "MinimumWords", "int"),
        ("require_text", "RequireText", "bool"),
    ] {
        let expected =
            format!("class _LayoutSpec{pascal_name}Kwargs(TypedDict, total=False):\n    {field_name}: {python_type}");
        assert!(
            facade.contains(&expected),
            "each optional field must retain its constructor keyword and value type; missing:\n\
             {expected}\nin:\n{facade}"
        );
    }
    assert!(
        !facade.contains("**({"),
        "homogeneous dict unpacks trigger Pyrefly errors: {facade}"
    );
}

/// The `[tool.pyrefly]` section of the `pyproject.toml` the scaffold really writes into a
/// generated `packages/python`, with `project-includes` prepended so a bare temp directory is
/// checkable.
///
/// Deriving the checker config from the scaffold instead of hand-writing one is the whole point:
/// the first version of this harness wrote `[tool.pyrefly]\nproject-includes = [...]` and nothing
/// else, so it ran under pyrefly's `default` preset while every real consumer runs under the
/// `preset = "strict"` the scaffold emits. `[open-unpacking]` — the error that rejected the
/// optional-kwarg helper's open `TypedDict` at every `**helper(...)` call site — fires only under
/// `strict`, so the harness passed on output that did not type-check for anyone. ~keep
fn scaffolded_pyrefly_project_config(project_includes: &str) -> String {
    let files = crate::scaffold::languages::scaffold_python(&ApiSurface::default(), &python_config())
        .expect("python scaffold succeeds");
    let pyproject = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("pyproject.toml"))
        .expect("the scaffold emits packages/python/pyproject.toml")
        .content
        .clone();
    let marker = "[tool.pyrefly]\n";
    let start = pyproject
        .find(marker)
        .expect("the scaffolded pyproject.toml carries a [tool.pyrefly] section");
    let section = &pyproject[start..];
    assert!(
        section.contains("preset = \"strict\""),
        "this harness is only meaningful while the scaffold ships the strict preset; a consumer \
         running something laxer makes every assertion below vacuous:\n{section}"
    );
    format!(
        "{marker}project-includes = [\"{project_includes}\"]\n{}",
        &section[marker.len()..]
    )
}

#[test]
#[allow(clippy::print_stderr)] // narrow: reports a toolchain skip on a machine without pyrefly ~keep
fn generated_mixed_optional_kwargs_pass_pyrefly_and_typed_sabotage_fails() {
    let pyrefly = match which::which("pyrefly") {
        Ok(path) => path,
        Err(error) if std::env::var_os("ALEF_REQUIRE_PYREFLY").is_some() => {
            panic!("ALEF_REQUIRE_PYREFLY is set but pyrefly is unavailable: {error}")
        }
        Err(error) => {
            eprintln!(
                "SKIP generated_mixed_optional_kwargs_pass_pyrefly_and_typed_sabotage_fails: \
                 pyrefly is not on PATH ({error}); the strict-preset type check did NOT run. Set \
                 ALEF_REQUIRE_PYREFLY=1 to turn this skip into a failure."
            );
            return;
        }
    };
    let field_specs = [
        (
            "max_ocr_output_fragmented_word_ratio",
            crate::core::ir::PrimitiveType::F64,
            "float",
        ),
        ("min_ocr_mean_confidence", crate::core::ir::PrimitiveType::F64, "float"),
        (
            "min_words_for_ocr_output_check",
            crate::core::ir::PrimitiveType::Usize,
            "int",
        ),
        (
            "max_ocr_output_dict_invalid_word_ratio",
            crate::core::ir::PrimitiveType::F64,
            "float",
        ),
        ("min_undecodable_ratio", crate::core::ir::PrimitiveType::F64, "float"),
        (
            "enable_provenance_ocr_routing",
            crate::core::ir::PrimitiveType::Bool,
            "bool",
        ),
        (
            "min_provenance_fallback_ratio",
            crate::core::ir::PrimitiveType::F64,
            "float",
        ),
    ];
    let fields = field_specs
        .iter()
        .map(|(name, primitive, _)| FieldDef {
            name: name.to_string(),
            ty: TypeRef::Primitive(primitive.clone()),
            typed_default: Some(DefaultValue::FunctionCall(format!("test_lib::default_{name}"))),
            ..Default::default()
        })
        .collect();
    let facade = render_facade_and_stub(&surface(fields, Vec::new(), Vec::new())).0;
    let directory = tempfile::tempdir().expect("temporary generated Python package");
    let package = directory.path().join("test_lib");
    std::fs::create_dir_all(&package).expect("create Python package");
    std::fs::write(package.join("api.py"), &facade).expect("write emitted facade");
    std::fs::write(package.join("__init__.py"), "").expect("write package init");
    std::fs::write(
        directory.path().join("pyproject.toml"),
        scaffolded_pyrefly_project_config("test_lib/**/*.py"),
    )
    .expect("write Pyrefly project config");
    let stub_params = field_specs
        .iter()
        .map(|(name, _, python_type)| format!("{name}: {python_type} | None = None"))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        package.join("_test_lib.pyi"),
        format!("class LayoutSpec:\n    def __init__(self, *, {stub_params}) -> None: ...\ndef render(*, spec: LayoutSpec) -> str: ...\n"),
    )
    .expect("write native binding stub");
    let option_fields = field_specs
        .iter()
        .map(|(name, _, python_type)| format!("    {name}: {python_type} | None = None"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        package.join("options.py"),
        format!("from dataclasses import dataclass\n@dataclass\nclass LayoutSpec:\n{option_fields}\n"),
    )
    .expect("write public options module");
    let checked = std::process::Command::new(&pyrefly)
        .current_dir(directory.path())
        .arg("check")
        .arg(".")
        .output()
        .expect("pyrefly generated facade check must run");
    assert!(
        checked.status.success(),
        "pyrefly rejected the real emitted facade:\n{}\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );

    // Three negative controls. The clean run above is only worth something if this harness can
    // still see each way this surface can go wrong, so each control corrupts exactly one thing and
    // pyrefly must reject it with a named code. (a) and (b) are the two guarantees the one-key
    // `TypedDict` buys over a plain `dict[str, V]` -- the value type AND the key name. (c) guards
    // the checker configuration itself rather than the codegen. ~keep
    let bool_helper = "_optional_layout_spec_enable_provenance_ocr_routing";
    let clean_config = scaffolded_pyrefly_project_config("test_lib/**/*.py");
    let check_rejected = |label: &str, api_py: &str, config: &str, expected_marker: &str| {
        std::fs::write(package.join("api.py"), api_py).unwrap_or_else(|_| panic!("write `{label}` facade"));
        std::fs::write(directory.path().join("pyproject.toml"), config)
            .unwrap_or_else(|_| panic!("write `{label}` pyrefly config"));
        let output = std::process::Command::new(&pyrefly)
            .current_dir(directory.path())
            .arg("check")
            .arg(".")
            .output()
            .unwrap_or_else(|_| panic!("pyrefly `{label}` control must run"));
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success() && report.contains(expected_marker),
            "negative control `{label}` must be rejected with `{expected_marker}`; a run without it \
             means this harness would not catch that defect:\n{report}"
        );
    };

    // (a) A value whose type disagrees with the keyword it is unpacked into.
    let wrong_value = facade.replace(
        &format!("**{bool_helper}(value.enable_provenance_ocr_routing)"),
        &format!("**{bool_helper}(\"wrong\")"),
    );
    assert_ne!(wrong_value, facade, "control (a) must change the facade");
    check_rejected("wrong value type", &wrong_value, &clean_config, "[bad-argument-type]");

    // (b) A typo in the kwargs key. This is the check a plain `dict[str, V]` cannot make at all:
    // with a `dict` the misspelled keyword is invisible and pyrefly reports nothing.
    let typoed_key = facade
        .replace(
            "    enable_provenance_ocr_routing: bool\n",
            "    enable_provenance_ocr_routng: bool\n",
        )
        .replace(
            "{\"enable_provenance_ocr_routing\": value}",
            "{\"enable_provenance_ocr_routng\": value}",
        );
    assert_ne!(typoed_key, facade, "control (b) must change the facade");
    check_rejected("typoed kwargs key", &typoed_key, &clean_config, "[unexpected-keyword]");

    // (c) The clean facade, checked with `open-unpacking = false` removed from the scaffolded
    // config. `[open-unpacking]` must come straight back: that proves the extracted config really
    // is the strict one (the code fires under no other preset) AND that the suppression the
    // scaffold emits is load-bearing rather than decorative. If someone deletes it from
    // `scaffold::languages::python`, this control stops failing and the test reports it here
    // instead of the whole harness quietly going green on unusable output. ~keep
    let without_suppression = clean_config.replace("open-unpacking = false\n", "");
    assert_ne!(
        without_suppression, clean_config,
        "the scaffolded config must carry `open-unpacking = false` for the emitted `**helper(...)` \
         unpacks to type-check at all:\n{clean_config}"
    );
    check_rejected("suppression removed", &facade, &without_suppression, "[open-unpacking]");
}

/// `options.py` as the backend writes it, for the dataclass half of the same contract.
fn render_options_py(api: &ApiSurface) -> String {
    crate::backends::pyo3::Pyo3Backend
        .generate_public_api(api, &python_config())
        .expect("public API generation succeeds")
        .into_iter()
        .find(|file| file.path.ends_with("options.py"))
        .expect("options.py is generated")
        .content
}

/// A hand-written `impl Default` that fills one field from a zero-argument function call and the
/// rest from literals -- the shape that folds to `FunctionCall` for that one field and to
/// value-carrying variants for its siblings (`extract::extractor::defaults`).
///
/// `allowed_marks` is deliberately a non-`Option`, non-`Named` `Vec<String>`: the omission unpack
/// used to be gated on `TypeRef::Named`, so exactly this shape had its `None` passed through.
fn mixed_literal_and_function_default_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            name: "strict".to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            typed_default: Some(DefaultValue::BoolLiteral(true)),
            ..Default::default()
        },
        FieldDef {
            name: "wrap_columns".to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize),
            typed_default: Some(DefaultValue::IntLiteral(80)),
            ..Default::default()
        },
        FieldDef {
            name: "tags".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            typed_default: Some(DefaultValue::Empty),
            ..Default::default()
        },
        FieldDef {
            name: "allowed_marks".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            typed_default: Some(DefaultValue::FunctionCall(
                "test_lib::default_allowed_marks".to_string(),
            )),
            ..Default::default()
        },
    ]
}

/// The same four fields with the function-call default replaced by a readable list literal, so
/// every field carries a value alef actually read. Nothing here may be omitted.
fn only_literal_default_fields() -> Vec<FieldDef> {
    let mut fields = mixed_literal_and_function_default_fields();
    fields[3].typed_default = Some(DefaultValue::ListLiteral(vec![
        DefaultValue::StringLiteral("bold".to_string()),
        DefaultValue::StringLiteral("italic".to_string()),
    ]));
    fields
}

/// REPRODUCTION: a non-`Option` `Vec<String>` whose Rust default is a function call. `options.py`
/// has no Python literal for it and defaults it to `None`, so the converter must withhold the
/// keyword and let the native constructor apply the Rust default. Passing the `None` through
/// reaches a non-`Option` pyo3 parameter and fails extraction with
/// `TypeError: 'None' is not an instance of 'Sequence'`, at a call site far from the dataclass.
#[test]
fn a_function_derived_default_on_a_non_named_field_is_omitted_not_passed_as_none() {
    let api = surface(mixed_literal_and_function_default_fields(), Vec::new(), Vec::new());
    let facade = render_facade_and_stub(&api).0;
    let arguments = constructor_call_arguments(&facade);

    assert_eq!(
        arguments,
        vec![
            "strict=value.strict".to_string(),
            "wrap_columns=value.wrap_columns".to_string(),
            "tags=value.tags".to_string(),
            r#"**_optional_layout_spec_allowed_marks(value.allowed_marks)"#.to_string(),
        ],
        "only the function-derived field may be omitted, and it must be omitted rather than \
         passed as `None`:\n{facade}"
    );
}

/// CONTROL: the same four fields with every default readable. No field may be omitted here, so a
/// fix that omitted every field -- or that widened the unpack to fields carrying a real value --
/// fails this even while the reproduction above passes.
#[test]
fn literal_derived_defaults_are_all_passed_as_plain_keyword_arguments() {
    let api = surface(only_literal_default_fields(), Vec::new(), Vec::new());
    let facade = render_facade_and_stub(&api).0;
    let arguments = constructor_call_arguments(&facade);

    assert_eq!(
        arguments,
        vec![
            "strict=value.strict".to_string(),
            "wrap_columns=value.wrap_columns".to_string(),
            "tags=value.tags".to_string(),
            "allowed_marks=value.allowed_marks".to_string(),
        ],
        "a field whose default alef read is never absent, so it must be passed by keyword:\n{facade}"
    );
}

/// The dataclass half of the contract: a field's declared type and its default are one fact, and
/// `OptionsFieldDefaults::admits_none` is what both are derived from. The three literal-derived
/// fields mirror their real Rust defaults; the function-derived one declares `| None` *because*
/// it defaults to `None`. Declaring `list[str] = None` (the type without the widening) or
/// `list[str] = field(default_factory=list)` (a `[]` the Rust source never specified) would each
/// break exactly one half of that pair.
#[test]
fn the_options_dataclass_pairs_each_declared_type_with_the_default_it_actually_carries() {
    let options = render_options_py(&surface(
        mixed_literal_and_function_default_fields(),
        Vec::new(),
        Vec::new(),
    ));

    for expected in [
        "strict: bool = True",
        "wrap_columns: int = 80",
        "tags: list[str] = field(default_factory=list)",
        "allowed_marks: list[str] | None = None",
    ] {
        assert!(
            options.contains(expected),
            "options.py must declare `{expected}`:\n{options}"
        );
    }
}

/// CONTROL for the dataclass assertions: with every default readable, no field is nullable and
/// the list default is the real literal. A change that made every dataclass field `| None = None`
/// would pass the test above and fail here.
#[test]
fn a_fully_literal_options_dataclass_declares_no_nullable_field() {
    let options = render_options_py(&surface(only_literal_default_fields(), Vec::new(), Vec::new()));

    assert!(
        options.contains(r#"allowed_marks: list[str] = field(default_factory=lambda: ["bold", "italic"])"#),
        "a readable list default must be rendered as itself, not widened to `| None`:\n{options}"
    );
    assert!(
        !options.contains("| None"),
        "no field of a fully literal-defaulted type may be declared nullable:\n{options}"
    );
}

/// A non-`Option` Rust field carrying NO serde-default marker at all, on a type that derives
/// `Default` -- `ResponseTool { tool_type: String, #[serde(flatten)] config: serde_json::Value }`.
///
/// `replace_constructor_with_serde_rename` gives such a field `config=Self::default().config` with
/// a non-`Option<String>` parameter, and the `.pyi` stub says so (`config: str = ...`, per
/// `rust_default_constructor_fields_do_not_claim_none_is_accepted`): omission is allowed, `None`
/// is not. `options.py` has no Python literal for `serde_json::Value::default()`, so it declares
/// `config: str | None = None` -- which means the facade MUST withhold the keyword. Passing that
/// `None` straight into the non-`Option` parameter is both a pyrefly `[bad-argument-type]` and a
/// runtime extraction failure for anyone who constructs the dataclass without the field.
#[test]
fn should_withhold_a_none_default_from_a_non_option_parameter_that_has_no_serde_default_marker() {
    let fields = vec![
        FieldDef {
            name: "tool_type".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        },
        FieldDef {
            name: "config".to_string(),
            ty: TypeRef::Json,
            ..Default::default()
        },
    ];
    let api = surface(fields, Vec::new(), Vec::new());
    let (facade, stub) = render_facade_and_stub(&api);
    let options = render_options_py(&api);

    assert!(
        options.contains("config: str | None = None"),
        "options.py cannot render `serde_json::Value::default()`, so the field stays nullable:\n{options}"
    );
    assert!(
        stub.contains("config: str = ..."),
        "the native parameter is not an `Option`, so the stub must not claim `None` is accepted:\n{stub}"
    );
    let json_coerced = "(json.dumps(value.config) if isinstance(value.config, (dict, list)) else value.config)";
    assert_eq!(
        constructor_call_arguments(&facade),
        vec![
            "tool_type=value.tool_type".to_string(),
            format!("**_optional_layout_spec_config({json_coerced})"),
        ],
        "a field `options.py` defaults to `None` must be omitted, not passed, whether or not it \
         carries a `#[serde(default)]` marker:\n{facade}"
    );
}
