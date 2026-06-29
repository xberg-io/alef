use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize)]
pub struct DocsRenderContext {
    pub krate: CrateDocsContext,
    pub languages: Vec<String>,
    pub references: Vec<ReferenceDoc>,
    pub api_references: Vec<ReferenceDoc>,
    pub cli: CliSurface,
    pub mcp: McpSurface,
    pub snippets: SnippetIndexContext,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CrateDocsContext {
    pub name: String,
    pub version: String,
    pub description: String,
    pub repository: String,
    pub license: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReferenceDoc {
    pub kind: String,
    pub title: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CliSurface {
    pub commands: Vec<CliCommand>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CliCommand {
    pub name: String,
    pub path: String,
    pub about: String,
    pub options: Vec<CliOption>,
    pub positionals: Vec<CliOption>,
    pub subcommands: Vec<CliCommand>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CliOption {
    pub name: String,
    pub long: Option<String>,
    pub short: Option<String>,
    pub value_name: Option<String>,
    pub ty: String,
    pub default: Option<String>,
    pub required: bool,
    pub help: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct McpSurface {
    pub tools: Vec<McpItem>,
    pub prompts: Vec<McpItem>,
    pub resources: Vec<McpItem>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct McpItem {
    pub name: String,
    pub title: String,
    pub description: String,
    pub handler: String,
    pub params_type: Option<String>,
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SnippetIndexContext {
    pub dirs: Vec<String>,
    pub snippets: Vec<SnippetContext>,
    pub counts_by_language: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnippetContext {
    pub id: Option<String>,
    pub path: String,
    pub language: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
}
