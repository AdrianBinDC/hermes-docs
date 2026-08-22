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
    pub rank: usize,
    pub category: String,
    pub path: String,
    pub heading: String,
    pub url: String,
    /// Raw BM25 score from Tantivy (higher = stronger lexical match).
    pub score: f32,
    pub body: String,
    /// Length of `body` in bytes.
    pub body_bytes: usize,
    /// True when `body_bytes` exceeds `OVERSIZED_CHUNK_BYTES`.
    pub oversized: bool,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct CategoryHits {
    pub category: String,
    pub hits: Vec<SearchHit>,
}

/// Retrieval quality signals for the caller (not ranking instructions).
#[derive(serde::Serialize, Clone, Debug)]
pub struct QuerySignals {
    pub max_bm25: f32,
    pub hit_count: usize,
    pub query_tokens: Vec<String>,
    /// Query tokens excluding brand-only noise (`hermes`, `agent`, …).
    pub content_tokens: Vec<String>,
    pub matched_content_tokens: Vec<String>,
    /// `matched_content_tokens.len() / content_tokens.len()`, or `1.0` when
    /// there are no content tokens (brand-only / stopword-only query).
    pub token_coverage: f32,
    /// False when content tokens exist but not all appear in retrieved hits.
    pub on_topic: bool,
    /// True when the index is empty (docs root resolved but no chunks were
    /// indexed). Distinguishes "no docs content" from "off-topic query".
    pub docs_empty: bool,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct QueryOutput {
    pub signals: QuerySignals,
    pub categories: Vec<CategoryHits>,
}

const BASE_URL: &str = "https://hermes-agent.nousresearch.com/docs";

/// Chunks whose body exceeds this are flagged `oversized` in JSON output.
const OVERSIZED_CHUNK_BYTES: usize = 4096;

/// Strip from BM25 query — question glue words only.
fn is_search_noise_token(t: &str) -> bool {
    matches!(
        t,
        "how"
            | "do"
            | "i"
            | "a"
            | "an"
            | "the"
            | "to"
            | "for"
            | "of"
            | "and"
            | "or"
            | "in"
            | "on"
            | "is"
            | "are"
            | "with"
            | "my"
            | "me"
            | "what"
            | "does"
            | "can"
            | "please"
            | "help"
            | "many"
            | "much"
            | "have"
            | "has"
            | "her"
            | "his"
            | "their"
            | "our"
            | "your"
            | "into"
            | "from"
            | "set"
            | "up"
            | "setup"
    )
}

/// Strip from `on_topic` / content-token signals — includes generic config verbs.
fn is_signal_noise_token(t: &str) -> bool {
    is_search_noise_token(t)
        || matches!(
            t,
            "set"
                | "up"
                | "setup"
                | "configure"
                | "configuration"
                | "default"
                | "main"
                | "use"
                | "get"
                | "make"
                | "new"
                | "all"
                | "any"
        )
}

fn is_brand_token(t: &str) -> bool {
    matches!(
        t,
        "hermes" | "agent" | "nous" | "nousresearch" | "docs" | "documentation"
    )
}

fn tokens_filtered(cleaned: &str, noise: fn(&str) -> bool) -> Vec<String> {
    cleaned
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| t.len() >= 3 && !noise(t))
        .collect()
}

fn query_tokens(cleaned: &str) -> Vec<String> {
    tokens_filtered(cleaned, is_signal_noise_token)
}

/// Light query expansion for BM25 (not ranking overrides).
fn expand_search_tokens(cleaned: &str, mut tokens: Vec<String>) -> Vec<String> {
    let lower = cleaned.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    for (verb, ing) in [("set", "setting"), ("configure", "configuring")] {
        if words.contains(&verb) && !tokens.iter().any(|t| t == ing) {
            tokens.push(ing.to_string());
        }
    }

    // Docs often say "main model" where users say "default model".
    if tokens.iter().any(|t| t == "default")
        && tokens.iter().any(|t| t == "model")
        && !tokens.iter().any(|t| t == "main")
    {
        tokens.push("main".to_string());
    }

    tokens
}

/// BM25 query string: search tokens (milder stopword list than signals).
fn search_query(cleaned: &str, raw: &str) -> String {
    let tokens = expand_search_tokens(cleaned, tokens_filtered(cleaned, is_search_noise_token));
    if tokens.is_empty() {
        if cleaned.is_empty() {
            escape_user_query(raw)
        } else {
            cleaned.to_string()
        }
    } else {
        tokens.join(" ")
    }
}

fn content_tokens(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .filter(|t| !is_brand_token(t))
        .cloned()
        .collect()
}

fn category_for_path(path: &str) -> String {
    path.split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("other")
        .to_string()
}

/// Bucket order in JSON output. `guides` is supplementary — listed after canonical sections.
fn category_bucket_priority(name: &str) -> u8 {
    match name {
        "getting-started" => 0,
        "user-guide" => 1,
        "reference" => 2,
        "integrations" => 3,
        "developer-guide" => 4,
        "guides" => 10,
        _ => 5,
    }
}

fn category_hit_cap(category: &str, per_cat: usize) -> usize {
    if category == "guides" {
        per_cat.min(2)
    } else {
        per_cat
    }
}

fn hit_text_blob(path: &str, heading: &str, body: &str) -> String {
    format!("{path}\n{heading}\n{body}").to_lowercase()
}

fn matched_content_tokens(content: &[String], hits: &[SearchHit]) -> Vec<String> {
    if content.is_empty() || hits.is_empty() {
        return Vec::new();
    }
    let blob: String = hits
        .iter()
        .map(|h| hit_text_blob(&h.path, &h.heading, &h.body))
        .collect::<Vec<_>>()
        .join("\n");
    content
        .iter()
        .filter(|t| blob.contains(t.as_str()))
        .cloned()
        .collect()
}

/// Flatten category buckets into score-sorted hits (for tests / markdown).
pub fn flat_hits(out: &QueryOutput) -> Vec<&SearchHit> {
    let mut hits: Vec<&SearchHit> = out.categories.iter().flat_map(|c| c.hits.iter()).collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits
}

const DIVERSITY_TOP_PATHS: usize = 5;
const DIVERSITY_CHUNKS_PER_PATH: usize = 5;
const DIVERSITY_GLOBAL_TOP: usize = 15;
/// Minimum path representation before score-order fill (avoids one doc monopolizing cap).
const DIVERSITY_SEAT_PATHS: usize = 2;

/// Merge global top hits with up to K chunks from each of the top P paths per category.
fn diversify_hits(scored: &[SearchHit]) -> Vec<SearchHit> {
    if scored.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<SearchHit> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    let mut push = |hit: SearchHit| {
        let key = (hit.path.clone(), hit.heading.clone());
        if seen.insert(key) {
            out.push(hit);
        }
    };

    for hit in scored.iter().take(DIVERSITY_GLOBAL_TOP) {
        push(hit.clone());
    }

    let mut by_category: std::collections::HashMap<String, Vec<SearchHit>> =
        std::collections::HashMap::new();
    for hit in scored {
        by_category
            .entry(hit.category.clone())
            .or_default()
            .push(hit.clone());
    }

    for cat_hits in by_category.into_values() {
        let mut by_path: std::collections::HashMap<String, Vec<SearchHit>> =
            std::collections::HashMap::new();
        for hit in cat_hits {
            by_path.entry(hit.path.clone()).or_default().push(hit);
        }

        let mut path_peaks: Vec<(String, f32)> = by_path
            .iter()
            .map(|(path, hits)| {
                let peak = hits.iter().map(|h| h.score).fold(0.0_f32, f32::max);
                (path.clone(), peak)
            })
            .collect();
        path_peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (path, _) in path_peaks.into_iter().take(DIVERSITY_TOP_PATHS) {
            let mut chunks = by_path.remove(&path).unwrap_or_default();
            chunks.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for hit in chunks.into_iter().take(DIVERSITY_CHUNKS_PER_PATH) {
                push(hit);
            }
        }
    }

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (i, hit) in out.iter_mut().enumerate() {
        hit.rank = i + 1;
    }
    out
}

/// Fill category buckets: one hit from each top path, then score-order fill to cap.
fn bucket_into_categories(
    diversified: &[SearchHit],
    per_cat: usize,
) -> std::collections::BTreeMap<String, Vec<SearchHit>> {
    let mut by_cat_hits: std::collections::HashMap<String, Vec<SearchHit>> =
        std::collections::HashMap::new();
    for hit in diversified {
        by_cat_hits
            .entry(hit.category.clone())
            .or_default()
            .push(hit.clone());
    }

    let mut by_cat: std::collections::BTreeMap<String, Vec<SearchHit>> =
        std::collections::BTreeMap::new();

    for (cat, cat_hits) in by_cat_hits {
        let cap = category_hit_cap(&cat, per_cat);
        let max_per_path_in_cat = cap.div_ceil(2).max(2);
        let mut by_path: std::collections::HashMap<String, Vec<SearchHit>> =
            std::collections::HashMap::new();
        for hit in cat_hits {
            by_path.entry(hit.path.clone()).or_default().push(hit);
        }

        let mut paths: Vec<(String, Vec<SearchHit>)> = by_path.into_iter().collect();
        for (_, chunks) in &mut paths {
            chunks.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        paths.sort_by(|a, b| {
            let pa = a.1.first().map_or(0.0, |h| h.score);
            let pb = b.1.first().map_or(0.0, |h| h.score);
            pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut out: Vec<SearchHit> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        let mut path_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        let try_push = |hit: SearchHit,
                        out: &mut Vec<SearchHit>,
                        seen: &mut std::collections::HashSet<(String, String)>,
                        path_counts: &mut std::collections::HashMap<String, usize>|
         -> bool {
            let key = (hit.path.clone(), hit.heading.clone());
            if !seen.insert(key) {
                return false;
            }
            *path_counts.entry(hit.path.clone()).or_insert(0) += 1;
            out.push(hit);
            true
        };

        // Seat: best chunk from the top few paths so secondary docs are not crowded out.
        for (_, chunks) in paths.iter().take(DIVERSITY_SEAT_PATHS.min(paths.len())) {
            if out.len() >= cap {
                break;
            }
            if let Some(hit) = chunks.first() {
                try_push(hit.clone(), &mut out, &mut seen, &mut path_counts);
            }
        }

        // Fill remaining slots by score, up to max_per_path_in_cat chunks per path.
        let mut remaining: Vec<SearchHit> = diversified
            .iter()
            .filter(|h| h.category == cat)
            .cloned()
            .collect();
        remaining.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for hit in remaining {
            if out.len() >= cap {
                break;
            }
            let n = path_counts.get(&hit.path).copied().unwrap_or(0);
            if n >= max_per_path_in_cat {
                continue;
            }
            try_push(hit, &mut out, &mut seen, &mut path_counts);
        }

        by_cat.insert(cat, out);
    }

    by_cat
}

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
        "no docs root found (checked --docs-path, $HERMES_DOCS_PATH, $HERMES_HOME, ~/.hermes)"
            .into(),
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
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .is_some_and(|ext| ext == "md" || ext == "mdx")
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
                .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
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
/// `###` chunks keep a parent prefix (`H2 > H3`) so advanced subsections stay identifiable.
fn chunk_markdown(content: &str, rel_path: &str) -> Vec<(String, String)> {
    let mut chunks: Vec<(String, String)> = Vec::new();
    let mut heading = String::new();
    let mut parent_h2 = String::new();
    let mut body_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
            let body = body_lines.join("\n").trim().to_string();
            if !body.is_empty() {
                chunks.push((heading.clone(), body));
            }
            let title = trimmed.trim_start_matches('#').trim().to_string();
            if trimmed.starts_with("## ") {
                heading.clone_from(&title);
                parent_h2 = title;
            } else if parent_h2.is_empty() {
                heading = title;
            } else {
                heading = format!("{parent_h2} > {title}");
            }
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
        let heading = content.lines().find(|l| l.starts_with("# ")).map_or_else(
            || {
                Path::new(rel_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(rel_path)
                    .to_string()
            },
            |l| l[2..].trim().to_string(),
        );
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

/// `<hermes-agent>/website/docs` → `<hermes-agent>`.
fn hermes_agent_root(docs_root: &Path) -> Option<&Path> {
    docs_root.parent()?.parent()
}

/// Escape `QueryParser` specials so a raw user question is safe.
fn escape_user_query(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '+' | '-' | '&' | '|' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '"' | '~'
            | '*' | '?' | ':' | '\\' | '/' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Get the Hermes git SHA and version string.
fn hermes_identity(docs_root: &Path) -> (String, String) {
    let hermes_root = hermes_agent_root(docs_root);

    let sha = hermes_root
        .and_then(|r| {
            std::process::Command::new("git")
                .args(["-C", &r.to_string_lossy(), "rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        })
        .unwrap_or_default();

    let version = std::process::Command::new("hermes")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.lines().next().map(|l| l.trim().to_owned())
        })
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
    let json =
        serde_json::to_string_pretty(&state).map_err(|e| SearchError::Internal(e.to_string()))?;
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

    for entry in WalkDir::new(docs_root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext == "md" || ext == "mdx");
        if !path.is_file() || !is_md {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
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
            let (current_sha, current_version) = hermes_identity(docs_root);
            let docs_match = s.docs_path == docs_root.to_string_lossy();
            let sha_match = s.hermes_git_sha == current_sha;
            let version_match = s.hermes_version == current_version;
            let fingerprint_match = s.fingerprint == current_fingerprint;
            !(docs_match && sha_match && version_match && fingerprint_match)
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

fn extract_hits(
    searcher: &tantivy::Searcher,
    top_hits: &[(f32, tantivy::DocAddress)],
    f_path: tantivy::schema::Field,
    f_heading: tantivy::schema::Field,
    f_body: tantivy::schema::Field,
) -> Result<Vec<SearchHit>, SearchError> {
    let mut hits = Vec::new();
    for (score, doc_addr) in top_hits {
        let doc: tantivy::TantivyDocument = searcher
            .doc(*doc_addr)
            .map_err(|e| SearchError::Internal(e.to_string()))?;
        let path = doc
            .get_first(f_path)
            .and_then(|v| match v {
                OwnedValue::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("")
            .to_string();
        let heading = doc
            .get_first(f_heading)
            .and_then(|v| match v {
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
        let category = category_for_path(&path);

        hits.push(SearchHit {
            rank: 0,
            category,
            url: url_for_relpath(&path),
            path,
            heading,
            score: *score,
            body_bytes: body.len(),
            oversized: body.len() > OVERSIZED_CHUNK_BYTES,
            body,
        });
    }
    Ok(hits)
}

/// Run a BM25 query and return hits grouped by top-level docs category.
///
/// Scores are raw BM25. Callers (or an LLM) interpret `signals` + categories;
/// this function does not pick a "primary" document.
pub fn query(cache_dir: &Path, query_str: &str, top_k: usize) -> Result<QueryOutput, SearchError> {
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

    let cleaned: String = query_str
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '.' || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let tokens = query_tokens(&cleaned);
    let content = content_tokens(&tokens);

    let mut query_parser = QueryParser::for_index(&index, vec![f_path, f_heading, f_body]);
    query_parser.set_field_boost(f_path, 10.0);
    query_parser.set_field_boost(f_heading, 2.5);
    query_parser.set_field_boost(f_body, 1.0);

    let parsed = query_parser.parse_query(&search_query(&cleaned, query_str));
    let q = parsed.map_err(|e| SearchError::Internal(format!("parse query failed: {e}")))?;

    let per_cat = top_k.max(1);
    let candidate_limit =
        (per_cat * DIVERSITY_TOP_PATHS * DIVERSITY_CHUNKS_PER_PATH * 32).clamp(512, 2500);
    let top_hits: Vec<(f32, tantivy::DocAddress)> = searcher
        .search(&q, &TopDocs::with_limit(candidate_limit))
        .map_err(|e| SearchError::Internal(format!("search failed: {e}")))?;

    let mut scored = extract_hits(&searcher, &top_hits, f_path, f_heading, f_body)?;
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let matched = matched_content_tokens(&content, &scored);
    let token_coverage = if content.is_empty() {
        1.0
    } else {
        let m = matched.len().min(u32::MAX as usize) as f32;
        let c = content.len().min(u32::MAX as usize) as f32;
        m / c
    };
    let on_topic = if content.is_empty() {
        !scored.is_empty()
    } else {
        matched.len() == content.len()
    };

    let max_bm25 = scored.first().map_or(0.0, |h| h.score);

    if scored.is_empty() && searcher.num_docs() == 0 {
        return Ok(QueryOutput {
            signals: QuerySignals {
                max_bm25,
                hit_count: 0,
                query_tokens: tokens,
                content_tokens: content,
                matched_content_tokens: Vec::new(),
                token_coverage: 0.0,
                on_topic: false,
                docs_empty: true,
            },
            categories: Vec::new(),
        });
    }

    if !on_topic {
        return Ok(QueryOutput {
            signals: QuerySignals {
                max_bm25,
                hit_count: 0,
                query_tokens: tokens,
                content_tokens: content,
                matched_content_tokens: matched,
                token_coverage,
                on_topic: false,
                docs_empty: false,
            },
            categories: Vec::new(),
        });
    }

    let diversified = diversify_hits(&scored);
    let by_cat = bucket_into_categories(&diversified, per_cat);
    let mut categories: Vec<CategoryHits> = by_cat
        .into_iter()
        .map(|(category, hits)| CategoryHits { category, hits })
        .collect();
    categories.sort_by(|a, b| {
        category_bucket_priority(&a.category)
            .cmp(&category_bucket_priority(&b.category))
            .then_with(|| {
                let sa = a.hits.first().map_or(0.0, |h| h.score);
                let sb = b.hits.first().map_or(0.0, |h| h.score);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let hit_count: usize = categories.iter().map(|c| c.hits.len()).sum();

    Ok(QueryOutput {
        signals: QuerySignals {
            max_bm25,
            hit_count,
            query_tokens: tokens,
            content_tokens: content,
            matched_content_tokens: matched,
            token_coverage,
            on_topic: true,
            docs_empty: false,
        },
        categories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn copy_tree(src_dir: &Path, dst_dir: &Path) {
        for entry in fs::read_dir(src_dir).unwrap() {
            let entry = entry.unwrap();
            let target = dst_dir.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir_all(&target).unwrap();
                copy_tree(entry.path().as_ref(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    fn setup_fixtures(dir: &Path) {
        let src = Path::new("tests/fixtures/docs");
        fs::create_dir_all(dir).unwrap();
        copy_tree(src, dir);
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
            SearchError::Internal(msg) => panic!("expected DocsNotFound, got: {msg:?}"),
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
        assert!(headings.contains(&"Section A > Sub A"));
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

        let out = query(&cache, "configuration", 3).unwrap();
        let hits = flat_hits(&out);
        assert!(!hits.is_empty(), "expected results for 'configuration'");
        assert!(hits[0].path.contains("config"));
        assert!(hits[0].url.contains("hermes-agent.nousresearch.com/docs"));
        assert!(out.signals.on_topic);
    }

    #[test]
    fn test_index_and_query_no_hits() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");
        fs::create_dir_all(&cache).unwrap();

        reindex(&docs, &cache).unwrap();

        let out = query(&cache, "zzzzzqqqqqqxxx", 5).unwrap();
        assert!(flat_hits(&out).is_empty());
        assert!(!out.signals.on_topic);
    }

    #[test]
    fn test_empty_docs_tree_sets_docs_empty() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(&cache, "configuration", 5).unwrap();
        assert!(
            out.signals.docs_empty,
            "expected docs_empty=true; signals={:?}",
            out.signals
        );
        assert!(!out.signals.on_topic);
        assert_eq!(out.signals.hit_count, 0);
        assert!(out.categories.is_empty());
    }

    #[test]
    fn test_non_empty_docs_tree_not_flagged_docs_empty() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        // Even an off-topic query on a populated tree must not set docs_empty.
        let out = query(&cache, "zzzzzqqqqqqxxx", 5).unwrap();
        assert!(
            !out.signals.docs_empty,
            "populated tree must not set docs_empty; signals={:?}",
            out.signals
        );
        assert!(!out.signals.on_topic);
    }

    #[test]
    fn test_off_topic_query_clears_categories() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(
            &cache,
            "how many dildos does the hermes girl have in her dresser drawer",
            5,
        )
        .unwrap();
        assert!(
            !out.signals.on_topic,
            "expected off-topic; signals={:?}",
            out.signals
        );
        assert!(
            out.categories.is_empty(),
            "off-topic queries must not dump docs; got {:?}",
            out.categories
        );
        assert_eq!(out.signals.hit_count, 0);
        assert!(out.signals.content_tokens.iter().any(|t| t == "dildos"));
        assert!(out.signals.matched_content_tokens.is_empty());
    }

    #[test]
    fn test_partial_content_token_match_is_off_topic() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(docs.join("misc")).unwrap();
        fs::write(
            docs.join("misc/random.md"),
            "# Random\n\nA girl opened her dresser drawer for storage.\n",
        )
        .unwrap();
        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(
            &cache,
            "how many dildos does the hermes girl have in her dresser drawer",
            5,
        )
        .unwrap();
        assert!(
            !out.signals.on_topic,
            "partial overlap must not count as on-topic; matched={:?}",
            out.signals.matched_content_tokens
        );
        assert!(out.categories.is_empty());
        assert_eq!(out.signals.hit_count, 0);
    }

    #[test]
    fn test_per_path_diversity_includes_setup_chunks() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(docs.join("user-guide/messaging")).unwrap();
        fs::write(
            docs.join("user-guide/messaging/discord.md"),
            r#"# Discord Setup

## Step 1: Create a Discord Application

Visit the Discord Developer Portal and create an application.

## How Hermes Behaves

Discord gateway model uses DISCORD_ALLOWED_USERS for access control.

## Step 8: Configure Hermes Agent

### Option A: Interactive Setup (Recommended)

Run hermes gateway setup and select Discord.

### Option B: Manual Configuration

Add to ~/.hermes/.env:

DISCORD_BOT_TOKEN=your-bot-token
DISCORD_ALLOWED_USERS=284102345871466496

### Start the Gateway

Run hermes gateway to bring the bot online.
"#,
        )
        .unwrap();
        fs::write(
            docs.join("user-guide/noise.md"),
            r#"# Noise

## Configure everything

Configure configure configure discord discord for filler BM25 score.
"#,
        )
        .unwrap();

        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(&cache, "how do I configure Discord?", 5).unwrap();
        assert!(out.signals.on_topic);

        let discord_hits: Vec<_> = flat_hits(&out)
            .into_iter()
            .filter(|h| h.path.contains("messaging/discord"))
            .collect();
        assert!(
            discord_hits.len() >= 2,
            "expected multiple discord.md chunks; got {}",
            discord_hits.len()
        );

        let bodies: String = discord_hits.iter().map(|h| h.body.as_str()).collect();
        assert!(
            bodies.contains("DISCORD_BOT_TOKEN"),
            "expected token literal from discord.md; hits={discord_hits:?}"
        );
        assert!(
            bodies.contains("hermes gateway setup") || bodies.contains("hermes gateway"),
            "expected gateway commands from discord.md; hits={discord_hits:?}"
        );
    }

    #[test]
    fn test_categories_include_messaging_and_guides() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(docs.join("user-guide/messaging")).unwrap();
        fs::create_dir_all(docs.join("guides")).unwrap();
        fs::write(
            docs.join("user-guide/messaging/telegram.md"),
            r#"# Telegram

## Gateway setup

Configure Telegram with TELEGRAM_BOT_TOKEN in ~/.hermes/.env.
Run hermes gateway setup, then start the gateway for Telegram messaging.
"#,
        )
        .unwrap();
        fs::write(
            docs.join("guides/local-ollama-setup.md"),
            r#"# Local Ollama setup

## Platforms

You can also mention Telegram briefly:

```yaml
platforms:
  telegram:
    token: "..."
```

This guide is mostly about Ollama, not Telegram gateway setup.
"#,
        )
        .unwrap();

        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(&cache, "how do I configure Telegram?", 5).unwrap();
        assert!(out.signals.on_topic);
        assert!(
            out.signals
                .matched_content_tokens
                .iter()
                .any(|t| t == "telegram"),
            "signals={:?}",
            out.signals
        );
        let paths: Vec<&str> = flat_hits(&out).iter().map(|h| h.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.contains("telegram")),
            "expected telegram paths; got {paths:?}"
        );
        let tg_bodies: String = flat_hits(&out)
            .iter()
            .filter(|h| h.path.contains("messaging/telegram"))
            .map(|h| h.body.as_str())
            .collect();
        if !tg_bodies.is_empty() {
            assert!(
                tg_bodies.contains("TELEGRAM_BOT_TOKEN") || tg_bodies.contains("BotFather"),
                "expected setup literals in telegram.md chunks; bodies={tg_bodies:?}"
            );
        }
        let cats: Vec<&str> = out.categories.iter().map(|c| c.category.as_str()).collect();
        assert!(
            cats.contains(&"user-guide") || cats.contains(&"guides"),
            "expected category buckets; got {cats:?}"
        );
        if cats.contains(&"user-guide") && cats.contains(&"guides") {
            let ug = cats.iter().position(|&c| c == "user-guide").unwrap();
            let g = cats.iter().position(|&c| c == "guides").unwrap();
            assert!(
                ug < g,
                "user-guide should appear before guides; got {cats:?}"
            );
        }
    }

    #[test]
    fn test_default_model_surfaces_configuring_models() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(docs.join("user-guide")).unwrap();
        fs::create_dir_all(docs.join("user-guide/features")).unwrap();
        fs::write(
            docs.join("user-guide/configuring-models.md"),
            r#"# Configuring Models

## Setting the main model

Open the dashboard and click Models. Pick your provider and model ID.
The main model field in config.yaml controls what the agent uses.

```yaml
model:
  provider: openrouter
  default: anthropic/claude-sonnet-4
```
"#,
        )
        .unwrap();
        fs::write(
            docs.join("user-guide/features/extending-the-dashboard.md"),
            r#"# Dashboard

## Plugins

The default layout uses default theme colors and default plugin slots.
SDK.components.Button is the default export for dashboard plugins.
"#,
        )
        .unwrap();

        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(&cache, "how do I set the default model?", 5).unwrap();
        assert!(out.signals.on_topic, "signals={:?}", out.signals);
        assert_eq!(
            out.signals.content_tokens,
            vec!["model".to_string()],
            "signal stopwords should leave only 'model'"
        );
        let paths: Vec<&str> = flat_hits(&out).iter().map(|h| h.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.contains("configuring-models")),
            "expected configuring-models.md in hits; got {paths:?}"
        );
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

    #[test]
    fn test_hermes_agent_root_is_two_parents() {
        let docs = Path::new("/home/you/.hermes/hermes-agent/website/docs");
        assert_eq!(
            hermes_agent_root(docs),
            Some(Path::new("/home/you/.hermes/hermes-agent"))
        );
    }

    #[test]
    fn test_escape_user_query() {
        assert_eq!(
            escape_user_query("config.yaml: providers"),
            r"config.yaml\: providers"
        );
        assert_eq!(escape_user_query("foo (bar)"), r"foo \(bar\)");
    }

    #[test]
    fn test_oversized_chunk_flag() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(docs.join("reference")).unwrap();
        let big_body = "x".repeat(5000);
        fs::write(
            docs.join("reference/long.md"),
            format!("# Long\n\n## Huge section\n\n{big_body}"),
        )
        .unwrap();
        fs::create_dir_all(docs.join("config")).unwrap();
        fs::write(
            docs.join("config/small.md"),
            "# Small\n\n## Short section\n\nTiny body text.\n",
        )
        .unwrap();

        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();

        let out = query(&cache, "huge section", 5).unwrap();
        let hits = flat_hits(&out);
        assert!(
            hits.iter().any(|h| h.path.contains("long.md")),
            "expected long.md in hits"
        );
        let big = hits
            .iter()
            .find(|h| h.path.contains("long.md"))
            .expect("long.md hit missing");
        assert_eq!(big.body_bytes, 5000);
        assert!(
            big.oversized,
            "expected oversized flag for >4096 byte chunk"
        );

        let out = query(&cache, "short section", 5).unwrap();
        let hits_small = flat_hits(&out);
        let small = hits_small
            .iter()
            .find(|h| h.path.contains("small.md"))
            .expect("small.md hit missing");
        assert!(
            !small.oversized,
            "small chunk must not be flagged oversized"
        );
        assert_eq!(small.body_bytes, small.body.len());
    }

    #[test]
    fn test_query_with_special_chars() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        setup_fixtures(&docs);
        let cache = tmp.path().join("cache");
        reindex(&docs, &cache).unwrap();
        let out = query(&cache, "how do I set HERMES_HOME (config.yaml)?", 5).unwrap();
        assert!(
            flat_hits(&out)
                .iter()
                .any(|h| h.body.contains("HERMES_HOME")),
            "punctuation in the question must not empty-out retrieval; got {out:?}"
        );
    }
}
