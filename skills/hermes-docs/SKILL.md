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

Retrieve grounded Hermes Agent documentation and answer strictly from the
retrieved content.

## When to use

- User asks how to configure something in Hermes (config.yaml, .env, flags)
- User asks about Hermes environment variables
- User asks how Hermes works internally (toolsets, messaging, providers, integrations)
- User asks about a specific Hermes feature or CLI behavior

## Procedure

Run exactly ONE command, passing the user's question as the query:

```bash
scripts/hermes-docs "<user's question>"
```

Rules:

1. **One invocation only.** Do not run the script multiple times with
   rephrased queries. Do not refine the query in a loop.
2. **Answer only from stdout.** The script prints ranked markdown chunks with
   paths, URLs, and scores. Cite the file path and URL for every claim.
3. **Never invent config.** If retrieval returns no hits or the chunks do not
   contain the answer, say so explicitly. Do not fabricate config keys, env
   vars, or defaults.
4. **Never fall back to other search tools.** Do not use `rg`, `grep`,
   `search_files`, `glob`, or any file-reading tool to look up docs. Do not
   fetch the docs website. The script is the only retrieval path.

## Output format the script produces

```markdown
# Hermes docs search: <query>

## 1. <relpath> — <heading>
URL: https://hermes-agent.nousresearch.com/docs/<url-path>
Score: <score>

<body>

## 2. ...
```

Zero hits prints `_No hits._` and exits 0.

## Exit codes

- 0 — success (including zero hits)
- 1 — docs path missing or unreadable
- 2 — internal error
