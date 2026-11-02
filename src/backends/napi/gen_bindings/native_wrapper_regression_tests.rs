//! Compile complete canonical Node crates so ordinary wrapper bodies cannot escape the regression gate.

mod fixture;

use fixture::{Expected, NativeFixture};

#[test]
#[ignore = "requires installed pinned Rust and cached offline NAPI dependencies"]
fn async_unit_free_static_and_instance_wrappers_compile() {
    let fixture = NativeFixture::emit(
        include_str!("native_wrapper_regression_tests/async_unit_core.rs"),
        Expected {
            types: 1,
            enums: 1,
            functions: 3,
            methods: 3,
        },
    );
    fixture.assert_async_unit("consume_unit", false);
    fixture.assert_async_unit("consume_unit", true);
    fixture.assert_async_unit("consume_static", true);
    fixture.assert_wrapper("json_control", false);
    fixture.check();
}

#[test]
#[ignore = "requires installed pinned Rust and cached offline NAPI dependencies"]
fn json_values_free_static_and_instance_wrappers_compile() {
    let fixture = NativeFixture::emit(
        include_str!("native_wrapper_regression_tests/json_core.rs"),
        Expected {
            types: 2,
            enums: 0,
            functions: 3,
            methods: 4,
        },
    );
    fixture.assert_wrapper("json_control", false);
    fixture.assert_wrapper("echo_json", true);
    fixture.assert_wrapper("echo_json_async", true);
    fixture.assert_wrapper("echo_json_static", true);
    fixture.assert_serde_dto("JsPacket");
    fixture.check();
}

#[test]
#[ignore = "requires installed pinned Rust and cached offline NAPI dependencies"]
fn borrowed_and_optional_json_wrappers_compile() {
    let fixture = NativeFixture::emit(
        include_str!("native_wrapper_regression_tests/borrowed_json_core.rs"),
        Expected {
            types: 1,
            enums: 0,
            functions: 3,
            methods: 5,
        },
    );
    fixture.assert_wrapper("json_borrowed", false);
    fixture.assert_wrapper("optional_json", false);
    fixture.assert_wrapper("json_borrowed", true);
    fixture.assert_wrapper("optional_json", true);
    fixture.assert_wrapper("borrowed_optional_json", true);
    fixture.assert_wrapper("optional_json_static", true);
    fixture.check();
}
