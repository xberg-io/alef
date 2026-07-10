use crate::core::ir::CoreWrapper;

/// Apply CoreWrapper transformations to a binding→core conversion expression.
/// Wraps the value expression with Arc::new(), .into() for Cow, etc.
pub fn apply_core_wrapper_to_core(
    conversion: &str,
    name: &str,
    core_wrapper: &CoreWrapper,
    vec_inner_core_wrapper: &CoreWrapper,
    optional: bool,
) -> String {
    if *vec_inner_core_wrapper == CoreWrapper::Arc {
        return conversion
            .replace(
                ".map(Into::into).collect()",
                ".map(|v| std::sync::Arc::new(v.into())).collect()",
            )
            .replace(
                "map(|v| v.into_iter().map(Into::into)",
                "map(|v| v.into_iter().map(|v| std::sync::Arc::new(v.into()))",
            );
    }

    match core_wrapper {
        CoreWrapper::None => conversion.to_string(),
        CoreWrapper::Cow | CoreWrapper::Box => {
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(Into::into)")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.into()")
                } else if expr == "Default::default()" {
                    conversion.to_string()
                } else {
                    format!("{name}: ({expr}).into()")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Arc => {
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if expr == "Default::default()" {
                    conversion.to_string()
                } else if optional {
                    format!("{name}: {expr}.map(|v| std::sync::Arc::new(v))")
                } else {
                    format!("{name}: std::sync::Arc::new({expr})")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Bytes => {
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                let already_converted_non_opt =
                    expr == format!("val.{name}.into()") || expr == format!("val.{name}.to_vec().into()");
                let already_converted_opt = expr
                    .strip_prefix(&format!("val.{name}"))
                    .map(|s| s == ".map(Into::into)" || s == ".map(|v| v.to_vec().into())")
                    .unwrap_or(false);
                if already_converted_non_opt || already_converted_opt {
                    conversion.to_string()
                } else if optional {
                    format!("{name}: {expr}.map(Into::into)")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.into()")
                } else if expr == "Default::default()" {
                    conversion.to_string()
                } else {
                    format!("{name}: ({expr}).into()")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::ArcMutex => {
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(|v| std::sync::Arc::new(std::sync::Mutex::new(v.into())))")
                } else if expr == format!("val.{name}") {
                    format!("{name}: std::sync::Arc::new(std::sync::Mutex::new(val.{name}.into()))")
                } else {
                    format!("{name}: std::sync::Arc::new(std::sync::Mutex::new(({expr}).into()))")
                }
            } else {
                conversion.to_string()
            }
        }
    }
}
