use super::*;
use crate::codegen::generators::AsyncPattern;
use crate::core::ir::{CoreWrapper, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("crate::{name}"),
        original_rust_path: String::new(),
        variants,
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_pyo3_data_enum_emits_string_methods() {
    let generated = gen_pyo3_data_enum(
        &enum_def("StructureKind", vec![variant("Other", vec![field("value")])]),
        "core",
    );

    assert!(
        generated.contains("fn __str__(&self) -> PyResult<String>"),
        "{generated}"
    );
    assert!(generated.contains("serde_json::to_value(&self.inner)"), "{generated}");
    assert!(
        generated.contains("fn __repr__(&self) -> PyResult<String>"),
        "{generated}"
    );
}

#[test]
fn gen_pyo3_data_enum_emits_default_when_core_derives_default() {
    // A data enum whose core type derives `Default` is surfaced as a variant marked
    // `#[default]` (`is_default = true`). The wrapper must keep its delegating `Default`.
    let mut default_variant = variant("Pending", vec![]);
    default_variant.is_default = true;
    let generated = gen_pyo3_data_enum(
        &enum_def(
            "EnrichStatus",
            vec![default_variant, variant("Done", vec![field("value")])],
        ),
        "core",
    );

    assert!(
        generated.contains("impl Default for EnrichStatus"),
        "expected delegating Default impl when a variant is #[default]: {generated}"
    );
    assert!(generated.contains("Self { inner: Default::default() }"), "{generated}");
}

#[test]
fn gen_pyo3_data_enum_emits_default_when_core_has_manual_default() {
    let mut enum_def = enum_def(
        "ClassificationMode",
        vec![variant("Known", vec![]), variant("Custom", vec![field("value")])],
    );
    enum_def.has_default = true;

    let generated = gen_pyo3_data_enum(&enum_def, "core");

    assert!(
        generated.contains("impl Default for ClassificationMode"),
        "expected delegating Default impl when the core enum has a manual Default impl: {generated}"
    );
    assert!(generated.contains("Self { inner: Default::default() }"), "{generated}");
}

#[test]
fn gen_pyo3_data_enum_omits_default_when_core_lacks_default() {
    // No variant is marked `#[default]`, so the core enum does NOT implement `Default`.
    // Emitting a delegating `Default` would fail with E0277 on the core type, so the
    // wrapper `Default` impl must be omitted entirely.
    let generated = gen_pyo3_data_enum(
        &enum_def(
            "ChunkingReason",
            vec![variant("TooLong", vec![field("limit")]), variant("Forced", vec![])],
        ),
        "core",
    );

    assert!(
        !generated.contains("impl Default for ChunkingReason"),
        "expected no Default impl when no variant is #[default]: {generated}"
    );
    assert!(
        !generated.contains("inner: Default::default()"),
        "expected no inner: Default::default() when core lacks Default: {generated}"
    );
}

#[test]
fn gen_pyo3_data_enum_wraps_string_for_internally_tagged_enum() {
    // For an internally-tagged enum (`#[serde(tag = "...")]`), serde cannot deserialize a
    // bare JSON string into the enum. The `__new__` string branch must wrap it as
    // `{"<tag>": s}` so serde can resolve the variant.
    let mut def = enum_def(
        "ImageOutputFormat",
        vec![variant("Png", vec![]), variant("Jpeg", vec![field("quality")])],
    );
    def.serde_tag = Some("type".to_string());

    let generated = gen_pyo3_data_enum(&def, "core");

    assert!(
        generated.contains(r#"serde_json::to_string(&serde_json::json!({ "type": s }))"#),
        "expected tagged string wrap for internally-tagged enum: {generated}"
    );
    assert!(
        !generated.contains("serde_json::to_string(&s)"),
        "internally-tagged enum must not emit the bare-string path: {generated}"
    );
}

#[test]
fn gen_pyo3_data_enum_keeps_bare_string_for_externally_tagged_enum() {
    // An externally-tagged enum (no `#[serde(tag)]`) accepts a bare string for unit variants,
    // so the string branch must keep the existing `to_string(&s)` behavior.
    let generated = gen_pyo3_data_enum(
        &enum_def("StructureKind", vec![variant("Other", vec![field("value")])]),
        "core",
    );

    assert!(
        generated.contains("serde_json::to_string(&s)"),
        "expected bare-string path for externally-tagged enum: {generated}"
    );
    assert!(
        !generated.contains("serde_json::json!({"),
        "externally-tagged enum must not wrap the string in a tag object: {generated}"
    );
}

fn typed_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef { ty, ..field(name) }
}

fn static_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        is_static: true,
        ..Default::default()
    }
}

#[test]
fn variant_constructors_emit_factory_per_struct_variant() {
    use crate::codegen::type_mapper::IdentityMapper;
    // `Shape` with two struct variants → one `#[staticmethod]` constructor each.
    let mut def = enum_def(
        "Shape",
        vec![
            variant(
                "Circle",
                vec![typed_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
            ),
            variant(
                "Rect",
                vec![
                    typed_field("width", TypeRef::Primitive(PrimitiveType::F64)),
                    typed_field("height", TypeRef::Primitive(PrimitiveType::F64)),
                ],
            ),
        ],
    );
    def.serde_tag = Some("type".to_string());

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    // Constructors always collide with the variant accessor of the same name, so they use
    // the `_factory_<name>` Rust ident plus `#[pyo3(name = "<name>")]`.
    assert!(generated.contains(r#"#[pyo3(name = "circle")]"#), "{generated}");
    assert!(
        generated.contains("pub fn _factory_circle(radius: f64) -> Self"),
        "{generated}"
    );
    assert!(
        generated.contains("Self { inner: crate::Shape::Circle { radius } }"),
        "{generated}"
    );
    assert!(generated.contains(r#"#[pyo3(name = "rect")]"#), "{generated}");
    assert!(
        generated.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
        "{generated}"
    );
    assert!(
        generated.contains("Self { inner: crate::Shape::Rect { width, height } }"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_convert_named_dto_fields() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A field whose type is a binding DTO (Named) converts inline via `.into()` in the struct
    // literal — no typed `let <field>_core: <path>` binding, so the core type path is never
    // named (non-re-exported core types resolve through inference).
    let def = enum_def(
        "Wrapper",
        vec![variant(
            "Llm",
            vec![typed_field("llm", TypeRef::Named("LlmConfig".to_string()))],
        )],
    );

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
        generated.contains("pub fn _factory_llm(llm: LlmConfig) -> Self"),
        "{generated}"
    );
    assert!(
        generated.contains("Self { inner: crate::Wrapper::Llm { llm: llm.into() } }"),
        "{generated}"
    );
    // No typed let-binding naming the core path.
    assert!(!generated.contains("llm_core"), "{generated}");
}

#[test]
fn variant_constructors_box_named_field_when_core_is_boxed() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A variant field whose core type is `Box<T>` (Named T) must be boxed in the factory:
    // `result.into()` alone fails because no `From<Binding> for Box<Core>` exists. Mirrors the
    // From/Into path in `conversions::binding_to_core::render`.
    let boxed = FieldDef {
        is_boxed: true,
        ..typed_field("result", TypeRef::Named("CrawlPageResult".to_string()))
    };
    let def = enum_def("CrawlEvent", vec![variant("Page", vec![boxed])]);

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
        generated.contains("Self { inner: crate::CrawlEvent::Page { result: Box::new(result.into()) } }"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_coerce_dataclass_backed_dto_field() {
    use crate::codegen::generators::gen_pyo3_data_enum_with_coercion;
    use crate::codegen::type_mapper::IdentityMapper;
    use ahash::AHashSet;
    // A payload field whose Named type is a dataclass-backed config DTO accepts the public wrapper
    // (a @dataclass) or a dict via `&Bound<PyAny>` and is coerced through the runtime helper, rather
    // than demanding the compiled `#[pyclass]`. The factory therefore takes `py` and is fallible.
    // (Regression: enum-variant payloads must coerce like struct fields do.)
    let def = enum_def(
        "Wrapper",
        vec![variant(
            "Llm",
            vec![typed_field("llm", TypeRef::Named("LlmConfig".to_string()))],
        )],
    );
    let coercible: AHashSet<&str> = ["LlmConfig"].into_iter().collect();

    let generated = gen_pyo3_data_enum_with_coercion(&def, "core", Some(&IdentityMapper), &coercible);

    assert!(
        generated
            .contains("pub fn _factory_llm(py: Python<'_>, llm: &Bound<'_, pyo3::types::PyAny>) -> PyResult<Self>"),
        "{generated}"
    );
    assert!(
        generated.contains(
            "Ok(Self { inner: crate::Wrapper::Llm { llm: __alef_coerce_dto(py, llm, __ALEF_WIRE_LLM_CONFIG)? } })"
        ),
        "{generated}"
    );
    // The compiled-type-only path is gone.
    assert!(!generated.contains("llm: llm.into()"), "{generated}");
    // The module-level runtime helper is emitted exactly when needed.
    assert!(
        crate::codegen::generators::data_enum_needs_dto_coercion(&def, &coercible),
        "expected coercion to be flagged"
    );
}

#[test]
fn variant_constructors_coerce_only_dto_leaving_primitive_typed() {
    use crate::codegen::generators::gen_pyo3_data_enum_with_coercion;
    use crate::codegen::type_mapper::IdentityMapper;
    use ahash::AHashSet;
    // Coercion is gated strictly: only the dataclass-backed DTO field becomes `&Bound<PyAny>`; the
    // sibling primitive keeps its concrete type and shorthand init.
    let def = enum_def(
        "Job",
        vec![variant(
            "Run",
            vec![
                typed_field("retries", TypeRef::Primitive(PrimitiveType::U32)),
                typed_field("config", TypeRef::Named("RunConfig".to_string())),
            ],
        )],
    );
    let coercible: AHashSet<&str> = ["RunConfig"].into_iter().collect();

    let generated = gen_pyo3_data_enum_with_coercion(&def, "core", Some(&IdentityMapper), &coercible);

    assert!(
        generated.contains(
            "pub fn _factory_run(py: Python<'_>, retries: u32, config: &Bound<'_, pyo3::types::PyAny>) -> PyResult<Self>"
        ),
        "{generated}"
    );
    assert!(
        generated.contains(
            "Ok(Self { inner: crate::Job::Run { retries, config: __alef_coerce_dto(py, config, __ALEF_WIRE_RUN_CONFIG)? } })"
        ),
        "{generated}"
    );
}

#[test]
fn variant_constructors_box_optional_named_field_when_core_is_boxed() {
    use crate::codegen::type_mapper::IdentityMapper;
    // `Option<Box<T>>` (Named T): convert through the Option, then box each element.
    let boxed_opt = FieldDef {
        is_boxed: true,
        optional: true,
        ..typed_field("result", TypeRef::Named("CrawlPageResult".to_string()))
    };
    let def = enum_def("CrawlEvent", vec![variant("Page", vec![boxed_opt])]);

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
        generated.contains("result: result.map(Into::into).map(Box::new)"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_skip_coercion_for_non_dataclass_named_field() {
    use crate::codegen::generators::{data_enum_needs_dto_coercion, gen_pyo3_data_enum_with_coercion};
    use crate::codegen::type_mapper::IdentityMapper;
    use ahash::AHashSet;
    // A Named field that is NOT a dataclass-backed config DTO (e.g. a native-return type) stays on
    // the `.into()` path and the factory remains infallible — no `py` param, no helper.
    let def = enum_def(
        "Wrapper",
        vec![variant(
            "Llm",
            vec![typed_field("llm", TypeRef::Named("LlmConfig".to_string()))],
        )],
    );
    let empty: AHashSet<&str> = AHashSet::new();

    let generated = gen_pyo3_data_enum_with_coercion(&def, "core", Some(&IdentityMapper), &empty);

    assert!(
        generated.contains("pub fn _factory_llm(llm: LlmConfig) -> Self"),
        "{generated}"
    );
    assert!(generated.contains("llm: llm.into()"), "{generated}");
    assert!(!generated.contains("__alef_coerce_dto"), "{generated}");
    assert!(!data_enum_needs_dto_coercion(&def, &empty), "should not flag coercion");
}

#[test]
fn variant_constructors_coerce_vec_and_map_of_dto_payloads() {
    use crate::codegen::generators::gen_pyo3_data_enum_with_coercion;
    use crate::codegen::type_mapper::IdentityMapper;
    use ahash::AHashSet;
    // A `Vec<DTO>` payload coerces each element (`__alef_coerce_dto_seq`) and a `Map<_, DTO>`
    // payload coerces each value (`__alef_coerce_dto_map`); both accept lists/dicts of public
    // wrappers, not compiled instances.
    let def = enum_def(
        "Pipeline",
        vec![variant(
            "Run",
            vec![
                typed_field(
                    "stages",
                    TypeRef::Vec(Box::new(TypeRef::Named("StageConfig".to_string()))),
                ),
                typed_field(
                    "overrides",
                    TypeRef::Map(
                        Box::new(TypeRef::String),
                        Box::new(TypeRef::Named("StageConfig".to_string())),
                    ),
                ),
            ],
        )],
    );
    let coercible: AHashSet<&str> = ["StageConfig"].into_iter().collect();

    let generated = gen_pyo3_data_enum_with_coercion(&def, "core", Some(&IdentityMapper), &coercible);

    assert!(
        generated.contains("stages: __alef_coerce_dto_seq(py, stages, __ALEF_WIRE_STAGE_CONFIG)?"),
        "{generated}"
    );
    assert!(
        generated.contains("overrides: __alef_coerce_dto_map(py, overrides, __ALEF_WIRE_STAGE_CONFIG)?"),
        "{generated}"
    );
    // Both params arrive as PyAny (a list / dict of public wrappers).
    assert!(
        generated.contains("stages: &Bound<'_, pyo3::types::PyAny>"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_pair_interleaved_field_exprs_by_position() {
    use crate::codegen::type_mapper::IdentityMapper;
    // Interleave a primitive, a Named-DTO (`.into()`), and a Vec<Named> DTO
    // (`.into_iter().map(Into::into).collect()`) so each field's init lands in its own
    // struct-literal slot, converted inline.
    let def = enum_def(
        "Job",
        vec![variant(
            "Run",
            vec![
                typed_field("retries", TypeRef::Primitive(PrimitiveType::U32)),
                typed_field("config", TypeRef::Named("RunConfig".to_string())),
                typed_field("steps", TypeRef::Vec(Box::new(TypeRef::Named("Step".to_string())))),
            ],
        )],
    );

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
            generated.contains(
                "Self { inner: crate::Job::Run { retries, config: config.into(), steps: steps.into_iter().map(Into::into).collect() } }"
            ),
            "{generated}"
        );
}

#[test]
fn variant_constructors_skip_unit_tuple_and_excluded() {
    use crate::codegen::type_mapper::IdentityMapper;
    let mut tuple_variant = variant("Pair", vec![typed_field("_0", TypeRef::String)]);
    tuple_variant.is_tuple = true;
    let mut excluded = variant("Hidden", vec![typed_field("value", TypeRef::String)]);
    excluded.binding_excluded = true;

    let def = enum_def(
        "Mixed",
        vec![
            variant("Empty", vec![]),
            tuple_variant,
            excluded,
            variant("Real", vec![typed_field("value", TypeRef::String)]),
        ],
    );

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    // Unit, tuple, and binding_excluded variants get no constructor.
    assert!(!generated.contains("_factory_empty"), "{generated}");
    assert!(!generated.contains("_factory_pair"), "{generated}");
    assert!(!generated.contains("_factory_hidden"), "{generated}");
    // The struct variant still gets one.
    assert!(
        generated.contains("pub fn _factory_real(value: String) -> Self"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_skip_variant_with_sanitized_field() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A sanitized field (e.g. core `[(u32, u32); 4]` downgraded to String) has no faithful
    // binding value, so the core variant cannot be built — skip the whole variant.
    let mut sanitized = typed_field("points", TypeRef::String);
    sanitized.sanitized = true;
    let def = enum_def(
        "OcrBoundingGeometry",
        vec![
            variant("Quadrilateral", vec![sanitized]),
            variant(
                "Rectangle",
                vec![typed_field("left", TypeRef::Primitive(PrimitiveType::U32))],
            ),
        ],
    );

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(!generated.contains("_factory_quadrilateral"), "{generated}");
    assert!(generated.contains("_factory_rectangle"), "{generated}");
}

#[test]
fn variant_constructors_skip_variant_with_binding_excluded_field() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A `binding_excluded` field (e.g. `#[alef(skip)]`) is hidden from the binding surface, so
    // no value exists to fill it — skip the variant rather than emit a broken literal.
    let mut excluded = typed_field("entries", TypeRef::String);
    excluded.binding_excluded = true;
    let def = enum_def(
        "NodeContent",
        vec![
            variant("MetadataBlock", vec![excluded]),
            variant("Title", vec![typed_field("text", TypeRef::String)]),
        ],
    );

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(!generated.contains("_factory_metadata_block"), "{generated}");
    assert!(generated.contains("_factory_title"), "{generated}");
}

#[test]
fn variant_constructors_pass_through_genuine_optional_core_field() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A field whose CORE type is `Option<String>` (optional=true) keeps the binding Option
    // unchanged in the struct literal — no unwrap.
    let mut opt = typed_field("value", TypeRef::String);
    opt.optional = true;
    let def = enum_def("AnnotationKind", vec![variant("Custom", vec![opt])]);

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
        generated.contains("Self { inner: crate::AnnotationKind::Custom { value } }"),
        "{generated}"
    );
    assert!(!generated.contains("unwrap_or_default"), "{generated}");
}

#[test]
fn variant_constructors_unwrap_promoted_optional_field() {
    use crate::codegen::type_mapper::IdentityMapper;
    // `extra` is a non-optional core field (`Vec<String>`, defaulted) that FOLLOWS an optional
    // field, so pyo3 promotes it to `Option<Vec<String>>` in the signature. The core field is
    // still `Vec<String>`, so the struct literal must `unwrap_or_default()` the promoted param.
    let mut model_file = typed_field("model_file", TypeRef::String);
    model_file.optional = true;
    let extra = typed_field("extra", TypeRef::Vec(Box::new(TypeRef::String)));
    let def = enum_def("RerankerModelType", vec![variant("Custom", vec![model_file, extra])]);

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
        generated.contains("pub fn _factory_custom(model_file: Option<String>, extra: Option<Vec<String>>) -> Self"),
        "{generated}"
    );
    assert!(
            generated.contains(
                "Self { inner: crate::RerankerModelType::Custom { model_file, extra: extra.unwrap_or_default().into_iter().collect() } }"
            ),
            "{generated}"
        );
}

#[test]
fn variant_constructors_convert_optional_path_field() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A `Path` field maps to `String` in the binding but `PathBuf` in core; an optional one
    // must convert through the Option (`.map(Into::into)`), not pass the `Option<String>`
    // through unchanged (which would mismatch `Option<PathBuf>`).
    let mut cache_dir = typed_field("cache_dir", TypeRef::Path);
    cache_dir.optional = true;
    let def = enum_def(
        "ChunkSizing",
        vec![variant(
            "Tokenizer",
            vec![typed_field("model", TypeRef::String), cache_dir],
        )],
    );

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    assert!(
        generated
            .contains("Self { inner: crate::ChunkSizing::Tokenizer { model, cache_dir: cache_dir.map(Into::into) } }"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_use_inline_into_for_non_reexported_core_type() {
    use crate::codegen::type_mapper::IdentityMapper;
    // The core type lives at a nested module path (`pkg::enrich::EnrichStatus`), and the field
    // type is also non-re-exported. The inline `.into()` must not name any core type path, so
    // the generated body resolves the target via inference from the variant literal.
    let mut def = enum_def(
        "EnrichStatus",
        vec![variant(
            "Completed",
            vec![typed_field("result", TypeRef::Named("EnrichResult".to_string()))],
        )],
    );
    def.rust_path = "pkg::enrich::EnrichStatus".to_string();

    let generated = gen_pyo3_data_enum_with_mapper(&def, "pkg", Some(&IdentityMapper));

    assert!(
        generated.contains("Self { inner: pkg::enrich::EnrichStatus::Completed { result: result.into() } }"),
        "{generated}"
    );
    // The brittle `let result_core: pkg::EnrichResult` path annotation must not appear.
    assert!(!generated.contains("EnrichResult ="), "{generated}");
    assert!(!generated.contains("result_core"), "{generated}");
}

#[test]
fn variant_constructors_yield_to_hand_written_method() {
    use crate::codegen::type_mapper::IdentityMapper;
    // A hand-written `impl` method named `circle` wins; no generated constructor for Circle.
    let mut def = enum_def(
        "Shape",
        vec![
            variant(
                "Circle",
                vec![typed_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
            ),
            variant(
                "Square",
                vec![typed_field("side", TypeRef::Primitive(PrimitiveType::F64))],
            ),
        ],
    );
    def.methods = vec![static_method("circle")];

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

    // No generated constructor body for Circle (consumer method wins).
    assert!(
        !generated.contains("Self { inner: core::Shape::Circle"),
        "consumer method must win for Circle: {generated}"
    );
    // Square is untouched by the consumer method, so it gets a constructor.
    assert!(
        generated.contains("pub fn _factory_square(side: f64) -> Self"),
        "{generated}"
    );
}

#[test]
fn variant_constructors_absent_without_mapper() {
    // Without a mapper, no variant constructors are generated (back-compat).
    let def = enum_def(
        "Shape",
        vec![variant(
            "Circle",
            vec![typed_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
        )],
    );
    let generated = gen_pyo3_data_enum(&def, "core");
    assert!(!generated.contains("_factory_circle"), "{generated}");
}

#[test]
fn gen_pyo3_unit_enum_emits_string_methods() {
    let cfg = RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &[],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "",
        enum_attrs: &["pyclass(eq, eq_int, from_py_object)"],
        enum_derives: &["Clone", "PartialEq"],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "core",
        async_pattern: AsyncPattern::None,
        has_serde: true,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
        source_crate_remaps: &[],
        emit_delegating_default_for_types: None,
    };
    let generated = gen_enum(&enum_def("StructureKind", vec![variant("Function", Vec::new())]), &cfg);

    assert!(
        generated.contains("fn __str__(&self) -> PyResult<String>"),
        "{generated}"
    );
    assert!(generated.contains("serde_json::to_value(self)"), "{generated}");
}
