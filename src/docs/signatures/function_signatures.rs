use super::*;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn test_render_python_fn_sig_basic() {
    let func = make_function(
        "convert",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        None,
    );
    let sig = render_python_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "def convert(source: str) -> str");
}

#[test]
fn test_render_python_fn_sig_async() {
    let func = make_function("fetch", vec![], TypeRef::String, true, None);
    let sig = render_python_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "def fetch() -> str");
}

#[test]
fn test_render_python_fn_sig_optional_param() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_python_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "def search(query: str, limit: int = None) -> list[str]");
}

#[test]
fn test_render_python_fn_sig_complex_return_type() {
    let func = make_function(
        "get_mapping",
        vec![],
        TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::I32)),
        ),
        false,
        None,
    );
    let sig = render_python_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "def get_mapping() -> dict[str, int]");
}

#[test]
fn test_render_rust_fn_sig_basic() {
    let func = make_function(
        "convert",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        None,
    );
    let sig = render_rust_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn convert(source: &str) -> String");
}

#[test]
fn test_render_rust_fn_sig_async() {
    let func = make_function("fetch", vec![], TypeRef::String, true, None);
    let sig = render_rust_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub async fn fetch() -> String");
}

#[test]
fn test_render_rust_fn_sig_optional_param() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_rust_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn search(query: &str, limit: Option<u32>) -> Vec<String>");
}

#[test]
fn test_render_rust_fn_sig_error_type_with_return() {
    let func = make_function(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        Some("ParseError"),
    );
    let sig = render_rust_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn parse(source: &str) -> Result<Ast, ParseError>");
}

#[test]
fn test_render_rust_fn_sig_error_type_unit_return() {
    let func = make_function("save", vec![], TypeRef::Unit, false, Some("IoError"));
    let sig = render_rust_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn save() -> Result<(), IoError>");
}

#[test]
fn test_render_go_fn_sig_basic() {
    let func = make_function(
        "convert",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        None,
    );
    let sig = render_go_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "func Convert(source string) string");
}

#[test]
fn test_render_go_fn_sig_async() {
    let func = make_function("fetch", vec![], TypeRef::String, true, None);
    let sig = render_go_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "func Fetch() string");
}

#[test]
fn test_render_go_fn_sig_optional_param() {
    let func = make_function(
        "search",
        vec![make_param("limit", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_go_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "func Search(limit uint32) []string");
}

#[test]
fn test_render_go_fn_sig_error_type_with_return() {
    let func = make_function(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        Some("ParseError"),
    );
    let sig = render_go_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "func Parse(source string) (Ast, error)");
}

#[test]
fn test_render_go_fn_sig_error_type_unit_return() {
    let func = make_function("save", vec![], TypeRef::Unit, false, Some("IoError"));
    let sig = render_go_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "func Save() error");
}

#[test]
fn test_render_java_fn_sig_basic() {
    let func = make_function(
        "convert",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        None,
    );
    let sig = render_java_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static String convert(String source)");
}

#[test]
fn test_render_java_fn_sig_async() {
    let func = make_function("fetch", vec![], TypeRef::String, true, None);
    let sig = render_java_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static String fetch()");
}

#[test]
fn test_render_java_fn_sig_optional_param() {
    let func = make_function(
        "search",
        vec![make_param("limit", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_java_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static List<String> search(int limit)");
}

#[test]
fn test_render_java_fn_sig_error_type() {
    let func = make_function(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        Some("ParseError"),
    );
    let sig = render_java_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static Ast parse(String source) throws ParseError");
}

#[test]
fn test_render_csharp_fn_sig_basic() {
    let func = make_function(
        "convert",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        None,
    );
    let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static string Convert(string source)");
}

#[test]
fn test_render_csharp_fn_sig_async() {
    let func = make_function("fetch", vec![], TypeRef::String, true, None);
    let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static async Task<string> FetchAsync()");
}

#[test]
fn test_render_csharp_fn_sig_optional_param() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
    assert_eq!(
        sig,
        "public static List<string> Search(string query, uint? limit = null)"
    );
}

#[test]
fn test_render_csharp_fn_sig_complex_return_type() {
    let func = make_function(
        "get_mapping",
        vec![],
        TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(PrimitiveType::I32)),
        ),
        false,
        None,
    );
    let sig = render_csharp_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static Dictionary<string, int> GetMapping()");
}

#[test]
fn test_param_list_python_optional_uses_none_default() {
    let func = make_function(
        "run",
        vec![
            make_param("input", TypeRef::String, false),
            make_param("config", TypeRef::Named("Config".to_string()), true),
        ],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_python_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "def run(input: str, config: Config = None) -> None");
}

#[test]
fn test_param_list_node_optional_uses_question_mark() {
    let func = make_function(
        "run",
        vec![
            make_param("input", TypeRef::String, false),
            make_param("config", TypeRef::Named("Config".to_string()), true),
        ],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_typescript_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "function run(input: string, config?: Config): void");
}

#[test]
fn test_param_list_go_no_optional_syntax() {
    let func = make_function(
        "run",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_go_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "func Run(input string)");
}

#[test]
fn test_param_list_rust_string_params_use_refs() {
    let func = make_function(
        "process",
        vec![
            make_param("name", TypeRef::String, false),
            make_param("initial", TypeRef::Char, false),
            make_param("data", TypeRef::Bytes, false),
        ],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_rust_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn process(name: &str, initial: &str, data: &[u8])");
}

#[test]
fn test_param_list_php_uses_dollar_prefix() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_php_fn_sig(&func, TEST_PREFIX);
    assert_eq!(
        sig,
        "public static function search(string $query, ?int $limit = null): array<string>"
    );
}

#[test]
fn test_render_kotlin_fn_sig_no_error_no_return() {
    let func = make_function(
        "run",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_kotlin_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "fun run(input: String)");
}

#[test]
fn test_render_kotlin_fn_sig_with_optional_and_return() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_kotlin_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "fun search(query: String, limit: Int? = null): List<String>");
}

#[test]
fn test_render_kotlin_fn_sig_with_error_emits_throws_annotation() {
    let func = make_function(
        "convert",
        vec![make_param("html", TypeRef::String, false)],
        TypeRef::String,
        false,
        Some("ConversionError"),
    );
    let sig = render_kotlin_fn_sig(&func, TEST_PREFIX);
    assert_eq!(
        sig,
        "@Throws(ConversionError::class)\nfun convert(html: String): String"
    );
}

#[test]
fn test_render_swift_fn_sig_no_error_no_return() {
    let func = make_function(
        "run",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_swift_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static func run(input: String)");
}

#[test]
fn test_render_swift_fn_sig_with_optional_param_emits_nil_default() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_swift_fn_sig(&func, TEST_PREFIX);
    assert_eq!(
        sig,
        "public static func search(query: String, limit: UInt32? = nil) -> [String]"
    );
}

#[test]
fn test_render_swift_fn_sig_with_error_emits_throws() {
    let func = make_function(
        "convert",
        vec![make_param("html", TypeRef::String, false)],
        TypeRef::String,
        false,
        Some("ConversionError"),
    );
    let sig = render_swift_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "public static func convert(html: String) throws -> String");
}

#[test]
fn test_render_dart_fn_sig_required_only() {
    let func = make_function(
        "run",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_dart_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "void run(String input)");
}

#[test]
fn test_render_dart_fn_sig_optional_param_uses_bracketed_positional() {
    let func = make_function(
        "search",
        vec![
            make_param("query", TypeRef::String, false),
            make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true),
        ],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
    );
    let sig = render_dart_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "List<String> search(String query, [int? limit])");
}

#[test]
fn test_render_zig_fn_sig_no_error() {
    let func = make_function(
        "search",
        vec![make_param("query", TypeRef::String, false)],
        TypeRef::Primitive(PrimitiveType::U32),
        false,
        None,
    );
    let sig = render_zig_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn search(query: [:0]const u8) u32");
}

#[test]
fn test_render_zig_fn_sig_with_error_emits_error_union() {
    let func = make_function(
        "convert",
        vec![make_param("html", TypeRef::String, false)],
        TypeRef::String,
        false,
        Some("ConversionError"),
    );
    let sig = render_zig_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn convert(html: [:0]const u8) ConversionError![:0]const u8");
}

#[test]
fn test_render_zig_fn_sig_optional_param_prefixes_question_mark() {
    let func = make_function(
        "search",
        vec![make_param("limit", TypeRef::Primitive(PrimitiveType::U32), true)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_zig_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "pub fn search(limit: ?u32) void");
}

#[test]
fn test_render_method_signature_kotlin_static_emits_jvmstatic() {
    let method = make_method(
        "default",
        vec![],
        TypeRef::Named("ParseOptions".into()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "ParseOptions", Language::Kotlin, TEST_PREFIX);
    assert_eq!(sig, "@JvmStatic\nfun default(): ParseOptions");
}

#[test]
fn test_render_method_signature_swift_instance_with_throws() {
    let method = make_method(
        "apply_update",
        vec![make_param("update", TypeRef::Named("ParseOptionsUpdate".into()), false)],
        TypeRef::Unit,
        false,
        false,
        Some("ConversionError"),
    );
    let sig = render_method_signature(&method, "ParseOptions", Language::Swift, TEST_PREFIX);
    assert_eq!(sig, "public func applyUpdate(update: ParseOptionsUpdate) throws");
}

#[test]
fn test_render_method_signature_dart_instance_method() {
    let method = make_method(
        "classify_link",
        vec![make_param("href", TypeRef::String, false)],
        TypeRef::Named("LinkType".into()),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "LinkMetadata", Language::Dart, TEST_PREFIX);
    assert_eq!(sig, "LinkType classifyLink(String href)");
}

#[test]
fn test_render_method_signature_zig_instance_includes_self_receiver() {
    let method = make_method(
        "warnings",
        vec![],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "ParseOutput", Language::Zig, TEST_PREFIX);
    assert_eq!(sig, "pub fn warnings(self: *const ParseOutput) []const [:0]const u8");
}

#[test]
fn test_render_method_signature_zig_static_omits_self() {
    let method = make_method(
        "create",
        vec![],
        TypeRef::Named("ParseOptions".into()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "ParseOptions", Language::Zig, TEST_PREFIX);
    assert_eq!(sig, "pub fn create() ParseOptions");
}

#[test]
fn test_render_method_signature_kotlin_android_shares_kotlin_renderer() {
    let method = make_method(
        "convert",
        vec![make_param("html", TypeRef::String, false)],
        TypeRef::String,
        false,
        true,
        Some("ConversionError"),
    );
    let sig = render_method_signature(&method, "Converter", Language::KotlinAndroid, TEST_PREFIX);
    assert_eq!(
        sig,
        "@Throws(ConversionError::class)\n@JvmStatic\nfun convert(html: String): String"
    );
}
