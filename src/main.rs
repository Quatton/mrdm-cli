use anyhow::{Context, Result};
use clap::Parser;

// TODO: change it to `mrdm todo list`
#[derive(Parser)]
struct Cli {
    /// The pattern to look for
    pattern: String,
    /// The path to the file to read
    path: std::path::PathBuf,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    let content = std::fs::read_to_string(&args.path)
        .with_context(|| format!("could not read file `{}`", args.path.display()))?;
    for (i, line) in content.lines().enumerate() {
        if line.contains(&args.pattern) {
            println!("{}: {}", i, line.trim());
        }
    }
    Ok(())
}
