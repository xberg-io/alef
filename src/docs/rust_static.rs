use super::context::{CliCommand, CliOption, CliSurface, McpItem, McpSurface};
use anyhow::Context as _;
use heck::ToKebabCase;
use quote::ToTokens;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use syn::{Fields, FnArg, Item, Type};

pub fn extract_cli_surface(sources: &[PathBuf]) -> anyhow::Result<CliSurface> {
    let parsed = parse_sources(sources)?;
    let mut structs = HashMap::new();
    let mut enums = HashMap::new();

    for file in &parsed {
        for item in &file.items {
            match item {
                Item::Struct(item) => {
                    if has_derive(&item.attrs, "Parser")
                        || has_derive(&item.attrs, "Args")
                        || item.attrs.iter().any(|attr| attr.path().is_ident("command"))
                    {
                        structs.insert(item.ident.to_string(), item.clone());
                    }
                }
                Item::Enum(item) if has_derive(&item.attrs, "Subcommand") => {
                    enums.insert(item.ident.to_string(), item.clone());
                }
                _ => {}
            }
        }
    }

    let mut commands = Vec::new();
    let mut roots: Vec<_> = structs
        .values()
        .filter(|item| has_derive(&item.attrs, "Parser"))
        .collect();
    roots.sort_by_key(|item| item.ident.to_string());

    for root in roots {
        commands.push(command_from_struct(root, &structs, &enums, None));
    }

    Ok(CliSurface { commands })
}

pub fn extract_mcp_surface(sources: &[PathBuf]) -> anyhow::Result<McpSurface> {
    let parsed = parse_sources(sources)?;
    let mut surface = McpSurface::default();

    for file in &parsed {
        for item in &file.items {
            let Item::Impl(item_impl) = item else {
                continue;
            };
            for item in &item_impl.items {
                let syn::ImplItem::Fn(method) = item else {
                    continue;
                };
                for attr_name in ["tool", "prompt", "resource"] {
                    let Some(tokens) = attr_tokens(&method.attrs, attr_name) else {
                        continue;
                    };
                    let name = quoted_value(&tokens, "name").unwrap_or_else(|| method.sig.ident.to_string());
                    let description = quoted_value(&tokens, "description")
                        .or_else(|| first_doc_paragraph(&method.attrs))
                        .unwrap_or_default();
                    let annotations = annotation_map(&tokens);
                    let title = annotations
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| name.replace('_', " ").to_title_case());
                    let item = McpItem {
                        name,
                        title,
                        description,
                        handler: method.sig.ident.to_string(),
                        params_type: method_params_type(method),
                        annotations,
                    };
                    match attr_name {
                        "tool" => surface.tools.push(item),
                        "prompt" => surface.prompts.push(item),
                        "resource" => surface.resources.push(item),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    surface.tools.sort_by(|left, right| left.name.cmp(&right.name));
    surface.prompts.sort_by(|left, right| left.name.cmp(&right.name));
    surface.resources.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(surface)
}

fn parse_sources(sources: &[PathBuf]) -> anyhow::Result<Vec<syn::File>> {
    let mut parsed = Vec::new();
    for source in sources {
        if !source.exists() {
            continue;
        }
        let content = std::fs::read_to_string(source)
            .with_context(|| format!("failed to read docs source {}", source.display()))?;
        parsed.push(
            syn::parse_file(&content).with_context(|| format!("failed to parse docs source {}", source.display()))?,
        );
    }
    Ok(parsed)
}

fn command_from_struct(
    item: &syn::ItemStruct,
    structs: &HashMap<String, syn::ItemStruct>,
    enums: &HashMap<String, syn::ItemEnum>,
    forced_name: Option<String>,
) -> CliCommand {
    let name = forced_name
        .or_else(|| command_name(&item.attrs))
        .unwrap_or_else(|| item.ident.to_string().to_kebab_case());
    let about = command_about(&item.attrs)
        .or_else(|| first_doc_paragraph(&item.attrs))
        .unwrap_or_default();
    let mut command = CliCommand {
        path: name.clone(),
        name,
        about,
        ..CliCommand::default()
    };

    let Fields::Named(fields) = &item.fields else {
        return command;
    };

    for field in &fields.named {
        process_command_field(field, &mut command, structs, enums);
    }

    command
}

/// Add a single named clap field to `command`, resolving `#[command(subcommand)]`
/// and `#[command(flatten)]` — flattened args are expanded inline rather than
/// emitted as an opaque struct row. Shared by struct-derived commands and
/// struct-like enum-variant commands so both expand flattened args identically.
fn process_command_field(
    field: &syn::Field,
    command: &mut CliCommand,
    structs: &HashMap<String, syn::ItemStruct>,
    enums: &HashMap<String, syn::ItemEnum>,
) {
    if has_attr_word(&field.attrs, "command", "subcommand") {
        if let Some(enum_name) = type_last_ident(&field.ty)
            && let Some(en) = enums.get(&enum_name)
        {
            command.subcommands = commands_from_enum(en, structs, enums, &command.path);
        }
        return;
    }
    if has_attr_word(&field.attrs, "command", "flatten") {
        if let Some(struct_name) = type_last_ident(&field.ty)
            && let Some(flattened) = structs.get(&struct_name)
        {
            let mut flattened_command = command_from_struct(flattened, structs, enums, Some(command.name.clone()));
            command.options.append(&mut flattened_command.options);
            command.positionals.append(&mut flattened_command.positionals);
        }
        return;
    }
    let option = option_from_field(field);
    if option.long.is_some() || option.short.is_some() || option.ty == "bool" {
        command.options.push(option);
    } else {
        command.positionals.push(option);
    }
}

fn commands_from_enum(
    item: &syn::ItemEnum,
    structs: &HashMap<String, syn::ItemStruct>,
    enums: &HashMap<String, syn::ItemEnum>,
    parent_path: &str,
) -> Vec<CliCommand> {
    let mut commands = Vec::new();
    for variant in &item.variants {
        let name = command_name(&variant.attrs).unwrap_or_else(|| variant.ident.to_string().to_kebab_case());
        let about = command_about(&variant.attrs)
            .or_else(|| first_doc_paragraph(&variant.attrs))
            .unwrap_or_default();
        let mut command = CliCommand {
            path: format!("{parent_path} {name}"),
            name,
            about,
            ..CliCommand::default()
        };

        match &variant.fields {
            Fields::Named(fields) => {
                for field in &fields.named {
                    process_command_field(field, &mut command, structs, enums);
                }
            }
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let ty = &fields.unnamed.first().expect("checked len").ty;
                if let Some(struct_name) = type_last_ident(ty)
                    && let Some(args) = structs.get(&struct_name)
                {
                    let nested = command_from_struct(args, structs, enums, Some(command.name.clone()));
                    command.options = nested.options;
                    command.positionals = nested.positionals;
                    command.subcommands = nested.subcommands;
                }
            }
            Fields::Unnamed(fields) => {
                for (index, field) in fields.unnamed.iter().enumerate() {
                    let mut option = option_from_field(field);
                    if option.name.is_empty() {
                        option.name = format!("arg{}", index + 1);
                    }
                    command.positionals.push(option);
                }
            }
            Fields::Unit => {}
        }
        commands.push(command);
    }
    commands.sort_by(|left, right| left.name.cmp(&right.name));
    commands
}

fn option_from_field(field: &syn::Field) -> CliOption {
    let name = field.ident.as_ref().map(ToString::to_string).unwrap_or_default();
    let arg_tokens = attr_tokens(&field.attrs, "arg").unwrap_or_default();
    let long = if let Some(value) = quoted_value(&arg_tokens, "long") {
        Some(value)
    } else if has_bare_word(&arg_tokens, "long") {
        Some(name.to_kebab_case())
    } else {
        None
    };
    let short = quoted_value(&arg_tokens, "short").or_else(|| char_value(&arg_tokens, "short"));
    let default = quoted_value(&arg_tokens, "default_value")
        .or_else(|| quoted_value(&arg_tokens, "default_value_t"))
        .or_else(|| bare_value(&arg_tokens, "default_value_t"));
    CliOption {
        name,
        long,
        short,
        value_name: quoted_value(&arg_tokens, "value_name"),
        ty: type_to_string(&field.ty),
        default,
        required: has_bare_word(&arg_tokens, "required"),
        help: first_doc_paragraph(&field.attrs).unwrap_or_default(),
    }
}

fn method_params_type(method: &syn::ImplItemFn) -> Option<String> {
    method.sig.inputs.iter().find_map(|input| {
        let FnArg::Typed(pat_ty) = input else {
            return None;
        };
        let text = type_to_string(&pat_ty.ty);
        text.contains("Parameters").then_some(text)
    })
}

fn has_derive(attrs: &[syn::Attribute], derive_name: &str) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }
        let paths = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated);
        paths.is_ok_and(|paths| {
            paths
                .iter()
                .any(|path| path.segments.last().is_some_and(|segment| segment.ident == derive_name))
        })
    })
}

fn attr_tokens(attrs: &[syn::Attribute], attr_name: &str) -> Option<String> {
    attrs.iter().find_map(|attr| {
        attr.path()
            .is_ident(attr_name)
            .then(|| attr.meta.to_token_stream().to_string())
    })
}

fn command_name(attrs: &[syn::Attribute]) -> Option<String> {
    let tokens = attr_tokens(attrs, "command")?;
    quoted_value(&tokens, "name")
}

fn command_about(attrs: &[syn::Attribute]) -> Option<String> {
    let tokens = attr_tokens(attrs, "command")?;
    quoted_value(&tokens, "about").or_else(|| quoted_value(&tokens, "long_about"))
}

fn has_attr_word(attrs: &[syn::Attribute], attr_name: &str, word: &str) -> bool {
    attr_tokens(attrs, attr_name).is_some_and(|tokens| has_bare_word(&tokens, word))
}

fn first_doc_paragraph(attrs: &[syn::Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(meta) = &attr.meta
            && let syn::Expr::Lit(expr_lit) = &meta.value
            && let syn::Lit::Str(lit) = &expr_lit.lit
        {
            let line = lit.value().trim().to_string();
            if line.is_empty() {
                if !lines.is_empty() {
                    break;
                }
            } else {
                lines.push(line);
            }
        }
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

fn annotation_map(tokens: &str) -> BTreeMap<String, String> {
    let Some(start) = tokens.find("annotations") else {
        return BTreeMap::new();
    };
    let mut map = BTreeMap::new();
    let tail = &tokens[start..];
    for key in [
        "title",
        "read_only_hint",
        "destructive_hint",
        "idempotent_hint",
        "open_world_hint",
    ] {
        if let Some(value) = quoted_value(tail, key).or_else(|| bare_value(tail, key)) {
            map.insert(key.to_string(), value);
        }
    }
    map
}

fn quoted_value(tokens: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = \"");
    let start = tokens.find(&needle)? + needle.len();
    let rest = &tokens[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn char_value(tokens: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = '");
    let start = tokens.find(&needle)? + needle.len();
    let rest = &tokens[start..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn bare_value(tokens: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = ");
    let start = tokens.find(&needle)? + needle.len();
    let rest = &tokens[start..];
    let value: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .collect();
    (!value.is_empty()).then_some(value)
}

fn has_bare_word(tokens: &str, word: &str) -> bool {
    tokens
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|part| part == word)
}

fn type_last_ident(ty: &Type) -> Option<String> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(inner) = arg
                && let Some(name) = type_last_ident(inner)
            {
                return Some(name);
            }
        }
    }
    Some(segment.ident.to_string())
}

fn type_to_string<T: ToTokens + ?Sized>(ty: &T) -> String {
    let text = ty.to_token_stream().to_string();
    text.replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" , ", ", ")
        .replace("& '", "&'")
}

trait TitleCase {
    fn to_title_case(&self) -> String;
}

impl TitleCase for str {
    fn to_title_case(&self) -> String {
        let mut out = String::with_capacity(self.len());
        let mut uppercase = true;
        for ch in self.chars() {
            if ch == '_' || ch == '-' {
                out.push(' ');
                uppercase = true;
            } else if uppercase {
                out.extend(ch.to_uppercase());
                uppercase = false;
            } else {
                out.push(ch);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_mcp_tool_attribute() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mcp.rs");
        std::fs::write(
            &source,
            r#"
            struct Server;
            #[tool_router]
            impl Server {
                /// Fallback docs.
                #[tool(description = "Do work", annotations(title = "Do Work", read_only_hint = true))]
                async fn do_work(&self, Parameters(params): Parameters<crate::Params>) {}
            }
            "#,
        )
        .unwrap();
        let surface = extract_mcp_surface(&[source]).unwrap();
        assert_eq!(surface.tools.len(), 1);
        assert_eq!(surface.tools[0].name, "do_work");
        assert_eq!(surface.tools[0].description, "Do work");
        assert_eq!(surface.tools[0].title, "Do Work");
    }

    #[test]
    fn extracts_clap_parser_subcommands() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("cli.rs");
        std::fs::write(
            &source,
            r#"
            use clap::{Parser, Subcommand};
            #[derive(Parser)]
            #[command(name = "demo", about = "Demo CLI")]
            struct Cli {
                #[command(subcommand)]
                command: Commands,
            }
            #[derive(Subcommand)]
            enum Commands {
                /// Convert input.
                Convert {
                    /// Input file
                    input: String,
                    /// Output file
                    #[arg(short, long, value_name = "FILE")]
                    output: Option<String>,
                },
            }
            "#,
        )
        .unwrap();
        let surface = extract_cli_surface(&[source]).unwrap();
        assert_eq!(surface.commands[0].name, "demo");
        assert_eq!(surface.commands[0].subcommands[0].name, "convert");
        assert_eq!(
            surface.commands[0].subcommands[0].options[0].long.as_deref(),
            Some("output")
        );
        assert_eq!(surface.commands[0].subcommands[0].positionals[0].name, "input");
    }

    #[test]
    fn expands_command_flatten_in_enum_variant_commands() {
        // `#[command(flatten)]` on a struct-like enum-variant field (e.g. `extract`/`batch`
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("cli.rs");
        std::fs::write(
            &source,
            r#"
            use clap::{Args, Parser, Subcommand};
            #[derive(Parser)]
            #[command(name = "demo")]
            struct Cli {
                #[command(subcommand)]
                command: Commands,
            }
            #[derive(Subcommand)]
            enum Commands {
                /// Extract a document.
                Extract {
                    /// Document path.
                    path: String,
                    #[command(flatten)]
                    overrides: Overrides,
                },
            }
            #[derive(Args)]
            struct Overrides {
                /// Enable OCR.
                #[arg(long)]
                ocr: bool,
            }
            "#,
        )
        .unwrap();
        let surface = extract_cli_surface(&[source]).unwrap();
        let extract = &surface.commands[0].subcommands[0];
        assert_eq!(extract.name, "extract");
        assert!(
            extract.options.iter().any(|option| option.name == "ocr"),
            "flattened field `ocr` must be expanded inline, got options: {:?}",
            extract.options.iter().map(|option| &option.name).collect::<Vec<_>>()
        );
        assert!(
            !extract
                .options
                .iter()
                .chain(&extract.positionals)
                .any(|option| option.name == "overrides"),
            "flattened struct must not appear as an opaque `overrides` row"
        );
        assert_eq!(extract.positionals[0].name, "path");
    }
}
