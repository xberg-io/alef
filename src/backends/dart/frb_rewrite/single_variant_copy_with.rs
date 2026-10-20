use regex::Regex;

// ~keep Freezed 4 chooses FRB's private constructor for a one-variant union's base
// copyWith. Reuse its concrete implementation so casts and nullable sentinels stay exact.
pub fn repair_single_variant_copy_with(source: &str) -> String {
    let broken = Regex::new(r"return _then\(([A-Za-z_][A-Za-z0-9_]*)\._\(\s*\)\);")
        .expect("private constructor pattern must compile");
    broken
        .replace_all(source, |captures: &regex::Captures<'_>| {
            let name = regex::escape(&captures[1]);
            let concrete = Regex::new(&format!(r"(?s)return _then\({name}_[A-Za-z0-9_]+\(\n.*?\n[ \t]*\)\);"))
                .expect("concrete constructor pattern must compile");
            let candidates = concrete.find_iter(source).collect::<Vec<_>>();
            if candidates.len() == 1 {
                candidates[0].as_str().to_string()
            } else {
                captures[0].to_string()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::repair_single_variant_copy_with;

    #[test]
    fn should_repair_freezed_single_variant_copy_with_without_losing_nullable_fields() {
        let source = include_str!("../../../../tests/fixtures/dart_single_error.freezed.dart");
        assert_eq!(source.matches("return _then(RequestError._(").count(), 1);
        let repaired = repair_single_variant_copy_with(source);
        assert!(!repaired.contains("return _then(RequestError._("));
        assert_eq!(repaired.matches("return _then(RequestError_Rejected(").count(), 2);
        assert_eq!(
            repaired
                .matches("retryAfter: freezed == retryAfter ? _self.retryAfter : retryAfter")
                .count(),
            2
        );
        assert_eq!(repair_single_variant_copy_with(&repaired), repaired);
    }

    #[test]
    fn should_leave_ambiguous_or_unmatched_constructors_unchanged() {
        let source = "return _then(Sample._(\n\n  ));\nreturn _then(Sample_First(\nvalue: _self.value,\n  ));\nreturn _then(Sample_Second(\nvalue: _self.value,\n  ));";
        assert_eq!(repair_single_variant_copy_with(source), source);
        assert_eq!(repair_single_variant_copy_with("plain source"), "plain source");
    }
}
