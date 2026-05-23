pub(crate) fn type_sort_key(name: &str) -> (u8, &str) {
    match name {
        "ConversionOptions" => (0, name),
        "ConversionResult" => (1, name),
        _ => (2, name),
    }
}

pub(crate) fn is_update_type(name: &str) -> bool {
    name.ends_with("Update")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_update_type() {
        assert!(is_update_type("ConversionOptionsUpdate"));
        assert!(!is_update_type("ConversionOptions"));
    }

    #[test]
    fn test_type_sort_key_ordering() {
        assert!(type_sort_key("ConversionOptions") < type_sort_key("ConversionResult"));
        assert!(type_sort_key("ConversionResult") < type_sort_key("SomeOtherType"));
    }
}
