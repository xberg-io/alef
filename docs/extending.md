# Extending Alef

Alef works as an opinionated codegen engine: it handles extraction, language backend dispatch,
and scaffolding. Domain-specific logic (HTTP service APIs, plugin systems, GraphQL schema validation)
lives outside alef as extensions, so consumers can reuse the engine without coupling.

## Why Extensions

Previous alef versions bundled HTTP-domain concerns directly: `LifecycleHookDef`, `WebSocketRouteDef`,
`SseRouteDef`, `ErrorTypeDef`, RFC 9457 error mappings. This prevented libraries without service
declarations (data-binding libraries, document processors) from using alef cleanly. The Extension
trait extracts this: one trait, three deployment modes, same interface.

## The Extension Trait

```rust
pub trait Extension: Send + Sync {
    fn name(&self) -> &str;

    fn parse_config(&self, raw: Option<&toml::Value>) -> Result<ExtensionConfig> {
        Ok(ExtensionConfig::empty())
    }

    fn augment_surface(
        &self,
        _api: &mut ApiSurface,
        _cfg: &ExtensionConfig,
    ) -> Result<()> {
        Ok(())
    }

    fn emit_for_language(
        &self,
        _api: &ApiSurface,
        _cfg: &ExtensionConfig,
        _language: Language,
        _env: &TemplateEnv,
    ) -> Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    fn transform_emitted_files(
        &self,
        _api: &ApiSurface,
        _cfg: &ExtensionConfig,
        _language: Language,
        _files: &mut Vec<GeneratedFile>,
        _env: &TemplateEnv,
    ) -> Result<()> {
        Ok(())
    }
}
```

Four methods, all with default no-op impls. Override what you need:

- **`name()`** — identifier string (required). Used as the TOML config key: `[extensions.<name>]`.
- **`parse_config(raw)`** — parse TOML into typed `ExtensionConfig`. Receives the
  `[extensions.<name>]` section from `alef.toml`, or `None` when that section is absent.
- **`augment_surface(api, cfg)`** — mutate `ApiSurface` after extraction. Default: no-op.
- **`emit_for_language(api, cfg, language, env)`** — return extra `GeneratedFile`s for one language.
- **`transform_emitted_files(...)`** — rewrite emitted files after backend and extension generation.

## ExtensionConfig

```rust
pub struct ExtensionConfig {
    data: Option<Box<dyn Any + Send + Sync>>,
    raw: toml::Value,
}
```

`ExtensionConfig` wraps opaque typed data plus the raw TOML. You own serialization:

```rust
impl MyExtension {
    fn parse_config_impl(raw: Option<&toml::Value>) -> Result<MyConfig> {
        // Use serde_json, toml, or your choice
        let cfg = raw.as_ref().map(|v| toml::from_str(...)).transpose()?;
        Ok(cfg.unwrap_or_default())
    }
}

impl Extension for MyExtension {
    fn parse_config(&self, raw: Option<&toml::Value>) -> Result<ExtensionConfig> {
        let typed = Self::parse_config_impl(raw)?;
        Ok(ExtensionConfig::with_data(Box::new(typed)))
    }

    fn emit_for_language(&self, api, cfg, lang, env) -> Result<Vec<GeneratedFile>> {
        let my_cfg: &MyConfig = cfg.downcast_ref()?;
        // Use my_cfg.field values
    }
}
```

## TemplateEnv

Extensions emit Jinja templates without depending on minijinja directly:

```rust
pub struct TemplateEnv {
    // opaque handle
}

impl TemplateEnv {
    pub fn add_template(&self, name: &str, source: &str) -> Result<()> { }
    pub fn render(&self, name: &str, context: &serde_json::Value) -> Result<String> { }
}
```

In `emit_for_language`, register and render templates:

```rust
env.add_template("my_template.jinja", include_str!("templates/my_template.jinja"))?;
let rendered = env.render("my_template.jinja", &serde_json::json!({
    "api": api,
    "config": my_cfg,
    "language": format!("{:?}", language),
}))?;
Ok(vec![GeneratedFile {
    path: "generated/my_file.rs".into(),
    content: rendered,
}])
```

## Mode 1: Linked Extension

Consumer crate depends on alef (not alef), implements the trait, ships a CLI binary.

### Directory Layout

```text
crates/
  my-alef-ext/          # Library crate
    src/
      lib.rs            # pub struct MyExtension; impl Extension
      config.rs         # MyConfig deserialization
      ir.rs             # Domain types (LifecycleHookDef, etc.)
      emit/
        pyo3.rs
        napi.rs
        ... (per-language emission)
    templates/          # Jinja templates
  my-alef/              # Thin CLI crate
    src/
      main.rs
    Cargo.toml
        depends-on: my-alef-ext, alef
```

### Example main.rs

```rust
use my_alef_ext::MyExtension;

fn main() {
    alef::run_with_extensions(vec![
        Box::new(MyExtension::default())
    ])
}
```

### Cargo.toml

```toml
[[bin]]
name = "my-alef"
path = "src/main.rs"

[dependencies]
alef = { version = "0.1", git = "https://github.com/kreuzberg-dev/alef" }
my-alef-ext = { path = "../my-alef-ext" }
```

### Taskfile Integration

Update your repo's `task alef:generate` to use your custom binary:

```bash
cargo run -p my-alef -- all --clean --format=false
```

## Mode 2: Dynamic Extension

Compile an extension as a dylib (`.so`, `.dylib`, `.dll`). Alef loads it at runtime via a C-ABI factory function.

### When to Use

- Extension author and framework consumer are separate organizations
- Consumer can't or won't take a Rust dependency
- You want to distribute the extension separately from the framework

### ABI Rules

Factory function must be `extern "C"`, return `Box<dyn Extension>`, and have no parameters:

```rust
#[no_mangle]
pub extern "C" fn alef_extension_factory() -> Box<dyn alef::Extension> {
    Box::new(MyExtension::default())
}
```

Compile with `cargo build --release --crate-type=cdylib`.

### Invocation

Enable the optional `dylib-loader` feature in alef:

```bash
alef generate --load-extension path/to/libmy_extension.so
```

### Security

Loaded dylibs run with full process privileges. Load them from trusted sources.

## Mode 3: Template-only Extension

Declare `[[extensions.template]]` blocks in `alef.toml`. Alef's built-in `TemplateExtension` renders
them — no custom Rust code required.

### Alef.toml

```toml
[[extensions.template]]
name = "custom_schema"
template = "templates/schema.jinja"
output_path = "generated/schema.json"

[[extensions.template]]
name = "custom_readme"
template = "templates/extra_readme.jinja"
output_path = "generated/extra.md"
languages = ["python", "node"]  # Optional: limit to specific languages
```

### Template Context

Templates receive a JSON context with the full `ApiSurface` and can introspect the workspace configuration:

```jinja
{{ api.functions | length }} functions
{% for func in api.functions %}
  - {{ func.name }}({{ func.parameters | length }} params)
{% endfor %}
```

### Template-only Use Cases

- Adding extra output files without modifying alef
- Template rendering without custom logic
- Extension author and framework consumer can collaborate on templates

## Choosing a Mode

| Requirement | Linked | Dynamic | Template-only |
|---|---|---|---|
| Full control over logic | Yes | Yes | No |
| Type-safe configuration | Yes | Yes | Limited |
| No Rust dependency | No | No | Yes |
| Per-language conditionals | Yes | Yes | Yes |
| Can introspect/mutate ApiSurface | Yes | Yes | No |
| Simplest to author | No | No | Yes |
| Preferred for frameworks | Yes | No | No |

## Where Extension Config Blocks Live in alef.toml

Extensions parse their own sections. By convention:

```toml
[extensions.my_domain]
setting_1 = "value"
setting_2 = 123

[[extensions.my_domain.subsection]]
name = "item1"
```

Alef passes the entire `[extensions.my_domain]` (or `extensions.my_domain.*`) section to
`MyExtension::parse_config()`. Use your favorite TOML deserializer (serde, toml-rs, manual parsing)
to extract typed config.

For template extensions, blocks use `[[extensions.template]]` as a reserved list.
