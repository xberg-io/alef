use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::PrimitiveType;
use std::borrow::Cow;

/// TypeMapper for NAPI bindings.
/// JS numbers are 53-bit safe, so U64/Usize/Isize map to i64.
/// Named types get a configurable prefix (defaults to "Js").
/// Trait types are mapped to JsVisitorRef (a Clone-able wrapper around Object<'static>).
pub struct NapiMapper {
    pub prefix: String,
    /// Names of types in the IR that are trait definitions (TypeDef::is_trait == true).
    pub trait_type_names: AHashSet<String>,
    /// Names of capsule types configured under `[crates.node.capsule_types]`.
    /// These reference an external ecosystem-library type — no `Js` prefix.
    pub capsule_type_names: AHashSet<String>,
}

impl NapiMapper {
    pub fn new(prefix: String) -> Self {
        Self {
            prefix,
            trait_type_names: AHashSet::new(),
            capsule_type_names: AHashSet::new(),
        }
    }

    pub fn with_traits_and_capsules(
        prefix: String,
        trait_type_names: AHashSet<String>,
        capsule_type_names: AHashSet<String>,
    ) -> Self {
        Self {
            prefix,
            trait_type_names,
            capsule_type_names,
        }
    }
}

impl TypeMapper for NapiMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "u8",
            PrimitiveType::U16 => "u16",
            PrimitiveType::U32 => "u32",
            PrimitiveType::U64 => "i64",
            PrimitiveType::I8 => "i8",
            PrimitiveType::I16 => "i16",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::F32 => "f64", // NAPI-RS doesn't impl FromNapiValue for f32
            PrimitiveType::F64 => "f64",
            PrimitiveType::Usize => "i64",
            PrimitiveType::Isize => "i64",
        })
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.trait_type_names.contains(name) {
            // Trait types cannot be used as bare Object<'static> fields because
            // Object doesn't implement Clone. Use JsVisitorRef wrapper: a newtype that
            // wraps napi::Object and implements Clone via Arc.
            Cow::Borrowed("JsVisitorRef")
        } else if self.capsule_type_names.contains(name) {
            // Capsule types reference an external ecosystem-library type
            // (e.g. `Language` from `tree-sitter`). Emit the bare name so callers
            // resolve it via the ambient `use` of the ecosystem package.
            Cow::Borrowed(name)
        } else {
            Cow::Owned(format!("{}{name}", self.prefix))
        }
    }

    /// NAPI uses i64 for Duration (JS numbers are 53-bit safe).
    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("i64")
    }

    /// NAPI v3's `serde-json` feature provides FromNapiValue/ToNapiValue for
    /// `serde_json::Value`, so JS callers can pass arbitrary objects/values
    /// directly without first stringifying them.
    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("serde_json::Value")
    }

    /// NAPI v3 keeps `Buffer` under `napi::bindgen_prelude::Buffer`. Using `Vec<u8>`
    /// would cause napi to treat the field as a JS `Array` and call
    /// `napi_get_array_length` on it — which fails with "Failed to get Array length"
    /// when JS passes a `Buffer`/`Uint8Array` (which is what test fixtures emit).
    /// `Buffer` accepts both `Buffer` and `Uint8Array` from JS and gives Rust
    /// borrowed access to the bytes without copying.
    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("napi::bindgen_prelude::Buffer")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }
}
