# BUILD.md — hermes-docs

Build a press-button Hermes skill named **hermes-docs** that retrieves grounding
markdown from the local Hermes Agent docs tree via a Rust BM25 CLI.

You are alone. Do not wait for a human. Implement everything below. Commit after
each phase. Prefer small, working increments over speculation.

## Success criteria (stop when all true)

1. `cargo test` passes in this repo.
2. `cargo build --release` produces `hermes-docs-search`.
3. Binary is installed to `~/.cache/hermes-docs-search/bin/hermes-docs-search`
   (and/or available on PATH).
4. `skills/hermes-docs/SKILL.md` exists with `name: hermes-docs` and instructs
   **one** invocation of `scripts/hermes-docs` — no `rg`, no `search_files`,
   no multi-step doc hunting.
5. `skills/hermes-docs/scripts/hermes-docs` runs a query and prints markdown.
6. Against real docs (if present at `$HERMES_HOME/hermes-agent/website/docs` or
   `~/.hermes/hermes-agent/website/docs`), a query like `configuration` returns
   ranked chunks with paths + `https://hermes-agent.nousresearch.com/docs/...` URLs.
7. `~/.cache/hermes-docs-search/state.json` records `hermes_git_sha` / version;
   after forcing a state mismatch (edit sha), the next query reindexes.
8. `.github/workflows/release.yml` builds the release asset names listed in the
   contract. `README.md` has a one-liner skill install.
9. If Hermes docs path is missing, create/use crate fixtures and still pass tests;
   note the gap in the final commit message.

## Non-goals

- Docker, MCP, embeddings, scraping the live site
- Indexing Hermes repo-root `docs/` (ADRs) — only `website/docs`
- Asking the LLM to search the filesystem itself

---

## Phase 0 — write `CONTRACT.md` (do this first)

Create `CONTRACT.md` with exactly these rules (you may expand examples, not change names).

### CLI

```
hermes-docs-search [OPTIONS] <QUERY>
  -k, --top-k <N>     default 5
  --json
  --reindex
  --docs-path <PATH>
  --cache-dir <PATH>  default: $XDG_CACHE_HOME/hermes-docs-search
                      or ~/.cache/hermes-docs-search
```

### Docs root resolution order

1. `--docs-path`
2. `$HERMES_DOCS_PATH`
3. `$HERMES_HOME/hermes-agent/website/docs`
4. `~/.hermes/hermes-agent/website/docs`

### State file `{cache-dir}/state.json`

```json
{
  "hermes_git_sha": "...",
  "hermes_version": "...",
  "docs_path": "...",
  "indexed_at": "ISO-8601"
}
```

Reindex when current Hermes git SHA, version, or docs fingerprint differs from state.

Hermes identity:

1. `git -C <hermes-agent-root> rev-parse HEAD` (hermes-agent-root = parent of `website/`)
2. `hermes --version` if available
3. Docs fingerprint (aggregate mtime/size) as fallback

### Default stdout template

```markdown
# Hermes docs search: <query>

## 1. <relpath> — <heading>
URL: https://hermes-agent.nousresearch.com/docs/<url-path>
Score: <score>

<body>

## 2. ...
```

Zero hits: still exit 0, body `_No hits._`

### Exit codes

- 0 success
- 1 docs path missing/unreadable
- 2 internal error

### Names

- Binary: `hermes-docs-search`
- Cache binary path: `~/.cache/hermes-docs-search/bin/hermes-docs-search`
- Release assets: `hermes-docs-search-{target}.tar.gz` for
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`
- Skill: `hermes-docs`
- Wrapper: `skills/hermes-docs/scripts/hermes-docs`

Commit: `Add CONTRACT.md`

---

## Phase 1 — Rust engine (Track A)

**Files only:** `Cargo.toml`, `crates/hermes-docs-search/**` (or single-crate layout if simpler—keep binary name `hermes-docs-search`).

Implement:

- Chunk `*.md` / `*.mdx` on `##` / `###` (strip light MDX import noise)
- BM25 via Tantivy
- Auto-reindex per contract state
- Markdown + `--json` output
- Unit tests + fixtures under the crate (`tests/fixtures/docs/...` with a few fake pages)

Then:

```bash
cargo test
cargo build --release
mkdir -p ~/.cache/hermes-docs-search/bin
cp target/release/hermes-docs-search ~/.cache/hermes-docs-search/bin/
```

Commit: `Implement hermes-docs-search BM25 CLI`

---

## Phase 2 — Skill (Track B)

**Files only:** `skills/hermes-docs/SKILL.md`

- Frontmatter `name: hermes-docs`
- Description must trigger on Hermes config / how Hermes works / env / messaging / toolsets
- Procedure: run `scripts/hermes-docs` **once** with the user question; answer only from stdout; cite paths/URLs; never invent config if retrieval fails
- Explicitly forbid: `rg`, `search_files`, reading doc paths directly, web fetch, refine loops

Commit: `Add hermes-docs skill`

---

## Phase 3 — Wrapper (Track C)

**Files only:** `skills/hermes-docs/scripts/hermes-docs` (POSIX sh, executable)

Resolve binary: PATH → `~/.cache/hermes-docs-search/bin/hermes-docs-search` →
download release asset for this OS/arch (best-effort) → else if `cargo` available,
build from repo and install to cache → exec with all args → propagate exit code.

Overnight: download may fail; cache copy from Phase 1 must make the wrapper succeed.

Commit: `Add hermes-docs bootstrap wrapper`

---

## Phase 4 — Release + README (Track D)

**Files only:** `.github/workflows/release.yml`, `README.md`

- Workflow: build/upload the four `hermes-docs-search-{target}.tar.gz` assets on tag/release
- README: what hermes-docs is; one-liner to install the skill into Hermes; note that first
  query may build/fetch the binary; no Docker

Commit: `Add release workflow and README`

---

## Phase 5 — Integrate / verify

1. Run: `~/.cache/hermes-docs-search/bin/hermes-docs-search "configuration" -k 3`
   (or via `skills/hermes-docs/scripts/hermes-docs`)
2. Confirm markdown shape matches contract
3. Confirm `state.json` written
4. Corrupt `hermes_git_sha` in state → rerun → confirm reindex (stderr or newer `indexed_at`)
5. If `~/.hermes/skills` exists, symlink or copy `skills/hermes-docs` there and mention
   in the final commit message that the user can run `/hermes-docs` in Hermes

Final commit: `Verify hermes-docs end-to-end`

---

## Parallelism note

Phases 1–4 only share `CONTRACT.md`. A single overnight agent should still run them
**in order 0→1→2→3→4→5** (binary before wrapper verify). Do not skip tests.
