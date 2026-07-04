# fe

`fe` is a small Rust command-line wrapper that caches read-only `qmd` retrieval commands.

It stores command output in a local cache keyed by the requested QMD arguments, the selected QMD index fingerprint, and relevant environment settings. It does not ship any QMD indexes, document corpora, memory files, embeddings, or cached retrieval outputs.

## What It Does

- `fe "<query>"` runs `qmd query --no-rerank "<query>"`.
- `fe search "<query>"` runs cached `qmd search`.
- `fe query "<query>"` runs cached full `qmd query`.
- `fe vsearch "<query>"` runs cached vector search.
- `fe qmd ...` passes through directly to `qmd`.
- `fe --refresh <mode> ...` bypasses the read cache and stores fresh successful output.
- `fe --no-cache <mode> ...` bypasses both cache reads and cache writes.

## Privacy Boundary

This repository is source code only.

Do not commit:

- QMD indexes, SQLite databases, embeddings, or cache directories.
- Daily memory notes, research corpora, private documents, transcripts, or retrieval outputs.
- `.env` files, API keys, tokens, credentials, or machine-local configuration.

The runtime cache defaults to the user's local cache directory under `qmd/fe-cache-v2`. `--cache-stats` prints that local cache path for the current machine.

## Requirements

- Rust toolchain with Cargo.
- `qmd` available on `PATH` for retrieval commands.
SQLite is bundled via `rusqlite`, so no external `sqlite3` binary is needed. If the index cannot be read, `fe` falls back to file metadata fingerprinting.

## Build

```bash
cargo build --release
```

The binary is written to `target/release/fe`.

## Validate

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Install Locally

```bash
cargo install --path .
```

## Publishing Checklist

Before making this repository public:

1. Confirm the git history contains only source/docs/config intended for public release.
2. Run a secret and local-path scan over tracked files.
3. Confirm `.gitignore` excludes local caches, databases, indexes, and environment files.
4. Decide on a license.
5. Confirm README examples do not reference private collection names, private paths, or private data.
