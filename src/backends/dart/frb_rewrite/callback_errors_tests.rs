use super::*;
use crate::backends::dart::frb_rewrite::unsigned::check_unsigned_before_encoding;

const CALLBACK: &str = include_str!("fixtures/callback_u64.frb.txt");
const WRITER: &str =
    "void sse_encode_u_64(BigInt self, SseSerializer serializer) { serializer.buffer.putBigUint64(self); }";

#[test]
fn callback_return_serialization_uses_existing_error_transport_and_releases_failed_buffer() {
    let rewritten = check_unsigned_before_encoding(&format!("{CALLBACK}\n{WRITER}"));
    let encode = rewritten.find("sse_encode_u_64(rawOutput.value, serializer);").unwrap();
    let catch = rewritten.find("} catch (e, s)").unwrap();
    let release = rewritten.find("rustVecU8Free(failed.ptr, failed.rustVecLen)").unwrap();
    let error = rewritten
        .find("sse_encode_AnyhowException(rawError.value, serializer);")
        .unwrap();
    let deliver = rewritten.find("dartFnDeliverOutput(").unwrap();
    assert!(encode < catch && catch < release && release < error && error < deliver);
    assert!(rewritten.contains("rawOutput = null;"));
    assert!(rewritten.contains("if (!const bool.fromEnvironment('dart.library.js_interop'))"));
    assert_eq!(rewritten.matches("dartFnDeliverOutput(").count(), 1);
    assert_eq!(
        rewritten
            .matches("BigInt _alefCheckU64BeforeEncode(BigInt value)")
            .count(),
        1
    );
    validate_callback_serialization(&rewritten).unwrap();
    assert_eq!(check_unsigned_before_encoding(&rewritten), rewritten);
}

#[test]
fn callback_container_return_uses_the_actual_recursive_encoder_inside_the_catch() {
    let list = CALLBACK
        .replace("Output_u_64_", "Output_list_u_64_")
        .replace("FutureOr<BigInt> Function", "FutureOr<List<BigInt>> Function")
        .replace("Box<BigInt>? rawOutput", "Box<List<BigInt>>? rawOutput")
        .replace(
            "sse_encode_u_64(rawOutput.value",
            "sse_encode_list_u_64(rawOutput.value",
        );
    let rewritten = check_unsigned_before_encoding(&format!("{list}\n{CALLBACK}\n{WRITER}"));
    assert_eq!(rewritten.matches("rawOutput = null;").count(), 2);
    let encode = rewritten
        .find("sse_encode_list_u_64(rawOutput.value, serializer);")
        .unwrap();
    assert!(encode < rewritten.find("} catch (e, s)").unwrap());
    assert!(rewritten.contains("Box<List<BigInt>>? rawOutput"));
    assert!(rewritten.contains("FutureOr<List<BigInt>> Function(BigInt) raw"));
    validate_callback_serialization(&rewritten).unwrap();
    assert_eq!(check_unsigned_before_encoding(&rewritten), rewritten);
}

#[test]
fn unsupported_or_partially_recognized_callback_shapes_fail_before_generation_is_accepted() {
    let unsupported = CALLBACK.replace("SseSerializer(generalizedFrbRustBinding)", "unknownSerializer(binding)");
    let rewritten = check_unsigned_before_encoding(&format!("{unsupported}\n{CALLBACK}\n{WRITER}"));
    let error = validate_callback_serialization(&rewritten).unwrap_err();
    assert!(error.to_string().contains("2 callbacks but 1 guarded error paths"));
    assert!(rewritten.contains("unknownSerializer(binding)"));
    assert_eq!(catch_callback_serialization(&unsupported), unsupported);
}

#[test]
fn files_without_guarded_unsigned_writers_preserve_existing_callback_source() {
    assert_eq!(check_unsigned_before_encoding(CALLBACK), CALLBACK);
    validate_callback_serialization(CALLBACK).unwrap();
}
