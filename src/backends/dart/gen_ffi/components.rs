//! Dart wrappers over the shared C component-manager ABI.

pub(super) fn emit(prefix: &str, out: &mut String) {
    out.push_str(&format!(
        r#"typedef _ComponentLoadNative = Int32 Function(Pointer<Utf8> component);
typedef _ComponentLoadDart = int Function(Pointer<Utf8> component);
typedef _ComponentStringNative = Pointer<Utf8> Function(Pointer<Utf8> component);
typedef _ComponentStringDart = Pointer<Utf8> Function(Pointer<Utf8> component);

void _ensureComponentPlatform() {{
  if (Platform.isAndroid || Platform.isIOS) {{
    throw UnsupportedError(
      'Downloadable native components are unsupported on mobile targets; bundle the feature profile into the application instead.',
    );
  }}
}}

String _takeComponentString(Pointer<Utf8> pointer) {{
  if (pointer == nullptr) {{
    _checkError();
    throw StateError('Native component operation returned no result');
  }}
  try {{
    return pointer.toDartString();
  }} finally {{
    _freeString(pointer.cast<Char>());
  }}
}}

/// Ensure a signed native component is downloaded, verified, and loaded.
void componentLoad(String component) {{
  _ensureComponentPlatform();
  final nativeComponent = component.toNativeUtf8();
  try {{
    final operation = _lib.lookupFunction<_ComponentLoadNative, _ComponentLoadDart>(
      '{prefix}_component_load',
    );
    if (operation(nativeComponent) != 0) _checkError();
  }} finally {{
    calloc.free(nativeComponent);
  }}
}}

/// Prefetch one component, or every configured component when [component] is null.
List<String> componentPrefetch([String? component]) {{
  _ensureComponentPlatform();
  final nativeComponent = component?.toNativeUtf8();
  try {{
    final operation = _lib.lookupFunction<_ComponentStringNative, _ComponentStringDart>(
      '{prefix}_component_prefetch',
    );
    final json = _takeComponentString(operation(nativeComponent ?? nullptr));
    return (jsonDecode(json) as List<dynamic>).cast<String>();
  }} finally {{
    if (nativeComponent != null) calloc.free(nativeComponent);
  }}
}}

/// Return `missing`, `cached:<path>`, or `loaded:<path>` for a component.
String componentStatus(String component) => _componentStringCall(
      component,
      '{prefix}_component_status',
    );

/// Return the verified content-addressed cache path for a component.
String componentCachePath(String component) => _componentStringCall(
      component,
      '{prefix}_component_cache_path',
    );

String _componentStringCall(String component, String symbol) {{
  _ensureComponentPlatform();
  final nativeComponent = component.toNativeUtf8();
  try {{
    final operation = _lib.lookupFunction<_ComponentStringNative, _ComponentStringDart>(symbol);
    return _takeComponentString(operation(nativeComponent));
  }} finally {{
    calloc.free(nativeComponent);
  }}
}}

"#,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_owned_string_and_mobile_safe_component_api() {
        let mut generated = String::new();
        emit("demo_core", &mut generated);

        assert!(generated.contains("demo_core_component_load"));
        assert!(generated.contains("demo_core_component_prefetch"));
        assert!(generated.contains("_freeString(pointer.cast<Char>())"));
        assert!(generated.contains("Platform.isAndroid || Platform.isIOS"));
        assert!(generated.contains("jsonDecode(json)"));
    }
}
