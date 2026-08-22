# CONTRACT.md — hermes-docs

This contract is the single source of truth for the `hermes-docs` skill and its
Rust retrieval CLI, `hermes-docs-search`. The CLI provides BM25 retrieval over
the local Hermes Agent docs tree so an agent can get grounded markdown chunks
(with source paths and doc URLs) in a single invocation.

Do not rename any identifier in this document. Examples may be expanded; names may not.

---

## CLI

```
hermes-docs-search [OPTIONS] <QUERY>
  -k, --top-k <N>     max results to return (default 5)
  --json              emit machine-readable JSON instead of the default markdown template
  --reindex           force a full reindex before answering (ignore cached state)
  --docs-path <PATH>  explicit docs root (highest-priority docs-root override)
  --cache-dir <PATH>  where state.json + the index live
                      default: $XDG_CACHE_HOME/hermes-docs-search
                      or ~/.cache/hermes-docs-search
```

`<QUERY>` is a single required positional argument: the user's question,
unquoted word(s) passed through to the BM25 engine verbatim.

### Examples

```bash
# Ranked markdown for the top 5 chunks mentioning "configuration"
hermes-docs-search "configuration"

# Top 3 as JSON (for programmatic consumers)
hermes-docs-search --json -k 3 "how to set the model"

# Force a rebuild of the index, then answer
hermes-docs-search --reindex "messaging"

# Point at a non-default docs tree
hermes-docs-search --docs-path /path/to/website/docs "toolsets"
```

---

## Docs root resolution order

The docs root is the directory containing the Hermes website documentation
markdown (the `website/docs` tree). Resolution is first-match-wins:

1. `--docs-path` CLI flag
2. `$HERMES_DOCS_PATH` environment variable
3. `$HERMES_HOME/hermes-agent/website/docs` (only if `$HERMES_HOME` is set)
4. `~/.hermes/hermes-agent/website/docs`

The first candidate that exists and is a readable directory is used. If none
exist, the CLI exits with code 1.

---

## State file `{cache-dir}/state.json`

After each (re)index, the CLI writes:

```json
{
  "hermes_git_sha": "...",
  "hermes_version": "...",
  "docs_path": "...",
  "indexed_at": "ISO-8601"
}
```

- `hermes_git_sha` — output of `git -C <hermes-agent-root> rev-parse HEAD`, or
  `""` when unavailable.
- `hermes_version` — output of `hermes --version` (trimmed), or `""` when
  unavailable.
- `docs_path` — the absolute docs root actually indexed.
- `indexed_at` — RFC 3339 / ISO-8601 UTC timestamp of the index build.

**Reindex triggers.** On every query the CLI compares the *current* identity of
the docs against `state.json` and reindexes when any of these differ:

- current Hermes git SHA vs `hermes_git_sha`
- current Hermes version vs `hermes_version`
- docs fingerprint vs the fingerprint recorded at index time

`--reindex` forces a rebuild regardless of state.

### Hermes identity resolution

Identity is resolved in this priority order (the first available source wins):

1. `git -C <hermes-agent-root> rev-parse HEAD`
   where `<hermes-agent-root>` is the parent of `website/`
   (i.e. the docs root with `/website/docs` stripped off).
2. `hermes --version` — the version string reported by the `hermes` CLI, if it
   is on `PATH`.
3. Docs fingerprint — an aggregate of the docs tree's file mtimes and sizes —
   as a final fallback when neither git nor the `hermes` CLI is available.

The docs fingerprint is computed deterministically from the set of `.md`/`.mdx`
files (path, mtime, size) so any content change is detected even without git.

---

## Default stdout template

For a non-zero number of hits, stdout is:

```markdown
# Hermes docs search: <query>

## 1. <relpath> — <heading>
URL: https://hermes-agent.nousresearch.com/docs/<url-path>
Score: <score>

<body>

## 2. ...
```

- `<relpath>` — path of the source file relative to the docs root.
- `<heading>` — the `##`/`###` heading the chunk was split on, or the file's
  top-level title when the chunk has no heading.
- `<url-path>` — the source file's path relative to the docs root with its
  extension dropped (e.g. `configuration.md` → `configuration`), so the URL is
  `https://hermes-agent.nousresearch.com/docs/<url-path>`.
- `<score>` — the BM25 score, rendered with limited precision.
- `<body>` — the chunk text (import noise stripped, code fences preserved).

Chunks are listed in descending score order and numbered from 1.

### Zero hits

When no chunk matches, the CLI still exits 0 and prints:

```markdown
# Hermes docs search: <query>

_No hits._
```

### `--json` shape

```json
{
  "query": "<query>",
  "docs_path": "<absolute docs root>",
  "results": [
    {
      "rank": 1,
      "path": "<relpath>",
      "heading": "<heading>",
      "url": "https://hermes-agent.nousresearch.com/docs/<url-path>",
      "score": 12.34,
      "body": "<chunk text>"
    }
  ]
}
```

---

## Exit codes

- `0` — success (including a successful query with zero hits)
- `1` — docs path missing or unreadable (no docs root could be resolved, or it
  cannot be read)
- `2` — internal error (index build failure, I/O failure, unexpected panic)

---

## Names

- Binary: `hermes-docs-search`
- Cache binary path: `~/.cache/hermes-docs-search/bin/hermes-docs-search`
- Release assets: `hermes-docs-search-{target}.tar.gz` for
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
  - `x86_64-apple-darwin`
  - `aarch64-apple-darwin`
- Skill: `hermes-docs`
- Wrapper: `skills/hermes-docs/scripts/hermes-docs`
