# hermes-docs

A press-button Hermes skill that retrieves grounded markdown from the local
Hermes Agent docs tree via a Rust BM25 CLI (`hermes-docs-search`).

Install the skill into Hermes:

```bash
cp -r skills/hermes-docs ~/.hermes/skills/
```

The first query uses a cached binary when present. Without one, the wrapper builds
from source if `cargo` and this repo are available. To download a release instead,
set `HERMES_DOCS_RELEASE` to the GitHub release asset base URL (for example
`https://github.com/AdrianBinDC/hermes-docs/releases/download/v0.1.0`).
Subsequent queries are instant once the binary is in cache.

No Docker required.
