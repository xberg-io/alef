use super::*;

// ---------------------------------------------------------------------------
// render_method_signature — Python
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_python_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def get_text(self, page: int) -> str");
}

#[test]
fn test_render_method_signature_python_async() {
    let method = make_method("process", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def process(self) -> str");
}

#[test]
fn test_render_method_signature_python_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "@staticmethod\ndef create(name: str) -> Document");
}

#[test]
fn test_render_method_signature_python_optional_return() {
    let method = make_method(
        "find",
        vec![make_param("query", TypeRef::String, false)],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def find(self, query: str) -> str | None");
}

#[test]
fn test_render_method_signature_python_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def parse(self, source: str) -> Ast");
}

// ---------------------------------------------------------------------------
// render_method_signature — Node / TypeScript
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_node_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "getText(page: number): string");
}

#[test]
fn test_render_method_signature_node_async() {
    let method = make_method("process", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "process(): Promise<string>");
}

#[test]
fn test_render_method_signature_node_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "static create(name: string): Document");
}

#[test]
fn test_render_method_signature_node_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "find(): string | null");
}

#[test]
fn test_render_method_signature_node_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "parse(source: string): Ast");
}

// ---------------------------------------------------------------------------
// render_method_signature — Rust
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_rust_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn get_text(&self, page: u32) -> String");
}

#[test]
fn test_render_method_signature_rust_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub async fn fetch(&self) -> String");
}

#[test]
fn test_render_method_signature_rust_static() {
    let method = make_method(
        "new",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn new(name: &str) -> Document");
}

#[test]
fn test_render_method_signature_rust_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("Node".to_string()))),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Tree", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn find(&self) -> Option<Node>");
}

#[test]
fn test_render_method_signature_rust_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn parse(&self, source: &str) -> Result<Ast, ParseError>");
}

// ---------------------------------------------------------------------------
// render_method_signature — Go
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_go_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Document) GetText(page uint32) string");
}

#[test]
fn test_render_method_signature_go_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Client) Fetch() string");
}

#[test]
fn test_render_method_signature_go_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Corpus) Find() *string");
}

#[test]
fn test_render_method_signature_go_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Parser) Parse(source string) (Ast, error)");
}

#[test]
fn test_render_method_signature_go_error_type_unit_return() {
    let method = make_method("save", vec![], TypeRef::Unit, false, false, Some("IoError"));
    let sig = render_method_signature(&method, "File", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *File) Save() error");
}

// ---------------------------------------------------------------------------
// render_method_signature — Ruby
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_ruby_sync_with_params() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def get_text(page)");
}

#[test]
fn test_render_method_signature_ruby_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def self.create(name)");
}

#[test]
fn test_render_method_signature_ruby_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def fetch()");
}

#[test]
fn test_render_method_signature_ruby_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def find()");
}

#[test]
fn test_render_method_signature_ruby_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def parse()");
}

// ---------------------------------------------------------------------------
// render_method_signature — PHP
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_php_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function getText(int $page): string");
}

#[test]
fn test_render_method_signature_php_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public static function create(string $name): Document");
}

#[test]
fn test_render_method_signature_php_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function find(): ?string");
}

#[test]
fn test_render_method_signature_php_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function fetch(): string");
}

#[test]
fn test_render_method_signature_php_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function parse(): string");
}

// ---------------------------------------------------------------------------
// render_method_signature — Java
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_java_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public String getText(int page)");
}

#[test]
fn test_render_method_signature_java_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public static Document create(String name)");
}

#[test]
fn test_render_method_signature_java_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public Optional<String> find()");
}

#[test]
fn test_render_method_signature_java_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public String fetch()");
}

#[test]
fn test_render_method_signature_java_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public Ast parse(String source) throws ParseError");
}

// ---------------------------------------------------------------------------
// render_method_signature — C#
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_csharp_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public string GetText(uint page)");
}

#[test]
fn test_render_method_signature_csharp_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public async Task<string> FetchAsync()");
}

#[test]
fn test_render_method_signature_csharp_async_already_suffixed() {
    let method = make_method("fetch_async", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public async Task<string> FetchAsync()");
}

#[test]
fn test_render_method_signature_csharp_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public string? Find()");
}

#[test]
fn test_render_method_signature_csharp_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public string Parse()");
}

// ---------------------------------------------------------------------------
// render_method_signature — Elixir
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_elixir_sync_with_params() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def get_text(page)");
}

#[test]
fn test_render_method_signature_elixir_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def fetch()");
}

#[test]
fn test_render_method_signature_elixir_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def find()");
}

#[test]
fn test_render_method_signature_elixir_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def parse()");
}

// ---------------------------------------------------------------------------
// render_method_signature — R
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_r_sync_with_params() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::R, TEST_PREFIX);
    assert_eq!(sig, "get_text(page)");
}

#[test]
fn test_render_method_signature_r_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::R, TEST_PREFIX);
    assert_eq!(sig, "fetch()");
}

#[test]
fn test_render_method_signature_r_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::R, TEST_PREFIX);
    assert_eq!(sig, "find()");
}

#[test]
fn test_render_method_signature_r_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::R, TEST_PREFIX);
    assert_eq!(sig, "parse()");
}

// ---------------------------------------------------------------------------
// render_method_signature — WASM (shares Node rendering)
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_wasm_sync() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Wasm, TEST_PREFIX);
    assert_eq!(sig, "getText(page: number): string");
}

#[test]
fn test_render_method_signature_wasm_static() {
    let method = make_method(
        "create",
        vec![],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Wasm, TEST_PREFIX);
    assert_eq!(sig, "static create(): Document");
}

// ---------------------------------------------------------------------------
