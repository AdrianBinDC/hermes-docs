# AGENTS

hermes-docs: a BM25 docs-search CLI (`hermes-docs-search`) and a Hermes skill that retrieves grounded markdown from the local Hermes Agent docs tree.

CONTRACT.md is the source of truth for CLI flags, output formats, and skill behavior. Do not invent requirements beyond what CONTRACT.md specifies.

## Lint / diff hygiene

When fixing Clippy or rustfmt issues:

- Prefer **minimal diffs**: fix the lint; do not restyle neighboring code or rewrite for taste.
- Do not expand `matches!` / match arms onto one-item-per-line for formatting alone.
- Prefer a **targeted allow with a one-line why** (inline `#[allow(clippy::…)]`, or a project allow in `Cargo.toml` `[lints.clippy]`) over absurd casts, clamps, or micro-refactors when the lint is pedantic noise with no realistic failure mode.
- Do not claim "pedantic clean" while silencing lints without documenting the allow.
- Machine truth: `clippy.toml`, `Cargo.toml` `[lints.clippy]`, and CI
  (`cargo clippy --all-targets -- -D warnings`). Do not invent a stricter
  pedantic policy than those files encode; do not add `-W clippy::pedantic` on
  the CLI in a way that overrides documented `[lints.clippy]` allows.
