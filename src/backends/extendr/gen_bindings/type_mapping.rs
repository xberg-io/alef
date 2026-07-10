use std::borrow::Cow;

use crate::codegen::generators::{AsyncPattern, RustBindingConfig};
use crate::codegen::type_mapper::TypeMapper;

use super::ExtendrBackend;

impl ExtendrBackend {
    pub(super) fn binding_config<'a>(core_import: &'a str, lossy_skip_types: &'a [String]) -> RustBindingConfig<'a> {
        RustBindingConfig {
            struct_attrs: &[],
            field_attrs: &[],
            struct_derives: &["Clone"],
            // #[extendr] on impl blocks registers the struct as an R class, which enables
            // the ToVectorValue trait bound required for returning struct types from #[extendr]
            method_block_attr: Some("extendr"),
            constructor_attr: "",
            static_attr: None,
            function_attr: "#[extendr]",
            enum_attrs: &[],
            enum_derives: &["Clone", "PartialEq"],
            needs_signature: false,
            signature_prefix: "",
            signature_suffix: "",
            core_import,
            async_pattern: AsyncPattern::TokioBlockOn,
            has_serde: true,
            type_name_prefix: "",
            option_duration_on_defaults: false,
            opaque_type_names: &[],
            // The extendr backend uses a separate #[extendr] free-function kwargs constructor
            skip_impl_constructor: true,
            cast_uints_to_i32: true,
            cast_large_ints_to_f64: true,
            // extendr's #[extendr] macro generates TryFrom<&Robj> for &T (references only).
            named_non_opaque_params_by_ref: true,
            lossy_skip_types,
            serializable_opaque_type_names: &[],
            never_skip_cfg_field_names: &[],
            emit_delegating_default_impl: false,
            // extendr's `#[extendr]` macro fails to expand impl blocks that contain
            skip_methods_when_not_delegatable: true,
            source_crate_remaps: &[],
            emit_delegating_default_for_types: None,
        }
    }
}

impl TypeMapper for ExtendrBackend {
    fn primitive(&self, prim: &crate::core::ir::PrimitiveType) -> Cow<'static, str> {
        use crate::core::ir::PrimitiveType;
        match prim {
            PrimitiveType::Bool => Cow::Borrowed("bool"),
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32 => Cow::Borrowed("i32"),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                Cow::Borrowed("f64")
            }
            PrimitiveType::F32 | PrimitiveType::F64 => Cow::Borrowed("f64"),
        }
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }
}
