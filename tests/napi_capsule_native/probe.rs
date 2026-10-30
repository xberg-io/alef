#[napi]
pub fn matches_tag(env: &napi::Env, value: napi::Unknown<'_>, expected: bool) -> napi::Result<bool> {
    use napi::JsValue;
    let tag = napi::sys::napi_type_tag {
        lower: if expected { 1 } else { 3 },
        upper: 2,
    };
    let mut matches = false;
    let status = unsafe { napi::sys::napi_check_object_type_tag(env.raw(), value.raw(), &tag, &mut matches) };
    if status != napi::sys::Status::napi_ok {
        return Err(napi::Error::new(
            napi::Status::GenericFailure,
            format!("type tag check: {status}"),
        ));
    }
    Ok(matches)
}
