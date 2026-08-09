use crate::core::hash::{self, CommentStyle};

pub(super) fn generate(package: &str, main_class: &str, prefix: &str) -> String {
    let prefix_upper = prefix.to_uppercase();
    format!(
        r#"{header}package {package};

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.invoke.MethodHandle;
import java.util.List;

/** Verified downloadable native-component management. */
public final class Components {{
    private static final ObjectMapper MAPPER = new ObjectMapper();

    private Components() {{ }}

    /** Downloads, verifies, loads, and pins a configured component. */
    public static void componentLoad(final String component) throws {main_class}Exception {{
        try (Arena arena = Arena.ofConfined()) {{
            int result = (int) NativeLib.{prefix_upper}_COMPONENT_LOAD.invoke(arena.allocateFrom(component));
            if (result != 0) {{
                throw nativeError("component load");
            }}
        }} catch ({main_class}Exception error) {{
            throw error;
        }} catch (Throwable error) {{
            throw new {main_class}Exception("component load failed", error);
        }}
    }}

    /** Downloads and verifies every configured component. */
    public static List<String> componentPrefetch() throws {main_class}Exception {{
        return componentPrefetchInternal(MemorySegment.NULL);
    }}

    /** Downloads and verifies one configured component. */
    public static List<String> componentPrefetch(final String component) throws {main_class}Exception {{
        try (Arena arena = Arena.ofConfined()) {{
            return componentPrefetchInternal(arena.allocateFrom(component));
        }}
    }}

    /** Returns missing, cached:&lt;path&gt;, or loaded:&lt;path&gt;. */
    public static String componentStatus(final String component) throws {main_class}Exception {{
        return callString(NativeLib.{prefix_upper}_COMPONENT_STATUS, component, "component status");
    }}

    /** Returns the content-addressed cache path for a configured component. */
    public static String componentCachePath(final String component) throws {main_class}Exception {{
        return callString(NativeLib.{prefix_upper}_COMPONENT_CACHE_PATH, component, "component cache path");
    }}

    private static List<String> componentPrefetchInternal(final MemorySegment component) throws {main_class}Exception {{
        try {{
            MemorySegment result = (MemorySegment) NativeLib.{prefix_upper}_COMPONENT_PREFETCH.invoke(component);
            String json = takeString(result, "component prefetch");
            return MAPPER.readValue(json, new TypeReference<List<String>>() {{ }});
        }} catch ({main_class}Exception error) {{
            throw error;
        }} catch (Throwable error) {{
            throw new {main_class}Exception("component prefetch failed", error);
        }}
    }}

    private static String callString(
            final MethodHandle method,
            final String component,
            final String operation) throws {main_class}Exception {{
        try (Arena arena = Arena.ofConfined()) {{
            MemorySegment result = (MemorySegment) method.invoke(arena.allocateFrom(component));
            return takeString(result, operation);
        }} catch ({main_class}Exception error) {{
            throw error;
        }} catch (Throwable error) {{
            throw new {main_class}Exception(operation + " failed", error);
        }}
    }}

    private static String takeString(final MemorySegment result, final String operation) throws Throwable {{
        if (result.equals(MemorySegment.NULL)) {{
            throw nativeError(operation);
        }}
        try {{
            return result.reinterpret(Long.MAX_VALUE).getString(0);
        }} finally {{
            NativeLib.{prefix_upper}_FREE_STRING.invoke(result);
        }}
    }}

    private static {main_class}Exception nativeError(final String operation) throws Throwable {{
        int code = (int) NativeLib.{prefix_upper}_LAST_ERROR_CODE.invoke();
        MemorySegment context = (MemorySegment) NativeLib.{prefix_upper}_LAST_ERROR_CONTEXT.invoke();
        String message = context.equals(MemorySegment.NULL)
                ? operation + " failed"
                : context.reinterpret(Long.MAX_VALUE).getString(0);
        return new {main_class}Exception(code, message);
    }}
}}
"#,
        header = hash::header(CommentStyle::DoubleSlash),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_java_panama_component_manager() {
        let generated = generate("dev.demo", "DemoRs", "demo");
        assert!(generated.contains("void componentLoad(final String component)"));
        assert!(generated.contains("List<String> componentPrefetch()"));
        assert!(generated.contains("String componentStatus(final String component)"));
        assert!(generated.contains("NativeLib.DEMO_COMPONENT_CACHE_PATH"));
        assert!(generated.contains("NativeLib.DEMO_FREE_STRING.invoke(result)"));
    }
}
