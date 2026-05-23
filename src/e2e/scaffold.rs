//! Fixture scaffolding for `alef e2e init` and `alef e2e scaffold`.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use anyhow::{Context, Result};
use std::path::Path;

static FIXTURE_SCHEMA: &str = include_str!("schema/fixture.schema.json");

/// Create the fixtures directory structure and write the schema file.
/// Called by `alef e2e init`.
pub fn init_fixtures(e2e_config: &E2eConfig, _config: &ResolvedCrateConfig) -> Result<Vec<String>> {
    let fixtures_dir = Path::new(&e2e_config.fixtures);
    let mut created = Vec::new();

    // 1. Create fixtures directory
    if !fixtures_dir.exists() {
        std::fs::create_dir_all(fixtures_dir)
            .with_context(|| format!("failed to create fixtures dir: {}", fixtures_dir.display()))?;
        created.push(fixtures_dir.display().to_string());
    }

    // 2. Write schema.json
    let schema_path = fixtures_dir.join("schema.json");
    std::fs::write(&schema_path, FIXTURE_SCHEMA)
        .with_context(|| format!("failed to write {}", schema_path.display()))?;
    created.push(schema_path.display().to_string());

    // 3. Create smoke directory
    let smoke_dir = fixtures_dir.join("smoke");
    if !smoke_dir.exists() {
        std::fs::create_dir_all(&smoke_dir)
            .with_context(|| format!("failed to create smoke dir: {}", smoke_dir.display()))?;
        created.push(smoke_dir.display().to_string());
    }

    // 4. Write smoke/basic.json example fixture
    let basic_path = smoke_dir.join("basic.json");
    let basic_fixture = build_example_fixture(e2e_config);
    std::fs::write(&basic_path, basic_fixture).with_context(|| format!("failed to write {}", basic_path.display()))?;
    created.push(basic_path.display().to_string());

    Ok(created)
}

/// Create a new fixture file from a template.
/// Called by `alef e2e scaffold --id <id> --category <cat> --description <desc>`.
pub fn scaffold_fixture(
    e2e_config: &E2eConfig,
    _config: &ResolvedCrateConfig,
    id: &str,
    category: &str,
    description: &str,
) -> Result<String> {
    let fixtures_dir = Path::new(&e2e_config.fixtures);
    let category_dir = fixtures_dir.join(category);

    // 1. Create category directory
    if !category_dir.exists() {
        std::fs::create_dir_all(&category_dir)
            .with_context(|| format!("failed to create category dir: {}", category_dir.display()))?;
    }

    // 2. Write fixture file
    let fixture_path = category_dir.join(format!("{id}.json"));
    let fixture = build_scaffold_fixture(e2e_config, id, description);
    std::fs::write(&fixture_path, fixture).with_context(|| format!("failed to write {}", fixture_path.display()))?;

    Ok(fixture_path.display().to_string())
}

/// Build the example fixture JSON for `init`.
fn build_example_fixture(e2e_config: &E2eConfig) -> String {
    let mut input_fields = Vec::new();
    for arg in &e2e_config.call.args {
        let value = example_value_for_type(&arg.arg_type);
        input_fields.push(format!("    \"{}\": {value}", arg.field));
    }

    let input_block = if input_fields.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n  }}", input_fields.join(",\n"))
    };

    // Use the first arg's field for the not_empty assertion if available
    let first_field = e2e_config
        .call
        .args
        .first()
        .map(|a| a.field.as_str())
        .unwrap_or("result");

    format!(
        r#"{{
  "id": "basic_smoke",
  "description": "Basic smoke test verifying the function returns without error",
  "input": {input_block},
  "assertions": [
    {{ "type": "not_error" }},
    {{ "type": "not_empty", "field": "{first_field}" }}
  ]
}}
"#
    )
}

/// Build a scaffold fixture JSON for `scaffold`.
fn build_scaffold_fixture(e2e_config: &E2eConfig, id: &str, description: &str) -> String {
    let mut input_fields = Vec::new();
    for arg in &e2e_config.call.args {
        let value = empty_value_for_type(&arg.arg_type);
        input_fields.push(format!("    \"{}\": {value}", arg.field));
    }

    let input_block = if input_fields.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n  }}", input_fields.join(",\n"))
    };

    format!(
        r#"{{
  "id": "{id}",
  "description": "{description}",
  "input": {input_block},
  "assertions": [
    {{ "type": "not_error" }}
  ]
}}
"#
    )
}

/// Return an example value for a given arg type (for init).
fn example_value_for_type(arg_type: &str) -> &'static str {
    match arg_type {
        "string" => "\"example\"",
        "int" | "integer" => "0",
        "float" | "number" => "0.0",
        "bool" | "boolean" => "true",
        "json_object" => "{}",
        "bytes" => "\"\"",
        _ => "\"\"",
    }
}

/// Return an empty/default value for a given arg type (for scaffold).
fn empty_value_for_type(arg_type: &str) -> &'static str {
    match arg_type {
        "string" => "\"\"",
        "int" | "integer" => "0",
        "float" | "number" => "0.0",
        "bool" | "boolean" => "false",
        "json_object" => "{}",
        "bytes" => "\"\"",
        _ => "\"\"",
    }
}
