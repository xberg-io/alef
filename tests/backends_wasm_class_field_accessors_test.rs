//! alef#470, #472, #473: every place a generated wasm binding takes a handle to a class.
//!
//! wasm-bindgen lowers a by-value exported-struct argument through `__destroy_into_raw()`, so a
//! by-value parameter kills the handle the caller passed. Three emitters take one: the struct
//! setter (#470), the tagged-data-enum payload setter (#473) and the struct constructor (#472).
//! These tests pin the borrow that replaces it where a borrow is expressible, the
//! `clear{Field}()` companion that restores clearing (`Option<&T>` has no `OptionFromWasmAbi`
//! impl, so the borrowed setter cannot accept `null`), and -- just as deliberately -- the
//! constructor position where a borrow is *not* expressible and the consumed argument is
//! documented instead.
//!
//! These are source-text assertions and cannot see the defect itself: pre-fix output compiles.
//! `backends::wasm::wasm_bindgen_js_oracle` is the check that reads the emitted JS.

use alef::backends::wasm::WasmBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn resolved_wasm_config() -> alef::core::config::ResolvedCrateConfig {
    resolved_wasm_config_with("")
}

/// `extra` is appended to the `[crates.wasm]` table, so a test can add `type_overrides` or
/// `exclude_types` without repeating the whole document.
fn resolved_wasm_config_with(extra: &str) -> alef::core::config::ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.wasm]
{extra}
"#
    ))
    .expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

fn named(name: &str) -> TypeRef {
    TypeRef::Named(name.to_string())
}

fn palette_type() -> TypeDef {
    TypeDef {
        name: "Palette".to_string(),
        rust_path: "test_lib::Palette".to_string(),
        fields: vec![field("label", TypeRef::String, false)],
        is_clone: true,
        has_default: true,
        ..Default::default()
    }
}

fn handle_type() -> TypeDef {
    TypeDef {
        name: "RendererHandle".to_string(),
        rust_path: "test_lib::RendererHandle".to_string(),
        is_opaque: true,
        ..Default::default()
    }
}

fn theme_type() -> TypeDef {
    TypeDef {
        name: "Theme".to_string(),
        rust_path: "test_lib::Theme".to_string(),
        fields: vec![
            field("palette", named("Palette"), false),
            field("fallback_palette", named("Palette"), true),
            // An `Optional` TypeRef stores an `Option` even when the IR left the flag false;
            // the setter's `Some(..)` and the clear method both have to follow the stored
            // shape, not the flag, or the assignment is an `E0308`. ~keep
            field("alt_palette", TypeRef::Optional(Box::new(named("Palette"))), false),
            field("renderer", named("RendererHandle"), true),
            field("label", TypeRef::String, true),
        ],
        is_clone: true,
        ..Default::default()
    }
}

/// A type whose own API already mints `clear_renderer`, so alef's companion would collide with
/// it as `E0592`.
fn guarded_theme_type() -> TypeDef {
    TypeDef {
        name: "GuardedTheme".to_string(),
        rust_path: "test_lib::GuardedTheme".to_string(),
        fields: vec![field("renderer", named("RendererHandle"), true)],
        methods: vec![MethodDef {
            name: "clear_renderer".to_string(),
            return_type: TypeRef::Unit,
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        is_clone: true,
        ..Default::default()
    }
}

fn generated_lib_rs(types: Vec<TypeDef>) -> String {
    generated_lib_rs_for(types, Vec::new(), &resolved_wasm_config())
}

fn generated_lib_rs_for(
    types: Vec<TypeDef>,
    enums: Vec<EnumDef>,
    config: &alef::core::config::ResolvedCrateConfig,
) -> String {
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "1.0.0".to_string(),
        types,
        enums,
        ..Default::default()
    };
    WasmBackend
        .generate_bindings(&api, config)
        .expect("wasm generation should succeed")
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content
        .clone()
}

/// A tagged data enum whose payload carries a class-typed field alongside a plain one.
fn layer_enum() -> EnumDef {
    EnumDef {
        name: "Layer".to_string(),
        rust_path: "test_lib::Layer".to_string(),
        serde_tag: Some("kind".to_string()),
        variants: vec![
            EnumVariant {
                name: "Solid".to_string(),
                fields: vec![
                    field("palette", named("Palette"), false),
                    field("label", TypeRef::String, false),
                ],
                ..Default::default()
            },
            EnumVariant {
                name: "Blank".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// A struct that does NOT derive `Default`, so `shared::constructor_parts` types its required
/// parameters by their own type rather than `Option<T>`.
fn render_options_type() -> TypeDef {
    TypeDef {
        name: "RenderOptions".to_string(),
        rust_path: "test_lib::RenderOptions".to_string(),
        fields: vec![
            field("palette", named("Palette"), false),
            field("title", TypeRef::String, false),
            field("fallback_palette", named("Palette"), true),
        ],
        is_clone: true,
        ..Default::default()
    }
}

/// The same shape with `#[derive(Default)]`, which routes the constructor through
/// `shared::config_constructor_parts_inner` and makes every parameter `Option<T>`.
fn defaulted_options_type() -> TypeDef {
    TypeDef {
        name: "DefaultedOptions".to_string(),
        rust_path: "test_lib::DefaultedOptions".to_string(),
        fields: vec![
            field("palette", named("Palette"), false),
            field("title", TypeRef::String, false),
        ],
        is_clone: true,
        has_default: true,
        ..Default::default()
    }
}

#[test]
fn class_typed_setters_borrow_instead_of_consuming() {
    let content = generated_lib_rs(vec![palette_type(), handle_type(), theme_type()]);
    for expected in [
        "pub fn set_palette(&mut self, value: &WasmPalette)",
        "self.palette = value.clone();",
        "pub fn set_fallback_palette(&mut self, value: &WasmPalette)",
        "self.fallback_palette = Some(value.clone());",
        "pub fn set_alt_palette(&mut self, value: &WasmPalette)",
        "self.alt_palette = Some(value.clone());",
        "pub fn set_renderer(&mut self, value: &WasmRendererHandle)",
        "self.renderer = Some(value.clone());",
    ] {
        assert!(content.contains(expected), "missing `{expected}`;\n{content}");
    }
    assert!(
        !content.contains("value: Option<WasmPalette>") && !content.contains("value: Option<WasmRendererHandle>"),
        "no class-typed setter may take its argument by value;\n{content}"
    );
    assert!(
        content.contains("pub fn set_label(&mut self, value: Option<String>)"),
        "a field that is not a generated class keeps the mapped by-value setter;\n{content}"
    );
}

#[test]
fn optional_class_fields_get_a_clear_companion() {
    let content = generated_lib_rs(vec![palette_type(), handle_type(), theme_type()]);
    for expected in [
        "#[wasm_bindgen(js_name = \"clearFallbackPalette\")]\n    pub fn clear_fallback_palette(&mut self) {\n        self.fallback_palette = None;",
        "#[wasm_bindgen(js_name = \"clearAltPalette\")]\n    pub fn clear_alt_palette(&mut self) {\n        self.alt_palette = None;",
        "#[wasm_bindgen(js_name = \"clearRenderer\")]\n    pub fn clear_renderer(&mut self) {\n        self.renderer = None;",
    ] {
        assert!(content.contains(expected), "missing:\n{expected}\n\nin:\n{content}");
    }
}

#[test]
fn required_and_non_class_fields_get_no_clear_companion() {
    let content = generated_lib_rs(vec![palette_type(), handle_type(), theme_type()]);
    assert!(
        !content.contains("pub fn clear_palette("),
        "a required field is never cleared -- its setter already accepts every legal value;\n{content}"
    );
    assert!(
        !content.contains("pub fn clear_label("),
        "an optional non-class field keeps a nullable setter, so it needs no companion;\n{content}"
    );
}

/// The companion stands down rather than shadow the consumer's own method: emitting both is
/// `E0592`, and alef's convenience accessor is the one that has to give way.
#[test]
fn clear_companion_stands_down_on_an_ident_the_type_already_mints() {
    let content = generated_lib_rs(vec![handle_type(), guarded_theme_type()]);
    assert_eq!(
        content.matches("pub fn clear_renderer(").count(),
        1,
        "exactly one `clear_renderer` may exist -- two is E0592;\n{content}"
    );
    assert!(
        !content.contains("self.renderer = None;"),
        "the surviving `clear_renderer` must be the consumer's method, not alef's companion;\n{content}"
    );
}

/// alef#473: the second accessor emitter. A payload field whose type is a generated class must
/// borrow exactly like the struct setter does, and -- because a payload field is *always* an
/// `Option`, whatever the variant declared -- every class-typed one needs a clear companion.
#[test]
fn tagged_enum_payload_setters_borrow_class_typed_arguments() {
    let content = generated_lib_rs_for(vec![palette_type()], vec![layer_enum()], &resolved_wasm_config());

    for expected in [
        "pub fn set_palette(&mut self, value: &WasmPalette)",
        "self.palette = Some(value.clone());",
        "#[wasm_bindgen(js_name = \"clearPalette\")]",
        "pub fn clear_palette(&mut self)",
    ] {
        assert!(content.contains(expected), "missing `{expected}`;\n{content}");
    }
    assert!(
        !content.contains("pub fn set_palette(&mut self, value: Option<WasmPalette>)"),
        "no payload setter may take a class-typed argument by value;\n{content}"
    );
    assert!(
        content.contains("pub fn set_label(&mut self, value: Option<String>)"),
        "a payload field that is not a generated class keeps the mapped by-value setter;\n{content}"
    );
    assert!(
        !content.contains("pub fn clear_label("),
        "a non-class payload field keeps a nullable setter, so it needs no companion;\n{content}"
    );
}

/// The payload emitter used to build its own override-free `WasmMapper`, which made
/// `class_backed_field_type`'s override guard inert: a `type_overrides` entry redirects the field
/// to a type that is not a generated class, and borrowing it is an `E0412`. Threading the real
/// mapper is what this pins.
#[test]
fn tagged_enum_payload_setter_respects_a_type_override() {
    let config = resolved_wasm_config_with("type_overrides = { Palette = \"JsValue\" }");
    let content = generated_lib_rs_for(vec![palette_type()], vec![layer_enum()], &config);

    assert!(
        content.contains("pub fn set_palette(&mut self, value: Option<JsValue>)"),
        "an overridden name is not a generated class, so its setter keeps the mapped type;\n{content}"
    );
    assert!(
        !content.contains("value: &JsValue") && !content.contains("value: &WasmPalette"),
        "nothing may be borrowed when the override redirected the type;\n{content}"
    );
    assert!(
        !content.contains("pub fn clear_palette("),
        "no clear companion for a field that is not class-backed;\n{content}"
    );
}

/// alef#472, the half that is fixable: a required class-typed parameter of a struct that does
/// not derive `Default` is borrowed, and the struct literal stores a clone.
#[test]
fn constructor_borrows_required_class_typed_arguments() {
    let content = generated_lib_rs(vec![palette_type(), render_options_type()]);

    assert!(
        content.contains("pub fn new(palette: &WasmPalette, title: String, fallbackPalette: Option<WasmPalette>)"),
        "the required class-typed parameter must be borrowed;\n{content}"
    );
    assert!(
        content.contains("palette: palette.clone()"),
        "a borrowed parameter has to be cloned into the struct literal;\n{content}"
    );
}

/// alef#472, the half that is NOT fixable, pinned so nobody "fixes" it into code that cannot
/// compile. `shared::config_constructor_parts_inner` types every parameter of a
/// `#[derive(Default)]` struct as `Option<T>`, and `Option<&T>` has no `OptionFromWasmAbi` impl,
/// so no parameter here is borrowable at all. The generated rustdoc is the only warning the
/// consumer gets, so it is part of the contract.
#[test]
fn defaulted_struct_constructor_still_takes_ownership_and_says_so() {
    let content = generated_lib_rs(vec![palette_type(), defaulted_options_type()]);

    assert!(
        content.contains("pub fn new(palette: Option<WasmPalette>, title: Option<String>)"),
        "a `#[derive(Default)]` constructor takes Option<T>, which cannot be borrowed;\n{content}"
    );
    assert!(
        content.contains("/// Takes ownership of `palette`."),
        "the owned argument must be named in the generated rustdoc;\n{content}"
    );
    assert!(
        content.contains("the generated setter borrows and leaves your handle alive."),
        "the doc must point the consumer at the setter that does not take ownership;\n{content}"
    );
}

/// The resolution of alef#479, pinned so it is not rediscovered as a bug.
///
/// A `#[derive(Default)]` struct's constructor cannot borrow any class-typed argument, and
/// dropping the parameter would change a published signature. It needs no fix because the safe
/// path is already fully generated: an arg-free `default()` factory, a `new()` whose every
/// parameter is `Option<T>` and therefore omittable from JS, and a borrowed setter per
/// class-typed field. `default()` plus setters reaches every state the constructor can.
#[test]
fn a_defaulted_struct_already_has_a_complete_ownership_safe_construction_path() {
    let content = generated_lib_rs(vec![palette_type(), defaulted_options_type()]);

    assert!(
        content.contains("pub fn default() -> WasmDefaultedOptions"),
        "the arg-free factory is half of the safe path;\n{content}"
    );
    assert!(
        content.contains("pub fn set_palette(&mut self, value: &WasmPalette)"),
        "the borrowed setter is the other half, and it must reach the class-typed field;\n{content}"
    );
    assert!(
        content.contains("pub fn set_title(&mut self, value: String)"),
        "every other field must be reachable by setter too, or the path is not complete;\n{content}"
    );
}

/// The optional parameter of an otherwise-borrowable constructor hits the same wall, and is
/// documented by the same rustdoc. A constructor that owns nothing gets no such doc.
#[test]
fn only_a_constructor_that_takes_a_handle_carries_the_ownership_note() {
    let consuming = generated_lib_rs(vec![palette_type(), render_options_type()]);
    assert!(
        consuming.contains("/// Takes ownership of `fallbackPalette`."),
        "the optional class-typed parameter is still taken by value and must say so;\n{consuming}"
    );

    let clean = generated_lib_rs(vec![palette_type()]);
    assert!(
        !clean.contains("/// Takes ownership of "),
        "a constructor with no class-typed parameter must carry no ownership note;\n{clean}"
    );
}
