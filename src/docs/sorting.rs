pub(crate) fn type_sort_key(name: &str) -> (u8, &str) {
    match name {
        "ParseOptions" => (0, name),
        "ParseOutput" => (1, name),
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
        assert!(is_update_type("ParseOptionsUpdate"));
        assert!(!is_update_type("ParseOptions"));
    }

    #[test]
    fn test_type_sort_key_ordering() {
        assert!(type_sort_key("ParseOptions") < type_sort_key("ParseOutput"));
        assert!(type_sort_key("ParseOutput") < type_sort_key("SomeOtherType"));
    }
}
