//! Emits `extern "Rust"` blocks for the `#[swift_bridge::bridge]` module.
//!
//! Covers type declarations, enum declarations, and top-level function declarations.
//! Trait bridge extern blocks live in `trait_bridge.rs`.

use crate::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge};
use alef_core::ir::{EnumDef, FunctionDef, TypeDef, TypeRef};
use alef_core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::HashSet;

pub(crate) fn emit_extern_block_for_type(ty: &TypeDef, exclude_fields: &HashSet<String>) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&format!("        type {};\n", ty.name));

    // Constructor — use bridge_type to avoid nested generics that swift-bridge 0.1.59
    // cannot parse (Vec<Vec<T>>, HashMap<K,V>); those become String (JSON).
    // Excluded fields are omitted from the constructor params.
    if !ty.fields.is_empty() {
        let params: Vec<String> = ty
            .fields
            .iter()
            .filter(|f| {
                let field_key = format!("{}.{}", ty.name, f.name.to_snake_case());
                !exclude_fields.contains(&field_key)
            })
            .map(|f| {
                let bridge_ty = bridge_type(&f.ty);
                let bridge_ty = if f.optional && !needs_json_bridge(&f.ty) {
                    format!("Option<{bridge_ty}>")
                } else {
                    bridge_ty
                };
                let name = f.name.to_snake_case();
                format!("{name}: {bridge_ty}")
            })
            .collect();
        block.push_str("        #[swift_bridge(init)]\n");
        block.push_str(&format!(
            "        fn new({}) -> {};\n",
            params.join(", "),
            ty.name
        ));
    }

    // Getters — excluded fields still get a getter declaration (the impl emits unimplemented!()).
    for field in &ty.fields {
        let bridge_ty = bridge_type(&field.ty);
        let bridge_ty = if field.optional && !needs_json_bridge(&field.ty) {
            format!("Option<{bridge_ty}>")
        } else {
            bridge_ty
        };
        let name = field.name.to_snake_case();
        block.push_str(&format!("        fn {name}(&self) -> {bridge_ty};\n"));
    }

    block.push_str("    }\n\n");
    block
}

pub(crate) fn emit_extern_block_for_enum(en: &EnumDef) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&format!("        type {};\n", en.name));
    block.push_str("    }\n\n");
    block
}

pub(crate) fn emit_extern_block_for_functions(functions: &[FunctionDef]) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    for f in functions {
        // Escape Swift reserved keywords; swift-bridge emits the bridge fn name
        // verbatim into Swift, so `fn subscript(...)` would become invalid Swift.
        let fn_name = swift_ident(&f.name.to_snake_case());
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let bridge_ty = bridge_type(&p.ty);
                let bridge_ty = if p.optional {
                    format!("Option<{bridge_ty}>")
                } else {
                    bridge_ty
                };
                let name = swift_ident(&p.name.to_snake_case());
                format!("{name}: {bridge_ty}")
            })
            .collect();
        let params_str = params.join(", ");

        let return_ty = if f.error_type.is_some() {
            // Result<ReturnType, String> for error-throwing functions
            let ok_ty = bridge_type(&f.return_type);
            if matches!(f.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            bridge_type(&f.return_type)
        };

        // swift-bridge 0.1.59 does not support the `#[swift_bridge(async)]`
        // attribute (the build script's parser rejects it). To bridge async
        // functions, we declare them as plain `fn` in the extern block — the
        // wrapper will block on the future at the bridge boundary.
        block.push_str(&format!(
            "        fn {fn_name}({params_str}) -> {return_ty};\n"
        ));
    }

    block.push_str("    }\n\n");
    block
}
