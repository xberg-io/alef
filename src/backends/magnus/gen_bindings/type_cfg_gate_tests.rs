//! End-to-end regression coverage for the Magnus (Ruby) struct-declaration half of the
//! `candle_ocr`/Windows defect: a HOST-owned `TypeDef::cfg` (a struct gated behind a Cargo
//! feature the core crate itself declares, e.g. `#[cfg(feature = "candle-ocr")]` on a
//! candle-backend options struct) was never re-emitted onto the generated Ruby wrapper at all.
//! Every function and method already carries its own `cfg` forward via `prepend_cfg`
//! (`func.cfg`/`method.cfg` in `gen_bindings::mod`); the two loops over `api.types` that emit a
//! struct's own declaration and its `From` conversions never consulted `typ.cfg` the same way,
//! so a consumer whose own feature set disabled the gate still got an unconditional reference to
//! a type the core crate never compiled in -- 41 ungated `<core>::candle_ocr::*` references on
//! Ruby/Windows, the sole failure in a downstream crate's Publish Release dry run.
//!
//! `MagnusBackend::generate_bindings` is exercised end to end (not `classes::gen_struct` /
//! `classes::gen_from_binding_to_core_filtered` directly), since the defect was in the call
//! sites in `gen_bindings::mod`, not in those generators themselves.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef,
    TypeRef,
};

fn magnus_config() -> ResolvedCrateConfig {
    let toml_src = "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\n\
                     sources = [\"src/lib.rs\"]\n[crates.ruby]\ngem_name = \"test_lib\"\n";
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A HOST-owned (`rust_path` shares the crate's own `core_import`, "test_lib") struct gated
/// behind a Cargo feature, with a function taking it as a parameter and another returning it --
/// exercising both the binding->core and core->binding conversion loops, not just the
/// declaration. ~keep
fn gated_struct_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GatedOptions".to_string(),
            rust_path: "test_lib::GatedOptions".to_string(),
            cfg: Some(r#"feature = "candle-ocr""#.to_string()),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            is_clone: true,
            ..Default::default()
        }],
        functions: vec![
            FunctionDef {
                name: "make_options".to_string(),
                rust_path: "test_lib::make_options".to_string(),
                return_type: TypeRef::Named("GatedOptions".to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "use_options".to_string(),
                rust_path: "test_lib::use_options".to_string(),
                params: vec![ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("GatedOptions".to_string()),
                    ..Default::default()
                }],
                return_type: TypeRef::Unit,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

#[test]
fn generate_bindings_gates_struct_declaration_behind_its_own_cfg_end_to_end() {
    let api = gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\n#[derive(Clone"),
        "a HOST-owned type's own `#[cfg(...)]` must be re-emitted directly above its generated \
         Ruby struct declaration, or a consumer whose feature set disables the gate still \
         references a type the core crate never compiled in, got:\n{lib_rs}"
    );
}

#[test]
fn generate_bindings_gates_binding_to_core_conversion_behind_the_type_cfg_end_to_end() {
    let api = gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\nimpl From<GatedOptions> for test_lib::GatedOptions"),
        "the binding->core `From` impl for a cfg-gated type must carry the same `#[cfg(...)]`, \
         got:\n{lib_rs}"
    );
}

#[test]
fn generate_bindings_gates_core_to_binding_conversion_behind_the_type_cfg_end_to_end() {
    let api = gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\nimpl From<test_lib::GatedOptions> for GatedOptions"),
        "the core->binding `From` impl for a cfg-gated type must carry the same `#[cfg(...)]`, \
         got:\n{lib_rs}"
    );
}

/// Positive control: an ungated type must never pick up a stray `#[cfg(...)]` -- proves the fix
/// reads `typ.cfg` per-type rather than gating every struct unconditionally.
#[test]
fn generate_bindings_never_gates_a_struct_with_no_cfg() {
    let mut api = gated_struct_api();
    api.types[0].cfg = None;
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        !lib_rs.contains("#[cfg("),
        "an ungated struct must not be wrapped in a `#[cfg(...)]` attribute, got:\n{lib_rs}"
    );
}

// -- Reference-site coverage: `ruby_init` registrations, per-field converters, and enum match
// arms all name a type/member that `typ.cfg`/`field.cfg` already say may not exist -- the
// declaration-only fix above (`type_cfg_gate_tests`'s original three tests) left every one of
// these ungated. See `gen_bindings::functions::module_init::gate_statement`,
// `FieldDef::cfg_within`, and `classes::gen_struct`'s per-field `cfg` context. ~keep

/// A HOST-owned struct gated behind a Cargo feature, with two fields (one plain, one `content`
/// to exercise the `to_s` registration) and one instance method -- exercising every
/// `ruby_init` registration site the type's own methods/fields feed: the kwargs constructor,
/// each field accessor, `to_s`, and the instance method. ~keep
fn gated_struct_with_members_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TokenReductionConfig".to_string(),
            rust_path: "test_lib::TokenReductionConfig".to_string(),
            cfg: Some(r#"feature = "quality""#.to_string()),
            fields: vec![
                FieldDef {
                    name: "threshold".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                },
                FieldDef {
                    name: "content".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            methods: vec![MethodDef {
                name: "enable_parallel".to_string(),
                receiver: Some(ReceiverKind::Ref),
                return_type: TypeRef::Unit,
                ..Default::default()
            }],
            is_clone: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Method registrations (category 1): the kwargs constructor, both field accessors, `to_s`, and
/// the instance method must each carry `TokenReductionConfig`'s own `#[cfg(...)]` in `ruby_init`
/// -- `method!`/`function!` resolve `TokenReductionConfig::{member}` as a path, so an ungated
/// registration for a type the gate compiles out is a hard `E0433`/`E0425`, not a missing Ruby
/// method.
#[test]
fn generate_bindings_gates_every_ruby_init_registration_behind_the_type_cfg_end_to_end() {
    let api = gated_struct_with_members_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let gate = "#[cfg(feature = \"quality\")]";

    let expectations = [
        (
            "constructor",
            "class.define_singleton_method(\"new\", function!(TokenReductionConfig::new, -1))",
        ),
        (
            "threshold accessor",
            "class.define_method(\"threshold\", method!(TokenReductionConfig::threshold, 0))",
        ),
        (
            "content accessor",
            "class.define_method(\"content\", method!(TokenReductionConfig::content, 0))",
        ),
        (
            "to_s",
            "class.define_method(\"to_s\", method!(TokenReductionConfig::to_s, 0))",
        ),
        (
            "instance method",
            "class.define_method(\"enable_parallel\", method!(TokenReductionConfig::enable_parallel, 0))",
        ),
    ];
    for (label, registration) in expectations {
        assert!(
            lib_rs.contains(&format!("{gate}\n    {registration}")),
            "the {label} registration must carry `{gate}` in `ruby_init`, got:\n{lib_rs}"
        );
    }
}

/// Control: with no type-level `cfg`, none of the same five registrations carry a `#[cfg(...)]`
/// -- proves the fix reads `typ.cfg` per-type rather than gating every registration
/// unconditionally.
#[test]
fn generate_bindings_never_gates_ruby_init_registrations_with_no_type_cfg() {
    let mut api = gated_struct_with_members_api();
    api.types[0].cfg = None;
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        !lib_rs.contains("#[cfg("),
        "an ungated type's registrations must carry no `#[cfg(...)]` at all, got:\n{lib_rs}"
    );
}

/// An UNGATED struct with two fields: one plain, one (`sparse_embedding`) whose OWN type
/// (`SparseEmbedding`) is independently cfg-gated behind a different feature than the
/// containing struct -- the containing struct's `impl` block is not itself wrapped in any
/// `#[cfg(...)]`, so nothing but the field's own gate protects this reference. ~keep
fn field_gated_struct_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Container".to_string(),
                rust_path: "test_lib::Container".to_string(),
                fields: vec![
                    FieldDef {
                        name: "plain".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        ..Default::default()
                    },
                    FieldDef {
                        name: "sparse_embedding".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Named("SparseEmbedding".to_string()))),
                        optional: true,
                        cfg: Some(r#"feature = "embeddings""#.to_string()),
                        ..Default::default()
                    },
                ],
                is_clone: true,
                ..Default::default()
            },
            TypeDef {
                name: "SparseEmbedding".to_string(),
                rust_path: "test_lib::SparseEmbedding".to_string(),
                cfg: Some(r#"feature = "embeddings""#.to_string()),
                fields: vec![FieldDef {
                    name: "vector".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32))),
                    ..Default::default()
                }],
                is_clone: true,
                ..Default::default()
            },
        ],
        // `convertible_types` alone is not sufficient for a binding->core `From` impl to be
        // emitted: `gen_bindings::mod`'s emission loop also requires the type appear in
        // `input_type_names` (a function/method parameter or return type), the same gate that
        // decides whether a consumer can ever construct one to pass in. Without this, `Container`
        // is eligible but unused, and the binding->core impl is silently never generated. ~keep
        functions: vec![FunctionDef {
            name: "use_container".to_string(),
            rust_path: "test_lib::use_container".to_string(),
            params: vec![ParamDef {
                name: "container".to_string(),
                ty: TypeRef::Named("Container".to_string()),
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Converters (category 2) plus the field declaration and accessor that must agree with them:
/// `Container` itself carries no `#[cfg(...)]`, so only `sparse_embedding`'s own field-level
/// gate protects every one of its references -- the struct's field declaration, the kwargs
/// constructor's `SparseEmbedding::try_convert(v)` line, the accessor `fn`, and the accessor's
/// `ruby_init` registration.
#[test]
fn generate_bindings_gates_field_references_behind_the_field_cfg_end_to_end() {
    let api = field_gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let gate = "#[cfg(feature = \"embeddings\")]";

    assert!(
        lib_rs.contains(&format!("{gate}\n    sparse_embedding: Option<SparseEmbedding>,")),
        "the struct's own field declaration must carry the field's `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("{gate}\n    sparse_embedding: match kwargs.get"))
            && lib_rs.contains("Some(v) => Some(SparseEmbedding::try_convert(v)"),
        "the kwargs constructor's converter for `sparse_embedding` must carry the field's \
         `#[cfg(...)]` and still call `SparseEmbedding::try_convert`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!(
            "{gate}\n    fn sparse_embedding(&self) -> Option<SparseEmbedding>"
        )),
        "the accessor `fn` must carry the field's `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!(
            "{gate}\n    class.define_method(\"sparse_embedding\", method!(Container::sparse_embedding, 0))"
        )),
        "the accessor's `ruby_init` registration must carry the field's `#[cfg(...)]`, got:\n{lib_rs}"
    );
}

/// Control: the `plain` field (no `cfg`) must never pick up a stray `#[cfg(...)]` anywhere it is
/// referenced -- proves the fix reads `field.cfg` per-field rather than gating every field once
/// any sibling field is gated.
#[test]
fn generate_bindings_never_gates_an_ungated_sibling_field() {
    let api = field_gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("    plain: u32,"),
        "the ungated `plain` field must still be declared, got:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("#[cfg(feature = \"embeddings\")]\n    plain: u32,"),
        "the ungated `plain` field's declaration must not pick up the sibling field's \
         `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("    class.define_method(\"plain\", method!(Container::plain, 0))?;"),
        "the ungated `plain` field's accessor must still be registered, got:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains(
            "#[cfg(feature = \"embeddings\")]\n    class.define_method(\"plain\", method!(Container::plain, 0))"
        ),
        "the ungated `plain` field's accessor registration must not carry `#[cfg(...)]`, \
         got:\n{lib_rs}"
    );
}

/// Category-2 converters on the OTHER two sites that also name `sparse_embedding`: the
/// `From<test_lib::Container>`/`From<Container>` struct-literal conversions in both directions.
/// `Container` itself carries no `#[cfg(...)]`, so a struct literal that unconditionally lists
/// `sparse_embedding` breaks the moment the field's own declaration (already gated) drops it --
/// `E0560`/`E0609` -- unless the SAME field gate is repeated on this literal entry too.
#[test]
fn generate_bindings_gates_field_references_in_both_from_impls_end_to_end() {
    let api = field_gated_struct_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let gate = "#[cfg(feature = \"embeddings\")]";

    assert!(
        lib_rs.contains("impl From<test_lib::Container> for Container"),
        "the core->binding conversion must be generated, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("{gate}\n            sparse_embedding:"))
            || lib_rs.contains(&format!("{gate}\n        sparse_embedding:")),
        "the core->binding struct literal's `sparse_embedding` entry must carry the field's \
         `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("impl From<Container> for test_lib::Container"),
        "the binding->core conversion must be generated, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("{gate}\n            sparse_embedding:"))
            || lib_rs.contains(&format!("{gate}\n        sparse_embedding:")),
        "the binding->core struct literal's `sparse_embedding` entry must carry the field's \
         `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains(&format!("{gate}\n            plain:"))
            && !lib_rs.contains(&format!("{gate}\n        plain:")),
        "the ungated `plain` field must not pick up the sibling field's `#[cfg(...)]` in either \
         conversion, got:\n{lib_rs}"
    );
}

/// A HOST-owned data enum with one ungated variant (`Pdf`) and one variant (`Docx`) gated
/// behind a Cargo feature -- match arms (category 3): the generated `From<core::FormatMetadata>`
/// conversion must gate `Docx`'s arm and stay exhaustive without a dead catch-all, since a
/// HOST-owned cfg-gated variant and its conversion arm compile in or out together (see
/// `codegen::conversions::enums::enum_conversion_needs_catch_all`'s doc comment). This is a
/// regression/confirmation test: tracing `classes::gen_enum` +
/// `codegen::conversions::gen_enum_from_core_to_binding_cfg` shows this case was already handled
/// correctly by existing machinery before this task's changes; it is added here because the task
/// requires coverage for this category regardless. ~keep
fn gated_variant_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "FormatMetadata".to_string(),
            rust_path: "test_lib::FormatMetadata".to_string(),
            has_serde: true,
            variants: vec![
                EnumVariant {
                    name: "Pdf".to_string(),
                    is_default: true,
                    fields: vec![FieldDef {
                        name: "pages".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                EnumVariant {
                    name: "Docx".to_string(),
                    cfg: Some(r#"feature = "docx""#.to_string()),
                    fields: vec![FieldDef {
                        name: "sections".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn generate_bindings_gates_the_core_to_binding_match_arm_for_a_gated_variant_and_stays_exhaustive() {
    let api = gated_variant_enum_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("impl From<test_lib::FormatMetadata> for FormatMetadata"),
        "the core->binding conversion must be generated, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("#[cfg(feature = \"docx\")]"),
        "the `Docx` variant's match arm must carry its own `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("_ => Default::default(),"),
        "a HOST-owned cfg-gated variant's arm compiles in or out together with the variant \
         itself, so the match needs no catch-all to stay exhaustive -- a stray one would be \
         dead code under `-D warnings` whenever the feature is enabled, got:\n{lib_rs}"
    );
}

/// A struct (`Envelope`) whose `payload` field carries NO `#[cfg(...)]` of its own -- mirroring
/// a core field like `Chunk.sparse_embedding: Option<crate::SparseEmbedding>`, whose core type
/// always compiles (a same-named stub backs it when the real feature is off) so the field
/// itself is correctly ungated -- but whose named type (`Payload`) is independently cfg-gated as
/// its OWN Magnus binding. Nothing but the referenced type's gate protects this reference. ~keep
fn field_references_gated_type_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Envelope".to_string(),
                rust_path: "test_lib::Envelope".to_string(),
                fields: vec![
                    FieldDef {
                        name: "label".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                    FieldDef {
                        name: "payload".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Named("Payload".to_string()))),
                        optional: true,
                        ..Default::default()
                    },
                ],
                is_clone: true,
                ..Default::default()
            },
            TypeDef {
                name: "Payload".to_string(),
                rust_path: "test_lib::Payload".to_string(),
                cfg: Some(r#"feature = "payload""#.to_string()),
                fields: vec![FieldDef {
                    name: "bytes".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
                    ..Default::default()
                }],
                is_clone: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// Category 2 (converters) plus the struct-field declaration, accessor, and registration that
/// must agree with it -- but here the field itself starts with NO `cfg`, so the ONLY source for
/// the gate is `Payload`'s own `typ.cfg`, folded onto the field before any emission site runs.
#[test]
fn generate_bindings_gates_an_ungated_fields_reference_to_a_gated_type_end_to_end() {
    let api = field_references_gated_type_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let gate = "#[cfg(feature = \"payload\")]";

    assert!(
        lib_rs.contains(&format!("{gate}\n    payload: Option<Payload>,")),
        "the struct's own field declaration must carry the REFERENCED type's `#[cfg(...)]` even \
         though the field itself has none, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("{gate}\n    payload: match kwargs.get"))
            && lib_rs.contains("Some(v) => Some(Payload::try_convert(v)"),
        "the kwargs constructor's converter for `payload` must carry `Payload`'s `#[cfg(...)]` \
         and still call `Payload::try_convert`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("{gate}\n    fn payload(&self) -> Option<Payload>")),
        "the accessor `fn` must carry `Payload`'s `#[cfg(...)]`, got:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!(
            "{gate}\n    class.define_method(\"payload\", method!(Envelope::payload, 0))"
        )),
        "the accessor's `ruby_init` registration must carry `Payload`'s `#[cfg(...)]`, \
         got:\n{lib_rs}"
    );
}

/// Control: the `label` field (`String`, no named type to look up) must stay completely
/// ungated -- proves the referenced-type lookup does not gate every field in a struct that
/// merely CONTAINS a gated-type reference somewhere else.
#[test]
fn generate_bindings_never_gates_a_sibling_field_with_no_referenced_type() {
    let api = field_references_gated_type_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("    label: String,"),
        "the ungated `label` field must still be declared, got:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("#[cfg(feature = \"payload\")]\n    label: String,"),
        "the `label` field must not pick up `Payload`'s `#[cfg(...)]` merely because a sibling \
         field references `Payload`, got:\n{lib_rs}"
    );
}

/// Two DISTINCT types sharing the short name `Ambiguous`, gated behind two DIFFERENT features --
/// a field naming `Ambiguous` cannot tell which definition it means, so the lookup must skip it
/// entirely rather than guessing (and possibly emitting a feature this crate never declares).
fn field_references_ambiguously_named_type_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Holder".to_string(),
                rust_path: "test_lib::Holder".to_string(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("Ambiguous".to_string()))),
                    optional: true,
                    ..Default::default()
                }],
                is_clone: true,
                ..Default::default()
            },
            TypeDef {
                name: "Ambiguous".to_string(),
                rust_path: "test_lib::a::Ambiguous".to_string(),
                cfg: Some(r#"feature = "a""#.to_string()),
                is_clone: true,
                ..Default::default()
            },
            TypeDef {
                name: "Ambiguous".to_string(),
                rust_path: "test_lib::b::Ambiguous".to_string(),
                cfg: Some(r#"feature = "b""#.to_string()),
                is_clone: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn generate_bindings_skips_an_ambiguous_short_name_instead_of_guessing_its_gate() {
    let api = field_references_ambiguously_named_type_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        !lib_rs.contains("#[cfg(feature = \"a\")]\n    value: Option<Ambiguous>,")
            && !lib_rs.contains("#[cfg(feature = \"b\")]\n    value: Option<Ambiguous>,"),
        "an ambiguous short name (two types share it under different gates) must not have \
         either candidate gate guessed onto the field, got:\n{lib_rs}"
    );
}

/// The enum half of the same defect. A HOST-owned enum gated behind a Cargo feature, used both
/// as a parameter and as a return type so both conversion directions are emitted. The enum
/// DECLARATION is deliberately not expected to carry the gate -- it names no core path -- but
/// every `impl From<...>` that does name one must. ~keep
fn gated_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "GatedMode".to_string(),
            rust_path: "test_lib::GatedMode".to_string(),
            cfg: Some(r#"feature = "candle-ocr""#.to_string()),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Precise".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        functions: vec![
            FunctionDef {
                name: "make_mode".to_string(),
                rust_path: "test_lib::make_mode".to_string(),
                return_type: TypeRef::Named("GatedMode".to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "use_mode".to_string(),
                rust_path: "test_lib::use_mode".to_string(),
                params: vec![ParamDef {
                    name: "mode".to_string(),
                    ty: TypeRef::Named("GatedMode".to_string()),
                    ..Default::default()
                }],
                return_type: TypeRef::Unit,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn generate_bindings_gates_enum_core_to_binding_conversion_behind_the_enum_cfg() {
    let api = gated_enum_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\nimpl From<test_lib::GatedMode> for GatedMode"),
        "the core->binding `From` impl for a cfg-gated ENUM must carry the enum's own \
         `#[cfg(...)]`; without it a feature-narrowed build names a core module that was never \
         compiled in, got:\n{lib_rs}"
    );
}

#[test]
fn generate_bindings_gates_enum_binding_to_core_conversion_behind_the_enum_cfg() {
    let api = gated_enum_api();
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("#[cfg(feature = \"candle-ocr\")]\nimpl From<GatedMode> for test_lib::GatedMode"),
        "the binding->core `From` impl for a cfg-gated ENUM must carry the enum's own \
         `#[cfg(...)]`, got:\n{lib_rs}"
    );
}

#[test]
fn generate_bindings_leaves_an_ungated_enum_conversion_ungated() {
    let mut api = gated_enum_api();
    api.enums[0].cfg = None;
    let config = magnus_config();
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        !lib_rs.contains("#[cfg("),
        "an enum with no `cfg` of its own must not acquire one, or every consumer loses the \
         conversion behind a feature that was never declared, got:\n{lib_rs}"
    );
}
