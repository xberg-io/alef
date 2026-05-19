use serde::{Deserialize, Serialize};

/// Per-language DTO/type generation style configuration.
///
/// Controls what type system is used for generated public API types in each language
/// (e.g., Python `@dataclass` vs `TypedDict` vs `pydantic.BaseModel`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DtoConfig {
    /// Python input type style (default: dataclass).
    #[serde(default)]
    pub python: PythonDtoStyle,
    /// Python output/return type style (default: same as `python`).
    #[serde(default)]
    pub python_output: Option<PythonDtoStyle>,
    /// TypeScript/Node type style (default: interface).
    #[serde(default)]
    pub node: NodeDtoStyle,
    /// Ruby type style (default: struct).
    #[serde(default)]
    pub ruby: RubyDtoStyle,
    /// PHP type style (default: readonly-class).
    #[serde(default)]
    pub php: PhpDtoStyle,
    /// Elixir type style (default: struct).
    #[serde(default)]
    pub elixir: ElixirDtoStyle,
    /// Go type style (default: struct).
    #[serde(default)]
    pub go: GoDtoStyle,
    /// Java type style (default: record).
    #[serde(default)]
    pub java: JavaDtoStyle,
    /// C# type style (default: record).
    #[serde(default)]
    pub csharp: CsharpDtoStyle,
    /// R type style (default: list).
    #[serde(default)]
    pub r: RDtoStyle,
}

impl DtoConfig {
    /// Resolve the Python output type style (falls back to input style).
    pub fn python_output_style(&self) -> PythonDtoStyle {
        self.python_output.unwrap_or(self.python)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PythonDtoStyle {
    #[default]
    Dataclass,
    TypedDict,
    Pydantic,
    Msgspec,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NodeDtoStyle {
    #[default]
    Interface,
    Zod,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RubyDtoStyle {
    #[default]
    Struct,
    DryStruct,
    Data,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PhpDtoStyle {
    #[default]
    ReadonlyClass,
    Array,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ElixirDtoStyle {
    #[default]
    Struct,
    TypedStruct,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GoDtoStyle {
    #[default]
    Struct,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum JavaDtoStyle {
    #[default]
    Record,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum JavaBuilderMode {
    /// Emit builder when field count >= 8 OR (nested type exists AND field count >= 5).
    #[default]
    Auto,
    /// Always emit builder for types with defaults.
    Always,
    /// Never emit builder.
    Never,
}

/// Java-specific DTO configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JavaDtoConfig {
    /// Builder mode: auto (default), always, or never.
    #[serde(default)]
    pub builder: JavaBuilderMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CsharpDtoStyle {
    #[default]
    Record,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RDtoStyle {
    #[default]
    List,
    R6,
}
