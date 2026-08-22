mod engine;
mod error;

use std::path::PathBuf;
use std::process;

use clap::Parser;
use engine::{query, SearchHit};
use error::SearchError;
use serde::Serialize;

#[derive(Parser, Debug)]
#[command(
    name = "hermes-docs-search",
    version,
    about = "BM25 full-text search over the local Hermes Agent documentation"
)]
struct Cli {
    /// The search query.
    query: String,

    /// Number of results to return (default: 5).
    #[arg(short = 'k', long = "top-k", default_value_t = 5, value_name = "N")]
    top_k: usize,

    /// Emit machine-readable JSON instead of the default markdown template.
    #[arg(long)]
    json: bool,

    /// Force a full reindex before answering (ignore cached state).
    #[arg(long)]
    reindex: bool,

    /// Explicit docs root (highest-priority docs-root override).
    #[arg(long, value_name = "PATH")]
    docs_path: Option<PathBuf>,

    /// Directory for state.json and the index (default: $XDG_CACHE_HOME/hermes-docs-search or ~/.cache/hermes-docs-search).
    #[arg(long, value_name = "PATH")]
    cache_dir: Option<PathBuf>,
}

#[derive(Serialize)]
struct JsonOutput {
    query: String,
    docs_path: String,
    results: Vec<SearchHit>,
}

fn main() {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(()) => {}
        Err(e) => {
            let exit_code = match &e {
                SearchError::DocsNotFound(_) => 1,
                _ => 2,
            };
            eprintln!("error: {e}");
            process::exit(exit_code);
        }
    }
}

fn run(cli: &Cli) -> Result<(), SearchError> {
    let docs_root = engine::resolve_docs_root(cli.docs_path.as_deref())?;
    let cache_dir = engine::resolve_cache_dir(cli.cache_dir.as_deref());

    if cli.reindex {
        engine::reindex(&docs_root, &cache_dir)?;
    } else {
        engine::ensure_indexed(&docs_root, &cache_dir)?;
    }

    let hits = query(&cache_dir, &cli.query, cli.top_k)?;

    if cli.json {
        let out = JsonOutput {
            query: cli.query.clone(),
            docs_path: docs_root.to_string_lossy().into_owned(),
            results: hits,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_markdown(&cli.query, &hits);
    }

    Ok(())
}

fn print_markdown(query: &str, hits: &[SearchHit]) {
    println!("# Hermes docs search: {query}");
    if hits.is_empty() {
        println!();
        println!("_No hits._");
        return;
    }
    for (i, hit) in hits.iter().enumerate() {
        let rank = i + 1;
        println!();
        println!("## {}. {} — {}", rank, hit.path, hit.heading);
        println!("URL: {}", hit.url);
        println!("Score: {:.4}", hit.score);
        println!();
        println!("{}", hit.body);
    }
}

impl std::convert::From<serde_json::Error> for SearchError {
    fn from(e: serde_json::Error) -> Self {
        SearchError::Internal(e.to_string())
    }
}
