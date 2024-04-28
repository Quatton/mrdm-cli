use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use config::Config;

use log::debug;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufWriter, Write},
    sync::{Arc, Mutex},
    thread,
};
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

    // TODO(0): `mrdm commit` collect TODOs into a change file and add an idempotency key so that you can move and rename

    // TODO(1): `mrdm commit` should help with committing with name and description
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
        // TODO(2): pattern should accept more tags like feat, fix, case-insensitive -> config file
        /// A comma separated pattern to search for in the TODOs
        /// example: "TODO,HACK,FIXME"
        #[arg(short)]
        pattern: Option<String>,

        /// The path to the file to search for TODOs
        path: Option<std::path::PathBuf>,

        /// Output file to write the TODOs to
        /// If not provided, it will write to stdout
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct CliConfig {
    patterns: Vec<String>,
    include: Vec<String>,
    out: Option<std::path::PathBuf>,
}

impl ::std::default::Default for CliConfig {
    fn default() -> Self {
        Self {
            patterns: vec!["TODO".to_string()],
            include: vec!["src/**/*".to_string()],
            out: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct TodoItem {
    title: String,
    category: String,
    path: std::path::PathBuf,
    line: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct TodoList {
    items: std::collections::HashMap<String, TodoItem>,
}

const CONFIG_PATH: &str = "mrdm.json";
const OUT_PATH: &str = ".mrdm/data.json";

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
    re: &Arc<Regex>,
    todo_items: &Arc<Mutex<TodoList>>,
) -> Result<()> {
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read file `{}`", &path.display()))?;

    let content_rewritten_buffer = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path.with_extension("tmp"))
        .with_context(|| format!("could not open file `{}`", &path.display()))?;

    let mut outbuf = BufWriter::new(Box::new(content_rewritten_buffer));

    // TODO(3): multiline support
    for (i, line) in content.lines().enumerate() {
        match re.captures(line) {
            Some(caps) => {
                let title = caps.name("title").unwrap().as_str();
                let category = caps.name("category").unwrap().as_str();
                // writeln!(
                //     outbuf,
                //     "- [ ] {}: {} ({}:{})",
                //     category,
                //     title.trim(),
                //     path.display(),
                //     i + 1,
                // )?;

                match todo_items.lock() {
                    Ok(mut todo_items) => {
                        let id = match caps.name("id") {
                            Some(id) => {
                                writeln!(
                                    outbuf,
                                    "{}",
                                    // as is
                                    line
                                )?;

                                id.as_str().to_string()
                            }
                            None => {
                                let current_idx = todo_items.items.len();
                                let id = format!("{}", current_idx);

                                writeln!(
                                    outbuf,
                                    "{}",
                                    re.replace(
                                        line,
                                        format!("$before// $category({}): $title", id)
                                    )
                                )?;

                                id
                            }
                        };

                        todo_items.items.insert(
                            format!("{}", id),
                            TodoItem {
                                title: title.to_string(),
                                category: category.to_string(),
                                path: path.to_path_buf(),
                                line: i + 1,
                            },
                        );
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!("could not lock todo_items: {}", e).into());
                    }
                }
            }
            None => match writeln!(outbuf, "{}", line) {
                Ok(_) => {}
                Err(e) => {
                    return Err(anyhow::anyhow!("could not write to temp file: {}", e).into());
                }
            },
        }
    }

    // overwrite the original file with the rewritten content
    std::fs::rename(path.with_extension("tmp"), path).with_context(|| {
        format!(
            "could not rename file `{}` to `{}`",
            &path.with_extension("tmp").display(),
            &path.display()
        )
    })?;

    Ok(())
}

fn create_regex(patterns: Vec<&str>) -> Result<Regex> {
    Regex::new(&format!(
        // TODO(5): escape even number of other quotation marks
        r#"^(?<before>[^"]*("[^"]*"[^"]*)*)//\s*(?<category>{})(\((?<id>\d+)\))?:\s*(?<title>.*)"#,
        patterns.join("|")
    ))
    .with_context(|| {
        format!(
            "could not create regex from pattern `{}`",
            patterns.join("|")
        )
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();
    let cfg = get_config();

    env_logger::init_from_env(env_logger::Env::new().default_filter_or("debug"));

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
                    out,
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

                    let re = Arc::new(create_regex(patterns).unwrap());

                    let paths = if let Some(path) = path {
                        vec![path]
                    } else {
                        cfg.include
                            .iter()
                            .map(|s| std::path::PathBuf::from(s))
                            .collect()
                    };

                    let out = if let Some(out) = out {
                        Some(out)
                    } else {
                        cfg.out
                    };

                    let mut outbuf: BufWriter<Box<dyn Write>> = match out {
                        Some(ref path) => {
                            let file = std::fs::OpenOptions::new()
                                .write(true)
                                .create(true)
                                .truncate(true)
                                .open(path)
                                .with_context(|| {
                                    format!("could not open file `{}`", &path.display())
                                })?;

                            std::io::BufWriter::new(Box::new(file))
                        }
                        None => std::io::BufWriter::new(Box::new(std::io::stdout())),
                    };

                    let is_stdout = out.is_none();

                    let todo_items = Arc::new(Mutex::new(TodoList {
                        items: std::collections::HashMap::new(),
                    }));

                    let mut handles = vec![];

                    for path in paths {
                        for entry in glob::glob(&path.to_string_lossy())? {
                            match entry {
                                Ok(path) => {
                                    let todo_items = Arc::clone(&todo_items);
                                    let re = Arc::clone(&re);
                                    debug!("processing file: {}", path.display());
                                    handles.push(thread::spawn(move || {
                                        list_todo(&path, &re, &todo_items)
                                    }));
                                }
                                Err(e) => eprintln!("error: {}", e),
                            }
                        }
                    }

                    for handle in handles {
                        handle.join().unwrap()?;
                    }

                    for (id, item) in todo_items.lock().unwrap().items.iter() {
                        writeln!(
                            outbuf,
                            "- [ ] {}({}): {} {}({}{}{})",
                            item.category,
                            id,
                            item.title.trim(),
                            if is_stdout { "" } else { "[link]" },
                            item.path.display(),
                            if is_stdout { ":" } else { "#L" },
                            item.line,
                        )?;
                    }

                    // if .mrdm directory does not exist, create it
                    std::fs::create_dir(".mrdm").ok();

                    // output the todo items to json
                    let data_out = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(OUT_PATH)
                        .with_context(|| format!("could not open file `{}`", &OUT_PATH))?;

                    let mut data_writer = BufWriter::new(data_out);

                    match todo_items.lock() {
                        Ok(todo_items) => {
                            serde_json::to_writer_pretty(&mut data_writer, &*todo_items)
                                .with_context(|| {
                                    format!("could not write to file `{}`", &OUT_PATH)
                                })?;
                        }
                        Err(e) => {
                            return Err(anyhow::anyhow!("could not lock todo_items: {}", e).into());
                        }
                    };
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex() {
        let re = create_regex(vec!["TODO", "FIXME"]).unwrap();

        let caps = re.captures("// TODO(6): test").unwrap();
        assert_eq!(caps.name("category").unwrap().as_str(), "TODO");
        assert_eq!(caps.name("title").unwrap().as_str(), "test");

        let caps = re.captures("// FIXME(2): test").unwrap();
        assert_eq!(caps.name("category").unwrap().as_str(), "FIXME");
        assert_eq!(caps.name("id").unwrap().as_str(), "2");
        assert_eq!(caps.name("title").unwrap().as_str(), "test");

        let caps = re
            .captures(
                r#"
            testing("// TODO: test");"#,
            )
            .is_none();

        assert_eq!(caps, true);
    }
}
