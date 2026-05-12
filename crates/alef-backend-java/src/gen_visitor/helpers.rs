//! Internal string-generation helpers for the Java visitor bridge.

use super::callbacks::CallbackSpec;

/// Generate camelCase stub variable name: stub + capitalize(java_method).
/// e.g. visitText -> stubVisitText
pub(super) fn stub_var_name(java_method: &str) -> String {
    let mut name = String::with_capacity(5 + java_method.len());
    name.push_str("stub");
    let mut chars = java_method.chars();
    if let Some(first) = chars.next() {
        for c in first.to_uppercase() {
            name.push(c);
        }
        name.push_str(chars.as_str());
    }
    name
}

pub(super) fn handle_method_name(java_method: &str) -> String {
    // camelCase: "handle" + capitalize first letter of java_method
    let mut name = String::with_capacity(7 + java_method.len());
    name.push_str("handle");
    let mut chars = java_method.chars();
    if let Some(first) = chars.next() {
        for c in first.to_uppercase() {
            name.push(c);
        }
        name.push_str(chars.as_str());
    }
    name
}

pub(super) fn iface_param_str(spec: &CallbackSpec) -> String {
    let mut params = vec!["final NodeContext context".to_string()];
    for ep in spec.extra {
        params.push(format!("final {} {}", ep.java_type, ep.java_name));
    }
    if spec.has_is_header {
        params.push("final boolean isHeader".to_string());
    }
    params.join(", ")
}

/// Build the `FunctionDescriptor` for one callback's upcall stub.
/// All callbacks: (ADDRESS ctx, ADDRESS userData, ..extra.., ADDRESS outCustom, ADDRESS outLen) -> JAVA_INT
/// Returns a multi-line string with 20-space continuation indent so no line exceeds 80 chars.
pub(super) fn callback_descriptor(spec: &CallbackSpec) -> String {
    let mut layouts = vec![
        "ValueLayout.ADDRESS".to_string(), // ctx
        "ValueLayout.ADDRESS".to_string(), // user_data
    ];
    for ep in spec.extra {
        for layout in ep.c_layouts {
            layouts.push((*layout).to_string());
        }
    }
    if spec.has_is_header {
        layouts.push("ValueLayout.JAVA_INT".to_string());
    }
    layouts.push("ValueLayout.ADDRESS".to_string()); // out_custom
    layouts.push("ValueLayout.ADDRESS".to_string()); // out_len
    let indent = "                    ";
    let args = layouts.join(&format!(",\n{indent}"));
    format!("FunctionDescriptor.of(\n{indent}ValueLayout.JAVA_INT,\n{indent}{args})")
}

/// Build the `MethodType` for `LOOKUP.bind(this, name, type)`.
/// Returns a multi-line string with 20-space continuation indent so no line exceeds 80 chars.
pub(super) fn callback_method_type(spec: &CallbackSpec) -> String {
    let mut types = vec![
        "MemorySegment.class".to_string(), // ctx
        "MemorySegment.class".to_string(), // user_data
    ];
    for ep in spec.extra {
        for layout in ep.c_layouts {
            types.push(layout_to_java_class(layout).to_string());
        }
    }
    if spec.has_is_header {
        types.push("int.class".to_string());
    }
    types.push("MemorySegment.class".to_string()); // out_custom
    types.push("MemorySegment.class".to_string()); // out_len
    let indent = "                    ";
    let args = types.join(&format!(",\n{indent}"));
    format!("MethodType.methodType(\n{indent}int.class,\n{indent}{args})")
}

pub(super) fn layout_to_java_class(layout: &str) -> &'static str {
    match layout {
        "ValueLayout.ADDRESS" => "MemorySegment.class",
        "ValueLayout.JAVA_INT" => "int.class",
        "ValueLayout.JAVA_LONG" => "long.class",
        _ => "long.class",
    }
}

/// Generate one `handle_*` instance method inside `VisitorBridge`.
pub(super) fn gen_handle_method(out: &mut String, spec: &CallbackSpec) {
    let mut params = vec![
        "final MemorySegment ctx".to_string(),
        "final MemorySegment userData".to_string(),
    ];
    for ep in spec.extra {
        for (c_idx, layout) in ep.c_layouts.iter().enumerate() {
            let java_ptype = match *layout {
                "ValueLayout.JAVA_INT" => "int",
                "ValueLayout.JAVA_LONG" => "long",
                _ => "MemorySegment",
            };
            params.push(format!("final {java_ptype} {}", raw_var_name(ep.java_name, c_idx)));
        }
    }
    if spec.has_is_header {
        params.push("final int isHeader".to_string());
    }
    params.push("final MemorySegment outCustom".to_string());
    params.push("final MemorySegment outLen".to_string());

    let method_name = handle_method_name(spec.java_method);
    let single_line = format!("    int {}({}) {{", method_name, params.join(", "));
    let single_line_fits = single_line.len() <= 80;

    let decode_lines: Vec<String> = spec
        .extra
        .iter()
        .map(|ep| {
            let mut decode = ep.decode.to_string();
            for (c_idx, _) in ep.c_layouts.iter().enumerate() {
                let placeholder = format!("raw_{}_{}", ep.java_name, c_idx);
                let var = raw_var_name(ep.java_name, c_idx);
                decode = decode.replace(&placeholder, &var);
            }
            format!("var {} = {};", ep.java_name, decode)
        })
        .collect();

    let mut call_args = vec!["context".to_string()];
    for ep in spec.extra {
        call_args.push(ep.java_name.to_string());
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    out.push_str(&crate::template_env::render(
        "handle_method.jinja",
        minijinja::context! {
            single_line_fits => single_line_fits,
            method_name => &method_name,
            params => &params,
            decode_lines => &decode_lines,
            has_is_header => spec.has_is_header,
            java_method => spec.java_method,
            call_args => &call_args,
        },
    ));
    out.push('\n');
}

pub(super) fn raw_var_name(java_name: &str, c_idx: usize) -> String {
    // camelCase: "raw" + capitalize first letter of java_name + "_" + index
    // e.g. raw_text_0 -> rawText0, raw_cells_1 -> rawCells1
    let mut name = String::with_capacity(4 + java_name.len() + 2);
    name.push_str("raw");
    let mut chars = java_name.chars();
    if let Some(first) = chars.next() {
        for c in first.to_uppercase() {
            name.push(c);
        }
        name.push_str(chars.as_str());
    }
    name.push_str(&c_idx.to_string());
    name
}

#[cfg(test)]
mod tests {
    use super::super::callbacks::CALLBACKS;
    use super::*;

    #[test]
    fn stub_var_name_capitalises_first_letter() {
        assert_eq!(stub_var_name("visitText"), "stubVisitText");
        assert_eq!(stub_var_name("visitElementStart"), "stubVisitElementStart");
    }

    #[test]
    fn handle_method_name_prefixes_handle() {
        assert_eq!(handle_method_name("visitText"), "handleVisitText");
        assert_eq!(handle_method_name("visitTableRow"), "handleVisitTableRow");
    }

    #[test]
    fn raw_var_name_camel_cases() {
        assert_eq!(raw_var_name("text", 0), "rawText0");
        assert_eq!(raw_var_name("cells", 1), "rawCells1");
    }

    #[test]
    fn layout_to_java_class_maps_correctly() {
        assert_eq!(layout_to_java_class("ValueLayout.ADDRESS"), "MemorySegment.class");
        assert_eq!(layout_to_java_class("ValueLayout.JAVA_INT"), "int.class");
        assert_eq!(layout_to_java_class("ValueLayout.JAVA_LONG"), "long.class");
        assert_eq!(layout_to_java_class("unknown"), "long.class");
    }

    #[test]
    fn callback_descriptor_includes_ctx_and_user_data() {
        let spec = &CALLBACKS[0]; // visit_text
        let descriptor = callback_descriptor(spec);
        assert!(
            descriptor.contains("FunctionDescriptor.of("),
            "must be FunctionDescriptor.of"
        );
        assert!(descriptor.contains("ValueLayout.JAVA_INT"), "must have int return type");
        // ctx + user_data always present
        assert!(descriptor.contains("ValueLayout.ADDRESS"), "must have ADDRESS layouts");
    }

    #[test]
    fn callback_method_type_includes_int_return() {
        let spec = &CALLBACKS[0];
        let method_type = callback_method_type(spec);
        assert!(
            method_type.contains("MethodType.methodType("),
            "must be MethodType.methodType"
        );
        assert!(method_type.contains("int.class"), "must have int return type");
    }

    #[test]
    fn iface_param_str_starts_with_node_context() {
        let spec = &CALLBACKS[0]; // visit_text has text param
        let params = iface_param_str(spec);
        assert!(
            params.starts_with("final NodeContext context"),
            "first param must be NodeContext"
        );
        assert!(params.contains("String text"), "must include text param");
    }
}
