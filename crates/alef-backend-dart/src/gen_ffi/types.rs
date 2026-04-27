use alef_core::ir::{EnumDef, TypeDef};
use heck::ToLowerCamelCase;

use super::type_map::dart_type;

/// Emit a Dart class for a struct-style type.
///
/// For types with no fields (opaque handles), the class wraps an opaque
/// `Pointer<Void>`. For value types with fields, fields are Dart-typed.
pub(super) fn emit_type(ty: &TypeDef, out: &mut String) {
    if !ty.doc.is_empty() {
        for line in ty.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }

    if ty.fields.is_empty() || ty.is_opaque {
        // Opaque handle: wrap a raw pointer.
        out.push_str(&format!("class {} {{\n", ty.name));
        out.push_str("  final Pointer<Void> _ptr;\n");
        out.push_str(&format!("  {}(this._ptr);\n", ty.name));
        out.push_str("}\n");
        return;
    }

    out.push_str(&format!("class {} {{\n", ty.name));
    for field in &ty.fields {
        let ty_str = dart_type(&field.ty, field.optional);
        let name = field.name.to_lower_camel_case();
        if !field.doc.is_empty() {
            for line in field.doc.lines() {
                out.push_str("  /// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str(&format!("  final {ty_str} {name};\n"));
    }
    if ty.fields.len() == 1 {
        let name = ty.fields[0].name.to_lower_camel_case();
        out.push_str(&format!("  {}(this.{name});\n", ty.name));
    } else {
        out.push_str(&format!("  {}({{\n", ty.name));
        for field in &ty.fields {
            let name = field.name.to_lower_camel_case();
            out.push_str(&format!("    required this.{name},\n"));
        }
        out.push_str("  });\n");
    }
    out.push_str("}\n");
}

/// Emit a Dart enum (unit variants only in FFI mode).
///
/// Data variants (tagged unions) cannot be expressed ergonomically via
/// `dart:ffi` since C has no stable tagged-union ABI. Non-unit variants
/// emit a `// TODO` comment and are skipped.
pub(super) fn emit_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        for line in en.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }

    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&format!("enum {} {{\n", en.name));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            if !variant.doc.is_empty() {
                for line in variant.doc.lines() {
                    out.push_str("  /// ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
            let vname = variant.name.to_lower_camel_case();
            let suffix = if idx + 1 == count { ";" } else { "," };
            out.push_str(&format!("  {vname}{suffix}\n"));
        }
        out.push_str("}\n");
    } else {
        // TODO: data variants are deferred for FFI mode. dart:ffi cannot represent
        // tagged unions ergonomically. Emit a unit placeholder for now.
        out.push_str(&format!(
            "// TODO: {} has data variants; dart:ffi tagged-union support is deferred.\n",
            en.name
        ));
        out.push_str(&format!("enum {} {{\n", en.name));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            let vname = variant.name.to_lower_camel_case();
            let suffix = if idx + 1 == count { ";" } else { "," };
            out.push_str(&format!("  {vname}{suffix}\n"));
        }
        out.push_str("}\n");
    }
}
