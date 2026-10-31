pub(super) fn replace_go_options(args: &str) -> String {
    if let Some(prefix) = args.strip_suffix(", nil") {
        format!("{prefix}, opts")
    } else if args.is_empty() {
        "opts".to_string()
    } else {
        format!("{args}, opts")
    }
}

pub(super) fn snippet_setup_line(line: String) -> String {
    line.lines()
        .map(|part| {
            if part.contains("t.Fatalf(") {
                format!("{})", part.replace("t.Fatalf(", "panic(fmt.Sprintf("))
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
