# CONTRACT.md — hermes-docs-search

Frozen interface for the `hermes-docs` skill and `hermes-docs-search` binary.
Implementers may expand examples; do not change names, flags, paths, or exit codes.

## CLI

```
hermes-docs-search [OPTIONS] <QUERY>
  -k, --top-k <N>     default 5
  --json              machine-readable output (skill uses markdown default)
  --reindex           force rebuild of the BM25 index
  --docs-path <PATH>  override docs root
  --cache-dir <PATH>  override cache (default: $XDG_CACHE_HOME/hermes-docs-search
                      or ~/.cache/hermes-docs-search)
```

`<QUERY>` is required unless only `--reindex` is used with no search (still prefer requiring a query for search invocations).

## Docs root resolution order

1. `--docs-path` if set
2. `$HERMES_DOCS_PATH` if set
3. `$HERMES_HOME/hermes-agent/website/docs`
4. `~/.hermes/hermes-agent/website/docs`

## Cache layout

- Index directory: `{cache-dir}/index/`
- State file: `{cache-dir}/state.json`

```json
{
  "hermes_git_sha": "...",
  "hermes_version": "...",
  "docs_path": "...",
  "docs_fingerprint": "...",
  "indexed_at": "ISO-8601"
}
```

Reindex when current Hermes git SHA, version, or docs fingerprint differs from state, or when `--reindex` is passed.

## Hermes identity

`hermes-agent-root` = parent of the `website/` directory that contains `docs/` (i.e. docs path `.../website/docs` → root `.../`).

1. `git -C <hermes-agent-root> rev-parse HEAD`
2. `hermes --version` when available (stdout trimmed)
3. Docs fingerprint: aggregate of relative path + file size + mtime for every `*.md` / `*.mdx` under docs root (stable hash string)

## Default stdout template (markdown)

```markdown
# Hermes docs search: <query>

## 1. <relpath> — <heading>
URL: https://hermes-agent.nousresearch.com/docs/<url-path>
Score: <score>

<body>

## 2. ...
```

- `<relpath>` is relative to docs root, using `/` separators, keeping the original extension in the path for citation, but `<url-path>` strips `.md` / `.mdx` and omits a trailing `/index`.
- Zero hits: exit `0`, print the `# Hermes docs search: <query>` header and `_No hits._`

## `--json` output

JSON array of objects:

```json
[
  {
    "path": "user-guide/configuration.md",
    "heading": "Providers",
    "url": "https://hermes-agent.nousresearch.com/docs/user-guide/configuration",
    "score": 12.4,
    "body": "..."
  }
]
```

Empty array on zero hits.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success (including zero hits) |
| 1 | Docs path missing or unreadable |
| 2 | Index/search internal error |

## Names

| Item | Value |
| --- | --- |
| Binary | `hermes-docs-search` |
| Cache binary | `~/.cache/hermes-docs-search/bin/hermes-docs-search` |
| Release assets | `hermes-docs-search-{target}.tar.gz` |
| Targets | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin` |
| Skill | `hermes-docs` |
| Wrapper | `skills/hermes-docs/scripts/hermes-docs` |

## Wrapper resolution order

`scripts/hermes-docs` resolves the binary as:

1. `hermes-docs-search` on `$PATH`
2. `~/.cache/hermes-docs-search/bin/hermes-docs-search` (or `$XDG_CACHE_HOME/hermes-docs-search/bin/...`)
3. Download matching GitHub Release asset (best-effort)
4. If `cargo` is available, build from this repo and install into the cache bin dir
5. Exec with all CLI args; propagate exit code
