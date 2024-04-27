use anyhow::{Context, Result};
use clap::{command, Args, Parser, Subcommand};
use regex::Regex;
use std::io::Write;

#[derive(Debug, Parser)] // requires `derive` feature
#[command(name = "mrdm")]
#[command(about = "A //TODO list utility for in-code project management", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage TODOs in a file
    Todo(TodoArgs),
    // TODO: `mrdm init` add a config file and path will detect project directory instead

    // TODO: `mrdm commit` collect TODOs into a change file and add an idempotency key so that you can move and rename

    // TODO: `mrdm commit` should help with committing with name and description
}

#[derive(Debug, Args)]
struct TodoArgs {
    #[command(subcommand)]
    command: TodoCommands,
}

#[derive(Debug, Subcommand)]
enum TodoCommands {
    /// List TODOs in a file
    List {
        // TODO: pattern should accept more tags like feat, fix, case-insensitive -> config file
        /// A pattern to search for in the TODOs
        /// default: "TODO"
        #[arg(short)]
        pattern: Option<String>,

        /// The path to the file to search for TODOs
        path: std::path::PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();
    let stdout = std::io::stdout();
    let mut handle = std::io::BufWriter::new(stdout);

    match args.command {
        Commands::Todo(todo_args) => {
            let todo_cmd = todo_args.command;

            match todo_cmd {
                TodoCommands::List {
                    pattern: _pattern,
                    path,
                } => {
                    let content = std::fs::read_to_string(&path)
                        .with_context(|| format!("could not read file `{}`", &path.display()))?;

                    let pattern = _pattern.unwrap_or("TODO".to_string());
                    // start with ^
                    // match anything not // until first // {pattern}:
                    // match anything after until end of line
                    let re =
                        Regex::new(&format!(r"[^/]*(?<todo>//\s*{}:\s*(?<title>.*))", pattern))
                            .unwrap();

                    // TODO: multiline support
                    for (i, line) in content.lines().enumerate() {
                        match re.captures(line) {
                            Some(caps) => {
                                let title = caps.name("title").unwrap().as_str();
                                writeln!(
                                    handle,
                                    "({}:{}) {}",
                                    path.display(),
                                    i + 1,
                                    title.trim()
                                )?;
                            }
                            None => {}
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
