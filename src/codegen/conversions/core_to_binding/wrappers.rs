use crate::core::ir::{CoreWrapper, TypeRef};

/// Apply CoreWrapper transformations for core→binding direction.
/// Unwraps Arc, converts Cow→String, Bytes→Vec<u8>.
pub(super) fn apply_core_wrapper_from_core(
    conversion: &str,
    name: &str,
    ty: &TypeRef,
    core_wrapper: &CoreWrapper,
    vec_inner_core_wrapper: &CoreWrapper,
    optional: bool,
) -> String {
    if *vec_inner_core_wrapper == CoreWrapper::Arc {
        return conversion
            .replace(".map(Into::into).collect()", ".map(|v| (*v).clone().into()).collect()")
            .replace(
                "map(|v| v.into_iter().map(Into::into)",
                "map(|v| v.into_iter().map(|v| (*v).clone().into())",
            );
    }

    match core_wrapper {
        CoreWrapper::None => conversion.to_string(),
        CoreWrapper::Cow | CoreWrapper::Box => {
            let prefix = format!("{name}: ");
            let already_some_wrapped = conversion
                .strip_prefix(&prefix)
                .is_some_and(|expr| expr.starts_with("Some("));
            if optional {
                format!("{name}: val.{name}.as_ref().map(|v| v.to_string())")
            } else if already_some_wrapped {
                format!("{name}: Some(val.{name}.to_string())")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        CoreWrapper::Arc => {
            if conversion.contains("{ inner: Arc::new(") {
                return conversion.replace("{ inner: Arc::new(v) }", "{ inner: v }").replace(
                    &format!("{{ inner: Arc::new(val.{name}) }}"),
                    &format!("{{ inner: val.{name} }}"),
                );
            }
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    let simple_passthrough = format!("val.{name}");
                    if expr == simple_passthrough {
                        format!("{name}: val.{name}.map(|v| (*v).clone().into())")
                    } else {
                        format!("{name}: {expr}")
                    }
                } else {
                    let string_passthrough = format!("val.{name}.to_string()");
                    let unwrapped = if expr == string_passthrough {
                        if matches!(ty, TypeRef::Json) {
                            format!("(*val.{name}).clone().to_string()")
                        } else {
                            expr.to_string()
                        }
                    } else {
                        expr.replace(&format!("val.{name}"), &format!("(*val.{name}).clone()"))
                    };
                    format!("{name}: {unwrapped}")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Bytes => {
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                let already_converted_non_opt = expr == format!("val.{name}.to_vec().into()");
                let already_converted_opt = expr == format!("val.{name}.map(|v| v.to_vec().into())");
                if already_converted_non_opt || already_converted_opt {
                    conversion.to_string()
                } else if optional {
                    format!("{name}: {expr}.map(|v| v.to_vec().into())")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.to_vec().into()")
                } else {
                    conversion.to_string()
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::ArcMutex => {
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    let string_passthrough = format!("val.{name}.map(|v| v.to_string())");
                    if expr == string_passthrough {
                        format!("{name}: val.{name}.map(|v| v.lock().unwrap().clone().into())")
                    } else {
                        format!("{name}: {expr}.map(|v| v.lock().unwrap().clone().into())")
                    }
                } else if expr == format!("val.{name}") || expr == format!("val.{name}.to_string()") {
                    format!("{name}: val.{name}.lock().unwrap().clone().into()")
                } else {
                    conversion.to_string()
                }
            } else {
                conversion.to_string()
            }
        }
    }
}
