# Jester

Reverse proxy & pluggable edge runtime written in Rust.

## Project Layout

```
jester/
├─ Cargo.toml                   # Workspace definition
├─ design/                      # High-level design docs
├─ crates/
│  ├─ jester-core/             # Library with config, plugin traits, proxy scaffolding
│  ├─ jester-cli/              # Binary crate housing CLI entry points
│  └─ jester-plugin-sdk/       # Shared SDK for plugin authors (Rust + future WIT bindings)
├─ examples/
│  └─ config/                  # Example configuration files
└─ plugins/
   └─ examples/                # Sample plugin placeholders
```

See `design/master-design.md` for the full architecture overview.

## Getting Started

```bash
cargo fmt
cargo check
```

Run the CLI help to explore available commands:

```bash
cargo run -p jester-cli -- --help
```

Key workflows:
- `cargo run -p jester-cli -- run --config path/to/config.toml` — boot the TLS listener and proxy traffic.
- `cargo run -p jester-cli -- config validate path/to/config.toml` — schema + semantic validation.
- `cargo run -p jester-cli -- plugins list --dir plugins` — enumerate plugin manifests (Tier B/C scaffolding).
- `cargo run -p jester-cli -- diag --config path/to/config.toml` — print the resolved config as JSON.

See `DEVELOPMENT_NOTES.md` for local TLS setup tips, testing guidance, and the v0.0.1 release checklist.
