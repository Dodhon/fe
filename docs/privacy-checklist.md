# Privacy Checklist

Use this checklist before changing repository visibility or publishing a release artifact.

## Allowlist

The public package may include:

- Rust source code.
- Cargo metadata.
- CI workflow files.
- README and documentation.
- Test code with synthetic input only.

## Denylist

The public package must not include:

- QMD index files or SQLite databases.
- Cache files or retrieval command outputs.
- Embeddings, document corpora, memory notes, research archives, transcripts, or private source files.
- API keys, tokens, cookies, auth headers, private URLs, or account identifiers.
- Machine-local absolute paths.

## Required Checks

Run these from the repository root:

```bash
git status --short --branch
git ls-files
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Then run a targeted scan over tracked files for private paths, credentials, and data artifacts. Treat any match as blocking unless it is inside this checklist as a generic warning term.

## Release Gate

Visibility can change only after the source tree, tracked files, and git history have been reviewed. If a secret was ever committed, rotate it before publishing.
