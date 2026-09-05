use super::*;

/// Builds an options-dataclass type (`has_default`, not a return type) carrying one
/// `Option<Json>` field, one bare `Json` field, one `Map<String, Json>` field and one
/// `Map<String, String>` field.
fn make_json_field_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StructuredDataResult".to_string(),
            rust_path: "test_lib::StructuredDataResult".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("value", TypeRef::Optional(Box::new(TypeRef::Json)), true),
                make_field("schema", TypeRef::Json, false),
                make_field(
                    "additional",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
                    false,
                ),
                make_field(
                    "metadata",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// `options.py` comes from `generate_public_api`, not `generate_bindings` -- the latter emits
/// exactly one file, the Rust `lib.rs` (gen_bindings/mod.rs:724), and `generate_type_stubs`
/// emits only `<module>.pyi`. Matches how `enum_options_regressions` reaches the same file.
fn generate_options_py(api: &ApiSurface) -> String {
    let backend = Pyo3Backend;
    let files = backend
        .generate_public_api(api, &make_config())
        .expect("generate_public_api failed");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("options.py"))
        .unwrap_or_else(|| {
            panic!(
                "options.py generated; got: {:?}",
                files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
            )
        })
        .content
        .clone()
}

/// Regression test for xberg#362: a `serde_json::Value` field was annotated `dict[str, Any]` in
/// the generated Python type hints while the pyclass field is actually a `String`.
///
/// `Pyo3Mapper::json()` (`src/backends/pyo3/type_map.rs`) returns the Rust type `String`, so a
/// Json-typed field is emitted as `String` / `Option<String>` on the `#[pyclass]` and
/// `#[pyo3(get)]` hands Python a `str` holding serialized JSON. PyO3 0.29 has no
/// `IntoPyObject for serde_json::Value` (serde_json appears only under `[dev-dependencies]`),
/// so there is no dict to hand back. The type hint must say `str`.
#[test]
fn json_fields_are_annotated_str_not_dict_in_options_dataclass() {
    let content = generate_options_py(&make_json_field_api());

    assert!(
        content.contains("value: str | None"),
        "Option<Json> must be annotated `str | None` to match the `Option<String>` pyclass field:\n{content}"
    );
    assert!(
        content.contains("schema: str"),
        "a bare Json field must be annotated `str` to match the `String` pyclass field:\n{content}"
    );
    assert!(
        !content.contains("value: dict[str, Any]"),
        "Option<Json> must no longer claim to be a dict — the runtime hands back a JSON str:\n{content}"
    );
    assert!(
        !content.contains("dict[str, dict[str, Any]]"),
        "Map<String, Json> must not claim dict values — the Rust field is HashMap<String, String>:\n{content}"
    );
    assert!(
        content.contains("additional: dict[str, str]"),
        "Map<String, Json> must be annotated `dict[str, str]`:\n{content}"
    );
}

/// Negative control: the fix must be scoped to `TypeRef::Json` only. A genuine
/// `Map<String, String>` field is unaffected and still renders as a `dict`, proving the change
/// did not blanket-rewrite dict-valued annotations into `str`.
#[test]
fn non_json_map_fields_still_render_as_dict() {
    let content = generate_options_py(&make_json_field_api());

    assert!(
        content.contains("metadata: dict[str, str]"),
        "Map<String, String> must still be annotated `dict[str, str]`:\n{content}"
    );
}

/// The `from typing import ...` line must not import `Any` when nothing references it.
///
/// `needs_any` used to be `emits_from_native_converters || <any field contains Json>`. Now that
/// Json fields render as `str`, the second clause no longer corresponds to any `Any` in the
/// output, so leaving it in place would emit an unused `from typing import Any` and trip ruff
/// F401 in the generated stubs (the class of defect fixed in alef 5db2813de).
///
/// This uses the one shape that discriminates the two clauses: a `has_default` type that is a
/// return type. `options_dataclass_type_names` filters out return types, so
/// `emits_from_native_converters` is false and no `native: Any` converter is emitted; the type
/// itself is not emitted either. The old Json clause would still have fired and imported `Any`
/// with nothing referencing it.
#[test]
fn any_import_is_not_emitted_for_json_fields_alone() {
    let mut api = make_json_field_api();
    api.types[0].is_return_type = true;

    let content = generate_options_py(&api);

    assert!(
        !content.contains("native: Any"),
        "precondition: this fixture must not emit from_native converters, else the test cannot \
         discriminate the two `needs_any` clauses:\n{content}"
    );
    let import_line = content
        .lines()
        .find(|line| line.starts_with("from typing import"))
        .unwrap_or("");
    assert!(
        !import_line.contains("Any"),
        "`Any` must not be imported when only Json fields (now rendered as `str`) would have \
         referenced it — the import would be unused and ruff F401 would fail:\n{import_line}"
    );
}
