use super::gen_from_lifetime_type_constructor;
use super::render::{gen_from_binding_to_core_cfg, gen_from_explicit_new_constructor};
use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{DefaultValue, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};

fn llm_field() -> FieldDef {
    FieldDef {
        name: "llm".to_string(),
        ty: TypeRef::Named("LlmConfig".to_string()),
        ..Default::default()
    }
}

fn alt_text_field() -> FieldDef {
    FieldDef {
        name: "alt_text".to_string(),
        ty: TypeRef::Named("CaptionAltTextMode".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Preserve".to_string())),
        ..Default::default()
    }
}

fn constructor(name: &str, owner: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![
            ParamDef {
                name: "llm".to_string(),
                ty: TypeRef::Named("LlmConfig".to_string()),
                ..Default::default()
            },
            ParamDef {
                name: "alt_text".to_string(),
                ty: TypeRef::Named("CaptionAltTextMode".to_string()),
                ..Default::default()
            },
        ],
        return_type: TypeRef::Named(owner.to_string()),
        is_static: true,
        ..Default::default()
    }
}

fn owner(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        fields: vec![llm_field(), alt_text_field()],
        ..Default::default()
    }
}

fn napi_config() -> ConversionConfig<'static> {
    ConversionConfig {
        type_name_prefix: "Js",
        optionalize_defaults: true,
        optionalize_bare_field_defaults: true,
        ..Default::default()
    }
}

#[test]
fn owner_default_and_bare_field_default_are_not_applied_twice() {
    let mut typ = owner("DefaultOwner");
    typ.has_default = true;

    let out = gen_from_binding_to_core_cfg(&typ, "sample_core", &napi_config());

    assert!(
        out.contains("if let Some(__v) = val.alt_text { __result.alt_text = __v.into(); }"),
        "{out}"
    );
    assert!(!out.contains("__v.map("), "{out}");
}

#[test]
fn explicit_new_defaults_omitted_bare_serde_field() {
    let mut typ = owner("ExplicitOwner");
    typ.methods.push(constructor("new", &typ.name));

    let out = gen_from_binding_to_core_cfg(&typ, "sample_core", &napi_config());

    assert!(out.contains("Self::new("), "{out}");
    assert!(
        out.contains("val.alt_text.map(|__v| __v.into()).unwrap_or_default()"),
        "{out}"
    );
}

#[test]
fn lifetime_constructor_defaults_omitted_bare_serde_field() {
    let mut typ = owner("LifetimeOwner");
    typ.has_lifetime_params = true;
    typ.methods.push(constructor("with_owned", &typ.name));

    let out = gen_from_lifetime_type_constructor(
        &typ,
        "sample_core::LifetimeOwner",
        "JsLifetimeOwner",
        "sample_core",
        &napi_config(),
    )
    .expect("lifetime constructor conversion");

    assert!(out.contains("LifetimeOwner::with_owned("), "{out}");
    assert!(
        out.contains("val.alt_text.map(|__v| __v.into()).unwrap_or_default()"),
        "{out}"
    );
}

#[test]
fn generated_owner_and_field_default_conversions_compile_and_run() {
    let mut default_owner = owner("DefaultOwner");
    default_owner.has_default = true;
    let default_impl = gen_from_binding_to_core_cfg(&default_owner, "sample_core", &napi_config());

    let mut explicit_owner = owner("ExplicitOwner");
    explicit_owner.methods.push(constructor("new", &explicit_owner.name));
    let explicit_impl = gen_from_explicit_new_constructor(
        &explicit_owner,
        "sample_core::ExplicitOwner",
        "JsExplicitOwner",
        &napi_config(),
    )
    .expect("explicit constructor conversion");

    let mut lifetime_owner = owner("LifetimeOwner");
    lifetime_owner.has_lifetime_params = true;
    lifetime_owner
        .methods
        .push(constructor("with_owned", &lifetime_owner.name));
    let lifetime_impl = gen_from_lifetime_type_constructor(
        &lifetime_owner,
        "sample_core::LifetimeOwner",
        "JsLifetimeOwner",
        "sample_core",
        &napi_config(),
    )
    .expect("lifetime constructor conversion");

    let source = format!(
        r#"
#![allow(dead_code)]
mod sample_core {{
    #[derive(Clone, Copy)] pub enum LlmConfig {{ Ready }}
    #[derive(Clone, Copy, Default)] pub enum CaptionAltTextMode {{ #[default] Preserve, Replace }}
    pub struct DefaultOwner {{ pub llm: LlmConfig, pub alt_text: CaptionAltTextMode }}
    impl Default for DefaultOwner {{
        fn default() -> Self {{ Self {{ llm: LlmConfig::Ready, alt_text: CaptionAltTextMode::Replace }} }}
    }}
    pub struct ExplicitOwner {{ pub llm: LlmConfig, pub alt_text: CaptionAltTextMode }}
    impl ExplicitOwner {{
        pub fn new(llm: LlmConfig, alt_text: CaptionAltTextMode) -> Self {{ Self {{ llm, alt_text }} }}
    }}
    pub struct LifetimeOwner<'a> {{
        pub llm: LlmConfig,
        pub alt_text: CaptionAltTextMode,
        marker: core::marker::PhantomData<&'a ()>,
    }}
    impl LifetimeOwner<'_> {{
        pub fn with_owned(llm: LlmConfig, alt_text: CaptionAltTextMode) -> Self {{
            Self {{ llm, alt_text, marker: core::marker::PhantomData }}
        }}
    }}
}}
#[derive(Clone, Copy)] enum JsLlmConfig {{ Ready }}
impl From<JsLlmConfig> for sample_core::LlmConfig {{
    fn from(_: JsLlmConfig) -> Self {{ Self::Ready }}
}}
#[derive(Clone, Copy)] enum JsCaptionAltTextMode {{ Preserve }}
impl From<JsCaptionAltTextMode> for sample_core::CaptionAltTextMode {{
    fn from(_: JsCaptionAltTextMode) -> Self {{ Self::Preserve }}
}}
struct JsDefaultOwner {{ llm: Option<JsLlmConfig>, alt_text: Option<JsCaptionAltTextMode> }}
struct JsExplicitOwner {{ llm: JsLlmConfig, alt_text: Option<JsCaptionAltTextMode> }}
struct JsLifetimeOwner {{ llm: JsLlmConfig, alt_text: Option<JsCaptionAltTextMode> }}
{default_impl}
{explicit_impl}
{lifetime_impl}
fn main() {{
    let defaulted: sample_core::DefaultOwner =
        JsDefaultOwner {{ llm: Some(JsLlmConfig::Ready), alt_text: None }}.into();
    let explicit: sample_core::ExplicitOwner =
        JsExplicitOwner {{ llm: JsLlmConfig::Ready, alt_text: None }}.into();
    let lifetime: sample_core::LifetimeOwner<'_> =
        JsLifetimeOwner {{ llm: JsLlmConfig::Ready, alt_text: None }}.into();
    assert!(matches!(defaulted.alt_text, sample_core::CaptionAltTextMode::Replace));
    assert!(matches!(explicit.alt_text, sample_core::CaptionAltTextMode::Preserve));
    assert!(matches!(lifetime.alt_text, sample_core::CaptionAltTextMode::Preserve));
}}
"#
    );
    let directory = tempfile::tempdir().expect("compile directory");
    let source_path = directory.path().join("field_defaults.rs");
    let binary_path = directory.path().join("field-defaults");
    std::fs::write(&source_path, &source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        compile.status.success(),
        "generated conversions must compile: {}\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(
        std::process::Command::new(binary_path)
            .status()
            .expect("run compile harness")
            .success()
    );
}
