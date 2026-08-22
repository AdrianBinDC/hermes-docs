# hermes-docs

A press-button Hermes skill that retrieves grounded markdown from the local
Hermes Agent docs tree via a Rust BM25 CLI (`hermes-docs-search`).

Install the skill into Hermes:

```bash
cp -r skills/hermes-docs ~/.hermes/skills/
```

The first query may build or fetch the binary; subsequent queries are instant.

No Docker required.
