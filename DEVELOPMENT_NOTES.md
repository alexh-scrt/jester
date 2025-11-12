# Development Notes

## Prerequisites
- Rust 1.75+ (use `rustup toolchain install stable`).
- `cargo` with network access for the first build (consider `cargo vendor` when working offline).
- TLS certificates for local testing (self-signed is fine).

## Project Layout (quick refresher)
- `crates/jester-core`: proxy runtime, configuration, router, TLS listener, metrics hooks.
- `crates/jester-cli`: developer CLI (`run`, `config`, `plugins`, `diag`, `tap` placeholder).
- `crates/jester-plugin-sdk`: manifests + WIT surface for plugins.
- `design/`: design docs (`master-design.md`, `v0.0.1-plan.md`).
- `examples/`: sample configs; `examples/config/minimal.jester.toml` is referenced by the CLI.

## Running the Proxy
1. Generate dev certificates (example using `mkcert`):
   ```bash
   mkcert -key-file certs/dev.key -cert-file certs/dev.crt localhost 127.0.0.1
   ```
2. Update `examples/config/minimal.jester.toml` to point to your cert/key paths (or create a new config file).
3. Launch:
   ```bash
   cargo run -p jester-cli -- run --config config/dev-config.toml --log-level debug
   ```
4. Hit the listener with any TLS client (`curl https://localhost:443 --resolve example.com:443:127.0.0.1` etc.).

### Config Helpers
- `cargo run -p jester-cli -- config validate path/to/config.toml`
- `cargo run -p jester-cli -- config example`
- `cargo run -p jester-cli -- diag --config path/to/config.toml`

Environment variables can be embedded inside configs using `${VAR:DEFAULT}` syntax; interpolation happens before parsing.

## Plugin Discovery (placeholder)
Place plugin manifests under `plugins/` (JSON files matching `PluginManifest`). List them with:
```bash
cargo run -p jester-cli -- plugins list --dir plugins
```
The runtime currently only logs the manifest discovery; loading/executing plugins is future work.

## Observability
- Logs default to INFO; use `--log-level trace` when debugging.
- Metrics are exported to logs through `metrics-exporter-log` with the target `jester::metrics`.
- `jester tap --route <name>` is a placeholder; it explains how to tail logs manually for now.

## Testing
- `cargo fmt` and `cargo clippy --all-targets` keep style in check.
- `cargo test` runs unit + integration tests (router matcher, config validation, etc.). TLS tests rely on generated fixtures; point `CERT_PATH`/`KEY_PATH` env vars in your tests if needed.
- If crates.io access is restricted, run `cargo vendor` and set `CARGO_HOME`/`.cargo/config.toml` accordingly.

## Release Checklist for v0.0.1
1. Ensure TLS listener boots and proxies to a local upstream.
2. Config validation fails loudly on missing listeners/routes.
3. CLI commands documented above behave as described.
4. Update `design/v0.0.1-plan.md` status if tasks are completed.
5. Tag `v0.0.1` after CI (`fmt`, `clippy`, `test`) is green.
