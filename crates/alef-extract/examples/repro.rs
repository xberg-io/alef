fn main() {
    let sources: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<&std::path::Path> = sources.iter().map(|s| std::path::Path::new(s)).collect();
    let surface = alef_extract::extractor::extract(&paths, "kreuzcrawl", "0.0.0", None).unwrap();
    println!("total types: {}", surface.types.len());
    for t in &surface.types {
        if t.name.contains("Batch") || t.name == "ScrapeResult" || t.name == "CrawlResult" {
            println!(
                "type: name={} rust_path={} fields={} opaque={}",
                t.name,
                t.rust_path,
                t.fields.len(),
                t.is_opaque
            );
        }
    }
    // Show duplicates by name
    let mut by_name: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for t in &surface.types {
        by_name.entry(t.name.clone()).or_default().push(t.rust_path.clone());
    }
    for (name, paths) in by_name.iter() {
        if paths.len() > 1 {
            println!("DUPLICATE type: {} → {:?}", name, paths);
        }
    }
    for f in &surface.functions {
        if f.name.starts_with("batch") {
            println!("fn: name={} return={:?}", f.name, f.return_type);
        }
    }
}
