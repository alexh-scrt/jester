# Plugin Examples

This directory will house reference plugins spanning all three execution tiers:

1. **Tier A** — in-process Rust filters compiled directly into the Jester binary.
2. **Tier B** — WASI modules using `jester-plugin-sdk` + the `wit` interface at `crates/jester-plugin-sdk/wit/http.wit`.
3. **Tier C** — ABI-stable dynamic libraries gated behind `--allow-unsafe-dylib`.

For now the folder serves as a placeholder so downstream contributors know where to add sample code.
