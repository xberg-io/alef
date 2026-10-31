/// FRB's unsigned scalar writer otherwise truncates BigInt with toSigned(64).
/// Guard the shared writer boundary as well as the facade, including raw calls and callbacks.
pub(super) fn check_unsigned_before_encoding(source: &str) -> String {
    let rewritten = source.replace(
        "serializer.buffer.putBigUint64(self);",
        "serializer.buffer.putBigUint64(_alefCheckU64BeforeEncode(self));",
    );
    if rewritten == source || source.contains("BigInt _alefCheckU64BeforeEncode(BigInt value)") {
        return rewritten;
    }
    let helper = crate::backends::dart::template_env::render("dart_u64_codec_guard.jinja", crate::alef_context! {});
    format!("{rewritten}\n{helper}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_unsigned_encoder_guards_are_complete_and_idempotent() {
        let unsigned = "void sse_encode_usize(BigInt self, SseSerializer serializer) {\n    serializer.buffer.putBigUint64(self);\n}\n";
        let signed =
            "void sse_encode_i_64(PlatformInt64 self, SseSerializer serializer) { serializer.buffer.putInt64(self); }";
        let source = format!("{unsigned}{unsigned}{signed}");
        let guarded = check_unsigned_before_encoding(&source);
        assert_eq!(
            guarded.matches("putBigUint64(_alefCheckU64BeforeEncode(self))").count(),
            2
        );
        assert!(guarded.contains("value.isNegative || value.bitLength > 64"));
        assert!(guarded.contains(signed));
        assert_eq!(check_unsigned_before_encoding(&guarded), guarded);
        assert_eq!(check_unsigned_before_encoding(signed), signed);
    }
}
