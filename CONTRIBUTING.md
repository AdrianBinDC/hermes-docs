# Contributing to hermes-docs

Thanks for taking a look. This is an **unofficial documentation companion** for [Hermes Agent](https://github.com/NousResearch/hermes-agent) — not a Nous Research project — and I’m glad you’re here.

If you’ve hit a query that wanders, a skill rule that misfires, or a retrieval gap in the docs index, a PR or issue is welcome. Small, focused changes are easiest to review.

## What helps most

- **Retrieval quality** — queries that should hit a canonical page but don’t (or that return noisy junk)
- **Skill behavior** — conflict handling, off-topic refusal, literal quoting from chunk `body` text
- **Docs & examples** — README clarity, grading queries, install notes
- **Tests** — especially around `src/engine.rs`; that’s where the BM25 pipeline lives and where CI watches coverage

Bug reports with a **question you asked**, what you **expected**, and what you **got** (paths, `signals`, or a short transcript) are gold. No need for a novel.

## Before you open a PR

1. Skim [CONTRACT.md](CONTRACT.md) for CLI flags, JSON shape, exit codes, and skill rules. Don’t invent behavior the contract doesn’t specify.
2. Run the checks CI runs:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo llvm-cov --fail-under-lines 85 --ignore-filename-regex 'main\.rs' --summary-only
   ```

3. Keep the diff tight. Fix the thing; don’t restyle the neighborhood (see [AGENTS.md](AGENTS.md) for lint/diff hygiene).

## Scope and expectations

- **Hermes Agent itself** — bugs or doc gaps in the upstream project belong in [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent). This repo only indexes whatever docs tree you point it at.
- **Breaking CLI or JSON changes** — need a CONTRACT.md update in the same PR.
- **Skill changes** — update [skills/hermes-docs/SKILL.md](skills/hermes-docs/SKILL.md) when agent-facing behavior changes.

I’ll do my best to respond to issues and PRs in a reasonable window. If you’re unsure whether something fits, open an issue first — that’s fine.

## License

By contributing, you agree that your contributions are licensed under the same [MIT License](LICENSE) as the rest of the project.
