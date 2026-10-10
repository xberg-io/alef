//! Getter, setter and clear-method generation for WASM struct fields.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::naming::to_node_name;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FieldDef, TypeRef};
use ahash::{AHashMap, AHashSet};

use super::types_helpers::{
    class_backed_field_type, class_backed_vec_element_type, complex_newtype_field_uses_jsvalue,
    is_bare_tagged_data_enum, is_copy_type, is_option_of_tagged_data_enum, is_vec_of_tagged_data_enum, optional_inner,
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

/// The per-struct name sets and mapper every accessor generator consults.
pub(super) struct AccessorEnv<'a> {
    pub(super) mapper: &'a WasmMapper,
    pub(super) enum_names: &'a AHashSet<String>,
    pub(super) tagged_data_enum_names: &'a AHashSet<String>,
    pub(super) untagged_ts_value_types: &'a AHashMap<String, String>,
    pub(super) class_type_names: &'a AHashSet<String>,
}

type Projection = (String, String);

fn js_name_attr(field: &FieldDef) -> String {
    let js_name = to_node_name(&field.name);
    if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    }
}

fn option_or_plain_jsvalue(field: &FieldDef) -> String {
    if stores_option(field) {
        "Option<JsValue>".to_string()
    } else {
        "JsValue".to_string()
    }
}

/// The type the stored field is exposed as when no enum/class special case applies.
fn mapped_field_type(field: &FieldDef, mapper: &WasmMapper) -> String {
    if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
        mapper.map_type(&field.ty)
    } else if field.optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    }
}

fn getter_field_type(field: &FieldDef, mapper: &WasmMapper, has_default: bool) -> String {
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    if complex_newtype_field_uses_jsvalue(field) {
        option_or_plain_jsvalue(field)
    } else if force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapped_field_type(field, mapper)
    }
}

fn untagged_ts_projection(field: &FieldDef, env: &AccessorEnv<'_>) -> Option<Projection> {
    let (is_optional, value_type) = untagged_ts_value_type(field, env.untagged_ts_value_types)?;
    Some(if is_optional {
        (
            format!("Option<{value_type}>"),
            format!("self.{}.clone().map(|v| v.unchecked_into())", field.name),
        )
    } else {
        (value_type, format!("self.{}.clone().unchecked_into()", field.name))
    })
}

fn vec_unit_enum_projection(
    field: &FieldDef,
    wrapper_enum_names: &AHashSet<String>,
    env: &AccessorEnv<'_>,
) -> Option<Projection> {
    if !is_vec_of_unit_enum(&field.ty, wrapper_enum_names, env.tagged_data_enum_names) {
        return None;
    }
    Some(if field.optional {
        (
            "Option<Vec<String>>".to_string(),
            format!(
                "self.{}.as_ref().map(|v| v.iter().map(|x| x.to_api_str().to_owned()).collect())",
                field.name
            ),
        )
    } else {
        (
            "Vec<String>".to_string(),
            format!(
                "self.{}.iter().map(|v| v.to_api_str().to_owned()).collect()",
                field.name
            ),
        )
    })
}

fn tagged_enum_projection(field: &FieldDef, env: &AccessorEnv<'_>) -> Option<Projection> {
    let tagged = env.tagged_data_enum_names;
    let clone_expr = format!("self.{}.clone()", field.name);
    if !field.optional && (is_vec_of_tagged_data_enum(&field.ty, tagged) || is_bare_tagged_data_enum(&field.ty, tagged))
    {
        return Some(("JsValue".to_string(), clone_expr));
    }
    if field.optional
        && (is_option_of_tagged_data_enum(&field.ty, tagged) || is_bare_tagged_data_enum(&field.ty, tagged))
    {
        return Some(("Option<JsValue>".to_string(), clone_expr));
    }
    None
}

fn wrapper_enum_projection(
    field: &FieldDef,
    wrapper_enum_names: &AHashSet<String>,
    env: &AccessorEnv<'_>,
) -> Option<Projection> {
    let is_wrapper_enum = |ty: &TypeRef| matches!(ty, TypeRef::Named(n) if wrapper_enum_names.contains(n) && !env.tagged_data_enum_names.contains(n));
    if field.optional {
        is_wrapper_enum(optional_inner(&field.ty)).then(|| {
            (
                "Option<String>".to_string(),
                format!("self.{}.map(|v| v.to_api_str().to_owned())", field.name),
            )
        })
    } else {
        is_wrapper_enum(&field.ty).then(|| {
            (
                "String".to_string(),
                format!("self.{}.to_api_str().to_owned()", field.name),
            )
        })
    }
}

fn optional_vec_of_struct_projection(field: &FieldDef, env: &AccessorEnv<'_>) -> Option<Projection> {
    let inner_ty = optional_inner(&field.ty);
    let is_optional_vec_of_struct = field.optional
        && matches!(
            inner_ty,
            TypeRef::Vec(elem) if matches!(elem.as_ref(), TypeRef::Named(n) if !env.enum_names.contains(n))
        )
        && !is_vec_of_tagged_data_enum(inner_ty, env.tagged_data_enum_names);
    is_optional_vec_of_struct.then(|| {
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
    })
}

fn default_projection(
    field: &FieldDef,
    field_type: String,
    wrapper_enum_names: &AHashSet<String>,
    env: &AccessorEnv<'_>,
) -> Projection {
    let copy_enum_names: AHashSet<String> = wrapper_enum_names
        .iter()
        .filter(|n| !env.tagged_data_enum_names.contains(*n))
        .cloned()
        .collect();
    let expr = if is_copy_type(&field.ty, &copy_enum_names) {
        format!("self.{}", field.name)
    } else {
        format!("self.{}.clone()", field.name)
    };
    (field_type, expr)
}

/// The `(return type, return expression)` pair of the getter for `field`.
fn getter_projection(field: &FieldDef, env: &AccessorEnv<'_>, field_type: String) -> Projection {
    if complex_newtype_field_uses_jsvalue(field) {
        return (field_type, format!("self.{}.clone()", field.name));
    }
    let wrapper_enum_names = wrapper_backed_enum_names(env.mapper, env.enum_names);
    untagged_ts_projection(field, env)
        .or_else(|| vec_unit_enum_projection(field, &wrapper_enum_names, env))
        .or_else(|| tagged_enum_projection(field, env))
        .or_else(|| wrapper_enum_projection(field, &wrapper_enum_names, env))
        .or_else(|| optional_vec_of_struct_projection(field, env))
        .unwrap_or_else(|| default_projection(field, field_type, &wrapper_enum_names, env))
}

fn getter_copy_doc(field: &FieldDef, env: &AccessorEnv<'_>) -> String {
    if class_backed_field_type(field, env.mapper, env.class_type_names).is_some() {
        format!(
            "/// Returns a detached copy of `{name}`.\n\
             ///\n\
             /// Changes to the returned value do not update this object. To update `{name}`,\n\
             /// read it, modify the copy, then assign the copy back to `{name}`.\n",
            name = to_node_name(&field.name),
        )
    } else if class_backed_vec_element_type(field, env.mapper, env.class_type_names).is_some() {
        format!(
            "/// Returns detached copies of the elements in `{name}`.\n\
             ///\n\
             /// Changes to the returned values do not update this object. Modify the copies,\n\
             /// then assign the array back to `{name}`.\n",
            name = to_node_name(&field.name),
        )
    } else {
        String::new()
    }
}

/// Generate a getter method for a field.
pub(super) fn gen_getter(field: &FieldDef, env: &AccessorEnv<'_>, has_default: bool) -> String {
    let field_type = getter_field_type(field, env.mapper, has_default);
    let (field_type, return_expr) = getter_projection(field, env, field_type);
    let js_name_attr = js_name_attr(field);
    let copy_doc = getter_copy_doc(field, env);

    format!(
        "{copy_doc}#[wasm_bindgen(getter{js_name_attr})]\npub fn {}(&self) -> {} {{\n    {}\n}}",
        field.name, field_type, return_expr
    )
}

fn ts_bridged_setter(field: &FieldDef, js_name_attr: &str, is_optional: bool, value_type: String) -> String {
    let (param_type, assign_expr) = if is_optional {
        (format!("Option<{value_type}>"), "value.map(Into::into)".to_string())
    } else {
        (value_type, "value.into()".to_string())
    };
    crate::backends::wasm::template_env::render(
        "ts_bridged_setter",
        crate::alef_context! {
            js_name_attr => js_name_attr,
            field_name => field.name,
            param_type => param_type,
            assign_expr => format!("self.{} = {assign_expr};", field.name),
        },
    )
    .trim_end()
    .to_string()
}

fn vec_unit_enum_setter(
    field: &FieldDef,
    env: &AccessorEnv<'_>,
    wrapper_enum_names: &AHashSet<String>,
    js_name_attr: &str,
) -> Option<String> {
    if !is_vec_of_unit_enum(&field.ty, wrapper_enum_names, env.tagged_data_enum_names) {
        return None;
    }
    let inner = vec_unit_enum_inner_name(
        &field.ty,
        wrapper_enum_names,
        env.tagged_data_enum_names,
        &env.mapper.prefix,
    )
    .expect("is_vec_of_unit_enum implied inner is a named unit enum");
    Some(if field.optional {
        format!(
            "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{name}(&mut self, value: Option<Vec<String>>) {{\n    \
             self.{name} = value.map(|v| v.into_iter().filter_map(|s| {inner}::from_api_str(&s)).collect());\n}}",
            name = field.name,
            inner = inner,
        )
    } else {
        format!(
            "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{name}(&mut self, value: Vec<String>) {{\n    \
             self.{name} = value.into_iter().filter_map(|s| {inner}::from_api_str(&s)).collect();\n}}",
            name = field.name,
            inner = inner,
        )
    })
}

fn class_field_setter(field: &FieldDef, env: &AccessorEnv<'_>, js_name_attr: &str) -> Option<String> {
    let class_type = class_backed_field_type(field, env.mapper, env.class_type_names)?;
    Some(
        crate::backends::wasm::template_env::render(
            "gen_class_field_setter",
            crate::alef_context! {
                js_name_attr => js_name_attr,
                setter_ident => format!("set_{}", field.name),
                field_ident => field.name,
                class_type => class_type,
                optional => stores_option(field),
            },
        )
        .trim_end()
        .to_string(),
    )
}

fn setter_field_type(field: &FieldDef, env: &AccessorEnv<'_>, has_default: bool) -> String {
    let tagged = env.tagged_data_enum_names;
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged);
    let is_option_tagged_enum = !is_vec_tagged_enum
        && (is_option_of_tagged_data_enum(&field.ty, tagged)
            || (field.optional && is_bare_tagged_data_enum(&field.ty, tagged)));
    let is_bare_tagged_enum =
        !is_vec_tagged_enum && !is_option_tagged_enum && is_bare_tagged_data_enum(&field.ty, tagged);

    if complex_newtype_field_uses_jsvalue(field) {
        option_or_plain_jsvalue(field)
    } else if force_optional {
        env.mapper.optional(&env.mapper.map_type(&field.ty))
    } else if is_vec_tagged_enum || is_bare_tagged_enum {
        "JsValue".to_string()
    } else if is_option_tagged_enum {
        "Option<JsValue>".to_string()
    } else {
        mapped_field_type(field, env.mapper)
    }
}

/// Generate a setter method for a field.
pub(super) fn gen_setter(field: &FieldDef, env: &AccessorEnv<'_>, has_default: bool) -> String {
    let wrapper_enum_names = wrapper_backed_enum_names(env.mapper, env.enum_names);
    let js_name_attr = js_name_attr(field);

    if let Some((is_optional, value_type)) = untagged_ts_value_type(field, env.untagged_ts_value_types) {
        return ts_bridged_setter(field, &js_name_attr, is_optional, value_type);
    }
    if let Some(setter) = vec_unit_enum_setter(field, env, &wrapper_enum_names, &js_name_attr) {
        return setter;
    }

    // wasm-bindgen's JS shim for a by-value exported struct argument calls
    // `__destroy_into_raw()` on it, so `opts.field = handle` would leave the caller holding a
    // dead handle ("null pointer passed to rust" on its next use). A borrow is passed as a plain
    // `__wbg_ptr` and leaves the handle alive. `Option<&T>` has no `OptionFromWasmAbi` impl, so
    // an optional field takes the same `&T` and wraps it in `Some` — clearing it is not
    // expressible through this accessor. ~keep
    if let Some(setter) = class_field_setter(field, env, &js_name_attr) {
        return setter;
    }

    let field_type = setter_field_type(field, env, has_default);
    let ownership_doc = if class_backed_vec_element_type(field, env.mapper, env.class_type_names).is_some() {
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
            crate::alef_context! {
                js_name => to_node_name(&rust_ident),
                clear_ident => rust_ident,
                field_ident => field.name,
            },
        )
        .trim_end()
        .to_string(),
    )
}
