//! Getter, setter and clear-method generation for WASM struct fields.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::naming::to_node_name;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FieldDef, TypeRef};
use ahash::{AHashMap, AHashSet};

use super::types_helpers::{
    class_backed_field_type, class_backed_vec_element_type, is_bare_tagged_data_enum, is_copy_type,
    is_option_of_tagged_data_enum, is_vec_of_tagged_data_enum, optional_inner,
};
use super::types_unit_enum::{is_vec_of_unit_enum, vec_unit_enum_inner_name};

/// The subset of `enum_names` the mapper still renders as this backend's generated wrapper.
///
/// Every enum-shaped accessor below emits something only the generated `{prefix}{Enum}` has —
/// `to_api_str`, `from_api_str`, or the `Copy` its derive provides. A `type_overrides` entry
/// replaces that wrapper with an arbitrary binding type (`JsValue`, `String`, anything the
/// consumer configured), for which none of those exist, while `gen_struct` types the field from
/// the same mapper — so an unfiltered name list makes the accessor and the field it reads
/// disagree. The mapper is the only authority on which type a name resolves to; ask it rather
/// than assuming a `TypeRef::Named` that is an enum is stored as the wrapper. ~keep
fn wrapper_backed_enum_names(mapper: &WasmMapper, enum_names: &AHashSet<String>) -> AHashSet<String> {
    enum_names
        .iter()
        .filter(|name| {
            let name = name.as_str();
            mapper.named(name).as_ref() == format!("{}{name}", mapper.prefix).as_str()
        })
        .cloned()
        .collect()
}

/// If `field` is a bare or `Option`-wrapped reference to an untagged data enum that has a
/// registered structural TS type (see `ts_union.rs`), returns `(is_optional, value_type_name)`
/// — the wasm-bindgen extern type the getter/setter should expose instead of bare `JsValue`.
/// `Vec`/`Map`-nested references aren't covered: those still collapse to a bare `JsValue` field
/// exactly as before, since `Vec<extern-type>` across the wasm-bindgen ABI wasn't verified. ~keep
fn untagged_ts_value_type(
    field: &FieldDef,
    untagged_ts_value_types: &AHashMap<String, String>,
) -> Option<(bool, String)> {
    if field.optional {
        match optional_inner(&field.ty) {
            TypeRef::Named(n) => untagged_ts_value_types.get(n).map(|v| (true, v.clone())),
            _ => None,
        }
    } else {
        match &field.ty {
            TypeRef::Named(n) => untagged_ts_value_types.get(n).map(|v| (false, v.clone())),
            _ => None,
        }
    }
}

/// Whether the generated struct stores this field as an `Option`.
///
/// Exactly the predicate `gen_struct` uses to type the stored field: an `Optional` TypeRef
/// stores an `Option` even when the IR left the `optional` flag false, and both the setter's
/// assignment and the clear method's existence have to agree with it or the generated
/// assignment is an `E0308`. ~keep
fn stores_option(field: &FieldDef) -> bool {
    field.optional || matches!(field.ty, TypeRef::Optional(_))
}

/// Generate a getter method for a field.
pub(super) fn gen_getter(
    field: &FieldDef,
    mapper: &WasmMapper,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
    has_default: bool,
    untagged_ts_value_types: &AHashMap<String, String>,
    class_type_names: &AHashSet<String>,
) -> String {
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let field_type = if force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
        mapper.map_type(&field.ty)
    } else if field.optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let js_name = to_node_name(&field.name);
    let js_name_attr = if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    };

    let wrapper_enum_names = wrapper_backed_enum_names(mapper, enum_names);
    let inner_ty = optional_inner(&field.ty);
    let is_optional_enum = field.optional
        && matches!(inner_ty, TypeRef::Named(n) if wrapper_enum_names.contains(n) && !tagged_data_enum_names.contains(n));
    let is_required_enum = !field.optional
        && matches!(field.ty, TypeRef::Named(ref n) if wrapper_enum_names.contains(n) && !tagged_data_enum_names.contains(n));
    let is_required_vec_tagged_enum = !field.optional && is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_required_bare_tagged_enum = !field.optional && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_optional_tagged_enum = field.optional
        && (is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names)
            || is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names));
    let is_vec_unit_enum =
        !field.optional && is_vec_of_unit_enum(&field.ty, &wrapper_enum_names, tagged_data_enum_names);
    let is_optional_vec_unit_enum =
        field.optional && is_vec_of_unit_enum(&field.ty, &wrapper_enum_names, tagged_data_enum_names);
    let is_optional_vec_of_struct = field.optional
        && matches!(
            inner_ty,
            TypeRef::Vec(elem) if matches!(elem.as_ref(), TypeRef::Named(n) if !enum_names.contains(n))
        )
        && !is_vec_of_tagged_data_enum(inner_ty, tagged_data_enum_names);
    let untagged_ts = untagged_ts_value_type(field, untagged_ts_value_types);

    let (field_type, return_expr) = if let Some((true, value_type)) = &untagged_ts {
        let expr = format!("self.{}.clone().map(|v| v.unchecked_into())", field.name);
        (format!("Option<{value_type}>"), expr)
    } else if let Some((false, value_type)) = &untagged_ts {
        let expr = format!("self.{}.clone().unchecked_into()", field.name);
        (value_type.clone(), expr)
    } else if is_vec_unit_enum {
        let expr = format!(
            "self.{}.iter().map(|v| v.to_api_str().to_owned()).collect()",
            field.name
        );
        ("Vec<String>".to_string(), expr)
    } else if is_optional_vec_unit_enum {
        let expr = format!(
            "self.{}.as_ref().map(|v| v.iter().map(|x| x.to_api_str().to_owned()).collect())",
            field.name
        );
        ("Option<Vec<String>>".to_string(), expr)
    } else if is_required_vec_tagged_enum || is_required_bare_tagged_enum {
        ("JsValue".to_string(), format!("self.{}.clone()", field.name))
    } else if is_optional_tagged_enum {
        ("Option<JsValue>".to_string(), format!("self.{}.clone()", field.name))
    } else if is_optional_enum {
        let expr = format!("self.{}.map(|v| v.to_api_str().to_owned())", field.name);
        ("Option<String>".to_string(), expr)
    } else if is_required_enum {
        let expr = format!("self.{}.to_api_str().to_owned()", field.name);
        ("String".to_string(), expr)
    } else if is_optional_vec_of_struct {
        let expr = format!(
            "self.{f}.as_ref().map(|items| {{\n        \
             let arr = js_sys::Array::new();\n        \
             for item in items {{\n            \
             arr.push(&JsValue::from(item.clone()));\n        \
             }}\n        \
             arr\n    }})",
            f = field.name
        );
        ("Option<js_sys::Array>".to_string(), expr)
    } else {
        let copy_enum_names: AHashSet<String> = wrapper_enum_names
            .iter()
            .filter(|n| !tagged_data_enum_names.contains(*n))
            .cloned()
            .collect();
        let expr = if is_copy_type(&field.ty, &copy_enum_names) {
            format!("self.{}", field.name)
        } else {
            format!("self.{}.clone()", field.name)
        };
        (field_type, expr)
    };

    let copy_doc = if class_backed_field_type(field, mapper, class_type_names).is_some() {
        format!(
            "/// Returns a detached copy of `{name}`.\n\
             ///\n\
             /// Changes to the returned value do not update this object. To update `{name}`,\n\
             /// read it, modify the copy, then assign the copy back to `{name}`.\n",
            name = to_node_name(&field.name),
        )
    } else if class_backed_vec_element_type(field, mapper, class_type_names).is_some() {
        format!(
            "/// Returns detached copies of the elements in `{name}`.\n\
             ///\n\
             /// Changes to the returned values do not update this object. Modify the copies,\n\
             /// then assign the array back to `{name}`.\n",
            name = to_node_name(&field.name),
        )
    } else {
        String::new()
    };

    format!(
        "{copy_doc}#[wasm_bindgen(getter{js_name_attr})]\npub fn {}(&self) -> {} {{\n    {}\n}}",
        field.name, field_type, return_expr
    )
}

/// Generate a setter method for a field.
pub(super) fn gen_setter(
    field: &FieldDef,
    mapper: &WasmMapper,
    enum_names: &AHashSet<String>,
    has_default: bool,
    tagged_data_enum_names: &AHashSet<String>,
    untagged_ts_value_types: &AHashMap<String, String>,
    class_type_names: &AHashSet<String>,
) -> String {
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_option_tagged_enum = !is_vec_tagged_enum
        && (is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names)
            || (field.optional && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names)));
    let is_bare_tagged_enum =
        !is_vec_tagged_enum && !is_option_tagged_enum && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let wrapper_enum_names = wrapper_backed_enum_names(mapper, enum_names);
    let is_vec_unit_enum =
        !field.optional && is_vec_of_unit_enum(&field.ty, &wrapper_enum_names, tagged_data_enum_names);
    let is_optional_vec_unit_enum =
        field.optional && is_vec_of_unit_enum(&field.ty, &wrapper_enum_names, tagged_data_enum_names);

    let js_name = to_node_name(&field.name);
    let js_name_attr = if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    };

    if let Some((is_optional, value_type)) = untagged_ts_value_type(field, untagged_ts_value_types) {
        let (param_type, assign_expr) = if is_optional {
            (format!("Option<{value_type}>"), "value.map(Into::into)".to_string())
        } else {
            (value_type, "value.into()".to_string())
        };
        return crate::backends::wasm::template_env::render(
            "ts_bridged_setter",
            minijinja::context! {
                js_name_attr => js_name_attr,
                field_name => field.name,
                param_type => param_type,
                assign_expr => format!("self.{} = {assign_expr};", field.name),
            },
        )
        .trim_end()
        .to_string();
    }

    if is_vec_unit_enum {
        let inner = vec_unit_enum_inner_name(&field.ty, &wrapper_enum_names, tagged_data_enum_names, &mapper.prefix)
            .expect("is_vec_of_unit_enum implied inner is a named unit enum");
        return format!(
            "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{name}(&mut self, value: Vec<String>) {{\n    \
             self.{name} = value.into_iter().filter_map(|s| {inner}::from_api_str(&s)).collect();\n}}",
            name = field.name,
            inner = inner,
        );
    }

    if is_optional_vec_unit_enum {
        let inner = vec_unit_enum_inner_name(&field.ty, &wrapper_enum_names, tagged_data_enum_names, &mapper.prefix)
            .expect("is_vec_of_unit_enum implied inner is a named unit enum");
        return format!(
            "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{name}(&mut self, value: Option<Vec<String>>) {{\n    \
             self.{name} = value.map(|v| v.into_iter().filter_map(|s| {inner}::from_api_str(&s)).collect());\n}}",
            name = field.name,
            inner = inner,
        );
    }

    // wasm-bindgen's JS shim for a by-value exported struct argument calls
    // `__destroy_into_raw()` on it, so `opts.field = handle` would leave the caller holding a
    // dead handle ("null pointer passed to rust" on its next use). A borrow is passed as a plain
    // `__wbg_ptr` and leaves the handle alive. `Option<&T>` has no `OptionFromWasmAbi` impl, so
    // an optional field takes the same `&T` and wraps it in `Some` — clearing it is not
    // expressible through this accessor. ~keep
    if let Some(class_type) = class_backed_field_type(field, mapper, class_type_names) {
        return crate::backends::wasm::template_env::render(
            "gen_class_field_setter",
            minijinja::context! {
                js_name_attr => js_name_attr,
                setter_ident => format!("set_{}", field.name),
                field_ident => field.name,
                class_type => class_type,
                optional => stores_option(field),
            },
        )
        .trim_end()
        .to_string();
    }

    let field_type = if force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else if is_vec_tagged_enum || is_bare_tagged_enum {
        "JsValue".to_string()
    } else if is_option_tagged_enum {
        "Option<JsValue>".to_string()
    } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
        mapper.map_type(&field.ty)
    } else if field.optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let ownership_doc = if class_backed_vec_element_type(field, mapper, class_type_names).is_some() {
        format!(
            "/// Takes ownership of every element in `{}`.\n\
             /// wasm-bindgen cannot borrow class values nested in an array; copy each retained\n\
             /// element with `copyForTransfer()` before passing the array.\n",
            to_node_name(&field.name)
        )
    } else {
        String::new()
    };
    format!(
        "{ownership_doc}#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{}(&mut self, value: {}) {{\n    self.{} = value;\n}}",
        field.name, field_type, field.name
    )
}

/// Generate the `clear{Field}()` companion for an optional class-typed field.
///
/// The borrowed setter (see `gen_setter`) cannot accept `null`, because `Option<&T>` has no
/// `OptionFromWasmAbi` impl, so without this method an optional class-typed field could be set
/// but never unset. Returns `None` for any field that is not stored as an `Option` of a
/// generated class.
///
/// Also returns `None` when `reserved_idents` already holds the method name: the struct's own
/// inherent method (or a field whose getter mints that identifier) would collide with this one
/// as `E0592`, and alef's convenience accessor is the one that has to give way -- silently
/// shadowing a consumer's real API would be worse than leaving the field non-clearable. ~keep
pub(super) fn gen_clear_method(
    field: &FieldDef,
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
    reserved_idents: &AHashSet<String>,
) -> Option<String> {
    if !stores_option(field) {
        return None;
    }
    class_backed_field_type(field, mapper, class_type_names)?;
    let rust_ident = format!("clear_{}", field.name);
    if reserved_idents.contains(&rust_ident) {
        return None;
    }
    Some(
        crate::backends::wasm::template_env::render(
            "gen_class_field_clear",
            minijinja::context! {
                js_name => to_node_name(&rust_ident),
                clear_ident => rust_ident,
                field_ident => field.name,
            },
        )
        .trim_end()
        .to_string(),
    )
}
