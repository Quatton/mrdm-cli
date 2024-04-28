use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use config::Config;

use log::debug;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{BufReader, BufWriter, Write},
    str::FromStr,
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

    Done {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
struct TodoItem {
    title: String,
    category: String,
    path: std::path::PathBuf,
    line: usize,
    done: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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

fn get_todos_from_one_file(
    path: &std::path::Path,
    re: &Arc<Regex>,
    todo_items: &Arc<Mutex<TodoList>>,
    current_length: Arc<Mutex<usize>>,
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
                                let current_idx = *current_length.lock().unwrap();
                                *current_length.lock().unwrap() += 1;
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
                                done: false,
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

fn get_todos(
    pattern: Option<String>,
    path: Option<std::path::PathBuf>,
    cfg: &CliConfig,
    current_length: &Arc<Mutex<usize>>,
) -> Result<HashMap<String, TodoItem>> {
    let pattern = pattern.unwrap_or(
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

    let mut handles = vec![];

    let todo_items = Arc::new(Mutex::new(TodoList {
        items: std::collections::HashMap::new(),
    }));

    for path in paths {
        for entry in glob::glob(&path.to_string_lossy())? {
            match entry {
                Ok(path) => {
                    let todo_items = Arc::clone(&todo_items);
                    let re = Arc::clone(&re);
                    let current_length = Arc::clone(&current_length);
                    debug!("processing file: {}", path.display());
                    handles.push(thread::spawn(move || {
                        get_todos_from_one_file(&path, &re, &todo_items, current_length)
                    }));
                }
                Err(e) => eprintln!("error: {}", e),
            }
        }
    }

    for handle in handles {
        handle.join().unwrap()?;
    }

    // FIXME(4): just added this to fix the integrity of the hashmap
    // sorted hashmap
    let mut todo_maps = todo_items
        .lock()
        .unwrap()
        .items
        .clone()
        .into_iter()
        .collect::<Vec<_>>();

    todo_maps.sort_by_key(|(id, _)| id.clone());

    Ok(HashMap::from_iter(todo_maps))
}

macro_rules! write_todo_items {
    ($todo_items:expr, $outbuf:expr, $is_stdout:expr) => {
        for (id, item) in $todo_items.into_iter() {
            writeln!(
                $outbuf,
                "- [{}] {}({}): {} {}({}{}{})",
                if item.done { "x" } else { " " },
                item.category,
                id,
                item.title.trim(),
                if $is_stdout { "" } else { "[link]" },
                item.path.display(),
                if $is_stdout { ":" } else { "#L" },
                item.line,
            )?;
        }
    };
}

fn get_outbuf(
    out: Option<std::path::PathBuf>,
    cfg: &CliConfig,
) -> Result<(BufWriter<Box<dyn Write>>, bool)> {
    let out = out.or_else(|| cfg.out.clone());

    match out {
        Some(ref path) => {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .with_context(|| format!("could not open file `{}`", &path.display()))?;

            Ok((BufWriter::new(Box::new(file)), false))
        }
        None => Ok((BufWriter::new(Box::new(std::io::stdout())), true)),
    }
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
                TodoCommands::List { out, pattern, path } => {
                    let data_in = std::fs::OpenOptions::new()
                        .read(true)
                        .open(std::path::PathBuf::from_str(OUT_PATH).unwrap())
                        .with_context(|| format!("could not open file `{}`", &OUT_PATH))?;
                    let rdr = BufReader::new(data_in);

                    let prev_todo = serde_json::from_reader(rdr).unwrap_or_else(|_| TodoList {
                        items: std::collections::HashMap::new(),
                    });

                    let current_length = Arc::new(Mutex::new(prev_todo.items.len()));

                    let todo_items = get_todos(pattern, path, &cfg, &current_length)?;

                    let (mut outbuf, is_stdout) = get_outbuf(out, &cfg)?;
                    write_todo_items!(todo_items, outbuf, is_stdout);
                }
                TodoCommands::Done { pattern, path, out } => {
                    // if .mrdm directory does not exist, create it
                    std::fs::create_dir(".mrdm").ok();

                    // output the todo items to json
                    let data_out = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .open(
                            std::path::PathBuf::from_str(OUT_PATH)
                                .unwrap()
                                .with_extension("tmp"),
                        )
                        .with_context(|| format!("could not open file `{}`", &OUT_PATH))?;

                    let data_in = std::fs::OpenOptions::new()
                        .read(true)
                        .open(std::path::PathBuf::from_str(OUT_PATH).unwrap())
                        .with_context(|| format!("could not open file `{}`", &OUT_PATH))?;

                    let data_writer = BufWriter::new(data_out);

                    let rdr = BufReader::new(data_in);

                    let prev_todo = serde_json::from_reader(rdr).unwrap_or_else(|_| TodoList {
                        items: std::collections::HashMap::new(),
                    });

                    let prev_iter = prev_todo.items.clone().into_iter();

                    let current_length = Arc::new(Mutex::new(
                        // it's not the length but rather max id
                        prev_iter
                            .map(|(id, _)| id.parse::<usize>().unwrap_or(0))
                            .max()
                            .unwrap_or(0),
                    ));
                    let curr_todo = get_todos(pattern, path, &cfg, &current_length)?;

                    let (mut outbuf, is_stdout) = get_outbuf(out, &cfg)?;

                    let prev_done_keys: HashSet<String> = prev_todo
                        .items
                        .iter()
                        .filter(|(_, item)| item.done)
                        .map(|(id, _)| id.clone())
                        .collect();

                    let prev_not_done_keys: HashSet<String> = prev_todo
                        .items
                        .iter()
                        .filter(|(_, item)| !item.done)
                        .map(|(id, _)| id.clone())
                        .collect();

                    let curr_keys: HashSet<String> = curr_todo.keys().cloned().collect();

                    let deleted_keys = prev_not_done_keys.difference(&curr_keys);
                    let undone_keys = prev_done_keys.intersection(&curr_keys);

                    let mut final_todo = prev_todo
                        .items
                        .into_iter()
                        .chain(curr_todo.into_iter())
                        .collect::<HashMap<_, _>>();

                    let stdout = std::io::stdout();

                    let mut handle = stdout.lock();

                    // set status of done items to true
                    for key in deleted_keys {
                        if let Some(item) = final_todo.get_mut(key.as_str()) {
                            // prompt user to confirm deletion
                            let prompt = format!(
                                "This todo item was removed from your codebase:\n\
                                - [ ] {}: {} {}({}{}{})\n\
                                Do you want to mark it as done or remove it from the list? (d/r)",
                                item.category,
                                item.title.trim(),
                                if is_stdout { "" } else { "[link]" },
                                item.path.display(),
                                if is_stdout { ":" } else { "#L" },
                                item.line,
                            );

                            writeln!(handle, "{}", prompt)?;

                            handle.flush()?;

                            let mut input = String::new();
                            std::io::stdin().read_line(&mut input)?;

                            if input.trim().to_lowercase() == "d" {
                                item.done = true;
                            } else {
                                final_todo.remove(key.as_str());
                            }
                        }
                    }

                    // items that were done but are now undone
                    for key in undone_keys {
                        let length = final_todo.len();
                        if let Some(item) = final_todo.get_mut(key.as_str()) {
                            // prompt user to confirm deletion
                            let prompt = format!(
                                "This todo item was marked as done but is now undone:\n\
                                - [x] {}: {} {}({}{}{})\n\
                                Do you want to mark it as undone or recreate it? (u/r)",
                                item.category,
                                item.title.trim(),
                                if is_stdout { "" } else { "[link]" },
                                item.path.display(),
                                if is_stdout { ":" } else { "#L" },
                                item.line,
                            );

                            writeln!(handle, "{}", prompt)?;

                            handle.flush()?;

                            let mut input = String::new();
                            std::io::stdin().read_line(&mut input)?;

                            if input.trim().to_lowercase() == "u" {
                                item.done = false;
                            } else {
                                let id = format!("{}", length);
                                let cloned_item = item.clone();

                                final_todo.insert(id.clone(), cloned_item);
                            }
                        }
                    }

                    let mut final_todo = final_todo.into_iter().collect::<Vec<_>>();

                    final_todo.sort_by_key(|(id, _)| id.clone());

                    write_todo_items!(&final_todo, outbuf, is_stdout);

                    // write to file
                    serde_json::to_writer_pretty(
                        data_writer,
                        &TodoList {
                            items: final_todo.into_iter().collect::<HashMap<_, _>>(),
                        },
                    )
                    .with_context(|| format!("could not write to file `{}`", &OUT_PATH))?;

                    // overwrite the original file with the rewritten content
                    std::fs::rename(
                        std::path::PathBuf::from_str(OUT_PATH)
                            .unwrap()
                            .with_extension("tmp"),
                        std::path::PathBuf::from_str(OUT_PATH).unwrap(),
                    )
                    .with_context(|| {
                        format!("could not rename file `{}` to `{}`", &OUT_PATH, &OUT_PATH)
                    })?;
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
