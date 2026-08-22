---
name: hermes-docs
description: >
  Answers questions about Hermes Agent configuration, how Hermes works,
  environment variables, messaging, toolsets, integrations, and provider setup
  by retrieving grounded markdown from the local Hermes Agent docs tree.
  Use this skill whenever the user asks about Hermes config, env vars,
  toolsets, providers, messaging, or any "how does Hermes do X" question.
---

# hermes-docs

Retrieve Hermes Agent documentation as structured JSON, then answer from it.

## When to use

- User asks how to configure something in Hermes (config.yaml, .env, flags)
- User asks about Hermes environment variables
- User asks how Hermes works internally (toolsets, messaging, providers, integrations)
- User asks about a specific Hermes feature or CLI behavior

## Procedure

Run exactly ONE command from the skill directory:

```bash
sh scripts/hermes-docs --json "<user's question>"
```

The wrapper is a **shell script** (not Node). Do not run it with `node`.
Do not pass `-8` — that is a common typo; the wrapper maps `-8` → `-k 3`.
When `--json` is set, the wrapper injects `-k 3` automatically if you omit
`-k` (per category). Do not add `-k` unless you intentionally want more hits.

Rules:

1. **One invocation only.** Do not rephrase and re-run.
2. **Answer from the JSON only.** Use `signals` and `categories` as data.
   Cite `path` + `url` for every claim. Do not invent config keys.
3. **Conflicting setup snippets — ask, do not merge (hard stop).** When hits
   disagree on the same install/config step (different files, keys, nesting,
   or file targets — e.g. `~/.hermes/.env` vs `config.yaml` `platforms.telegram`),
   **do not** pick one, stitch them, or invent a combined answer. Reply with a
   short clarification only:

   - Name the conflict in one sentence.
   - List each option with `path` + `url` and a one-line difference (quote the
     conflicting literals from `body` exactly).
   - Ask which source to follow.
   - **STOP.** Wait for the user. After they choose, answer from that source
     only (still from the JSON you already have; do not re-run unless they
     ask a new question).

   Prefer this over a confident wrong answer. The user is in the loop.

   Example shape (Telegram often triggers this):

   > The docs show two Telegram setup patterns. Which should I follow?
   > 1. `user-guide/messaging/telegram.md` — `hermes gateway setup` or
   >    `TELEGRAM_BOT_TOKEN` / `TELEGRAM_ALLOWED_USERS` in `~/.hermes/.env`
   > 2. `guides/local-ollama-setup.md` — `platforms.telegram` token in
   >    `config.yaml`
4. **Quote technical literals exactly** as they appear in chunk `body` text.
   Do not rename, abbreviate, or paraphrase:
   - `config.yaml` keys, nesting, and example values
   - `.env` variable names (e.g. `TELEGRAM_BOT_TOKEN`, not "the telegram token")
   - CLI commands and flags (e.g. `hermes gateway setup`, not "run the setup wizard")
   - File paths the doc shows (e.g. `~/.hermes/.env`, not "your env file")
   - YAML/JSON/code blocks — preserve spelling, casing, and structure

   If a literal is not in the retrieved chunks, say so. Do not guess from memory.

   Example — chunk says `TELEGRAM_ALLOWED_USERS=123456789`. Your answer uses
   that exact name and shape, not "allowed user IDs in config".
5. **Off-topic refusal (hard stop).** When `signals.on_topic` is false **or**
   `categories` is empty, reply with **one sentence only** — e.g. "The Hermes
   docs don't cover this." — then **STOP**. No humor, jokes, guesses, numbers,
   "vibes", meta commentary about the question, or explaining why it is
   nonsensical. Do not fabricate any answer, including playful ones. Do not
   search elsewhere.
6. **Never fall back** to `rg`, `grep`, `search_files`, `glob`, `read_file`, or
   opening docs under `~/.hermes/hermes-agent/website/docs` — even when JSON
   chunks look incomplete. Answer from JSON only; say what is missing.
   Do not read `~/.hermes/config.yaml` or fetch the docs website.

## JSON the script prints

```json
{
  "query": "...",
  "docs_path": "...",
  "signals": {
    "max_bm25": 0.0,
    "hit_count": 0,
    "query_tokens": [],
    "content_tokens": [],
    "matched_content_tokens": [],
    "token_coverage": 0.0,
    "on_topic": false
  },
  "categories": [
    {
      "category": "user-guide",
      "hits": [
        {
          "rank": 1,
          "category": "user-guide",
          "path": "...",
          "heading": "...",
          "url": "...",
          "score": 0.0,
          "body": "..."
        }
      ]
    }
  ]
}
```

`score` is raw BM25. Categories are top-level docs folders (`user-guide`,
`guides`, `reference`, …).

## Exit codes

- 0 — success (including off-topic / zero hits)
- 1 — docs path missing or unreadable
- 2 — internal error
