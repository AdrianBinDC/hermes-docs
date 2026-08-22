<p align="center">
  <a href="https://github.com/NousResearch/hermes-agent">
    <img src="https://raw.githubusercontent.com/NousResearch/hermes-agent/main/assets/banner.png" alt="Hermes Agent banner (from NousResearch/hermes-agent)" width="100%">
  </a>
</p>

<p align="center">
  <strong>Unofficial documentation companion for <a href="https://github.com/NousResearch/hermes-agent">Hermes Agent</a></strong><br>
  <em>Not affiliated with or endorsed by <a href="https://nousresearch.com">Nous Research</a>.</em><br>
  Banner artwork © Nous Research — shown here only to identify the Hermes Agent ecosystem this tool is built for.
</p>

# hermes-docs

<p align="center">
  <a href="https://github.com/AdrianBinDC/hermes-docs/actions/workflows/ci.yml"><img src="https://github.com/AdrianBinDC/hermes-docs/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#dev--testing"><img src="https://img.shields.io/badge/coverage-91%25-brightgreen" alt="Line coverage 91%"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Grounded answers from your local Hermes docs — not web guesswork.**

The official [Hermes Agent documentation](https://hermes-agent.nousresearch.com/docs) lives on the web — ask inside Hermes and it often wanders. **hermes-docs** is an unofficial documentation skill plus local BM25 CLI (`hermes-docs-search`) that retrieves tightly scoped, grounded markdown chunks from your local Hermes Agent docs tree, with citations back to the published pages.

> **Scope:** Unofficial community tool for use with [Hermes Agent](https://github.com/NousResearch/hermes-agent). It indexes the docs that ship with Hermes; it is not a substitute for Hermes itself, and it is not an official Nous Research project.

## Prerequisites

- Rust / [Cargo](https://doc.rust-lang.org/cargo/) for building from source
- Hermes docs tree at one of the resolved locations (see [CONTRACT.md](CONTRACT.md) for the full resolution order)

## Build from source

```bash
cargo build --release
```

The binary is `hermes-docs-search`.

## Install the skill

```bash
cp -r skills/hermes-docs ~/.hermes/skills/
```

Hermes also supports profile-scoped homes, so you can install into a specific
profile instead:

```bash
cp -r skills/hermes-docs ~/.hermes/profiles/<profile-name>/skills/
```

The first query uses a cached binary when present. Without one, the wrapper builds from source if `cargo` and this repo are available. To download a release instead, set `HERMES_DOCS_RELEASE` to the GitHub release asset base URL (for example `https://github.com/AdrianBinDC/hermes-docs/releases/download/v0.1.0`). Subsequent queries are instant once the binary is in cache.

No Docker required.

## CLI usage

```bash
# Ranked markdown for the top 5 chunks mentioning "configuration"
hermes-docs-search "configuration"

# Top 3 as JSON
hermes-docs-search --json -k 3 "how to set the model"

# Force a full reindex before answering
hermes-docs-search --reindex "messaging"

# Point at a non-default docs tree
hermes-docs-search --docs-path /path/to/website/docs "toolsets"
```

Flags:

- `-k, --top-k <N>` — max hits kept per category (default `5`)
- `--json` — emit machine-readable JSON
- `--reindex` — force a full reindex
- `--docs-path <PATH>` — explicit docs root override
- `--cache-dir <PATH>` — state/index location (default `$XDG_CACHE_HOME/hermes-docs-search` or `~/.cache/hermes-docs-search`)

The skill wrapper (`skills/hermes-docs/scripts/hermes-docs`) injects `-k 3` when called with `--json` unless `-k`/`--top-k` is already provided, and maps the common `-8` typo to `-k 3`. When retrieved doc chunks disagree on the same setup step, the skill asks which source to follow instead of merging answers (see [SKILL.md](skills/hermes-docs/SKILL.md) for the full rule).

## Dev / testing

```bash
cargo test
cargo llvm-cov --summary-only --ignore-filename-regex 'main\.rs'
```

Line coverage on `src/engine.rs` is currently **~91%** (CLI `main.rs` excluded). CI enforces a floor so it does not regress quietly.

Development tip: symlink the local skill into your Hermes skills folder so edits are live without re-copying:

```bash
ln -s $(pwd)/skills/hermes-docs ~/.hermes/skills/hermes-docs
```

(or the profile path `~/.hermes/profiles/<name>/skills/hermes-docs`).

## Documentation

- [CONTRACT.md](CONTRACT.md) — source of truth for CLI flags, output formats, exit codes, docs-root resolution, and skill behavior
- [docs/engine.md](docs/engine.md) — search engine query/index pipelines (Mermaid); also in [`src/engine.rs`](src/engine.rs) module docs
- [skills/hermes-docs/SKILL.md](skills/hermes-docs/SKILL.md) — agent behavior rules for the `hermes-docs` skill

## License

This project is [MIT](LICENSE) licensed.

Hermes Agent, its documentation site, and the banner above are property of [Nous Research](https://nousresearch.com) / [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent). This repository is an unofficial documentation companion / skill for use with Hermes Agent.
