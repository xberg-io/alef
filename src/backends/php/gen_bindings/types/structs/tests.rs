use super::*;

#[cfg(test)]
mod redundant_field_shorthand_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::PrimitiveType;

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    /// `from_json`-constructor path (`use_from_json == true`): a single-word field name
    /// (e.g. `provider`) is already valid snake_case, so `to_php_name` maps it to itself.
    /// The emitted `Self { ... }` literal must use field-init shorthand, not `provider:
    /// provider`.
    #[test]
    fn from_json_constructor_uses_shorthand_for_single_word_field() {
        let typ = TypeDef {
            name: "LlmConfig".to_string(),
            rust_path: "test_lib::LlmConfig".to_string(),
            fields: vec![field("provider", TypeRef::String, false)],
            has_default: true,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");

        assert!(
            !out.contains("provider: provider"),
            "single-word field must use shorthand, not `provider: provider`:\n{out}"
        );
        assert!(
            out.contains("{ provider }") || out.contains("{ provider, "),
            "expected field-init shorthand for `provider`:\n{out}"
        );
    }

    /// Same `from_json`-constructor path, but for a multi-word field whose PHP param name
    /// (`chunkSize`) genuinely differs from the Rust field name (`chunk_size`) — the real
    /// `field: expr` form must still be emitted.
    #[test]
    fn from_json_constructor_keeps_named_form_for_multi_word_field() {
        let typ = TypeDef {
            name: "ChunkConfig".to_string(),
            rust_path: "test_lib::ChunkConfig".to_string(),
            fields: vec![field("chunk_size", TypeRef::Primitive(PrimitiveType::U32), false)],
            has_default: true,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");

        assert!(
            out.contains("chunk_size: chunkSize"),
            "differing param name must keep explicit field-init form:\n{out}"
        );
    }

    /// Plain `#[php(constructor)]` path (no serde, no defaults): single-word field must
    /// also emit shorthand rather than `label: label`.
    #[test]
    fn plain_constructor_uses_shorthand_for_single_word_field() {
        let typ = TypeDef {
            name: "SimpleLabel".to_string(),
            rust_path: "test_lib::SimpleLabel".to_string(),
            fields: vec![field("label", TypeRef::String, false)],
            has_default: false,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            false,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");

        assert!(
            !out.contains("label: label"),
            "single-word field must use shorthand, not `label: label`:\n{out}"
        );
        assert!(
            out.contains("{ label }") || out.contains("{ label, "),
            "expected field-init shorthand for `label`:\n{out}"
        );
    }
}

#[cfg(test)]
mod optional_named_setter_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;

    fn mapper_with_enums(enum_names: &AHashSet<String>) -> PhpMapper {
        PhpMapper {
            enum_names: enum_names.clone(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    fn names(values: &[&str]) -> AHashSet<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    /// Generate the `#[php_impl]` block for a one-field struct, with the caller controlling
    /// which type names are enums and which are opaque.
    fn gen_one_field_impl(field: FieldDef, enum_names: &AHashSet<String>, opaque_types: &AHashSet<String>) -> String {
        let typ = TypeDef {
            name: "Consumer".to_string(),
            rust_path: "test_lib::Consumer".to_string(),
            fields: vec![field],
            has_default: true,
            has_serde: true,
            ..Default::default()
        };

        gen_struct_methods_with_exclude(
            &typ,
            &mapper_with_enums(enum_names),
            true,
            "test_lib",
            opaque_types,
            enum_names,
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate")
    }

    /// `class_derives!` — what `#[php_class]` expands to — implements `FromZval<'a>` only for
    /// `&'a T`, so an owned `T` parameter fails ext-php-rs's argument bound with E0277. The
    /// setter must therefore take `Option<&T>`, matching the shape `gen_php_function_params`
    /// already emits for optional non-opaque Named function parameters.
    #[test]
    fn optional_named_struct_setter_takes_a_shared_reference() {
        let out = gen_one_field_impl(
            field(
                "ocr",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrConfig".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("pub fn set_ocr(&mut self, value: Option<&OcrConfig>)"),
            "the setter must borrow the wrapped struct, not take it by value:\n{out}"
        );
        assert!(
            !out.contains("value: Option<OcrConfig>"),
            "an owned `Option<T>` parameter does not implement `FromZvalMut` and fails to \
             compile with E0277:\n{out}"
        );
    }

    /// The backing field is owned, so the setter has to clone out of the borrow before
    /// converting — the same `v.clone().into()` step `gen_php_call_args_vec` performs for a
    /// `&T` call argument.
    #[test]
    fn optional_named_struct_setter_clones_out_of_the_borrow() {
        let out = gen_one_field_impl(
            field(
                "ocr",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrConfig".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("self.ocr = value.map(|value| value.clone().into());"),
            "the setter must write a cloned, converted value through to the backing field:\n{out}"
        );
    }

    /// The getter is unaffected: `class_derives!` implements `IntoZval` for the owned `T`, so
    /// returning `Option<T>` by value is already correct and must stay as it is.
    #[test]
    fn optional_named_struct_getter_still_returns_an_owned_value() {
        let out = gen_one_field_impl(
            field(
                "ocr",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrConfig".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("pub fn get_ocr(&self) -> Option<OcrConfig>"),
            "the getter must keep returning an owned wrapper:\n{out}"
        );
    }

    /// A field typed as a bare `Named` but marked optional takes the same borrowed shape —
    /// the emitter must not fall back to the owned `Option<T>` form for it either.
    #[test]
    fn bare_named_optional_field_setter_takes_a_shared_reference() {
        let out = gen_one_field_impl(
            field("chunking", TypeRef::Named("ChunkingConfig".to_string()), true),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("pub fn set_chunking(&mut self, value: Option<&ChunkingConfig>)"),
            "an optional bare-Named field must also borrow its parameter:\n{out}"
        );
        assert!(
            out.contains("self.chunking = value.map(|value| value.clone().into());"),
            "an optional bare-Named field must clone out of the borrow too:\n{out}"
        );
    }

    /// Unchanged behaviour: an `Option<enum>` field maps to PHP `String` and is already a real
    /// `#[php(prop)]` promoted `readonly` constructor param, so it gets no mutator at all.
    #[test]
    fn optional_named_enum_field_gets_no_setter() {
        let enum_names = names(&["OcrStrategy"]);
        let out = gen_one_field_impl(
            field(
                "ocr_strategy",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrStrategy".to_string()))),
                true,
            ),
            &enum_names,
            &AHashSet::new(),
        );

        assert!(
            !out.contains("pub fn set_ocr_strategy"),
            "an enum field is a readonly promoted property and must not get a back-door \
             mutator:\n{out}"
        );
    }

    /// Unchanged behaviour: opaque handles are reached through their own wither methods, so
    /// they must not additionally get a conflicting `set_<field>`.
    #[test]
    fn optional_opaque_field_gets_no_setter() {
        let out = gen_one_field_impl(
            field(
                "engine",
                TypeRef::Optional(Box::new(TypeRef::Named("EngineHandle".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &names(&["EngineHandle"]),
        );

        assert!(
            !out.contains("pub fn set_engine"),
            "opaque fields must not get a generated setter:\n{out}"
        );
    }

    /// Unchanged behaviour: a scalar field is a plain `#[php(prop)]` and needs no method.
    #[test]
    fn optional_scalar_field_gets_no_setter() {
        let out = gen_one_field_impl(
            field("cache_namespace", TypeRef::Optional(Box::new(TypeRef::String)), true),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            !out.contains("pub fn set_cache_namespace"),
            "a scalar prop field must not get a generated setter:\n{out}"
        );
    }
}

#[cfg(test)]
mod constructor_param_order_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::PrimitiveType;

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    /// `optional` is set separately from `ty` — mirroring the real extractor's contract
    /// (`extract_field`/`unwrap_optional` in `src/extract/extractor/helpers/field_types.rs`),
    /// which always strips exactly one layer of `Option<T>` into `(T, true)`. A caller must NOT
    /// pass an already-`TypeRef::Optional`-wrapped `ty` alongside `optional: true` — that shape
    /// never occurs in real IR and double-wraps the generated param type (`Option<Option<T>>`),
    /// as `casts_optional_remapped_primitive_back_to_core` in the extendr tests demonstrates for
    /// the correct convention (bare `ty`, `optional` flag set on the side). ~keep
    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    /// Slice out just the `pub fn new(...)` parameter list, not the `Self { ... }` initializer
    /// that follows it. The two can name fields differently (the initializer uses raw Rust field
    /// names — `global_limit: globalLimit` — while the signature uses the camelCase PHP param
    /// name), and struct-literal field order is semantically irrelevant in Rust, so a search over
    /// the whole rendered block can match the initializer and pass (or fail) without checking the
    /// signature at all — the actual bug class this file exists to catch. Route every ordering
    /// assertion through this so the test can't drift onto testing something adjacent to what its
    /// name claims. ~keep
    fn constructor_signature(new_fn: &str) -> &str {
        let after_fn = new_fn
            .split_once("pub fn new(")
            .map(|(_, rest)| rest)
            .unwrap_or_else(|| panic!("no `pub fn new(` found in generated constructor:\n{new_fn}"));
        after_fn
            .split_once(") -> Self {")
            .map(|(sig, _)| sig)
            .unwrap_or_else(|| panic!("no `) -> Self {{` found in generated constructor:\n{new_fn}"))
    }

    fn param_index(signature: &str, param_name: &str) -> usize {
        signature
            .find(&format!("{param_name}:"))
            .unwrap_or_else(|| panic!("param `{param_name}` not found in constructor signature:\n{signature}"))
    }

    /// Regression for the `BudgetConfig` bug: the runtime `#[php(constructor)] fn new(...)` used
    /// raw field-declaration order, while the PHPStan stub (`type_stubs.rs`) stable-sorts
    /// required-before-optional. A struct whose declared field order puts an optional field first
    /// (`global_limit: Option<f64>`, mirroring `BudgetConfig`) used to emit `new(global_limit,
    /// model_limits, enforcement)` while the stub declared `(model_limits, enforcement,
    /// global_limit)` — a caller following the stub's signature positionally shipped its budget
    /// cap into the wrong parameter slot. `new`'s parameter order must match the stub's.
    #[test]
    fn required_fields_sort_before_optional_regardless_of_declaration_order() {
        let typ = TypeDef {
            name: "BudgetConfig".to_string(),
            rust_path: "test_lib::BudgetConfig".to_string(),
            fields: vec![
                field("global_limit", TypeRef::Primitive(PrimitiveType::F64), true),
                field("model_limits", TypeRef::String, false),
                field("enforcement", TypeRef::String, false),
            ],
            has_default: true,
            has_serde: true,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");

        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let sig = constructor_signature(new_fn);

        assert!(sig.contains("globalLimit: Option<f64>"), "{sig}");

        let model_limits_idx = param_index(sig, "modelLimits");
        let enforcement_idx = param_index(sig, "enforcement");
        let global_limit_idx = param_index(sig, "globalLimit");

        assert!(
            model_limits_idx < global_limit_idx && enforcement_idx < global_limit_idx,
            "required fields must precede the optional field despite declaration order: {sig}"
        );
    }

    /// Regression for the `RateLimitConfig` bug: a `Duration` field on a type with a `Default`
    /// impl is widened to an optional `Option<i64>` param (see the `optional =` computation in
    /// `gen_struct_methods_impl`) even though the field itself is not `Option<_>` in the IR. The
    /// widened field must sort as optional too — using the raw (pre-widening) `f.optional` for the
    /// sort key would leave a required-looking `Duration` field ahead of params that are
    /// genuinely optional in the IR, while the stub (which applies the identical widening) puts it
    /// last. A genuinely required third field (`name`) is included so the ordering assertion
    /// actually exercises the sort — with only two fields that both end up "optional", a stable
    /// sort is a correct no-op that preserves declaration order and proves nothing.
    #[test]
    fn duration_field_widened_by_default_impl_sorts_as_optional() {
        let typ = TypeDef {
            name: "RateLimitConfig".to_string(),
            rust_path: "test_lib::RateLimitConfig".to_string(),
            fields: vec![
                field("window", TypeRef::Duration, false),
                field("rpm", TypeRef::Primitive(PrimitiveType::U32), true),
                field("name", TypeRef::String, false),
            ],
            has_default: true,
            has_serde: true,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");

        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let sig = constructor_signature(new_fn);

        assert!(
            sig.contains("window: Option<i64>"),
            "Duration field on a Default-impl type must widen to Option<i64>: {sig}"
        );
        let name_idx = param_index(sig, "name");
        let window_idx = param_index(sig, "window");
        let rpm_idx = param_index(sig, "rpm");
        assert!(
            name_idx < window_idx && name_idx < rpm_idx,
            "the genuinely required field must precede both the raw-optional and the \
             widened-optional field despite `window` being declared first: {sig}"
        );
    }

    /// Same regression as `required_fields_sort_before_optional_regardless_of_declaration_order`,
    /// but for the plain `#[php(constructor)]` path (no serde, no defaults, no named params) —
    /// the second, independent code path in `gen_struct_methods_impl` that builds a `new(...)`
    /// straight from field declaration order.
    #[test]
    fn plain_constructor_path_also_sorts_required_before_optional() {
        let typ = TypeDef {
            name: "PlainConfig".to_string(),
            rust_path: "test_lib::PlainConfig".to_string(),
            fields: vec![
                field("nickname", TypeRef::String, true),
                field("host", TypeRef::String, false),
                field("port", TypeRef::Primitive(PrimitiveType::U32), false),
            ],
            has_default: false,
            has_serde: false,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            false,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");

        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let sig = constructor_signature(new_fn);

        assert!(sig.contains("nickname: Option<String>"), "{sig}");
        assert!(
            param_index(sig, "host") < param_index(sig, "nickname")
                && param_index(sig, "port") < param_index(sig, "nickname"),
            "required fields must precede the optional field despite declaration order: {sig}"
        );
    }

    #[test]
    fn bare_serde_default_stays_required_in_runtime_without_serde() {
        let mut alt_text = field("alt_text", TypeRef::Named("CaptionAltTextMode".to_string()), false);
        alt_text.default = Some("/* serde(default) */".to_string());
        alt_text.typed_default = Some(crate::core::ir::DefaultValue::EnumVariant("Preserve".to_string()));
        let typ = TypeDef {
            name: "CaptioningConfig".to_string(),
            rust_path: "test_lib::CaptioningConfig".to_string(),
            fields: vec![field("llm", TypeRef::String, false), alt_text],
            ..Default::default()
        };
        let enum_names: AHashSet<String> = ["CaptionAltTextMode".to_string()].into_iter().collect();

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            false,
            "test_lib",
            &AHashSet::new(),
            &enum_names,
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");
        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let signature = constructor_signature(new_fn);

        assert!(signature.contains("llm: String"), "{signature}");
        assert!(signature.contains("altText: &CaptionAltTextMode"), "{signature}");
        assert!(!signature.contains("altText: Option"), "{signature}");
    }

    #[test]
    fn bare_serde_default_before_required_field_stays_required_and_in_place() {
        let mut alt_text = field("alt_text", TypeRef::Named("CaptionAltTextMode".to_string()), false);
        alt_text.default = Some("/* serde(default) */".to_string());
        alt_text.typed_default = Some(crate::core::ir::DefaultValue::EnumVariant("Preserve".to_string()));
        let typ = TypeDef {
            name: "CaptioningConfig".to_string(),
            rust_path: "test_lib::CaptioningConfig".to_string(),
            fields: vec![
                field("llm", TypeRef::String, false),
                alt_text,
                field("prompt", TypeRef::String, false),
            ],
            ..Default::default()
        };
        let enum_names: AHashSet<String> = ["CaptionAltTextMode".to_string()].into_iter().collect();

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &enum_names,
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");
        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let signature = constructor_signature(new_fn);

        assert!(signature.contains("altText: &CaptionAltTextMode"), "{signature}");
        assert!(!signature.contains("altText: Option"), "{signature}");
        assert!(
            param_index(signature, "llm") < param_index(signature, "altText")
                && param_index(signature, "altText") < param_index(signature, "prompt"),
            "a non-suffix serde default must preserve declaration order: {signature}"
        );
        assert!(new_fn.contains("alt_text: altText"), "{new_fn}");
        assert!(!new_fn.contains("altText.unwrap_or_else"), "{new_fn}");
    }

    #[test]
    fn trailing_bare_serde_default_remains_optional() {
        let mut alt_text = field("alt_text", TypeRef::Named("CaptionAltTextMode".to_string()), false);
        alt_text.default = Some("/* serde(default) */".to_string());
        alt_text.typed_default = Some(crate::core::ir::DefaultValue::EnumVariant("Preserve".to_string()));
        let typ = TypeDef {
            name: "CaptioningConfig".to_string(),
            rust_path: "test_lib::CaptioningConfig".to_string(),
            fields: vec![field("llm", TypeRef::String, false), alt_text],
            ..Default::default()
        };
        let enum_names: AHashSet<String> = ["CaptionAltTextMode".to_string()].into_iter().collect();

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &enum_names,
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");
        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let signature = constructor_signature(new_fn);

        assert!(
            signature.contains("altText: Option<&CaptionAltTextMode>"),
            "{signature}"
        );
        assert!(new_fn.contains("altText.unwrap_or_else"), "{new_fn}");
    }

    #[test]
    fn trailing_defaulted_named_type_uses_its_default_when_omitted() {
        let mut sizing = field("sizing", TypeRef::Named("ChunkSizing".to_string()), false);
        sizing.default = Some("/* serde(default) */".to_string());
        sizing.typed_default = Some(crate::core::ir::DefaultValue::EnumVariant("Characters".to_string()));
        let typ = TypeDef {
            name: "ChunkingConfig".to_string(),
            rust_path: "test_lib::ChunkingConfig".to_string(),
            fields: vec![field("chunker_type", TypeRef::String, false), sizing],
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");
        let new_fn = out
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
        let signature = constructor_signature(new_fn);

        assert!(signature.contains("sizing: Option<&ChunkSizing>"), "{signature}");
        assert!(
            new_fn.contains("sizing: sizing.cloned().unwrap_or_default()"),
            "{new_fn}"
        );
    }
}

mod json_constructor_param_tests;
mod nested_struct_constructor_param_tests;
mod vec_named_struct_constructor_param_tests;

/// Cross-emitter parity between the `#[php(constructor)]` signature this module emits and the
/// PHP named-argument constructor call the e2e generator writes for the SAME `TypeDef`.
///
/// PHP named arguments are matched by parameter NAME, so these two emitters must agree letter for
/// letter or the generated call dies with `Unknown named parameter`. They read the same `FieldDef`
/// and used to lower it through two different helpers: this side through
/// `naming::to_php_name` (lower-camel only), the e2e side through `naming::public_field_name`
/// (lower-camel plus reserved-word escaping). The two agree on every field name except one that
/// is a PHP reserved word -- the escape appends `_` and nothing on the binding side ever does.
#[cfg(test)]
mod e2e_named_argument_parity_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;

    const NAMESPACE: &str = "Sample\\Bindings";
    const TYPE_NAME: &str = "SampleOptions";
    /// A PHP reserved word (`core::keywords::PHP_KEYWORDS`) that is NOT a Rust keyword and is
    /// already valid lower-camel-case, so the ONLY thing that can change its spelling on either
    /// side is reserved-word escaping. PHP itself accepts it as a parameter name and as a named
    /// argument, so escaping is not a second valid spelling -- it is the wrong one.
    const KEYWORD_FIELD: &str = "list";
    const PLAIN_FIELD: &str = "max_chars";

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    fn options_type() -> TypeDef {
        TypeDef {
            name: TYPE_NAME.to_string(),
            rust_path: format!("sample_core::{TYPE_NAME}"),
            fields: vec![
                FieldDef {
                    name: KEYWORD_FIELD.to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                FieldDef {
                    name: PLAIN_FIELD.to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// The parameter names the emitted `#[php(constructor)] pub fn new(...)` declares. Sliced out
    /// of the signature only, never the `Self { .. }` initializer that follows it -- the
    /// initializer uses raw Rust field names, so matching there would pass while the signature
    /// (the half PHP actually binds named arguments against) said something else.
    fn binding_constructor_params(binding: &str) -> Vec<String> {
        let new_fn = binding
            .split("#[php(constructor)]")
            .nth(1)
            .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{binding}"));
        let signature = new_fn
            .split_once("pub fn new(")
            .and_then(|(_, rest)| rest.split_once(") -> Self {"))
            .map(|(signature, _)| signature)
            .unwrap_or_else(|| panic!("no `pub fn new(..) -> Self {{` in:\n{new_fn}"));
        signature
            .split(',')
            .filter_map(|param| param.split_once(':'))
            .map(|(name, _)| name.trim().trim_start_matches("r#").to_string())
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// The named-argument keys the e2e generator writes in `new \Ns\Type(key: value, ...)`.
    fn e2e_named_arguments(literal: &str) -> Vec<String> {
        let args = literal
            .split_once('(')
            .and_then(|(_, rest)| rest.rsplit_once(')'))
            .map(|(args, _)| args)
            .unwrap_or_else(|| panic!("no argument list in the rendered DTO literal:\n{literal}"));
        args.split(',')
            .filter_map(|arg| arg.split_once(':'))
            .map(|(name, _)| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect()
    }

    #[test]
    fn should_name_e2e_named_arguments_exactly_as_the_php_constructor_declares_them() {
        let typ = options_type();
        let binding = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            false,
            "sample_core",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
        .expect("struct methods generate");
        let declared = binding_constructor_params(&binding);
        assert!(
            declared.iter().any(|name| name == KEYWORD_FIELD),
            "the binding must declare the reserved-word field unescaped, got {declared:?}:\n{binding}"
        );

        let mut object = serde_json::Map::new();
        object.insert(KEYWORD_FIELD.to_string(), serde_json::Value::String("a".to_string()));
        object.insert(PLAIN_FIELD.to_string(), serde_json::Value::String("b".to_string()));
        let fixture = serde_json::Value::Object(object);
        let literal = crate::e2e::codegen::php::values::render_native_php_dto(
            NAMESPACE,
            TYPE_NAME,
            &fixture,
            std::slice::from_ref(&typ),
            &[],
        )
        .expect("the fixture is an all-scalar DTO the e2e renderer constructs natively");

        for name in e2e_named_arguments(&literal) {
            assert!(
                declared.contains(&name),
                "e2e passes named argument `{name}:` but the constructor declares {declared:?}; \
                 PHP binds named arguments by parameter name, so this call cannot resolve.\n\
                 binding:\n{binding}\nliteral:\n{literal}"
            );
        }
        assert!(
            !literal.contains(&format!("{KEYWORD_FIELD}_:")),
            "a reserved-word field must not be escaped on the call side:\n{literal}"
        );
    }
}
