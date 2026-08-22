//! Core BM25 index and query logic for hermes-docs-search.

use std::path::{Path, PathBuf};

use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{OwnedValue, Schema, TextOptions};
use tantivy::{doc, Index, IndexWriter};
use walkdir::WalkDir;

use crate::error::SearchError;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SearchHit {
    pub path: String,
    pub heading: String,
    pub url: String,
    pub score: f32,
    pub body: String,
}

const BASE_URL: &str = "https://hermes-agent.nousresearch.com/docs";

/// Resolve the docs root directory.
///
/// Resolution order:
/// 1. `explicit` (from `--docs-path`)
/// 2. `$HERMES_DOCS_PATH`
/// 3. `$HERMES_HOME/hermes-agent/website/docs`
/// 4. `~/.hermes/hermes-agent/website/docs`
pub fn resolve_docs_root(explicit: Option<&Path>) -> Result<PathBuf, SearchError> {
    if let Some(p) = explicit {
        if !p.is_dir() {
            return Err(SearchError::DocsNotFound(format!(
                "no docs root found at '{}'",
                p.display()
            )));
        }
        return Ok(p.to_path_buf());
    }

    if let Ok(p) = std::env::var("HERMES_DOCS_PATH") {
        let path = PathBuf::from(p);
        if path.is_dir() {
            return Ok(path);
        }
    }

    if let Ok(home) = std::env::var("HERMES_HOME") {
        let p = Path::new(&home)
            .join("hermes-agent")
            .join("website")
            .join("docs");
        if p.is_dir() {
            return Ok(p);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let p = Path::new(&home)
            .join(".hermes")
            .join("hermes-agent")
            .join("website")
            .join("docs");
        if p.is_dir() {
            return Ok(p);
        }
    }

    Err(SearchError::DocsNotFound(
        "no docs root found (checked --docs-path, $HERMES_DOCS_PATH, $HERMES_HOME, ~/.hermes)".into(),
    ))
}

/// Determine the cache directory.
pub fn resolve_cache_dir(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return Path::new(&xdg).join("hermes-docs-search");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    Path::new(&home).join(".cache").join("hermes-docs-search")
}

fn index_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("index")
}

fn state_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("state.json")
}

/// Compute a deterministic fingerprint of the docs tree (path + mtime + size).
fn docs_fingerprint(docs_root: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let mut files: Vec<PathBuf> = WalkDir::new(docs_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|ext| ext == "md" || ext == "mdx")
                    .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();

    for p in files {
        p.to_string_lossy().hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(&p) {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos());
            mtime.hash(&mut hasher);
            meta.len().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn build_schema() -> Schema {
    let mut schema = Schema::builder();
    let text_opt = TextOptions::default()
        .set_indexing_options(
            tantivy::schema::TextFieldIndexing::default()
                .set_tokenizer("default")
                .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let _path = schema.add_text_field("path", text_opt.clone());
    let _heading = schema.add_text_field("heading", text_opt.clone());
    let _body = schema.add_text_field("body", text_opt);
    schema.build()
}

/// Split markdown content on `##` and `###` headings, stripping light MDX import noise.
fn chunk_markdown(content: &str, rel_path: &str) -> Vec<(String, String)> {
    let mut chunks: Vec<(String, String)> = Vec::new();
    let mut heading = String::new();
    let mut body_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
            let body = body_lines.join("\n").trim().to_string();
            if !body.is_empty() || !heading.is_empty() {
                if !body.is_empty() {
                    chunks.push((heading.clone(), body));
                }
            }
            heading = trimmed.trim_start_matches('#').trim().to_string();
            body_lines.clear();
        } else {
            let t = trimmed.trim();
            if t.starts_with("import ")
                || t.starts_with("export ")
                || t.starts_with("export default ")
            {
                continue;
            }
            body_lines.push(line);
        }
    }

    let body = body_lines.join("\n").trim().to_string();
    if !body.is_empty() {
        if heading.is_empty() {
            heading = Path::new(rel_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(rel_path)
                .to_string();
        }
        chunks.push((heading, body));
    }

    if chunks.is_empty() {
        let heading = content
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l[2..].trim().to_string())
            .unwrap_or_else(|| {
                Path::new(rel_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(rel_path)
                    .to_string()
            });
        chunks.push((heading, content.trim().to_string()));
    }

    chunks
}

fn url_for_relpath(rel: &str) -> String {
    let no_ext = rel
        .strip_suffix(".md")
        .or_else(|| rel.strip_suffix(".mdx"))
        .unwrap_or(rel);
    format!("{BASE_URL}/{no_ext}")
}

/// Get the Hermes git SHA and version string.
fn hermes_identity(docs_root: &Path) -> (String, String) {
    let hermes_root = docs_root
        .parent() // docs
        .and_then(|p| p.parent()) // website
        .and_then(|p| p.parent()); // hermes-agent

    let sha = hermes_root
        .and_then(|r| {
            std::process::Command::new("git")
                .args(["-C", &r.to_string_lossy(), "rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_default();

    let version = std::process::Command::new("hermes")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    (sha, version)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct State {
    hermes_git_sha: String,
    hermes_version: String,
    docs_path: String,
    indexed_at: String,
    #[serde(default)]
    fingerprint: u64,
}

fn load_state(cache_dir: &Path) -> Option<State> {
    let data = std::fs::read_to_string(state_path(cache_dir)).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_state(cache_dir: &Path, docs_root: &Path) -> Result<(), SearchError> {
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| SearchError::Internal(format!("failed to create cache dir: {e}")))?;
    let (sha, version) = hermes_identity(docs_root);
    let fingerprint = docs_fingerprint(docs_root);
    let state = State {
        hermes_git_sha: sha,
        hermes_version: version,
        docs_path: docs_root.to_string_lossy().to_string(),
        indexed_at: chrono::Utc::now().to_rfc3339(),
        fingerprint,
    };
    let json = serde_json::to_string_pretty(&state)
        .map_err(|e| SearchError::Internal(e.to_string()))?;
    std::fs::write(state_path(cache_dir), json)
        .map_err(|e| SearchError::Internal(format!("failed to write state: {e}")))?;
    Ok(())
}

fn index_docs(docs_root: &Path, cache_dir: &Path) -> Result<(), SearchError> {
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| SearchError::Internal(format!("failed to create cache dir: {e}")))?;

    let idx_dir = index_path(cache_dir);
    let _ = std::fs::remove_dir_all(&idx_dir);
    std::fs::create_dir_all(&idx_dir)
        .map_err(|e| SearchError::Internal(format!("failed to create index dir: {e}")))?;

    let schema = build_schema();
    let mmap_dir = MmapDirectory::open(&idx_dir)
        .map_err(|e| SearchError::Internal(format!("failed to open index dir: {e}")))?;
    let index = Index::open_or_create(mmap_dir, schema.clone())
        .map_err(|e| SearchError::Internal(format!("failed to open index: {e}")))?;

    let f_path = schema.get_field("path").unwrap();
    let f_heading = schema.get_field("heading").unwrap();
    let f_body = schema.get_field("body").unwrap();

    let mut writer: IndexWriter = index
        .writer(50_000_000)
        .map_err(|e| SearchError::Internal(format!("failed to create writer: {e}")))?;

    let mut doc_count = 0u64;

    for entry in WalkDir::new(docs_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext == "md" || ext == "mdx")
            .unwrap_or(false);
        if !path.is_file() || !is_md {
            continue;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let rel = path
            .strip_prefix(docs_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        for (heading, body) in chunk_markdown(&content, &rel) {
            let d = doc!(
                f_path => rel.clone(),
                f_heading => heading.clone(),
                f_body => body.clone(),
            );
            if writer.add_document(d).is_err() {
                return Err(SearchError::Internal(format!("failed to index {rel}")));
            }
            doc_count += 1;
        }
    }

    writer
        .commit()
        .map_err(|e| SearchError::Internal(format!("failed to commit index: {e}")))?;

    eprintln!("indexed {doc_count} chunks from {}", docs_root.display());
    Ok(())
}

/// Ensure the index exists and is up-to-date. Rebuilds if state differs.
pub fn ensure_indexed(docs_root: &Path, cache_dir: &Path) -> Result<(), SearchError> {
    let needs_reindex = match load_state(cache_dir) {
        None => true,
        Some(s) => {
            let current_fingerprint = docs_fingerprint(docs_root);
            let current_sha = hermes_identity(docs_root).0;
            let docs_match = s.docs_path == docs_root.to_string_lossy().to_string();
            let sha_match = current_sha.is_empty() || s.hermes_git_sha == current_sha;
            let fingerprint_match = s.fingerprint == current_fingerprint;
            !(docs_match && sha_match && fingerprint_match)
        }
    };

    if needs_reindex {
        eprintln!("reindexing docs at {}", docs_root.display());
        reindex(docs_root, cache_dir)?;
    }
    Ok(())
}

/// Force a full reindex.
pub fn reindex(docs_root: &Path, cache_dir: &Path) -> Result<(), SearchError> {
    index_docs(docs_root, cache_dir)?;
    save_state(cache_dir, docs_root)?;
    Ok(())
}

/// Run a BM25 query and return top-k hits.
pub fn query(
    cache_dir: &Path,
    query_str: &str,
    top_k: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let mmap_dir = MmapDirectory::open(index_path(cache_dir))
        .map_err(|e| SearchError::Internal(format!("failed to open index dir: {e}")))?;
    let index = Index::open(mmap_dir)
        .map_err(|e| SearchError::Internal(format!("failed to open index: {e}")))?;
    let reader = index
        .reader()
        .map_err(|e| SearchError::Internal(format!("failed to open reader: {e}")))?;
    let searcher = reader.searcher();

    let schema = index.schema();
    let f_path = schema.get_field("path").unwrap();
    let f_heading = schema.get_field("heading").unwrap();
    let f_body = schema.get_field("body").unwrap();

    let query_parser = QueryParser::for_index(&index, vec![f_body, f_heading]);
    let q = query_parser
        .parse_query(query_str)
        .map_err(|e| SearchError::Internal(format!("parse query failed: {e}")))?;

    let top_hits: Vec<(f32, tantivy::DocAddress)> = searcher
        .search(&q, &TopDocs::with_limit(top_k.max(1)))
        .map_err(|e| SearchError::Internal(format!("search failed: {e}")))?;

    let mut results = Vec::new();
    for (score, doc_addr) in top_hits {
        let doc: tantivy::TantivyDocument = searcher
            .doc(doc_addr)
            .map_err(|e| SearchError::Internal(e.to_string()))?;
        let path = doc
            .get_first(f_path)
            .and_then(|v| match v {
                OwnedValue::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("")
            .to_string();
        let heading = doc.get_first(f_heading).and_then(|v| match v {
                OwnedValue::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("")
            .to_string();
        let body = doc
            .get_first(f_body)
            .and_then(|v| match v {
                OwnedValue::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("")
            .to_string();

        results.push(SearchHit {
            url: url_for_relpath(&path),
            path,
            heading,
            score,
            body,
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_fixtures(dir: &Path) {
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::write(
            dir.join("config/configuration.md"),
            r#"# Configuration

## Configuration Files

Hermes uses a configuration file at ~/.hermes/config.yaml to store settings.
The configuration file controls model selection, API keys, and behavior.

## Environment Variables

You can override configuration via environment variables like HERMES_MODEL
and HERMES_API_KEY.
"#,
        )
        .unwrap();

        fs::create_dir_all(dir.join("tools")).unwrap();
        fs::write(
            dir.join("tools/toolsets.md"),
            r#"# Toolsets

## Available Toolsets

Hermes provides several toolsets including file operations, web search,
and code execution.

### Web Search Toolset

The web search toolset allows the agent to search the internet and retrieve
results. Configure it via the toolsets section in config.
"#,
        )
        .unwrap();

        fs::create_dir_all(dir.join("reference")).unwrap();
        fs::write(
            dir.join("reference/env.md"),
            r#"# Environment

## Environment Variables

The HERMES_HOME environment variable sets the base directory.
HERMES_DOCS_PATH can override where docs are read from.
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_resolve_docs_root_explicit() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        let root = resolve_docs_root(Some(&docs)).unwrap();
        assert_eq!(root, docs);
    }

    #[test]
    fn test_resolve_docs_root_missing() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nonexistent");
        let result = resolve_docs_root(Some(&missing));
        assert!(result.is_err());
        match result.unwrap_err() {
            SearchError::DocsNotFound(msg) => assert!(msg.contains("no docs root found")),
            other => panic!("expected DocsNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_chunk_markdown() {
        let content =
            "# Title\n\nIntro text.\n\n## Section A\n\nContent A.\n\n### Sub A\n\nContent sub.\n\n## Section B\n\nContent B.";
        let chunks = chunk_markdown(content, "test.md");
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks, got {}",
            chunks.len()
        );
        let headings: Vec<&str> = chunks.iter().map(|(h, _)| h.as_str()).collect();
        assert!(headings.contains(&"Section A"));
        assert!(headings.contains(&"Section B"));
    }

    #[test]
    fn test_chunk_markdown_strips_mdx_imports() {
        let content = r#"## Section
import { Component } from '@nextra/ui'
Some text here.
export const meta = { title: "Test" }
More text."#;
        let chunks = chunk_markdown(content, "test.mdx");
        let body: String = chunks
            .iter()
            .map(|(_, b)| b.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!body.contains("import { Component }"));
        assert!(!body.contains("export const"));
        assert!(body.contains("Some text here."));
    }

    #[test]
    fn test_index_and_query() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");
        fs::create_dir_all(&cache).unwrap();

        reindex(&docs, &cache).unwrap();

        let results = query(&cache, "configuration", 3).unwrap();
        assert!(!results.is_empty(), "expected results for 'configuration'");
        assert!(results[0].path.contains("config"));
        assert!(results[0].url.contains("hermes-agent.nousresearch.com/docs"));
    }

    #[test]
    fn test_index_and_query_no_hits() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");
        fs::create_dir_all(&cache).unwrap();

        reindex(&docs, &cache).unwrap();

        let results = query(&cache, "zzzzzqqqqqqxxx", 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_reindex_updates_state() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");

        reindex(&docs, &cache).unwrap();

        let state_file = state_path(&cache);
        assert!(state_file.exists());
        let content = fs::read_to_string(&state_file).unwrap();
        let state: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(state["docs_path"], docs.to_string_lossy().to_string());
        assert!(state["indexed_at"].as_str().unwrap().contains('T'));

        // Corrupt the fingerprint to force reindex
        let lines: Vec<&str> = content.lines().collect();
        let mut new_lines: Vec<String> = Vec::new();
        for line in lines {
            if line.contains("\"fingerprint\"") {
                new_lines.push(String::from("  \"fingerprint\": 0"));
            } else {
                new_lines.push(line.to_string());
            }
        }
        let new_content = new_lines.join("\n");
        fs::write(&state_file, new_content).unwrap();

        ensure_indexed(&docs, &cache).unwrap();
        let content2 = fs::read_to_string(&state_file).unwrap();
        let state2: serde_json::Value = serde_json::from_str(&content2).unwrap();
        assert_ne!(state2["fingerprint"], 0u64);
    }

    #[test]
    fn test_url_for_relpath() {
        assert_eq!(
            url_for_relpath("config/configuration.md"),
            "https://hermes-agent.nousresearch.com/docs/config/configuration"
        );
        assert_eq!(
            url_for_relpath("tools/toolsets.mdx"),
            "https://hermes-agent.nousresearch.com/docs/tools/toolsets"
        );
    }
}
