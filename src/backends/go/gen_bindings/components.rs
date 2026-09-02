use crate::backends::go::gen_bindings::binding_file::{format_go_code, strip_trailing_whitespace};

pub(super) fn generate(package: &str, ffi_prefix: &str, ffi_header: &str, ffi_lib_name: &str, to_root: &str) -> String {
    let source = format!(
        r#"// Package {package} provides Go bindings for the component manager.
package {package}

/*
#cgo CFLAGS: -I${{SRCDIR}}/include
#cgo LDFLAGS: -L${{SRCDIR}}/{to_root}target/release -l{ffi_lib_name}
#include <stdlib.h>
#include "{ffi_header}"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

// ComponentLoad downloads, verifies, loads, and pins a configured component.
func ComponentLoad(component string) error {{
	cComponent := C.CString(component)
	defer C.free(unsafe.Pointer(cComponent))
	if C.{ffi_prefix}_component_load(cComponent) != 0 {{
		return componentError("component load")
	}}
	return nil
}}

// ComponentPrefetch downloads and verifies one named component, or every configured
// component when called without an argument.
func ComponentPrefetch(component ...string) ([]string, error) {{
	if len(component) > 1 {{
		return nil, fmt.Errorf("component prefetch accepts at most one component, got %d", len(component))
	}}
	var cComponent *C.char
	if len(component) == 1 {{
		cComponent = C.CString(component[0])
		defer C.free(unsafe.Pointer(cComponent))
	}}
	result := C.{ffi_prefix}_component_prefetch(cComponent)
	if result == nil {{
		return nil, componentError("component prefetch")
	}}
	defer C.{ffi_prefix}_free_string(result)
	var paths []string
	if err := json.Unmarshal([]byte(C.GoString(result)), &paths); err != nil {{
		return nil, fmt.Errorf("decode component prefetch result: %w", err)
	}}
	return paths, nil
}}

// ComponentStatus returns missing, cached:<path>, or loaded:<path>.
func ComponentStatus(component string) (string, error) {{
	cComponent := C.CString(component)
	defer C.free(unsafe.Pointer(cComponent))
	result := C.{ffi_prefix}_component_status(cComponent)
	if result == nil {{
		return "", componentError("component status")
	}}
	defer C.{ffi_prefix}_free_string(result)
	return C.GoString(result), nil
}}

// ComponentCachePath returns the content-addressed cache path for a component.
func ComponentCachePath(component string) (string, error) {{
	cComponent := C.CString(component)
	defer C.free(unsafe.Pointer(cComponent))
	result := C.{ffi_prefix}_component_cache_path(cComponent)
	if result == nil {{
		return "", componentError("component cache path")
	}}
	defer C.{ffi_prefix}_free_string(result)
	return C.GoString(result), nil
}}

func componentError(operation string) error {{
	if err := lastError(); err != nil {{
		return fmt.Errorf("%s: %w", operation, err)
	}}
	return fmt.Errorf("%s failed without a native error", operation)
}}
"#,
    );
    format_go_code(&strip_trailing_whitespace(&source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_go_wrappers_over_shared_c_manager() {
        let generated = generate("demo", "demo", "demo.h", "demo_ffi", "../../");
        assert!(generated.contains("func ComponentLoad(component string) error"));
        assert!(generated.contains("func ComponentPrefetch(component ...string)"));
        assert!(generated.contains("C.demo_component_status(cComponent)"));
        assert!(generated.contains("C.demo_component_cache_path(cComponent)"));
        assert!(generated.contains("C.demo_free_string(result)"));
    }
}
