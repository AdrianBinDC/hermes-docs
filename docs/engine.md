# Search engine (`src/engine.rs`)

BM25 index + query pipeline for `hermes-docs-search`. Behavior contract lives in
[CONTRACT.md](../CONTRACT.md); this page is only for **how the Rust code is
shaped**.

Source of truth for the diagram also appears in the module docs at the top of
[`src/engine.rs`](../src/engine.rs).

## Query pipeline

```mermaid
flowchart TD
  openIndex[Open Tantivy index]
  sanitize[sanitize_query]
  tokenize[query_tokens and content_tokens]
  search[Parse BM25 query and search]
  extract[extract_hits]
  signals[Compute coverage and on_topic]
  emptyCheck{Index empty?}
  topicCheck{on_topic?}
  diversify[diversify_hits]
  orderCats[order_categories]
  outEmpty[QueryOutput docs_empty]
  outOff[QueryOutput off_topic]
  outOk[QueryOutput with categories]

  openIndex --> sanitize --> tokenize --> search --> extract --> signals
  signals --> emptyCheck
  emptyCheck -->|yes| outEmpty
  emptyCheck -->|no| topicCheck
  topicCheck -->|no| outOff
  topicCheck -->|yes| diversify --> orderCats --> outOk
```

## Index pipeline

```mermaid
flowchart TD
  ensure[ensure_indexed]
  stale{State stale or missing?}
  reindex[reindex]
  walk[Walk docs md and mdx]
  chunk[chunk_markdown]
  write[Write Tantivy docs]
  state[save_state fingerprint]

  ensure --> stale
  stale -->|no| doneNode[Use cached index]
  stale -->|yes| reindex --> walk --> chunk --> write --> state
```
