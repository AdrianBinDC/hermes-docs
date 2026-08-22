# hermes-docs

BM25 docs-search CLI (`hermes-docs-search`) plus a Hermes skill that retrieves grounded markdown chunks from the local Hermes Agent docs tree.

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
cp -r skills/hermes-docs ~/.hermes/profiles/<name>/skills/
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
```

Development tip: symlink the local skill into your Hermes skills folder so edits are live without re-copying:

```bash
ln -s $(pwd)/skills/hermes-docs ~/.hermes/skills/hermes-docs
```

(or the profile path `~/.hermes/profiles/<name>/skills/hermes-docs`).

## Contract

[CONTRACT.md](CONTRACT.md) is the full spec for CLI flags, output templates, JSON shape, exit codes, docs-root resolution, and skill behavior.
