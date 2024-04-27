use anyhow::{Context, Result};
use clap::{command, Args, Parser, Subcommand};
use config::Config;

use regex::Regex;
use serde_derive::{Deserialize, Serialize};
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

    // TODO: `mrdm commit` collect TODOs into a change file and add an idempotency key so that you can move and rename

    // TODO: `mrdm commit` should help with committing with name and description
    Init,
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
        /// A comma separated pattern to search for in the TODOs
        /// example: "TODO,HACK,FIXME"
        #[arg(short)]
        pattern: Option<String>,

        /// The path to the file to search for TODOs
        path: Option<std::path::PathBuf>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct CliConfig {
    patterns: Vec<String>,
    include: Vec<String>,
}

impl ::std::default::Default for CliConfig {
    fn default() -> Self {
        Self {
            patterns: vec!["TODO".to_string()],
            include: vec!["src/**/*".to_string()],
        }
    }
}

const CONFIG_PATH: &str = "mrdm.json";

fn get_config() -> CliConfig {
    // this will never error, if it does, then default config will be used
    if let Ok(current_dir) = std::env::current_dir() {
        let config_path = current_dir.join(CONFIG_PATH);

        if config_path.exists() {
            let file = config::File::new(config_path.to_str().unwrap(), config::FileFormat::Json);
            let settings = Config::builder()
                .add_source(file.required(false))
                .build()
                .unwrap();

            let settings: CliConfig = settings.try_deserialize().unwrap();

            return settings;
        }
    }

    CliConfig::default()
}

fn list_todo(
    path: &std::path::Path,
    re: &Regex,
    // any buffer that implements `std::io::Write`
    outbuf: &mut std::io::BufWriter<dyn std::io::Write>,
) -> Result<()> {
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read file `{}`", &path.display()))?;

    // TODO: multiline support
    for (i, line) in content.lines().enumerate() {
        match re.captures(line) {
            Some(caps) => {
                let title = caps.name("title").unwrap().as_str();
                let category = caps.name("category").unwrap().as_str();
                writeln!(
                    outbuf,
                    "({}:{}) {}: {}",
                    path.display(),
                    i + 1,
                    category,
                    title.trim()
                )?;
            }
            None => {}
        }
    }

    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();
    let stdout = std::io::stdout();
    let mut outbuf = std::io::BufWriter::new(stdout);
    let cfg = get_config();

    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    match args.command {
        Commands::Init => {
            // detect current directory
            let current_dir = std::env::current_dir()?;

            // make a mrdm.json file
            let config_path = current_dir.join(CONFIG_PATH);

            if config_path.exists() {
                // if file exists, then error as it should not be overwritten
                return Err(anyhow::anyhow!(
                    "config file `{}` already exists",
                    &config_path.display()
                )
                .into());
            }

            // write default config copied from ./config/mrdm.json
            let default_config = include_str!("./config/mrdm.json");

            std::fs::write(&config_path, default_config)
                .with_context(|| format!("could not write file `{}`", &config_path.display()))?;
        }
        Commands::Todo(todo_args) => {
            let todo_cmd = todo_args.command;

            match todo_cmd {
                TodoCommands::List {
                    pattern: _pattern,
                    path,
                } => {
                    let pattern = _pattern.unwrap_or(
                        cfg.patterns
                            .iter()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    let patterns = pattern.split(',').collect::<Vec<_>>();

                    let re = Regex::new(&format!(
                        r"[^/]*(?<todo>//\s*{}:\s*(?<title>.*))",
                        format!("(?<category>{})", patterns.join("|"))
                    ))
                    .unwrap();

                    let paths = if let Some(path) = path {
                        vec![path]
                    } else {
                        cfg.include
                            .iter()
                            .map(|s| std::path::PathBuf::from(s))
                            .collect()
                    };

                    for path in paths {
                        for entry in glob::glob(&path.to_string_lossy())? {
                            match entry {
                                Ok(path) => {
                                    list_todo(&path, &re, &mut outbuf)?;
                                }
                                Err(e) => eprintln!("error: {}", e),
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
